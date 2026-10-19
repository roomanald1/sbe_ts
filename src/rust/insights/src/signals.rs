use anyhow::Result;
use flate2::{read::MultiGzDecoder, write::GzEncoder, Compression};
use std::io::{Read, Write};
use serde::{Deserialize, Serialize};
use std::{collections::{HashMap, VecDeque}, path::PathBuf, sync::Arc, time::{Duration, Instant}};
use axum::{Json, Router, http::StatusCode, routing::get};
use tokio::{fs::OpenOptions, io::AsyncWriteExt, sync::Mutex};

const SIGNALS_BINARY_MAGIC: &[u8; 8] = b"SIGBIN01";

const PRICE_WINDOW_MS: u64 = 3_610_000;
const TRADE_WINDOW_MS: u64 = 60_000;
const MIN_SNAPSHOT_INTERVAL_MS: u64 = 15_000;
const MAX_BOOK_AGE_MS: u64 = 5_000;
const MAX_PRICE_LOOKBACK_GAP_MS: u64 = 5_000;
const WALK_FORWARD_MIN_TRAINING_SAMPLES: usize = 30;
const WALK_FORWARD_MIN_THRESHOLD_SAMPLES: usize = 10;
const WALK_FORWARD_TRAINING_WINDOW: usize = 200;
const WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL: usize = 100;
const TREND_LOGISTIC_CONFIDENCE_THRESHOLDS: [f64; 3] = [0.55, 0.60, 0.65];
const STRENGTH_THRESHOLDS: [f64; 8] = [0.0, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5];
const EXPLORATION_SCORE_THRESHOLDS: [f64; 5] = [0.1, 0.2, 0.3, 0.4, 0.5];

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
    /// Heuristic probability (0..1) that recent prices are trading in a range.
    #[serde(default)]
    pub ranging_probability: Option<f64>,
    /// Observed low/high over the five-minute analysis window.
    #[serde(default)]
    pub range_lower_bound: Option<f64>,
    #[serde(default)]
    pub range_upper_bound: Option<f64>,
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
    #[serde(default)]
    pub mispricing_vs_vwap_bps: Option<f64>,
    #[serde(default)]
    pub pressure_without_price_move: Option<f64>,
    #[serde(default)]
    pub aggressor_cost_bps: Option<f64>,
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
    #[serde(default)] pub features: Option<[f64; 4]>,
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
#[derive(Clone, Debug, Default, Serialize)]
pub struct SideWalkForwardMetrics {
    pub sample_count: usize,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    pub positive_net_rate_lower_95: Option<f64>,
}

#[derive(Default)]
struct SideWalkForwardAccumulator { samples: usize, correct: usize, positive_net: usize, net_total: f64 }
impl SideWalkForwardAccumulator {
    fn record(&mut self, correct: bool, net: f64) {
        self.samples += 1;
        if correct { self.correct += 1; }
        if net > 0.0 { self.positive_net += 1; }
        self.net_total += net;
    }
    fn metrics(&self) -> SideWalkForwardMetrics {
        let hit_rate = (self.samples > 0).then_some(self.correct as f64 / self.samples as f64);
        let positive_rate = (self.samples > 0).then_some(self.positive_net as f64 / self.samples as f64);
        SideWalkForwardMetrics {
            sample_count: self.samples,
            direction_match_rate: hit_rate,
            average_net_return_percent: (self.samples > 0).then_some(self.net_total / self.samples as f64),
            positive_net_rate: positive_rate,
            positive_net_rate_lower_95: wilson_lower_bound(self.positive_net, self.samples),
        }
    }
}

fn wilson_lower_bound(successes: usize, samples: usize) -> Option<f64> {
    if samples == 0 { return None; }
    let n = samples as f64;
    let z = 1.96_f64;
    let rate = successes as f64 / n;
    let denominator = 1.0 + z * z / n;
    let center = rate + z * z / (2.0 * n);
    let spread = z * ((rate * (1.0 - rate) / n + z * z / (4.0 * n * n)).max(0.0)).sqrt();
    Some(((center - spread) / denominator).clamp(0.0, 1.0))
}

