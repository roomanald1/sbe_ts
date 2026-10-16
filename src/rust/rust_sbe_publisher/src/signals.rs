use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, VecDeque}, path::PathBuf, sync::Arc};
use axum::{Json, Router, http::StatusCode, routing::get};
use tokio::{fs::OpenOptions, io::{AsyncBufReadExt, AsyncWriteExt, BufReader}, sync::Mutex};

const PRICE_WINDOW_MS: u64 = 310_000;
const TRADE_WINDOW_MS: u64 = 60_000;
const MIN_SNAPSHOT_INTERVAL_MS: u64 = 15_000;
const MAX_BOOK_AGE_MS: u64 = 5_000;
const MAX_PRICE_LOOKBACK_GAP_MS: u64 = 5_000;
const WALK_FORWARD_MIN_TRAINING_SAMPLES: usize = 30;
const WALK_FORWARD_MIN_THRESHOLD_SAMPLES: usize = 10;
const WALK_FORWARD_TRAINING_WINDOW: usize = 200;
const STRENGTH_THRESHOLDS: [f64; 8] = [0.0, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5];

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalSnapshot {
    pub symbol: String,
    pub timestamp: u64,
    pub direction: String,
    pub score: f64,
    pub strength: f64,
    pub components: SignalComponents,
    pub reasons: Vec<String>,
    pub stale: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalComponents {
    pub trend_1m_percent: Option<f64>,
    pub momentum_10s_percent: Option<f64>,
    pub trade_flow_imbalance: Option<f64>,
    pub book_imbalance: Option<f64>,
    pub volatility_1m_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_volatility_units: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalOutcome {
    pub record_type: String,
    pub symbol: String,
    pub signal_timestamp: u64,
    #[serde(default)]
    pub evaluated_timestamp: u64,
    pub horizon_seconds: u64,
    pub return_percent: f64,
    pub direction_match: bool,
    #[serde(default)] pub direction: String,
    #[serde(default)] pub strength: f64,
    #[serde(default)] pub net_return_percent: Option<f64>,
    #[serde(default)] pub execution_cost_percent: Option<f64>,
    #[serde(default)] pub trend_5m_volatility_units: Option<f64>,
    #[serde(default)] pub trend_5m_direction_match: Option<bool>,
    #[serde(default)] pub trend_5m_net_return_percent: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SignalPerformance {
    pub symbol: String,
    pub horizon_seconds: u64,
    pub sample_count: usize,
    pub directional_sample_count: usize,
    pub neutral_sample_count: usize,
    pub bullish_match_rate: Option<f64>,
    pub bearish_match_rate: Option<f64>,
    pub direction_match_rate: Option<f64>,
    pub average_return_percent: f64,
    pub net_sample_count: usize,
    pub average_net_return_percent: Option<f64>,
}
#[derive(Clone, Debug, Serialize)]
pub struct SignalWalkForward {
    pub symbol: String,
    pub horizon_seconds: u64,
    pub training_samples: usize,
    pub evaluated_signals: usize,
    pub selected_signals: usize,
    pub selected_strength_threshold: Option<f64>,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    pub baseline_direction_match_rate: Option<f64>,
    pub baseline_average_net_return_percent: Option<f64>,
    pub five_minute_trend_sample_count: usize,
    pub five_minute_trend_match_rate: Option<f64>,
    pub five_minute_trend_average_net_return_percent: Option<f64>,
}
#[derive(Clone, Debug)]
struct PendingOutcome {
    symbol: String,
    timestamp: u64,
    base_price: f64,
    direction: String,
    strength: f64,
    horizon_seconds: u64,
    entry_bid: Option<f64>,
    entry_ask: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
}
#[derive(Default)]
struct PerformanceTotals {
    sample_count: usize,
    directional_count: usize,
    neutral_count: usize,
    bullish_count: usize,
    bullish_matches: usize,
    bearish_count: usize,
    bearish_matches: usize,
    directional_matches: usize,
    raw_return_total: f64,
    net_count: usize,
    net_return_total: f64,
}

#[derive(Default)]
pub struct SignalEngine {
    prices: VecDeque<(u64, f64)>,
    pending: VecDeque<PendingOutcome>,
    trades: VecDeque<(u64, f64, bool)>,
    book_imbalance: Option<f64>,
    best_quote: Option<(u64, f64, f64)>,
    volatility_1m: Option<f64>,
    last_snapshot_at: u64,
}

fn directional_net_return(direction: &str, entry_price: f64, exit_price: f64, fee_rate: f64) -> Option<f64> {
    if !entry_price.is_finite() || !exit_price.is_finite() || entry_price <= 0.0 || exit_price <= 0.0 { return None; }
    let gross_return = match direction {
        "bullish" => (exit_price / entry_price - 1.0) * 100.0,
        "bearish" => (entry_price / exit_price - 1.0) * 100.0,
        _ => return None,
    };
    Some(gross_return - 2.0 * fee_rate * 100.0)
}

fn configured_signal_taker_fee_rate() -> f64 {
    let configured = std::env::var("SIGNALS_TAKER_FEE_RATE")
        .or_else(|_| std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE"))
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite())
        .map(f64::abs)
        .unwrap_or(0.001);
    configured.clamp(0.0, 0.02)
}

impl SignalEngine {
    pub fn on_price(&mut self, timestamp: u64, price: f64, volatility_1m: f64) -> Vec<SignalOutcome> {
        if !price.is_finite() || price <= 0.0 { return Vec::new(); }
        if self.prices.back().is_some_and(|sample| timestamp < sample.0) { return Vec::new(); }
        self.prices.push_back((timestamp, price));
        self.volatility_1m = volatility_1m.is_finite().then_some(volatility_1m);
        self.prune(timestamp);
        let mut outcomes = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some(pending) = self.pending.pop_front() {
            if timestamp >= pending.timestamp.saturating_add(pending.horizon_seconds * 1000) {
                let change = (price / pending.base_price - 1.0) * 100.0;
                let direction_match = match pending.direction.as_str() { "bullish" => change > 0.0, "bearish" => change < 0.0, _ => false };
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
                let fee_rate = configured_signal_taker_fee_rate();
                let net_return_percent = match (pending.direction.as_str(), pending.entry_bid, pending.entry_ask, exit_quote) {
                    ("bullish", _, Some(entry_ask), Some((_, exit_bid, _))) => directional_net_return("bullish", entry_ask, exit_bid, fee_rate),
                    ("bearish", Some(entry_bid), _, Some((_, _, exit_ask))) => directional_net_return("bearish", entry_bid, exit_ask, fee_rate),
                    _ => None,
                };
                let gross_directional_return = match pending.direction.as_str() { "bullish" => Some(change), "bearish" => Some(-change), _ => None };
                let execution_cost_percent = gross_directional_return.zip(net_return_percent).map(|(gross, net)| gross - net);
                let trend_5m_direction = pending.trend_5m_volatility_units.filter(|value| value.abs() >= 0.25).map(|value| if value > 0.0 { "bullish" } else { "bearish" });
                let trend_5m_direction_match = trend_5m_direction.map(|direction| if direction == "bullish" { change > 0.0 } else { change < 0.0 });
                let trend_5m_net_return_percent = match (trend_5m_direction, pending.entry_bid, pending.entry_ask, exit_quote) {
                    (Some("bullish"), _, Some(entry_ask), Some((_, exit_bid, _))) => directional_net_return("bullish", entry_ask, exit_bid, fee_rate),
                    (Some("bearish"), Some(entry_bid), _, Some((_, _, exit_ask))) => directional_net_return("bearish", entry_bid, exit_ask, fee_rate),
                    _ => None,
                };
                outcomes.push(SignalOutcome { record_type: "outcome".into(), symbol: pending.symbol, signal_timestamp: pending.timestamp, evaluated_timestamp: timestamp, horizon_seconds: pending.horizon_seconds, return_percent: change, direction_match, direction: pending.direction, strength: pending.strength, net_return_percent, execution_cost_percent, trend_5m_volatility_units: pending.trend_5m_volatility_units, trend_5m_direction_match, trend_5m_net_return_percent });
            } else { remaining.push_back(pending); }
        }
        self.pending = remaining;
        outcomes
    }
    pub fn track(&mut self, snapshot: &SignalSnapshot) {
        if let Some((_, base_price)) = self.prices.back().copied() {
            let quote = self.best_quote.filter(|(quote_time, _, _)| snapshot.timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
            // Keep short-term checks while adding swing-style horizons. Outcomes are
            // evaluated from backend price/book data and persisted with the same cost model.
            for horizon_seconds in [60, 300, 900, 3_600] {
                self.pending.push_back(PendingOutcome {
                    symbol: snapshot.symbol.clone(), timestamp: snapshot.timestamp, base_price,
                    direction: snapshot.direction.clone(), strength: snapshot.strength, horizon_seconds,
                    entry_bid: quote.map(|(_, bid, _)| bid), entry_ask: quote.map(|(_, _, ask)| ask),
                    trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                });
            }
        }
    }

    pub fn on_trade(&mut self, timestamp: u64, price: f64, size: f64, is_buy: bool) {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 { return; }
        self.trades.push_back((timestamp, price * size, is_buy));
        self.prune(timestamp);
    }

    pub fn on_book(&mut self, timestamp: u64, best_bid: f64, best_ask: f64, bid_sizes: &[f64], ask_sizes: &[f64]) {
        if best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid {
            self.best_quote = Some((timestamp, best_bid, best_ask));
        }
        let bid: f64 = bid_sizes.iter().copied().filter(|n| n.is_finite() && *n > 0.0).sum();
        let ask: f64 = ask_sizes.iter().copied().filter(|n| n.is_finite() && *n > 0.0).sum();
        if bid + ask > 0.0 { self.book_imbalance = Some((bid - ask) / (bid + ask)); }
    }

    pub fn snapshot(&mut self, symbol: &str, now: u64) -> Option<SignalSnapshot> {
        if now.saturating_sub(self.last_snapshot_at) < MIN_SNAPSHOT_INTERVAL_MS { return None; }
        self.prune(now);
        let latest = self.prices.back().copied()?;
        let trend = self.price_return(now.saturating_sub(60_000));
        let momentum = self.price_return(now.saturating_sub(10_000));
        let trend_5m = self.price_return(now.saturating_sub(300_000));
        let volatility_5m = self.realized_volatility(now.saturating_sub(300_000));
        let trend_5m_volatility_units = trend_5m.zip(volatility_5m).and_then(|(trend, volatility)| (volatility > 1e-9).then_some(trend / volatility));
        let flow = {
            let (buy, sell) = self.trades.iter().fold((0.0, 0.0), |(buy, sell), (_, value, is_buy)|
                if *is_buy { (buy + value, sell) } else { (buy, sell + value) });
            (buy + sell > 0.0).then_some((buy - sell) / (buy + sell))
        };
        let components = SignalComponents {
            trend_1m_percent: trend,
            momentum_10s_percent: momentum,
            trade_flow_imbalance: flow,
            book_imbalance: self.book_imbalance,
            volatility_1m_percent: self.volatility_1m.or_else(|| self.realized_volatility(now.saturating_sub(60_000))),
            trend_5m_percent: trend_5m,
            trend_5m_volatility_units,
        };
        let inputs = [trend.map(|v| (v / 0.25).clamp(-1.0, 1.0)), momentum.map(|v| (v / 0.12).clamp(-1.0, 1.0)), flow, self.book_imbalance];
        let count = inputs.iter().filter(|v| v.is_some()).count();
        if count == 0 { return None; }
        let score = inputs.iter().flatten().sum::<f64>() / count as f64;
        let direction = if score > 0.2 { "bullish" } else if score < -0.2 { "bearish" } else { "neutral" };
        let mut reasons = Vec::new();
        if let Some(v) = trend { reasons.push(format!("1m price change {v:.3}%")); }
        if let Some(v) = momentum { reasons.push(format!("10s price change {v:.3}%")); }
        if let Some(v) = flow { reasons.push(format!("buy/sell trade imbalance {v:.2}")); }
        if let Some(v) = self.book_imbalance { reasons.push(format!("top-book imbalance {v:.2}")); }
        self.last_snapshot_at = now;
        Some(SignalSnapshot { symbol: symbol.to_owned(), timestamp: now, direction: direction.to_owned(), score, strength: (score.abs() * count as f64 / 4.0).clamp(0.0, 1.0), components, reasons, stale: now.saturating_sub(latest.0) > 5_000 })
    }

    fn realized_volatility(&self, cutoff: u64) -> Option<f64> {
        let samples = self.prices.iter().filter(|sample| sample.0 >= cutoff).collect::<Vec<_>>();
        if samples.len() < 2 { return None; }
        let sum = samples.windows(2).map(|pair| {
            let change = (pair[1].1 / pair[0].1).ln();
            change * change
        }).sum::<f64>();
        Some(sum.sqrt() * 100.0)
    }

    fn price_return(&self, cutoff: u64) -> Option<f64> {
        let latest = self.prices.back()?.1;
        let (sample_time, first) = self.prices.iter().rev().find(|sample| sample.0 <= cutoff).copied()?;
        if cutoff.saturating_sub(sample_time) > MAX_PRICE_LOOKBACK_GAP_MS { return None; }
        Some((latest / first - 1.0) * 100.0)
    }

    fn prune(&mut self, now: u64) {
        while self.prices.front().is_some_and(|sample| sample.0 < now.saturating_sub(PRICE_WINDOW_MS)) { self.prices.pop_front(); }
        while self.trades.front().is_some_and(|sample| sample.0 < now.saturating_sub(TRADE_WINDOW_MS)) { self.trades.pop_front(); }
    }
}

fn select_strength_threshold(training: &[SignalOutcome]) -> Option<f64> {
    STRENGTH_THRESHOLDS.iter().copied().filter_map(|threshold| {
        let selected = training.iter().filter(|sample| sample.strength >= threshold).collect::<Vec<_>>();
        if selected.len() < WALK_FORWARD_MIN_THRESHOLD_SAMPLES { return None; }
        let mean = selected.iter().filter_map(|sample| sample.net_return_percent).sum::<f64>() / selected.len() as f64;
        (mean > 0.0).then_some((threshold, mean))
    }).max_by(|left, right| left.1.total_cmp(&right.1).then(left.0.total_cmp(&right.0))).map(|(threshold, _)| threshold)
}

#[derive(Clone)]
pub struct SignalStore { path: PathBuf, write_lock: Arc<Mutex<()>> }

impl SignalStore {
    pub fn from_environment() -> Result<Self> {
        let path = std::env::var_os("SIGNALS_FILE").map(PathBuf::from).unwrap_or(std::env::current_dir()?.join("signals.jsonl"));
        Ok(Self { path, write_lock: Arc::new(Mutex::new(())) })
    }

    pub fn router(self) -> Router {
        let history_store = self.clone();
        let performance_store = self.clone();
        let evaluation_store = self.clone();
        let best_store = self;
        Router::new()
            .route("/signals/history", get(move || {
                let store = history_store.clone();
                async move { match store.recent(200).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/performance", get(move || {
                let store = performance_store.clone();
                async move { match store.performance().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/best", get(move || {
                let store = best_store.clone();
                async move { match store.best(10).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/walk-forward", get(move || {
                let store = evaluation_store.clone();
                async move { match store.walk_forward().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
    }

    pub async fn recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut lines = BufReader::new(file).lines();
        let mut items = Vec::new();
        while let Some(line) = lines.next_line().await? {
            if let Ok(snapshot) = serde_json::from_str::<SignalSnapshot>(&line) { items.push(snapshot); }
        }
        let start = items.len().saturating_sub(limit);
        Ok(items.drain(start..).collect())
    }

    pub async fn best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let file = match tokio::fs::File::open(&self.path).await { Ok(file) => file, Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()), Err(error) => return Err(error.into()) };
        let mut lines = BufReader::new(file).lines();
        let mut latest: HashMap<String, SignalSnapshot> = HashMap::new();
        while let Some(line) = lines.next_line().await? {
            let Ok(snapshot) = serde_json::from_str::<SignalSnapshot>(&line) else { continue; };
            if snapshot.stale { continue; }
            let replace = latest.get(&snapshot.symbol).is_none_or(|current| current.timestamp < snapshot.timestamp);
            if replace { latest.insert(snapshot.symbol.clone(), snapshot); }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let mut items = latest.into_values().filter(|item| now.saturating_sub(item.timestamp) <= 60_000).collect::<Vec<_>>();
        items.sort_by(|a, b| b.strength.total_cmp(&a.strength).then_with(|| b.timestamp.cmp(&a.timestamp)));
        items.truncate(limit);
        Ok(items)
    }

    pub async fn performance(&self) -> Result<Vec<SignalPerformance>> {
        let outcomes = self.read_outcomes().await?;
        let mut totals: HashMap<(String, u64), PerformanceTotals> = HashMap::new();
        for outcome in outcomes {
            let total = totals.entry((outcome.symbol.clone(), outcome.horizon_seconds)).or_default();
            total.sample_count += 1;
            total.raw_return_total += outcome.return_percent;
            match outcome.direction.as_str() {
                "bullish" => {
                    total.directional_count += 1;
                    total.bullish_count += 1;
                    if outcome.direction_match { total.bullish_matches += 1; total.directional_matches += 1; }
                }
                "bearish" => {
                    total.directional_count += 1;
                    total.bearish_count += 1;
                    if outcome.direction_match { total.bearish_matches += 1; total.directional_matches += 1; }
                }
                "neutral" => total.neutral_count += 1,
                _ => {},
            }
            if let Some(net_return) = outcome.net_return_percent {
                total.net_count += 1;
                total.net_return_total += net_return;
            }
        }
        let mut result = totals.into_iter().map(|((symbol, horizon_seconds), total)| SignalPerformance {
            symbol,
            horizon_seconds,
            sample_count: total.sample_count,
            directional_sample_count: total.directional_count,
            neutral_sample_count: total.neutral_count,
            bullish_match_rate: (total.bullish_count > 0).then_some(total.bullish_matches as f64 / total.bullish_count as f64),
            bearish_match_rate: (total.bearish_count > 0).then_some(total.bearish_matches as f64 / total.bearish_count as f64),
            direction_match_rate: (total.directional_count > 0).then_some(total.directional_matches as f64 / total.directional_count as f64),
            average_return_percent: total.raw_return_total / total.sample_count as f64,
            net_sample_count: total.net_count,
            average_net_return_percent: (total.net_count > 0).then_some(total.net_return_total / total.net_count as f64),
        }).collect::<Vec<_>>();
        result.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.horizon_seconds.cmp(&b.horizon_seconds)));
        Ok(result)
    }

    pub async fn walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let outcomes = self.read_outcomes().await?;
        let mut grouped: HashMap<(String, u64), Vec<SignalOutcome>> = HashMap::new();
        for outcome in outcomes {
            grouped.entry((outcome.symbol.clone(), outcome.horizon_seconds)).or_default().push(outcome);
        }
        let mut result = Vec::new();
        for ((symbol, horizon_seconds), mut samples) in grouped {
            samples.sort_by_key(|sample| sample.signal_timestamp);
            let mut non_overlapping = Vec::new();
            let mut next_timestamp = 0;
            for sample in samples {
                if sample.signal_timestamp < next_timestamp { continue; }
                next_timestamp = sample.evaluated_timestamp.max(sample.signal_timestamp.saturating_add(horizon_seconds.saturating_mul(1000)));
                if sample.net_return_percent.is_some() && matches!(sample.direction.as_str(), "bullish" | "bearish") {
                    non_overlapping.push(sample);
                }
            }
            if non_overlapping.len() <= WALK_FORWARD_MIN_TRAINING_SAMPLES { continue; }

            let mut max_training_samples = 0;
            let mut evaluated_signals = 0;
            let mut selected_signals = 0;
            let mut correct = 0;
            let mut net_total = 0.0;
            let mut positive_net = 0;
            let mut baseline_correct = 0;
            let mut baseline_net_total = 0.0;
            let mut five_minute_trend_samples = 0;
            let mut five_minute_trend_correct = 0;
            let mut five_minute_trend_net_total = 0.0;
            let mut last_threshold = None;
            for index in WALK_FORWARD_MIN_TRAINING_SAMPLES..non_overlapping.len() {
                let start = index.saturating_sub(WALK_FORWARD_TRAINING_WINDOW);
                let sample = &non_overlapping[index];
                let training = non_overlapping[start..index].iter().filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp).cloned().collect::<Vec<_>>();
                if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES { continue; }
                max_training_samples = max_training_samples.max(training.len());
                evaluated_signals += 1;
                if sample.direction_match { baseline_correct += 1; }
                baseline_net_total += sample.net_return_percent.unwrap_or_default();
                if let (Some(matches), Some(net_return)) = (sample.trend_5m_direction_match, sample.trend_5m_net_return_percent) {
                    five_minute_trend_samples += 1;
                    if matches { five_minute_trend_correct += 1; }
                    five_minute_trend_net_total += net_return;
                }
                let Some(threshold) = select_strength_threshold(&training) else { continue; };
                last_threshold = Some(threshold);
                if sample.strength < threshold { continue; }
                selected_signals += 1;
                if sample.direction_match { correct += 1; }
                let net = sample.net_return_percent.unwrap_or_default();
                net_total += net;
                if net > 0.0 { positive_net += 1; }
            }
            if evaluated_signals == 0 { continue; }
            result.push(SignalWalkForward {
                symbol,
                horizon_seconds,
                training_samples: max_training_samples,
                evaluated_signals,
                selected_signals,
                selected_strength_threshold: last_threshold,
                direction_match_rate: (selected_signals > 0).then_some(correct as f64 / selected_signals as f64),
                average_net_return_percent: (selected_signals > 0).then_some(net_total / selected_signals as f64),
                positive_net_rate: (selected_signals > 0).then_some(positive_net as f64 / selected_signals as f64),
                baseline_direction_match_rate: (evaluated_signals > 0).then_some(baseline_correct as f64 / evaluated_signals as f64),
                baseline_average_net_return_percent: (evaluated_signals > 0).then_some(baseline_net_total / evaluated_signals as f64),
                five_minute_trend_sample_count: five_minute_trend_samples,
                five_minute_trend_match_rate: (five_minute_trend_samples > 0).then_some(five_minute_trend_correct as f64 / five_minute_trend_samples as f64),
                five_minute_trend_average_net_return_percent: (five_minute_trend_samples > 0).then_some(five_minute_trend_net_total / five_minute_trend_samples as f64),
            });
        }
        result.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.horizon_seconds.cmp(&b.horizon_seconds)));
        Ok(result)
    }

    async fn read_outcomes(&self) -> Result<Vec<SignalOutcome>> {
        let file = match tokio::fs::File::open(&self.path).await {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut lines = BufReader::new(file).lines();
        let mut outcomes = Vec::new();
        while let Some(line) = lines.next_line().await? {
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else { continue; };
            if value.get("record_type").and_then(|value| value.as_str()) != Some("outcome") { continue; }
            if let Ok(outcome) = serde_json::from_value(value) { outcomes.push(outcome); }
        }
        Ok(outcomes)
    }

    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) { tokio::fs::create_dir_all(parent).await?; }
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path).await?;
        file.write_all(&serde_json::to_vec(snapshot)?).await?;
        file.write_all(b"\n").await?;
        file.flush().await?;
        Ok(())
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) { tokio::fs::create_dir_all(parent).await?; }
        let mut file = OpenOptions::new().create(true).append(true).open(&self.path).await?;
        file.write_all(&serde_json::to_vec(outcome)?).await?;
        file.write_all(b"
").await?;
        file.flush().await?;
        Ok(())
    }

}