#[derive(Clone, Debug, Serialize)]
pub struct ProbabilityConfidenceMetrics {
    pub threshold: f64,
    pub selected_signals: usize,
    pub direction_match_rate: Option<f64>,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
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
    pub logistic_direction_match_rate: Option<f64>,
    pub logistic_average_net_return_percent: Option<f64>,
    pub logistic_positive_net_rate: Option<f64>,
    pub logistic_selected_signals: usize,
    pub trend_logistic_direction_match_rate: Option<f64>,
    pub trend_logistic_average_net_return_percent: Option<f64>,
    pub trend_logistic_positive_net_rate: Option<f64>,
    pub trend_logistic_selected_signals: usize,
    pub contrarian_trend_logistic_direction_match_rate: Option<f64>,
    pub contrarian_trend_logistic_average_net_return_percent: Option<f64>,
    pub contrarian_trend_logistic_positive_net_rate: Option<f64>,
    pub contrarian_trend_logistic_selected_signals: usize,
    pub contrarian_trend_logistic_confidence_bands: Vec<ProbabilityConfidenceMetrics>,
    pub trend_logistic_confidence_bands: Vec<ProbabilityConfidenceMetrics>,
    /// Shadow results for the live Auto Trader exploration gate, using historical score and fee-adjusted execution cost.
    pub exploration_score_bands: Vec<ProbabilityConfidenceMetrics>,
    pub signal_long: SideWalkForwardMetrics,
    pub signal_short: SideWalkForwardMetrics,
    /// All directional outcomes before any learned strength selection.
    pub all_signal_long: SideWalkForwardMetrics,
    pub all_signal_short: SideWalkForwardMetrics,
    pub logistic_long: SideWalkForwardMetrics,
    pub logistic_short: SideWalkForwardMetrics,
}
/// Side-by-side executable outcome label. A new enum variant preserves the
/// existing bincode Snapshot and Outcome layouts for persisted records.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalResearchFeatures {
    pub symbol: String,
    pub timestamp: u64,
    pub spread_bps: Option<f64>,
    pub microprice_offset_bps: Option<f64>,
    pub bid_notional_10bps: Option<f64>,
    pub ask_notional_10bps: Option<f64>,
    pub depth_imbalance_10bps: Option<f64>,
    pub depth_imbalance_change_1s: Option<f64>,
    pub depth_imbalance_change_5s: Option<f64>,
    pub trade_flow_5s: Option<f64>,
    pub trade_flow_15s: Option<f64>,
    pub trade_flow_60s: Option<f64>,
    pub trade_flow_acceleration: Option<f64>,
    pub signed_trade_notional_5s: Option<f64>,
    pub signed_trade_notional_15s: Option<f64>,
    pub trade_count_5s: u64,
    pub trade_count_60s: u64,
    pub average_trade_notional_5s: Option<f64>,
    pub price_response_bps_per_1000usd_5s: Option<f64>,
    pub btc_return_1m_percent: Option<f64>,
    pub btc_return_5m_percent: Option<f64>,
    pub eth_return_1m_percent: Option<f64>,
    pub eth_return_5m_percent: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExternalMarketFeatures {
    pub underlying: String,
    pub timestamp: u64,
    pub swap_instrument: String,
    pub funding_rate: Option<f64>,
    pub next_funding_rate: Option<f64>,
    pub open_interest_contracts: Option<f64>,
    pub open_interest_currency: Option<f64>,
    pub open_interest_change_percent: Option<f64>,
    pub mark_price: Option<f64>,
    pub index_price: Option<f64>,
    pub perp_basis_bps: Option<f64>,
    pub options_atm_iv: Option<f64>,
    pub options_25d_skew: Option<f64>,
    pub options_expiry_timestamp: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalBookSnapshot {
    pub symbol: String,
    pub timestamp: u64,
    pub bid_levels: Vec<(f64, f64)>,
    pub ask_levels: Vec<(f64, f64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SignalEvaluation {
    pub symbol: String,
    pub signal_timestamp: u64,
    pub evaluated_timestamp: u64,
    pub horizon_seconds: u64,
    pub signal_direction: String,
    pub signal_score: f64,
    pub ranging_probability: Option<f64>,
    pub reference_entry_price: f64,
    pub reference_exit_price: f64,
    pub entry_bid: Option<f64>,
    pub entry_ask: Option<f64>,
    pub entry_quote_timestamp: Option<u64>,
    pub exit_bid: Option<f64>,
    pub exit_ask: Option<f64>,
    pub exit_quote_timestamp: Option<u64>,
    pub taker_fee_rate: f64,
    pub long_gross_percent: f64,
    pub short_gross_percent: f64,
    pub long_net_percent: Option<f64>,
    pub short_net_percent: Option<f64>,
    pub path_high: f64,
    pub path_high_timestamp: u64,
    pub path_low: f64,
    pub path_low_timestamp: u64,
}

#[derive(Clone, Debug)]
pub struct CompletedSignalOutcome {
    pub legacy: SignalOutcome,
    pub evaluation: SignalEvaluation,
}

#[derive(Clone, Debug)]
struct BookHistorySample {
    timestamp: u64,
    bid: f64,
    ask: f64,
    bid_levels: Vec<(f64, f64)>,
    ask_levels: Vec<(f64, f64)>,
}

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
    features: Option<[f64; 4]>,
    score: f64,
    ranging_probability: Option<f64>,
    entry_quote_timestamp: Option<u64>,
    taker_fee_rate: f64,
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
    trades: VecDeque<(u64, f64, f64, bool, Option<f64>)>,
    book_imbalance: Option<f64>,
    best_quote: Option<(u64, f64, f64)>,
    book_bid_levels: Vec<(f64, f64)>,
    book_ask_levels: Vec<(f64, f64)>,
    book_history: VecDeque<BookHistorySample>,
    volatility_1m: Option<f64>,
    last_snapshot_at: u64,
}

fn feature_vector(components: &SignalComponents) -> Option<[f64; 4]> {
    let values = [
        components.trend_1m_percent.map(|v| (v / 0.25).clamp(-4.0, 4.0)),
        components.momentum_10s_percent.map(|v| (v / 0.12).clamp(-4.0, 4.0)),
        components.trade_flow_imbalance,
        components.book_imbalance,
    ];
    values.iter().all(Option::is_some).then(|| values.map(Option::unwrap))
}

fn train_logistic(samples: &[SignalOutcome]) -> Option<[f64; 5]> {
    let training = samples.iter().filter_map(|sample| Some((sample.features?, (sample.return_percent > 0.0) as u8 as f64))).collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES { return None; }
    let mut weights = [0.0; 5];
    for _ in 0..250 {
        let mut gradient = [0.0; 5];
        for (features, target) in &training {
            let z = weights[0] + features.iter().enumerate().map(|(index, value)| weights[index + 1] * value).sum::<f64>();
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 { gradient[index + 1] += error * features[index]; }
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..5 { weights[index] -= scale * gradient[index]; }
    }
    Some(weights)
}

fn logistic_up_probability(weights: &[f64; 5], features: &[f64; 4]) -> f64 {
    let z = weights[0] + features.iter().enumerate().map(|(index, value)| weights[index + 1] * value).sum::<f64>();
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn train_trend_logistic(samples: &[SignalOutcome]) -> Option<[f64; 6]> {
    let training = samples.iter().filter_map(|sample| Some((sample.features?, sample.trend_5m_volatility_units?.clamp(-4.0, 4.0), (sample.return_percent > 0.0) as u8 as f64))).collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES { return None; }
    let mut weights = [0.0; 6];
    for _ in 0..250 {
        let mut gradient = [0.0; 6];
        for (features, trend, target) in &training {
            let z = weights[0]
                + features.iter().enumerate().map(|(index, value)| weights[index + 1] * value).sum::<f64>()
                + weights[5] * trend;
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 { gradient[index + 1] += error * features[index]; }
            gradient[5] += error * trend;
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..6 { weights[index] -= scale * gradient[index]; }
    }
    Some(weights)
}

fn trend_logistic_up_probability(weights: &[f64; 6], features: &[f64; 4], trend: f64) -> f64 {
    let z = weights[0]
        + features.iter().enumerate().map(|(index, value)| weights[index + 1] * value).sum::<f64>()
        + weights[5] * trend.clamp(-4.0, 4.0);
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn sample_microprice(bid: f64, bid_size: f64, ask: f64, ask_size: f64) -> f64 {
    (ask * bid_size + bid * ask_size) / (bid_size + ask_size)
}

fn directional_net_return(direction: &str, entry_price: f64, exit_price: f64, fee_rate: f64) -> Option<f64> {
    if !entry_price.is_finite() || !exit_price.is_finite() || entry_price <= 0.0 || exit_price <= 0.0 { return None; }
    let gross_return = match direction {
        "bullish" => (exit_price / entry_price - 1.0) * 100.0,
        "bearish" => ((entry_price - exit_price) / entry_price) * 100.0,
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

fn breakout_watch(components: &SignalComponents, range_bounds: Option<(f64, f64)>, range_probability: Option<f64>) -> Option<(&'static str, f64)> {
    if range_probability? < 0.65 { return None; }
    let (lower, upper) = range_bounds?;
    let factors = [
        (components.trend_1m_percent.map(|value| (value / 0.25).clamp(-1.0, 1.0)), 0.15),
        (components.momentum_10s_percent.map(|value| (value / 0.12).clamp(-1.0, 1.0)), 0.20),
        (components.trade_flow_imbalance, 0.25),
        (components.book_imbalance, 0.25),
        (components.pressure_without_price_move.map(|value| -value), 0.15),
    ];
    let (weighted_bias, available_weight) = factors.iter().filter_map(|(value, weight)| value.map(|value| (value * weight, *weight)))
        .fold((0.0, 0.0), |(bias, weight_sum), (value, weight)| (bias + value, weight_sum + weight));
    if available_weight == 0.0 { return None; }
    let bias = weighted_bias / available_weight;
    if bias.abs() < 0.3 { return None; }

    let midpoint = (lower + upper) / 2.0;
    let width_percent = if midpoint > 0.0 { (upper - lower) / midpoint * 100.0 } else { f64::INFINITY };
    let expected_move = components.volatility_1m_percent.unwrap_or_default().abs() * 5.0_f64.sqrt();
    let compression = if expected_move > 0.0 { (1.0 - width_percent / (expected_move * 3.0)).clamp(0.0, 1.0) } else { 0.0 };
    let pressure = (bias.abs() * (0.7 + 0.3 * compression)).clamp(0.0, 1.0);
    (pressure >= 0.35).then_some((if bias > 0.0 { "UP" } else { "DOWN" }, pressure))
}

impl SignalEngine {
    pub fn on_price(&mut self, timestamp: u64, price: f64, volatility_1m: f64) -> Vec<CompletedSignalOutcome> {
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
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
                let fee_rate = pending.taker_fee_rate;
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
                let legacy = SignalOutcome { record_type: "outcome".into(), symbol: pending.symbol.clone(), signal_timestamp: pending.timestamp, evaluated_timestamp: timestamp, horizon_seconds: pending.horizon_seconds, return_percent: change, direction_match, direction: pending.direction.clone(), strength: pending.strength, net_return_percent, execution_cost_percent, trend_5m_volatility_units: pending.trend_5m_volatility_units, trend_5m_direction_match, trend_5m_net_return_percent, features: pending.features };

                let mut path_high = pending.base_price;
                let mut path_high_timestamp = pending.timestamp;
                let mut path_low = pending.base_price;
                let mut path_low_timestamp = pending.timestamp;
                for (sample_time, sample_price) in self.prices.iter().filter(|(sample_time, _)| *sample_time >= pending.timestamp && *sample_time <= timestamp) {
                    if *sample_price > path_high { path_high = *sample_price; path_high_timestamp = *sample_time; }
                    if *sample_price < path_low { path_low = *sample_price; path_low_timestamp = *sample_time; }
                }
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
                let (exit_bid, exit_ask, exit_quote_timestamp) = exit_quote.map_or((None, None, None), |(quote_time, bid, ask)| (Some(bid), Some(ask), Some(quote_time)));
                let long_gross_percent = (price / pending.base_price - 1.0) * 100.0;
                let short_gross_percent = ((pending.base_price - price) / pending.base_price) * 100.0;
                let long_net_percent = pending.entry_ask.zip(exit_bid).and_then(|(entry, exit)| directional_net_return("bullish", entry, exit, pending.taker_fee_rate));
                let short_net_percent = pending.entry_bid.zip(exit_ask).and_then(|(entry, exit)| directional_net_return("bearish", entry, exit, pending.taker_fee_rate));
                let evaluation = SignalEvaluation {
                    symbol: pending.symbol, signal_timestamp: pending.timestamp, evaluated_timestamp: timestamp,
                    horizon_seconds: pending.horizon_seconds, signal_direction: pending.direction,
                    signal_score: pending.score, ranging_probability: pending.ranging_probability,
                    reference_entry_price: pending.base_price, reference_exit_price: price,
                    entry_bid: pending.entry_bid, entry_ask: pending.entry_ask,
                    entry_quote_timestamp: pending.entry_quote_timestamp,
                    exit_bid, exit_ask, exit_quote_timestamp, taker_fee_rate: pending.taker_fee_rate,
                    long_gross_percent, short_gross_percent, long_net_percent, short_net_percent,
                    path_high, path_high_timestamp, path_low, path_low_timestamp,
                };
                outcomes.push(CompletedSignalOutcome { legacy, evaluation });
            } else { remaining.push_back(pending); }
        }
        self.pending = remaining;
        outcomes
    }
    pub fn return_context(&self, timestamp: u64) -> (Option<f64>, Option<f64>) {
        let fresh = self.prices.back().is_some_and(|(sample_time, _)| timestamp.saturating_sub(*sample_time) <= MAX_BOOK_AGE_MS);
        if !fresh { return (None, None); }
        (self.price_return(timestamp.saturating_sub(60_000)), self.price_return(timestamp.saturating_sub(300_000)))
    }

    pub fn research_features(&self, symbol: &str, timestamp: u64, btc: (Option<f64>, Option<f64>), eth: (Option<f64>, Option<f64>)) -> SignalResearchFeatures {
        let quote = self.best_quote.filter(|(quote_time, _, _)| *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
        let current_book = self.book_history.iter().rev().find(|sample| sample.timestamp <= timestamp && timestamp.saturating_sub(sample.timestamp) <= MAX_BOOK_AGE_MS);
        let depth = |sample: &BookHistorySample, bids: bool| {
            let levels = if bids { &sample.bid_levels } else { &sample.ask_levels };
            levels.iter().filter(|(price, _)| ((*price / ((sample.bid + sample.ask) / 2.0) - 1.0).abs() * 10_000.0) <= 10.0)
                .map(|(price, size)| price * size).sum::<f64>()
        };
        let (bid_notional, ask_notional) = current_book.map(|sample| (Some(depth(sample, true)), Some(depth(sample, false)))).unwrap_or((None, None));
        let imbalance = bid_notional.zip(ask_notional).and_then(|(bid, ask)| (bid + ask > 0.0).then_some((bid - ask) / (bid + ask)));
        let imbalance_at = |cutoff: u64| self.book_history.iter().rev().find(|sample| sample.timestamp <= cutoff).and_then(|sample| {
            let bid = depth(sample, true); let ask = depth(sample, false);
            (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
        });
        let current_imbalance = current_book.and_then(|sample| {
            let bid = depth(sample, true); let ask = depth(sample, false);
            (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
        });
        let current_mid = quote.map(|(_, bid, ask)| (bid + ask) / 2.0);
        let top_sizes = current_book.and_then(|sample| Some((*sample.bid_levels.first()?, *sample.ask_levels.first()?)));
        let microprice = current_mid.zip(top_sizes).and_then(|(mid, (bid, ask))| {
            (bid.1 + ask.1 > 0.0).then_some(((sample_microprice(bid.0, bid.1, ask.0, ask.1) / mid) - 1.0) * 10_000.0)
        });
        let spread = quote.and_then(|(_, bid, ask)| ((bid + ask) > 0.0).then_some((ask - bid) / ((ask + bid) / 2.0) * 10_000.0));
        let flow = |window_ms: u64| {
            let start = timestamp.saturating_sub(window_ms);
            let mut signed = 0.0; let mut total = 0.0; let mut count = 0u64;
            for (_, price, size, is_buy, _) in self.trades.iter().filter(|(trade_time, ..)| *trade_time >= start && *trade_time <= timestamp) {
                let notional = price * size; total += notional; signed += if *is_buy { notional } else { -notional }; count += 1;
            }
            (total > 0.0).then_some((signed / total, signed, total, count))
        };
        let flow5 = flow(5_000); let flow15 = flow(15_000); let flow60 = flow(60_000);
        let signed5 = flow5.map(|(_, signed, _, _)| signed);
        let signed15 = flow15.map(|(_, signed, _, _)| signed);
        let trade_count_5s = flow5.map_or(0, |(_, _, _, count)| count);
        let trade_count_60s = flow60.map_or(0, |(_, _, _, count)| count);
        let average_trade_notional_5s = flow5.and_then(|(_, _, total, count)| (count > 0).then_some(total / count as f64));
        let price_move_bps = self.price_return(timestamp.saturating_sub(5_000)).map(|value| value * 100.0);
        SignalResearchFeatures {
            symbol: symbol.to_owned(), timestamp, spread_bps: spread, microprice_offset_bps: microprice,
            bid_notional_10bps: bid_notional, ask_notional_10bps: ask_notional, depth_imbalance_10bps: imbalance,
            depth_imbalance_change_1s: current_imbalance.zip(imbalance_at(timestamp.saturating_sub(1_000))).map(|(now, old)| now - old),
            depth_imbalance_change_5s: current_imbalance.zip(imbalance_at(timestamp.saturating_sub(5_000))).map(|(now, old)| now - old),
            trade_flow_5s: flow5.map(|(value, _, _, _)| value), trade_flow_15s: flow15.map(|(value, _, _, _)| value),
            trade_flow_60s: flow60.map(|(value, _, _, _)| value), trade_flow_acceleration: flow5.map(|(value, _, _, _)| value).zip(flow60.map(|(value, _, _, _)| value)).map(|(short, long)| short - long),
            signed_trade_notional_5s: signed5, signed_trade_notional_15s: signed15, trade_count_5s, trade_count_60s,
            average_trade_notional_5s, price_response_bps_per_1000usd_5s: price_move_bps.zip(signed5).filter(|(_, signed)| signed.abs() > 1e-9).map(|(move_bps, signed)| move_bps * 1_000.0 / signed.abs().max(1.0)),
            btc_return_1m_percent: btc.0, btc_return_5m_percent: btc.1, eth_return_1m_percent: eth.0, eth_return_5m_percent: eth.1,
        }
    }

    pub fn book_snapshot(&self, symbol: &str, timestamp: u64) -> Option<SignalBookSnapshot> {
        let (quote_timestamp, _, _) = self.best_quote?;
        if quote_timestamp > timestamp || timestamp.saturating_sub(quote_timestamp) > MAX_BOOK_AGE_MS { return None; }
        Some(SignalBookSnapshot {
            symbol: symbol.to_owned(), timestamp,
            bid_levels: self.book_bid_levels.clone(), ask_levels: self.book_ask_levels.clone(),
        })
    }

    pub fn track(&mut self, snapshot: &SignalSnapshot) {
        if let Some((_, base_price)) = self.prices.back().copied() {
            let quote = self.best_quote.filter(|(quote_time, _, _)| *quote_time <= snapshot.timestamp && snapshot.timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS);
            // Keep short-term checks while adding swing-style horizons. Outcomes are
            // evaluated from backend price/book data and persisted with the same cost model.
            for horizon_seconds in [60, 300, 900, 3_600] {
                self.pending.push_back(PendingOutcome {
                    symbol: snapshot.symbol.clone(), timestamp: snapshot.timestamp, base_price,
                    direction: snapshot.direction.clone(), strength: snapshot.strength, horizon_seconds,
                    entry_bid: quote.map(|(_, bid, _)| bid), entry_ask: quote.map(|(_, _, ask)| ask),
                    trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                    features: feature_vector(&snapshot.components),
                    score: snapshot.score, ranging_probability: snapshot.ranging_probability,
                    entry_quote_timestamp: quote.map(|(quote_time, _, _)| quote_time),
                    taker_fee_rate: configured_signal_taker_fee_rate(),
                });
            }
        }
    }

    pub fn on_trade(&mut self, timestamp: u64, price: f64, size: f64, is_buy: bool) {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 { return; }
        let aggressor_cost_bps = self.best_quote
            .filter(|(quote_time, bid, ask)| *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS && bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
            .map(|(_, bid, ask)| {
                let midpoint = (bid + ask) / 2.0;
                let paid_vs_mid = if is_buy { price - midpoint } else { midpoint - price };
                paid_vs_mid / midpoint * 10_000.0
            })
            .filter(|cost| cost.is_finite());
        self.trades.push_back((timestamp, price, size, is_buy, aggressor_cost_bps));
        self.prune(timestamp);
    }

    pub fn on_book(&mut self, timestamp: u64, best_bid: f64, best_ask: f64, bid_sizes: &[f64], ask_sizes: &[f64], bid_levels: &[(f64, f64)], ask_levels: &[(f64, f64)]) {
        if best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid {
            self.best_quote = Some((timestamp, best_bid, best_ask));
        }
        self.book_bid_levels = bid_levels.iter().copied().filter(|(price, size)| price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0).collect();
        self.book_ask_levels = ask_levels.iter().copied().filter(|(price, size)| price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0).collect();
        if best_bid > 0.0 && best_ask >= best_bid {
            self.book_history.push_back(BookHistorySample { timestamp, bid: best_bid, ask: best_ask, bid_levels: self.book_bid_levels.clone(), ask_levels: self.book_ask_levels.clone() });
            while self.book_history.front().is_some_and(|sample| sample.timestamp < timestamp.saturating_sub(60_000)) { self.book_history.pop_front(); }
        }
        let bid: f64 = bid_sizes.iter().copied().filter(|n| n.is_finite() && *n > 0.0).sum();
        let ask: f64 = ask_sizes.iter().copied().filter(|n| n.is_finite() && *n > 0.0).sum();
        if bid + ask > 0.0 { self.book_imbalance = Some((bid - ask) / (bid + ask)); }
    }

    pub fn snapshot(&mut self, symbol: &str, now: u64) -> Option<SignalSnapshot> {
        if now.saturating_sub(self.last_snapshot_at) < MIN_SNAPSHOT_INTERVAL_MS { return None; }
        self.prune(now);
        let latest = self.prices.back().copied()?;
        let range_samples = self.prices.iter().filter(|sample| sample.0 >= now.saturating_sub(300_000)).collect::<Vec<_>>();
        let range_bounds = if range_samples.len() >= 12 {
            let low = range_samples.iter().map(|sample| sample.1).fold(f64::INFINITY, f64::min);
            let high = range_samples.iter().map(|sample| sample.1).fold(f64::NEG_INFINITY, f64::max);
            (low.is_finite() && high.is_finite() && low > 0.0 && high > low).then_some((low, high))
        } else { None };
        let ranging_probability = range_bounds.map(|(low, high)| {
            let width_percent = (high / low - 1.0) * 100.0;
            let net_move_percent = (latest.1 / range_samples[0].1 - 1.0).abs() * 100.0;
            // Heuristic only: range likelihood rises when price explores both
            // edges but makes little net progress over the five-minute window.
            let low_touches = range_samples.iter().filter(|sample| (sample.1 / low - 1.0) * 100.0 <= width_percent * 0.15).count();
            let high_touches = range_samples.iter().filter(|sample| (high / sample.1 - 1.0) * 100.0 <= width_percent * 0.15).count();
            let exploration = ((low_touches.min(3) + high_touches.min(3)) as f64 / 6.0).clamp(0.0, 1.0);
            let containment = (1.0 - net_move_percent / (width_percent * 0.6).max(0.01)).clamp(0.0, 1.0);
            (0.65 * containment + 0.35 * exploration).clamp(0.0, 1.0)
        });
        let trend = self.price_return(now.saturating_sub(60_000));
        let momentum = self.price_return(now.saturating_sub(10_000));
        let trend_5m = self.price_return(now.saturating_sub(300_000));
        let volatility_5m = self.realized_volatility(now.saturating_sub(300_000));
        let trend_5m_volatility_units = trend_5m.zip(volatility_5m).and_then(|(trend, volatility)| (volatility > 1e-9).then_some(trend / volatility));
        let flow = {
            let (buy, sell) = self.trades.iter().fold((0.0, 0.0), |(buy, sell), (_, price, size, is_buy, _)|
                if *is_buy { (buy + price * size, sell) } else { (buy, sell + price * size) });
            (buy + sell > 0.0).then_some((buy - sell) / (buy + sell))
        };
        let mispricing_vs_vwap_bps = {
            let cutoff = now.saturating_sub(60_000);
            let samples = self.trades.iter().filter(|sample| sample.0 >= cutoff).collect::<Vec<_>>();
            let (notional, quantity) = samples.iter().fold((0.0, 0.0), |(notional, quantity), sample|
                (notional + sample.1 * sample.2, quantity + sample.2));
            if samples.len() >= 5 && samples.last().zip(samples.first()).is_some_and(|(last, first)| last.0.saturating_sub(first.0) >= 10_000) && quantity > 0.0 {
                let vwap = notional / quantity;
                (vwap.is_finite() && vwap > 0.0).then_some((latest.1 / vwap - 1.0) * 10_000.0)
            } else { None }
        };
        let pressure_without_price_move = flow.zip(momentum).and_then(|(imbalance, movement)| {
            (imbalance.abs() >= 0.55 && movement.abs() <= 0.025).then_some(-imbalance)
        });
        let aggressor_cost_bps = {
            let (weighted_cost, notional) = self.trades.iter().filter(|sample| sample.0 >= now.saturating_sub(60_000))
                .filter_map(|sample| sample.4.map(|cost| (cost, sample.1 * sample.2)))
                .fold((0.0, 0.0), |(sum, weight), (cost, value)| (sum + cost * value, weight + value));
            (notional > 0.0).then_some(weighted_cost / notional)
        };
        let components = SignalComponents {
            trend_1m_percent: trend,
            momentum_10s_percent: momentum,
            trade_flow_imbalance: flow,
            book_imbalance: self.book_imbalance,
            volatility_1m_percent: self.volatility_1m.or_else(|| self.realized_volatility(now.saturating_sub(60_000))),
            trend_5m_percent: trend_5m,
            trend_5m_volatility_units,
            mispricing_vs_vwap_bps,
            pressure_without_price_move,
            aggressor_cost_bps,
        };
        let mispricing_signal = mispricing_vs_vwap_bps.map(|value| (-value / 20.0).clamp(-1.0, 1.0));
        let inputs = [
            trend.map(|v| (v / 0.25).clamp(-1.0, 1.0)),
            momentum.map(|v| (v / 0.12).clamp(-1.0, 1.0)),
            flow,
            self.book_imbalance,
            mispricing_signal,
            pressure_without_price_move,
        ];
        let count = inputs.iter().filter(|v| v.is_some()).count();
        if count == 0 { return None; }
        let score = inputs.iter().flatten().sum::<f64>() / count as f64;
        let direction = if score > 0.2 { "bullish" } else if score < -0.2 { "bearish" }
            else if ranging_probability.is_some_and(|probability| probability >= 0.65) { "ranging" } else { "neutral" };
        let mut reasons = Vec::new();
        if let Some(v) = trend { reasons.push(format!("1m price change {v:.3}%")); }
        if let Some(v) = momentum { reasons.push(format!("10s price change {v:.3}%")); }
        if let Some(v) = flow { reasons.push(format!("buy/sell trade imbalance {v:.2}")); }
        if let Some(v) = self.book_imbalance { reasons.push(format!("top-book imbalance {v:.2}")); }
        if let Some(v) = mispricing_vs_vwap_bps.filter(|value| value.abs() >= 8.0) {
            let relation = if v > 0.0 { "above" } else { "below" };
            reasons.push(format!("Price {:.1} bps {relation} 60s trade VWAP", v.abs()));
        }
        if let Some(v) = pressure_without_price_move {
            let pressure = if v < 0.0 { "buying" } else { "selling" };
            reasons.push(format!("Strong {pressure} pressure met little price movement"));
        }
        if let Some(v) = aggressor_cost_bps.filter(|value| *value >= 5.0) {
            reasons.push(format!("Aggressive traders paid an average {:.1} bps versus midpoint", v));
        }
        if direction == "ranging" {
            if let Some((low, high)) = range_bounds {
                reasons.push(format!("Price is ranging between {:.8} and {:.8} over five minutes", low, high));
                if let Some((side, pressure)) = breakout_watch(&components, range_bounds, ranging_probability) {
                    reasons.push(format!("Breakout watch: {side} · {:.0}% pressure (heuristic, not calibrated)", pressure * 100.0));
                }
            }
        }
        self.last_snapshot_at = now;
        Some(SignalSnapshot { symbol: symbol.to_owned(), timestamp: now, direction: direction.to_owned(), score, strength: (score.abs() * count as f64 / 6.0).clamp(0.0, 1.0), components, reasons, stale: now.saturating_sub(latest.0) > 5_000,
            ranging_probability, range_lower_bound: range_bounds.map(|(low, _)| low), range_upper_bound: range_bounds.map(|(_, high)| high) })
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

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignalDashboard {
    pub best: Vec<SignalSnapshot>,
    pub history: Vec<SignalSnapshot>,
    pub performance: Vec<SignalPerformance>,
    pub walk_forward: Vec<SignalWalkForward>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoTraderDashboard {
    pub best: Vec<SignalSnapshot>,
    pub walk_forward: Vec<SignalWalkForward>,
}

#[derive(Deserialize)]
struct EvaluationQuery {
    since: Option<u64>,
    limit: Option<usize>,
}

#[derive(Clone)]
pub struct SignalStore { path: PathBuf, write_lock: Arc<Mutex<()>>, recent_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>, best_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>, performance_cache: Arc<Mutex<Option<(Instant, Vec<SignalPerformance>)>>>, walk_forward_cache: Arc<Mutex<Option<(Instant, Vec<SignalWalkForward>)>>> }

#[derive(Serialize, Deserialize)]
enum StoredSignalRecord {
    Snapshot(SignalSnapshot),
    Outcome(SignalOutcome),
    Evaluation(SignalEvaluation),
    BookSnapshot(SignalBookSnapshot),
    ResearchFeatures(SignalResearchFeatures),
    ExternalMarket(ExternalMarketFeatures),
}

fn record_timestamp(record: &StoredSignalRecord) -> u64 {
    match record {
        StoredSignalRecord::Snapshot(snapshot) => snapshot.timestamp,
        StoredSignalRecord::Outcome(outcome) => outcome.signal_timestamp,
        StoredSignalRecord::Evaluation(evaluation) => evaluation.signal_timestamp,
        StoredSignalRecord::BookSnapshot(snapshot) => snapshot.timestamp,
        StoredSignalRecord::ResearchFeatures(features) => features.timestamp,
        StoredSignalRecord::ExternalMarket(features) => features.timestamp,
    }
}

fn daily_partition_path(base: &PathBuf, timestamp_ms: u64) -> PathBuf {
    let days = (timestamp_ms / 86_400_000) as i64;
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    let stem = base.file_stem().and_then(|value| value.to_str()).unwrap_or("signals");
    let extension = base.extension().and_then(|value| value.to_str()).unwrap_or("jsonl");
    base.with_file_name(format!("{stem}-{year:04}-{month:02}-{day:02}.{extension}"))
}

fn is_daily_partition(base: &PathBuf, candidate: &std::ffi::OsStr) -> bool {
    let Some(name) = candidate.to_str() else { return false; };
    let stem = base.file_stem().and_then(|value| value.to_str()).unwrap_or("signals");
    let extension = base.extension().and_then(|value| value.to_str()).unwrap_or("jsonl");
    let prefix = format!("{stem}-");
    let suffix = format!(".{extension}");
    let Some(date) = name.strip_prefix(&prefix).and_then(|value| value.strip_suffix(&suffix)) else { return false; };
    date.len() == 10 && date.bytes().enumerate().all(|(index, byte)| match index {
        4 | 7 => byte == b'-',
        _ => byte.is_ascii_digit(),
    })
}

fn encode_record(record: &StoredSignalRecord) -> Result<Vec<u8>> {
    let payload = bincode::serialize(record)?;
    anyhow::ensure!(payload.len() <= u32::MAX as usize, "signal record is too large");
    let mut bytes = Vec::with_capacity(payload.len() + 4);
    bytes.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    bytes.extend_from_slice(&payload);
    Ok(bytes)
}

fn decode_binary_records(data: &[u8]) -> Result<Vec<StoredSignalRecord>> {
    anyhow::ensure!(data.starts_with(SIGNALS_BINARY_MAGIC), "invalid binary signals header");
    let mut offset = SIGNALS_BINARY_MAGIC.len();
    let mut records = Vec::new();
    while offset < data.len() {
        anyhow::ensure!(data.len() - offset >= 4, "truncated signal record length");
        let length = u32::from_le_bytes(data[offset..offset + 4].try_into()?) as usize;
        offset += 4;
        anyhow::ensure!(data.len() - offset >= length, "truncated signal record payload");
        records.push(bincode::deserialize(&data[offset..offset + length])?);
        offset += length;
    }
    Ok(records)
}

fn decode_legacy_records(data: &[u8]) -> Result<Vec<StoredSignalRecord>> {
    let decoded = if data.starts_with(&[0x1f, 0x8b]) {
        let mut output = Vec::new();
        MultiGzDecoder::new(data).read_to_end(&mut output)?;
        output
    } else { data.to_vec() };
    let mut records = Vec::new();
    for line in decoded.split(|byte| *byte == b'\n') {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(line) else { continue; };
        if value.get("record_type").and_then(|field| field.as_str()) == Some("outcome") {
            if let Ok(outcome) = serde_json::from_value(value) { records.push(StoredSignalRecord::Outcome(outcome)); }
        } else if let Ok(snapshot) = serde_json::from_value(value) {
            records.push(StoredSignalRecord::Snapshot(snapshot));
        }
    }
    Ok(records)
}

fn encode_records(records: &[StoredSignalRecord]) -> Result<Vec<u8>> {
    let mut output = SIGNALS_BINARY_MAGIC.to_vec();
    for record in records { output.extend_from_slice(&encode_record(record)?); }
    Ok(output)
}

fn gzip_member(data: &[u8]) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(data)?;
    Ok(encoder.finish()?)
}

async fn read_signal_records(path: &PathBuf) -> Result<Vec<StoredSignalRecord>> {
    let data = match tokio::fs::read(path).await {
        Ok(data) => data,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    if data.starts_with(SIGNALS_BINARY_MAGIC) { decode_binary_records(&data) }
    else if data.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        MultiGzDecoder::new(data.as_slice()).read_to_end(&mut decoded)?;
        if decoded.starts_with(SIGNALS_BINARY_MAGIC) { decode_binary_records(&decoded) }
        else { decode_legacy_records(&data) }
    } else { decode_legacy_records(&data) }
}

fn accuracy_bucket(snapshot: &SignalSnapshot) -> u8 {
    let value = if snapshot.direction == "ranging" { snapshot.ranging_probability.unwrap_or_default() } else { snapshot.strength };
    (value.clamp(0.0, 0.999) * 10.0) as u8
}

fn calibrate_display_strength(snapshots: &mut [SignalSnapshot], records: &[StoredSignalRecord]) {
    const PRIOR_SAMPLES: f64 = 10.0;
    let mut history: HashMap<String, Vec<&SignalSnapshot>> = HashMap::new();
    let mut accuracies: HashMap<(String, String, u8), (usize, usize)> = HashMap::new();
    let mut outcomes_by_key: HashMap<(String, String, u8), (usize, usize)> = HashMap::new();
    let mut outcomes = Vec::new();

    for record in records {
        match record {
            StoredSignalRecord::Snapshot(snapshot) => history.entry(snapshot.symbol.clone()).or_default().push(snapshot),
            StoredSignalRecord::Outcome(outcome) => outcomes.push(outcome),
            StoredSignalRecord::Evaluation(_) | StoredSignalRecord::BookSnapshot(_) | StoredSignalRecord::ResearchFeatures(_) | StoredSignalRecord::ExternalMarket(_) => {},
        }
    }
    for items in history.values_mut() { items.sort_by_key(|snapshot| snapshot.timestamp); }

    for (symbol, items) in &history {
        for prediction in items.iter().filter(|item| item.direction == "ranging") {
            let (Some(lower), Some(upper)) = (prediction.range_lower_bound, prediction.range_upper_bound) else { continue; };
            let target = prediction.timestamp.saturating_add(300_000);
            let index = items.partition_point(|item| item.timestamp < target);
            let Some(observed) = items.get(index).filter(|item| item.timestamp <= target.saturating_add(15_000)) else { continue; };
            let (Some(future_low), Some(future_high)) = (observed.range_lower_bound, observed.range_upper_bound) else { continue; };
            let key = (symbol.clone(), "ranging".to_owned(), accuracy_bucket(prediction));
            let entry = accuracies.entry(key).or_default();
            entry.0 += 1;
            if future_low >= lower && future_high <= upper { entry.1 += 1; }
        }
    }
    for outcome in outcomes.iter().filter(|outcome| outcome.horizon_seconds == 60 && matches!(outcome.direction.as_str(), "bullish" | "bearish")) {
        let bucket = (outcome.strength.clamp(0.0, 0.999) * 10.0) as u8;
        let entry = outcomes_by_key.entry((outcome.symbol.clone(), outcome.direction.clone(), bucket)).or_default();
        entry.0 += 1;
        if outcome.direction_match { entry.1 += 1; }
    }

    // Combine adjacent score buckets so each estimate uses more historical examples.
    fn estimate(map: &HashMap<(String, String, u8), (usize, usize)>, snapshot: &SignalSnapshot) -> f64 {
        let bucket = accuracy_bucket(snapshot);
        let mut total = (0usize, 0usize);
        for adjacent in bucket.saturating_sub(1)..=bucket.saturating_add(1).min(9) {
            if let Some((samples, hits)) = map.get(&(snapshot.symbol.clone(), snapshot.direction.clone(), adjacent)) {
                total.0 += samples;
                total.1 += hits;
            }
        }
        (total.1 as f64 + PRIOR_SAMPLES * 0.5) / (total.0 as f64 + PRIOR_SAMPLES)
    }

    // The beta prior keeps small samples near 50% until enough resolved forecasts exist.
    for snapshot in snapshots {
        snapshot.strength = if snapshot.direction == "ranging" { estimate(&accuracies, snapshot) }
            else { estimate(&outcomes_by_key, snapshot) };
    }
}

impl SignalStore {
    pub fn from_environment() -> Result<Self> {
        let path = std::env::var_os("SIGNALS_FILE").map(PathBuf::from).unwrap_or(std::env::current_dir()?.join("signals.jsonl"));
        Ok(Self { path, write_lock: Arc::new(Mutex::new(())), recent_cache: Arc::new(Mutex::new(None)), best_cache: Arc::new(Mutex::new(None)), performance_cache: Arc::new(Mutex::new(None)), walk_forward_cache: Arc::new(Mutex::new(None)) })
    }

    async fn record_paths(&self) -> Result<Vec<PathBuf>> {
        let parent = self.path.parent().filter(|path| !path.as_os_str().is_empty()).unwrap_or_else(|| std::path::Path::new("."));
        let mut paths = vec![self.path.clone()];
        let mut directory = match tokio::fs::read_dir(parent).await {
            Ok(directory) => directory,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(paths),
            Err(error) => return Err(error.into()),
        };
        while let Some(entry) = directory.next_entry().await? {
            let name = entry.file_name();
            if is_daily_partition(&self.path, &name) { paths.push(entry.path()); }
        }
        paths.sort();
        Ok(paths)
    }

    async fn read_all_records(&self) -> Result<Vec<StoredSignalRecord>> {
        let mut records = Vec::new();
        for path in self.record_paths().await? {
            records.extend(read_signal_records(&path).await?);
        }
        Ok(records)
    }

    pub async fn migrate_to_binary(&self) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        let existing = match tokio::fs::read(&self.path).await {
            Ok(data) => data,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if existing.starts_with(&[0x1f, 0x8b]) {
            let mut decoded = Vec::new();
            MultiGzDecoder::new(existing.as_slice()).read_to_end(&mut decoded)?;
            if decoded.starts_with(SIGNALS_BINARY_MAGIC) { return Ok(()); }
        }
        if existing.starts_with(SIGNALS_BINARY_MAGIC) {
            tokio::fs::write(&self.path, gzip_member(&existing)?).await?;
        } else {
            let records = decode_legacy_records(&existing)?;
            tokio::fs::write(&self.path, gzip_member(&encode_records(&records)?)?).await?;
        }
        Ok(())
    }

    pub fn router(self) -> Router {
        let dashboard_store = self.clone();
        let history_store = self.clone();
        let performance_store = self.clone();
        let evaluation_store = self.clone();
        let raw_evaluation_store = self.clone();
        let book_snapshot_store = self.clone();
        let research_feature_store = self.clone();
        let external_market_store = self.clone();
        let auto_trader_store = self.clone();
        let best_store = self;
        Router::new()
            .route("/signals/dashboard", get(move || {
                let store = dashboard_store.clone();
                async move { match store.dashboard().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/history", get(move || {
                let store = history_store.clone();
                async move { match store.recent(200).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/performance", get(move || {
                let store = performance_store.clone();
                async move { match store.performance().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/auto-trader-dashboard", get(move || {
                let store = auto_trader_store.clone();
                async move { match store.auto_trader_dashboard().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/best", get(move || {
                let store = best_store.clone();
                async move { match store.best(10).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/walk-forward", get(move || {
                let store = evaluation_store.clone();
                async move { match store.walk_forward().await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/evaluations", get(move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                let store = raw_evaluation_store.clone();
                async move { match store.evaluations(query.since, query.limit.unwrap_or(10_000).clamp(1, 50_000)).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/book-snapshots", get(move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                let store = book_snapshot_store.clone();
                async move { match store.book_snapshots(query.since, query.limit.unwrap_or(10_000).clamp(1, 50_000)).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/research-features", get(move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                let store = research_feature_store.clone();
                async move { match store.research_features(query.since, query.limit.unwrap_or(10_000).clamp(1, 50_000)).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
            .route("/signals/external-market", get(move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                let store = external_market_store.clone();
                async move { match store.external_market_features(query.since, query.limit.unwrap_or(10_000).clamp(1, 50_000)).await { Ok(items) => Ok(Json(items)), Err(error) => Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string())) } }
            }))
    }

    pub async fn book_snapshots(&self, since: Option<u64>, limit: usize) -> Result<Vec<SignalBookSnapshot>> {
        let mut snapshots = self.read_all_records().await?.into_iter().filter_map(|record| match record {
            StoredSignalRecord::BookSnapshot(snapshot) if since.is_none_or(|timestamp| snapshot.timestamp >= timestamp) => Some(snapshot),
            _ => None,
        }).collect::<Vec<_>>();
        snapshots.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = snapshots.len().saturating_sub(limit);
        Ok(snapshots.drain(start..).collect())
    }

    pub async fn append_book_snapshot(&self, snapshot: &SignalBookSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::BookSnapshot(snapshot.clone())).await
    }

    pub async fn append_external_market_features(&self, features: &ExternalMarketFeatures) -> Result<()> {
        self.append_record(StoredSignalRecord::ExternalMarket(features.clone())).await
    }

    pub async fn external_market_features(&self, since: Option<u64>, limit: usize) -> Result<Vec<ExternalMarketFeatures>> {
        let mut features = self.read_all_records().await?.into_iter().filter_map(|record| match record {
            StoredSignalRecord::ExternalMarket(features) if since.is_none_or(|timestamp| features.timestamp >= timestamp) => Some(features),
            _ => None,
        }).collect::<Vec<_>>();
        features.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.underlying.cmp(&b.underlying)));
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn append_research_features(&self, features: &SignalResearchFeatures) -> Result<()> {
        self.append_record(StoredSignalRecord::ResearchFeatures(features.clone())).await
    }

    pub async fn research_features(&self, since: Option<u64>, limit: usize) -> Result<Vec<SignalResearchFeatures>> {
        let mut features = self.read_all_records().await?.into_iter().filter_map(|record| match record {
            StoredSignalRecord::ResearchFeatures(features) if since.is_none_or(|timestamp| features.timestamp >= timestamp) => Some(features),
            _ => None,
        }).collect::<Vec<_>>();
        features.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn evaluations(&self, since: Option<u64>, limit: usize) -> Result<Vec<SignalEvaluation>> {
        let mut evaluations = self.read_all_records().await?.into_iter().filter_map(|record| match record {
            StoredSignalRecord::Evaluation(evaluation) if since.is_none_or(|timestamp| evaluation.evaluated_timestamp >= timestamp) => Some(evaluation),
            _ => None,
        }).collect::<Vec<_>>();
        evaluations.sort_by(|a, b| a.evaluated_timestamp.cmp(&b.evaluated_timestamp).then(a.symbol.cmp(&b.symbol)).then(a.horizon_seconds.cmp(&b.horizon_seconds)));
        let start = evaluations.len().saturating_sub(limit);
        Ok(evaluations.drain(start..).collect())
    }

    pub async fn auto_trader_dashboard(&self) -> Result<AutoTraderDashboard> {
        let (best, walk_forward) = tokio::try_join!(self.best(100), self.walk_forward())?;
        Ok(AutoTraderDashboard { best, walk_forward })
    }

    pub async fn dashboard(&self) -> Result<SignalDashboard> {
        let (best, history, performance, walk_forward) = tokio::try_join!(
            self.best(10),
            self.recent(200),
            self.performance(),
            self.walk_forward(),
        )?;
        Ok(SignalDashboard { best, history, performance, walk_forward })
    }

    pub async fn recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        if limit > 200 { return self.load_recent(limit).await; }
        let mut cache = self.recent_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(30) {
                let start = items.len().saturating_sub(limit);
                return Ok(items[start..].to_vec());
            }
        }
        let items = self.load_recent(200).await?;
        let start = items.len().saturating_sub(limit);
        *cache = Some((Instant::now(), items.clone()));
        Ok(items[start..].to_vec())
    }

    async fn load_recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let records = self.read_all_records().await?;
        let mut items = records.iter().filter_map(|record| match record { StoredSignalRecord::Snapshot(snapshot) => Some(snapshot.clone()), _ => None }).collect::<Vec<_>>();
        calibrate_display_strength(&mut items, &records);
        let start = items.len().saturating_sub(limit);
        Ok(items.drain(start..).collect())
    }

    pub async fn best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        if limit > 10 { return self.load_best(limit).await; }
        let mut cache = self.best_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(30) {
                return Ok(items.iter().take(limit).cloned().collect());
            }
        }
        let items = self.load_best(10).await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items.into_iter().take(limit).collect())
    }

    async fn load_best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let records = self.read_all_records().await?;
        let mut latest: HashMap<String, SignalSnapshot> = HashMap::new();
        for record in &records {
            let StoredSignalRecord::Snapshot(snapshot) = record else { continue; };
            let snapshot = snapshot.clone();
            // Retain the latest ranging setup from disk across a service restart.
            // It remains visible until fresh feed data replaces it, with staleness
            // set below so clients can distinguish it from a live signal.
            if snapshot.stale && snapshot.direction != "ranging" { continue; }
            let replace = latest.get(&snapshot.symbol).is_none_or(|current| current.timestamp < snapshot.timestamp);
            if replace { latest.insert(snapshot.symbol.clone(), snapshot); }
        }
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        let mut items = latest.into_values().filter_map(|mut item| {
            let age = now.saturating_sub(item.timestamp);
            if age > 60_000 && item.direction != "ranging" { return None; }
            item.stale |= age > 60_000;
            Some(item)
        }).collect::<Vec<_>>();
        calibrate_display_strength(&mut items, &records);
        items.sort_by(|a, b| b.strength.total_cmp(&a.strength).then_with(|| b.timestamp.cmp(&a.timestamp)));
        items.truncate(limit);
        Ok(items)
    }

    pub async fn performance(&self) -> Result<Vec<SignalPerformance>> {
        let mut cache = self.performance_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) { return Ok(items.clone()); }
        }
        let items = self.compute_performance().await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }

    async fn compute_performance(&self) -> Result<Vec<SignalPerformance>> {
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
        let mut cache = self.walk_forward_cache.lock().await;
        if let Some((created, value)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) { return Ok(value.clone()); }
        }
        let value = self.compute_walk_forward().await?;
        *cache = Some((Instant::now(), value.clone()));
        Ok(value)
    }

    async fn compute_walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let outcomes = self.read_outcomes().await?;
        let snapshots = self.read_snapshots().await?;
        let snapshots_by_key = snapshots.into_iter().map(|snapshot| ((snapshot.symbol.clone(), snapshot.timestamp), snapshot)).collect::<HashMap<_, _>>();
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
            let mut signal_long = SideWalkForwardAccumulator::default();
            let mut signal_short = SideWalkForwardAccumulator::default();
            let mut all_signal_long = SideWalkForwardAccumulator::default();
            let mut all_signal_short = SideWalkForwardAccumulator::default();
            let mut logistic_long = SideWalkForwardAccumulator::default();
            let mut logistic_short = SideWalkForwardAccumulator::default();
            let mut baseline_correct = 0;
            let mut baseline_net_total = 0.0;
            let mut five_minute_trend_samples = 0;
            let mut five_minute_trend_correct = 0;
            let mut five_minute_trend_net_total = 0.0;
            let mut last_threshold = None;
            let mut logistic_selected = 0;
            let mut logistic_correct = 0;
            let mut logistic_positive = 0;
            let mut logistic_net_total = 0.0;
            let mut logistic_weights = None;
            let mut trend_logistic_selected = 0;
            let mut trend_logistic_correct = 0;
            let mut trend_logistic_positive = 0;
            let mut trend_logistic_net_total = 0.0;
            let mut trend_logistic_confident_selected = [0usize; 3];
            let mut trend_logistic_confident_correct = [0usize; 3];
            let mut trend_logistic_confident_positive = [0usize; 3];
            let mut trend_logistic_confident_net = [0.0f64; 3];
            let mut contrarian_selected = 0;
            let mut contrarian_correct = 0;
            let mut contrarian_positive = 0;
            let mut contrarian_net_total = 0.0;
            let mut contrarian_confident_selected = [0usize; 3];
            let mut contrarian_confident_correct = [0usize; 3];
            let mut contrarian_confident_positive = [0usize; 3];
            let mut contrarian_confident_net = [0.0f64; 3];
            let mut exploration_selected = [0usize; 5];
            let mut exploration_correct = [0usize; 5];
            let mut exploration_positive = [0usize; 5];
            let mut exploration_net = [0.0f64; 5];
            let mut trend_logistic_weights = None;
            for index in WALK_FORWARD_MIN_TRAINING_SAMPLES..non_overlapping.len() {
                let start = index.saturating_sub(WALK_FORWARD_TRAINING_WINDOW);
                let sample = &non_overlapping[index];
                let sample_net = sample.net_return_percent.unwrap_or_default();
                if sample.direction == "bullish" { all_signal_long.record(sample.direction_match, sample_net); }
                else if sample.direction == "bearish" { all_signal_short.record(sample.direction_match, sample_net); }
                if sample.horizon_seconds == 60 {
                    if let (Some(snapshot), Some(cost)) = (snapshots_by_key.get(&(sample.symbol.clone(), sample.signal_timestamp)), sample.execution_cost_percent) {
                        let can_explore = snapshot.strength >= 0.05
                            && snapshot.ranging_probability.unwrap_or(0.0) < 0.75
                            && matches!(snapshot.direction.as_str(), "bullish" | "bearish" | "neutral");
                        if can_explore {
                            let up = snapshot.score >= 0.0;
                            let matches = (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                            let net = if up { sample.return_percent } else { -sample.return_percent } - cost;
                            for (band, threshold) in EXPLORATION_SCORE_THRESHOLDS.iter().enumerate() {
                                if snapshot.score.abs() >= *threshold {
                                    exploration_selected[band] += 1;
                                    if matches { exploration_correct[band] += 1; }
                                    if net > 0.0 { exploration_positive[band] += 1; }
                                    exploration_net[band] += net;
                                }
                            }
                        }
                    }
                }
                let training = non_overlapping[start..index].iter().filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp).cloned().collect::<Vec<_>>();
                if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES { continue; }
                max_training_samples = max_training_samples.max(training.len());
                evaluated_signals += 1;
                if sample.direction_match { baseline_correct += 1; }
                baseline_net_total += sample.net_return_percent.unwrap_or_default();
                if (index - WALK_FORWARD_MIN_TRAINING_SAMPLES) % WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL == 0 {
                    let logistic_training = training.iter().filter(|prior| prior.features.is_some()).cloned().collect::<Vec<_>>();
                    logistic_weights = train_logistic(&logistic_training);
                    trend_logistic_weights = train_trend_logistic(&logistic_training);
                }
                if let (Some(weights), Some(features), Some(trend), Some(cost)) = (trend_logistic_weights.as_ref(), sample.features.as_ref(), sample.trend_5m_volatility_units, sample.execution_cost_percent) {
                    let probability = trend_logistic_up_probability(weights, features, trend);
                    let up = probability >= 0.5;
                    let matches = (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up { sample.return_percent } else { -sample.return_percent } - cost;
                    trend_logistic_selected += 1;
                    if matches { trend_logistic_correct += 1; }
                    if net > 0.0 { trend_logistic_positive += 1; }
                    trend_logistic_net_total += net;
                    let contrarian_up = probability < 0.5;
                    let contrarian_matches = (contrarian_up && sample.return_percent > 0.0) || (!contrarian_up && sample.return_percent < 0.0);
                    let contrarian_net = if contrarian_up { sample.return_percent } else { -sample.return_percent } - cost;
                    contrarian_selected += 1;
                    if contrarian_matches { contrarian_correct += 1; }
                    if contrarian_net > 0.0 { contrarian_positive += 1; }
                    contrarian_net_total += contrarian_net;
                    let confidence = probability.max(1.0 - probability);
                    for (band, threshold) in TREND_LOGISTIC_CONFIDENCE_THRESHOLDS.iter().enumerate() {
                        if confidence >= *threshold {
                            trend_logistic_confident_selected[band] += 1;
                            if matches { trend_logistic_confident_correct[band] += 1; }
                            if net > 0.0 { trend_logistic_confident_positive[band] += 1; }
                            trend_logistic_confident_net[band] += net;
                            contrarian_confident_selected[band] += 1;
                            if contrarian_matches { contrarian_confident_correct[band] += 1; }
                            if contrarian_net > 0.0 { contrarian_confident_positive[band] += 1; }
                            contrarian_confident_net[band] += contrarian_net;
                        }
                    }
                }
                if let (Some(weights), Some(features), Some(cost)) = (logistic_weights.as_ref(), sample.features.as_ref(), sample.execution_cost_percent) {
                    let up = logistic_up_probability(&weights, features) >= 0.5;
                    let matches = (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up { sample.return_percent } else { -sample.return_percent } - cost;
                    logistic_selected += 1;
                    if matches { logistic_correct += 1; }
                    if net > 0.0 { logistic_positive += 1; }
                    logistic_net_total += net;
                    if up { logistic_long.record(matches, net); } else { logistic_short.record(matches, net); }
                }
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
                if sample.direction == "bullish" { signal_long.record(sample.direction_match, net); }
                else if sample.direction == "bearish" { signal_short.record(sample.direction_match, net); }
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
                logistic_direction_match_rate: (logistic_selected > 0).then_some(logistic_correct as f64 / logistic_selected as f64),
                logistic_average_net_return_percent: (logistic_selected > 0).then_some(logistic_net_total / logistic_selected as f64),
                logistic_positive_net_rate: (logistic_selected > 0).then_some(logistic_positive as f64 / logistic_selected as f64),
                logistic_selected_signals: logistic_selected,
                trend_logistic_direction_match_rate: (trend_logistic_selected > 0).then_some(trend_logistic_correct as f64 / trend_logistic_selected as f64),
                trend_logistic_average_net_return_percent: (trend_logistic_selected > 0).then_some(trend_logistic_net_total / trend_logistic_selected as f64),
                trend_logistic_positive_net_rate: (trend_logistic_selected > 0).then_some(trend_logistic_positive as f64 / trend_logistic_selected as f64),
                trend_logistic_selected_signals: trend_logistic_selected,
                contrarian_trend_logistic_direction_match_rate: (contrarian_selected > 0).then_some(contrarian_correct as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_average_net_return_percent: (contrarian_selected > 0).then_some(contrarian_net_total / contrarian_selected as f64),
                contrarian_trend_logistic_positive_net_rate: (contrarian_selected > 0).then_some(contrarian_positive as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_selected_signals: contrarian_selected,
                contrarian_trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS.iter().enumerate().map(|(index, threshold)| {
                    let selected = contrarian_confident_selected[index];
                    ProbabilityConfidenceMetrics {
                        threshold: *threshold,
                        selected_signals: selected,
                        direction_match_rate: (selected > 0).then_some(contrarian_confident_correct[index] as f64 / selected as f64),
                        average_net_return_percent: (selected > 0).then_some(contrarian_confident_net[index] / selected as f64),
                        positive_net_rate: (selected > 0).then_some(contrarian_confident_positive[index] as f64 / selected as f64),
                    }
                }).collect(),
                exploration_score_bands: EXPLORATION_SCORE_THRESHOLDS.iter().enumerate().map(|(index, threshold)| {
                    let selected = exploration_selected[index];
                    ProbabilityConfidenceMetrics { threshold: *threshold, selected_signals: selected, direction_match_rate: (selected > 0).then_some(exploration_correct[index] as f64 / selected as f64), average_net_return_percent: (selected > 0).then_some(exploration_net[index] / selected as f64), positive_net_rate: (selected > 0).then_some(exploration_positive[index] as f64 / selected as f64) }
                }).collect(),
                trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS.iter().enumerate().map(|(index, threshold)| {
                    let selected = trend_logistic_confident_selected[index];
                    ProbabilityConfidenceMetrics {
                        threshold: *threshold,
                        selected_signals: selected,
                        direction_match_rate: (selected > 0).then_some(trend_logistic_confident_correct[index] as f64 / selected as f64),
                        average_net_return_percent: (selected > 0).then_some(trend_logistic_confident_net[index] / selected as f64),
                        positive_net_rate: (selected > 0).then_some(trend_logistic_confident_positive[index] as f64 / selected as f64),
                    }
                }).collect(),
                signal_long: signal_long.metrics(),
                signal_short: signal_short.metrics(),
                all_signal_long: all_signal_long.metrics(),
                all_signal_short: all_signal_short.metrics(),
                logistic_long: logistic_long.metrics(),
                logistic_short: logistic_short.metrics(),
            });
        }
        result.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.horizon_seconds.cmp(&b.horizon_seconds)));
        Ok(result)
    }

    async fn read_outcomes(&self) -> Result<Vec<SignalOutcome>> {
        Ok(self.read_all_records().await?.into_iter().filter_map(|record| match record { StoredSignalRecord::Outcome(outcome) => Some(outcome), _ => None }).collect())
    }

    async fn read_snapshots(&self) -> Result<Vec<SignalSnapshot>> {
        Ok(self.read_all_records().await?.into_iter().filter_map(|record| match record { StoredSignalRecord::Snapshot(snapshot) => Some(snapshot), _ => None }).collect())
    }

    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::Snapshot(snapshot.clone())).await
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        self.append_record(StoredSignalRecord::Outcome(outcome.clone())).await
    }

    pub async fn append_evaluation(&self, evaluation: &SignalEvaluation) -> Result<()> {
        self.append_record(StoredSignalRecord::Evaluation(evaluation.clone())).await
    }

    async fn append_record(&self, record: StoredSignalRecord) -> Result<()> {
        let _guard = self.write_lock.lock().await;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }

        // main() migrates the legacy file before serving requests. The binary log
        // is a concatenation of gzip members, so append only the new record rather
        // than decompressing the complete history on every signal update.
        let path = daily_partition_path(&self.path, record_timestamp(&record));
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .await?;
        let record_bytes = encode_record(&record)?;
        let member = if file.metadata().await?.len() == 0 {
            let mut initial = SIGNALS_BINARY_MAGIC.to_vec();
            initial.extend_from_slice(&record_bytes);
            gzip_member(&initial)?
        } else {
            gzip_member(&record_bytes)?
        };
        file.write_all(&member).await?;
        file.flush().await?;
        Ok(())
    }

}
