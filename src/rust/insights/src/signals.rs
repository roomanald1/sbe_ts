use anyhow::Result;
use axum::{http::StatusCode, routing::get, Json, Router};
use flate2::{read::MultiGzDecoder, write::GzEncoder, Compression};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot, Mutex};

const PRICE_WINDOW_MS: u64 = 3_610_000;
const TRADE_WINDOW_MS: u64 = 60_000;
const ABSORPTION_WINDOW_MS: u64 = 30_000;
const MAX_BOOK_AGE_MS: u64 = 5_000;
const MAX_PRICE_LOOKBACK_GAP_MS: u64 = 5_000;
const WALK_FORWARD_MIN_TRAINING_SAMPLES: usize = 30;
/// Heavy research capture is sampled independently from live signal generation.
// Persist one research/signal sample every ten seconds. Live signal snapshots
// are still computed on every feed event, while this cadence limits the
// historical database growth from snapshots, book data, and their outcomes.
pub const RESEARCH_SAMPLE_INTERVAL_MS: u64 = 10_000;
/// Recompute and publish live signals at most once per symbol per second.
pub const LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS: u64 = 1_000;
const WALK_FORWARD_MIN_THRESHOLD_SAMPLES: usize = 10;
const WALK_FORWARD_TRAINING_WINDOW: usize = 200;
const WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL: usize = 100;
const TREND_LOGISTIC_CONFIDENCE_THRESHOLDS: [f64; 3] = [0.55, 0.60, 0.65];
const STRENGTH_THRESHOLDS: [f64; 8] = [0.0, 0.1, 0.15, 0.2, 0.25, 0.3, 0.4, 0.5];
const EXPLORATION_SCORE_THRESHOLDS: [f64; 5] = [0.1, 0.2, 0.3, 0.4, 0.5];
const MISSED_OPPORTUNITY_SCORE_THRESHOLDS: [f64; 4] = [0.2, 0.3, 0.4, 0.5];
const MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS: [f64; 5] = [0.25, 0.35, 0.4, 0.45, 0.5];

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
    #[serde(default)]
    pub direction: String,
    #[serde(default)]
    pub strength: f64,
    #[serde(default)]
    pub net_return_percent: Option<f64>,
    #[serde(default)]
    pub execution_cost_percent: Option<f64>,
    #[serde(default)]
    pub trend_5m_volatility_units: Option<f64>,
    #[serde(default)]
    pub trend_5m_direction_match: Option<bool>,
    #[serde(default)]
    pub trend_5m_net_return_percent: Option<f64>,
    #[serde(default)]
    pub features: Option<[f64; 4]>,
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
struct SideWalkForwardAccumulator {
    samples: usize,
    correct: usize,
    positive_net: usize,
    net_total: f64,
}
impl SideWalkForwardAccumulator {
    fn record(&mut self, correct: bool, net: f64) {
        self.samples += 1;
        if correct {
            self.correct += 1;
        }
        if net > 0.0 {
            self.positive_net += 1;
        }
        self.net_total += net;
    }
    fn metrics(&self) -> SideWalkForwardMetrics {
        let hit_rate = (self.samples > 0).then_some(self.correct as f64 / self.samples as f64);
        let positive_rate =
            (self.samples > 0).then_some(self.positive_net as f64 / self.samples as f64);
        SideWalkForwardMetrics {
            sample_count: self.samples,
            direction_match_rate: hit_rate,
            average_net_return_percent: (self.samples > 0)
                .then_some(self.net_total / self.samples as f64),
            positive_net_rate: positive_rate,
            positive_net_rate_lower_95: wilson_lower_bound(self.positive_net, self.samples),
        }
    }
}

fn wilson_lower_bound(successes: usize, samples: usize) -> Option<f64> {
    if samples == 0 {
        return None;
    }
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
    /// Walk-forward results for models trained on synchronized microstructure and market-context features.
    pub research_logistic_long: SideWalkForwardMetrics,
    pub research_logistic_short: SideWalkForwardMetrics,
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
    /// Directional time to positive net after spread and configured taker fees.
    #[serde(default)]
    pub long_time_to_positive_net_seconds: Option<u64>,
    #[serde(default)]
    pub short_time_to_positive_net_seconds: Option<u64>,
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
    bid_levels: [(f64, f64); 7],
    ask_levels: [(f64, f64); 7],
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
    first_long_positive_net_timestamp: Option<u64>,
    first_short_positive_net_timestamp: Option<u64>,
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

#[derive(Clone, Copy)]
struct AbsorptionTrade {
    timestamp: u64,
    price: f64,
    size: f64,
    is_buy: bool,
    bid: f64,
    ask: f64,
    bid_depth: f64,
    ask_depth: f64,
}

pub struct SignalEngine {
    prices: VecDeque<(u64, f64)>,
    pending: VecDeque<PendingOutcome>,
    trades: VecDeque<(u64, f64, f64, bool, Option<f64>)>,
    absorption_trades: VecDeque<AbsorptionTrade>,
    book_imbalance: Option<f64>,
    best_quote: Option<(u64, f64, f64)>,
    book_bid_levels: Vec<(f64, f64)>,
    book_ask_levels: Vec<(f64, f64)>,
    book_history: VecDeque<BookHistorySample>,
    volatility_1m: Option<f64>,
    last_research_sample_at: Option<u64>,
    last_live_snapshot_at: Option<u64>,
}

impl Default for SignalEngine {
    fn default() -> Self {
        Self {
            prices: VecDeque::with_capacity(4_096),
            pending: VecDeque::with_capacity(1_024),
            trades: VecDeque::with_capacity(16_384),
            absorption_trades: VecDeque::with_capacity(8_192),
            book_imbalance: None,
            best_quote: None,
            book_bid_levels: Vec::with_capacity(16),
            book_ask_levels: Vec::with_capacity(16),
            book_history: VecDeque::with_capacity(512),
            volatility_1m: None,
            last_research_sample_at: None,
            last_live_snapshot_at: None,
        }
    }
}

fn feature_vector(components: &SignalComponents) -> Option<[f64; 4]> {
    let values = [
        components
            .trend_1m_percent
            .map(|v| (v / 0.25).clamp(-4.0, 4.0)),
        components
            .momentum_10s_percent
            .map(|v| (v / 0.12).clamp(-4.0, 4.0)),
        components.trade_flow_imbalance,
        components.book_imbalance,
    ];
    values
        .iter()
        .all(Option::is_some)
        .then(|| values.map(Option::unwrap))
}

fn train_logistic(samples: &[SignalOutcome]) -> Option<[f64; 5]> {
    let training = samples
        .iter()
        .filter_map(|sample| Some((sample.features?, (sample.return_percent > 0.0) as u8 as f64)))
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 5];
    for _ in 0..250 {
        let mut gradient = [0.0; 5];
        for (features, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>();
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 {
                gradient[index + 1] += error * features[index];
            }
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..5 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn logistic_up_probability(weights: &[f64; 5], features: &[f64; 4]) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>();
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn train_trend_logistic(samples: &[SignalOutcome]) -> Option<[f64; 6]> {
    let training = samples
        .iter()
        .filter_map(|sample| {
            Some((
                sample.features?,
                sample.trend_5m_volatility_units?.clamp(-4.0, 4.0),
                (sample.return_percent > 0.0) as u8 as f64,
            ))
        })
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 6];
    for _ in 0..250 {
        let mut gradient = [0.0; 6];
        for (features, trend, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>()
                + weights[5] * trend;
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..4 {
                gradient[index + 1] += error * features[index];
            }
            gradient[5] += error * trend;
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..6 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn trend_logistic_up_probability(weights: &[f64; 6], features: &[f64; 4], trend: f64) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>()
        + weights[5] * trend.clamp(-4.0, 4.0);
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn research_feature_vector(features: &SignalResearchFeatures) -> Option<[f64; 8]> {
    let values = [
        features
            .spread_bps
            .map(|value| (value / 10.0).clamp(0.0, 4.0)),
        features
            .microprice_offset_bps
            .map(|value| (value / 10.0).clamp(-4.0, 4.0)),
        features
            .depth_imbalance_10bps
            .map(|value| value.clamp(-1.0, 1.0)),
        features.trade_flow_5s.map(|value| value.clamp(-1.0, 1.0)),
        features.trade_flow_15s.map(|value| value.clamp(-1.0, 1.0)),
        features
            .trade_flow_acceleration
            .map(|value| value.clamp(-1.0, 1.0)),
        features
            .btc_return_1m_percent
            .map(|value| (value / 0.25).clamp(-4.0, 4.0)),
        features
            .eth_return_1m_percent
            .map(|value| (value / 0.25).clamp(-4.0, 4.0)),
    ];
    values
        .iter()
        .all(Option::is_some)
        .then(|| values.map(Option::unwrap))
}

fn train_research_logistic(
    samples: &[SignalOutcome],
    features: &HashMap<(String, u64), [f64; 8]>,
) -> Option<[f64; 9]> {
    let training = samples
        .iter()
        .filter_map(|sample| {
            Some((
                *features.get(&(sample.symbol.clone(), sample.signal_timestamp))?,
                (sample.return_percent > 0.0) as u8 as f64,
            ))
        })
        .collect::<Vec<_>>();
    if training.len() < WALK_FORWARD_MIN_TRAINING_SAMPLES {
        return None;
    }
    let mut weights = [0.0; 9];
    for _ in 0..250 {
        let mut gradient = [0.0; 9];
        for (features, target) in &training {
            let z = weights[0]
                + features
                    .iter()
                    .enumerate()
                    .map(|(index, value)| weights[index + 1] * value)
                    .sum::<f64>();
            let probability = 1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp());
            let error = probability - target;
            gradient[0] += error;
            for index in 0..8 {
                gradient[index + 1] += error * features[index];
            }
        }
        let scale = 0.04 / training.len() as f64;
        for index in 0..9 {
            weights[index] -= scale * gradient[index];
        }
    }
    Some(weights)
}

fn research_logistic_up_probability(weights: &[f64; 9], features: &[f64; 8]) -> f64 {
    let z = weights[0]
        + features
            .iter()
            .enumerate()
            .map(|(index, value)| weights[index + 1] * value)
            .sum::<f64>();
    1.0 / (1.0 + (-z.clamp(-20.0, 20.0)).exp())
}

fn sample_microprice(bid: f64, bid_size: f64, ask: f64, ask_size: f64) -> f64 {
    (ask * bid_size + bid * ask_size) / (bid_size + ask_size)
}

fn directional_net_return(
    direction: &str,
    entry_price: f64,
    exit_price: f64,
    fee_rate: f64,
) -> Option<f64> {
    if !entry_price.is_finite()
        || !exit_price.is_finite()
        || entry_price <= 0.0
        || exit_price <= 0.0
    {
        return None;
    }
    let gross_return = match direction {
        "bullish" => (exit_price / entry_price - 1.0) * 100.0,
        "bearish" => ((entry_price - exit_price) / entry_price) * 100.0,
        _ => return None,
    };
    Some(gross_return - 2.0 * fee_rate * 100.0)
}

fn stop_target_counterfactual_return(
    evaluation: &SignalEvaluation,
    direction: &str,
    stop_loss_percent: f64,
    take_profit_percent: f64,
) -> Option<f64> {
    if !stop_loss_percent.is_finite()
        || stop_loss_percent <= 0.0
        || !take_profit_percent.is_finite()
        || take_profit_percent <= 0.0
    {
        return None;
    }
    let (entry_bid, entry_ask) = evaluation.entry_bid.zip(evaluation.entry_ask)?;
    if !entry_bid.is_finite() || !entry_ask.is_finite() || entry_bid <= 0.0 || entry_ask < entry_bid
    {
        return None;
    }
    let half_spread = (entry_ask - entry_bid) / 2.0;
    let (best_net, worst_net, terminal_net) = match direction {
        "bullish" => (
            directional_net_return(
                "bullish",
                entry_ask,
                evaluation.path_high - half_spread,
                evaluation.taker_fee_rate,
            )?,
            directional_net_return(
                "bullish",
                entry_ask,
                evaluation.path_low - half_spread,
                evaluation.taker_fee_rate,
            )?,
            evaluation.long_net_percent,
        ),
        "bearish" => (
            directional_net_return(
                "bearish",
                entry_bid,
                evaluation.path_low + half_spread,
                evaluation.taker_fee_rate,
            )?,
            directional_net_return(
                "bearish",
                entry_bid,
                evaluation.path_high + half_spread,
                evaluation.taker_fee_rate,
            )?,
            evaluation.short_net_percent,
        ),
        _ => return None,
    };
    let target_hit = best_net >= take_profit_percent;
    let stop_hit = worst_net <= -stop_loss_percent;
    match (target_hit, stop_hit) {
        // If both barriers were touched and ordering is unavailable, count the
        // stop first. This keeps the counterfactual from overstating edge.
        (_, true) => Some(-stop_loss_percent),
        (true, false) => Some(take_profit_percent),
        (false, false) => terminal_net.filter(|value| value.is_finite()),
    }
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

fn breakout_watch(
    components: &SignalComponents,
    range_bounds: Option<(f64, f64)>,
    range_probability: Option<f64>,
) -> Option<(&'static str, f64)> {
    if range_probability? < 0.65 {
        return None;
    }
    let (lower, upper) = range_bounds?;
    let factors = [
        (
            components
                .trend_1m_percent
                .map(|value| (value / 0.25).clamp(-1.0, 1.0)),
            0.15,
        ),
        (
            components
                .momentum_10s_percent
                .map(|value| (value / 0.12).clamp(-1.0, 1.0)),
            0.20,
        ),
        (components.trade_flow_imbalance, 0.25),
        (components.book_imbalance, 0.25),
        (
            components.pressure_without_price_move.map(|value| -value),
            0.15,
        ),
    ];
    let (weighted_bias, available_weight) = factors
        .iter()
        .filter_map(|(value, weight)| value.map(|value| (value * weight, *weight)))
        .fold((0.0, 0.0), |(bias, weight_sum), (value, weight)| {
            (bias + value, weight_sum + weight)
        });
    if available_weight == 0.0 {
        return None;
    }
    let bias = weighted_bias / available_weight;
    if bias.abs() < 0.3 {
        return None;
    }

    let midpoint = (lower + upper) / 2.0;
    let width_percent = if midpoint > 0.0 {
        (upper - lower) / midpoint * 100.0
    } else {
        f64::INFINITY
    };
    let expected_move = components.volatility_1m_percent.unwrap_or_default().abs() * 5.0_f64.sqrt();
    let compression = if expected_move > 0.0 {
        (1.0 - width_percent / (expected_move * 3.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pressure = (bias.abs() * (0.7 + 0.3 * compression)).clamp(0.0, 1.0);
    (pressure >= 0.35).then_some((if bias > 0.0 { "UP" } else { "DOWN" }, pressure))
}

impl SignalEngine {
    /// Gate expensive live snapshot generation independently of feed ingestion.
    pub fn live_snapshot_due(&mut self, timestamp: u64) -> bool {
        let due = self.last_live_snapshot_at.map_or(true, |last| {
            timestamp < last
                || timestamp.saturating_sub(last) >= LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS
        });
        if due {
            self.last_live_snapshot_at = Some(timestamp);
        }
        due
    }

    /// Gate research feature extraction and persistence, independently of live snapshots.
    pub fn research_sample_due(&mut self, timestamp: u64) -> bool {
        let due = self.last_research_sample_at.map_or(true, |last| {
            timestamp < last || timestamp.saturating_sub(last) >= RESEARCH_SAMPLE_INTERVAL_MS
        });
        if due {
            self.last_research_sample_at = Some(timestamp);
        }
        due
    }

    pub fn on_price(
        &mut self,
        timestamp: u64,
        price: f64,
        volatility_1m: f64,
    ) -> Vec<CompletedSignalOutcome> {
        if !price.is_finite() || price <= 0.0 {
            return Vec::new();
        }
        if self
            .prices
            .back()
            .is_some_and(|sample| timestamp < sample.0)
        {
            return Vec::new();
        }
        self.prices.push_back((timestamp, price));
        self.volatility_1m = volatility_1m.is_finite().then_some(volatility_1m);
        self.prune(timestamp);
        let mut outcomes = Vec::new();
        let mut remaining = VecDeque::new();
        while let Some(mut pending) = self.pending.pop_front() {
            if pending.first_long_positive_net_timestamp.is_none()
                || pending.first_short_positive_net_timestamp.is_none()
            {
                let quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                if let Some((_, exit_bid, exit_ask)) = quote {
                    if pending.first_long_positive_net_timestamp.is_none()
                        && pending.entry_ask.is_some_and(|entry| {
                            directional_net_return(
                                "bullish",
                                entry,
                                exit_bid,
                                pending.taker_fee_rate,
                            )
                            .is_some_and(|net| net > 0.0)
                        })
                    {
                        pending.first_long_positive_net_timestamp = Some(timestamp);
                    }
                    if pending.first_short_positive_net_timestamp.is_none()
                        && pending.entry_bid.is_some_and(|entry| {
                            directional_net_return(
                                "bearish",
                                entry,
                                exit_ask,
                                pending.taker_fee_rate,
                            )
                            .is_some_and(|net| net > 0.0)
                        })
                    {
                        pending.first_short_positive_net_timestamp = Some(timestamp);
                    }
                }
            }
            if timestamp
                >= pending
                    .timestamp
                    .saturating_add(pending.horizon_seconds * 1000)
            {
                let change = (price / pending.base_price - 1.0) * 100.0;
                let direction_match = match pending.direction.as_str() {
                    "bullish" => change > 0.0,
                    "bearish" => change < 0.0,
                    _ => false,
                };
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                let fee_rate = pending.taker_fee_rate;
                let net_return_percent = match (
                    pending.direction.as_str(),
                    pending.entry_bid,
                    pending.entry_ask,
                    exit_quote,
                ) {
                    ("bullish", _, Some(entry_ask), Some((_, exit_bid, _))) => {
                        directional_net_return("bullish", entry_ask, exit_bid, fee_rate)
                    }
                    ("bearish", Some(entry_bid), _, Some((_, _, exit_ask))) => {
                        directional_net_return("bearish", entry_bid, exit_ask, fee_rate)
                    }
                    _ => None,
                };
                let gross_directional_return = match pending.direction.as_str() {
                    "bullish" => Some(change),
                    "bearish" => Some(-change),
                    _ => None,
                };
                let execution_cost_percent = gross_directional_return
                    .zip(net_return_percent)
                    .map(|(gross, net)| gross - net);
                let trend_5m_direction = pending
                    .trend_5m_volatility_units
                    .filter(|value| value.abs() >= 0.25)
                    .map(|value| if value > 0.0 { "bullish" } else { "bearish" });
                let trend_5m_direction_match = trend_5m_direction.map(|direction| {
                    if direction == "bullish" {
                        change > 0.0
                    } else {
                        change < 0.0
                    }
                });
                let trend_5m_net_return_percent = match (
                    trend_5m_direction,
                    pending.entry_bid,
                    pending.entry_ask,
                    exit_quote,
                ) {
                    (Some("bullish"), _, Some(entry_ask), Some((_, exit_bid, _))) => {
                        directional_net_return("bullish", entry_ask, exit_bid, fee_rate)
                    }
                    (Some("bearish"), Some(entry_bid), _, Some((_, _, exit_ask))) => {
                        directional_net_return("bearish", entry_bid, exit_ask, fee_rate)
                    }
                    _ => None,
                };
                let legacy = SignalOutcome {
                    record_type: "outcome".into(),
                    symbol: pending.symbol.clone(),
                    signal_timestamp: pending.timestamp,
                    evaluated_timestamp: timestamp,
                    horizon_seconds: pending.horizon_seconds,
                    return_percent: change,
                    direction_match,
                    direction: pending.direction.clone(),
                    strength: pending.strength,
                    net_return_percent,
                    execution_cost_percent,
                    trend_5m_volatility_units: pending.trend_5m_volatility_units,
                    trend_5m_direction_match,
                    trend_5m_net_return_percent,
                    features: pending.features,
                };

                let mut path_high = pending.base_price;
                let mut path_high_timestamp = pending.timestamp;
                let mut path_low = pending.base_price;
                let mut path_low_timestamp = pending.timestamp;
                for (sample_time, sample_price) in self.prices.iter().filter(|(sample_time, _)| {
                    *sample_time >= pending.timestamp && *sample_time <= timestamp
                }) {
                    if *sample_price > path_high {
                        path_high = *sample_price;
                        path_high_timestamp = *sample_time;
                    }
                    if *sample_price < path_low {
                        path_low = *sample_price;
                        path_low_timestamp = *sample_time;
                    }
                }
                let exit_quote = self.best_quote.filter(|(quote_time, _, _)| {
                    *quote_time <= timestamp
                        && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                });
                let (exit_bid, exit_ask, exit_quote_timestamp) = exit_quote
                    .map_or((None, None, None), |(quote_time, bid, ask)| {
                        (Some(bid), Some(ask), Some(quote_time))
                    });
                let long_gross_percent = (price / pending.base_price - 1.0) * 100.0;
                let short_gross_percent =
                    ((pending.base_price - price) / pending.base_price) * 100.0;
                let long_net_percent = pending.entry_ask.zip(exit_bid).and_then(|(entry, exit)| {
                    directional_net_return("bullish", entry, exit, pending.taker_fee_rate)
                });
                let short_net_percent =
                    pending.entry_bid.zip(exit_ask).and_then(|(entry, exit)| {
                        directional_net_return("bearish", entry, exit, pending.taker_fee_rate)
                    });
                let evaluation = SignalEvaluation {
                    symbol: pending.symbol,
                    signal_timestamp: pending.timestamp,
                    evaluated_timestamp: timestamp,
                    horizon_seconds: pending.horizon_seconds,
                    signal_direction: pending.direction,
                    signal_score: pending.score,
                    ranging_probability: pending.ranging_probability,
                    reference_entry_price: pending.base_price,
                    reference_exit_price: price,
                    entry_bid: pending.entry_bid,
                    entry_ask: pending.entry_ask,
                    entry_quote_timestamp: pending.entry_quote_timestamp,
                    exit_bid,
                    exit_ask,
                    exit_quote_timestamp,
                    taker_fee_rate: pending.taker_fee_rate,
                    long_gross_percent,
                    short_gross_percent,
                    long_net_percent,
                    short_net_percent,
                    path_high,
                    path_high_timestamp,
                    path_low,
                    path_low_timestamp,
                    long_time_to_positive_net_seconds: pending
                        .first_long_positive_net_timestamp
                        .map(|first| first.saturating_sub(pending.timestamp) / 1000),
                    short_time_to_positive_net_seconds: pending
                        .first_short_positive_net_timestamp
                        .map(|first| first.saturating_sub(pending.timestamp) / 1000),
                };
                outcomes.push(CompletedSignalOutcome { legacy, evaluation });
            } else {
                remaining.push_back(pending);
            }
        }
        self.pending = remaining;
        outcomes
    }
    pub fn return_context(&self, timestamp: u64) -> (Option<f64>, Option<f64>) {
        let fresh = self.prices.back().is_some_and(|(sample_time, _)| {
            timestamp.saturating_sub(*sample_time) <= MAX_BOOK_AGE_MS
        });
        if !fresh {
            return (None, None);
        }
        (
            self.price_return(timestamp.saturating_sub(60_000)),
            self.price_return(timestamp.saturating_sub(300_000)),
        )
    }

    pub fn research_features(
        &self,
        symbol: &str,
        timestamp: u64,
        btc: (Option<f64>, Option<f64>),
        eth: (Option<f64>, Option<f64>),
    ) -> SignalResearchFeatures {
        let quote = self.best_quote.filter(|(quote_time, _, _)| {
            *quote_time <= timestamp && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
        });
        let current_book = self.book_history.iter().rev().find(|sample| {
            sample.timestamp <= timestamp
                && timestamp.saturating_sub(sample.timestamp) <= MAX_BOOK_AGE_MS
        });
        let depth = |sample: &BookHistorySample, bids: bool| {
            let levels = if bids {
                &sample.bid_levels
            } else {
                &sample.ask_levels
            };
            levels
                .iter()
                .filter(|(price, _)| {
                    ((*price / ((sample.bid + sample.ask) / 2.0) - 1.0).abs() * 10_000.0) <= 10.0
                })
                .map(|(price, size)| price * size)
                .sum::<f64>()
        };
        let (bid_notional, ask_notional) = current_book
            .map(|sample| (Some(depth(sample, true)), Some(depth(sample, false))))
            .unwrap_or((None, None));
        let imbalance = bid_notional
            .zip(ask_notional)
            .and_then(|(bid, ask)| (bid + ask > 0.0).then_some((bid - ask) / (bid + ask)));
        let imbalance_at = |cutoff: u64| {
            self.book_history
                .iter()
                .rev()
                .find(|sample| sample.timestamp <= cutoff)
                .and_then(|sample| {
                    let bid = depth(sample, true);
                    let ask = depth(sample, false);
                    (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
                })
        };
        let current_imbalance = current_book.and_then(|sample| {
            let bid = depth(sample, true);
            let ask = depth(sample, false);
            (bid + ask > 0.0).then_some((bid - ask) / (bid + ask))
        });
        let current_mid = quote.map(|(_, bid, ask)| (bid + ask) / 2.0);
        let top_sizes = current_book
            .and_then(|sample| Some((*sample.bid_levels.first()?, *sample.ask_levels.first()?)));
        let microprice = current_mid.zip(top_sizes).and_then(|(mid, (bid, ask))| {
            (bid.1 + ask.1 > 0.0)
                .then_some(((sample_microprice(bid.0, bid.1, ask.0, ask.1) / mid) - 1.0) * 10_000.0)
        });
        let spread = quote.and_then(|(_, bid, ask)| {
            ((bid + ask) > 0.0).then_some((ask - bid) / ((ask + bid) / 2.0) * 10_000.0)
        });
        let flow = |window_ms: u64| {
            let start = timestamp.saturating_sub(window_ms);
            let mut signed = 0.0;
            let mut total = 0.0;
            let mut count = 0u64;
            for (_, price, size, is_buy, _) in self
                .trades
                .iter()
                .filter(|(trade_time, ..)| *trade_time >= start && *trade_time <= timestamp)
            {
                let notional = price * size;
                total += notional;
                signed += if *is_buy { notional } else { -notional };
                count += 1;
            }
            (total > 0.0).then_some((signed / total, signed, total, count))
        };
        let flow5 = flow(5_000);
        let flow15 = flow(15_000);
        let flow60 = flow(60_000);
        let signed5 = flow5.map(|(_, signed, _, _)| signed);
        let signed15 = flow15.map(|(_, signed, _, _)| signed);
        let trade_count_5s = flow5.map_or(0, |(_, _, _, count)| count);
        let trade_count_60s = flow60.map_or(0, |(_, _, _, count)| count);
        let average_trade_notional_5s =
            flow5.and_then(|(_, _, total, count)| (count > 0).then_some(total / count as f64));
        let price_move_bps = self
            .price_return(timestamp.saturating_sub(5_000))
            .map(|value| value * 100.0);
        SignalResearchFeatures {
            symbol: symbol.to_owned(),
            timestamp,
            spread_bps: spread,
            microprice_offset_bps: microprice,
            bid_notional_10bps: bid_notional,
            ask_notional_10bps: ask_notional,
            depth_imbalance_10bps: imbalance,
            depth_imbalance_change_1s: current_imbalance
                .zip(imbalance_at(timestamp.saturating_sub(1_000)))
                .map(|(now, old)| now - old),
            depth_imbalance_change_5s: current_imbalance
                .zip(imbalance_at(timestamp.saturating_sub(5_000)))
                .map(|(now, old)| now - old),
            trade_flow_5s: flow5.map(|(value, _, _, _)| value),
            trade_flow_15s: flow15.map(|(value, _, _, _)| value),
            trade_flow_60s: flow60.map(|(value, _, _, _)| value),
            trade_flow_acceleration: flow5
                .map(|(value, _, _, _)| value)
                .zip(flow60.map(|(value, _, _, _)| value))
                .map(|(short, long)| short - long),
            signed_trade_notional_5s: signed5,
            signed_trade_notional_15s: signed15,
            trade_count_5s,
            trade_count_60s,
            average_trade_notional_5s,
            price_response_bps_per_1000usd_5s: price_move_bps
                .zip(signed5)
                .filter(|(_, signed)| signed.abs() > 1e-9)
                .map(|(move_bps, signed)| move_bps * 1_000.0 / signed.abs().max(1.0)),
            btc_return_1m_percent: btc.0,
            btc_return_5m_percent: btc.1,
            eth_return_1m_percent: eth.0,
            eth_return_5m_percent: eth.1,
        }
    }

    pub fn book_snapshot(&self, symbol: &str, timestamp: u64) -> Option<SignalBookSnapshot> {
        let (quote_timestamp, _, _) = self.best_quote?;
        if quote_timestamp > timestamp
            || timestamp.saturating_sub(quote_timestamp) > MAX_BOOK_AGE_MS
        {
            return None;
        }
        Some(SignalBookSnapshot {
            symbol: symbol.to_owned(),
            timestamp,
            bid_levels: self.book_bid_levels.clone(),
            ask_levels: self.book_ask_levels.clone(),
        })
    }

    pub fn track(&mut self, snapshot: &SignalSnapshot) {
        if let Some((_, base_price)) = self.prices.back().copied() {
            let quote = self.best_quote.filter(|(quote_time, _, _)| {
                *quote_time <= snapshot.timestamp
                    && snapshot.timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
            });
            // Keep short-term checks while adding swing-style horizons. Outcomes are
            // evaluated from backend price/book data and persisted with the same cost model.
            for horizon_seconds in [60, 300, 900, 3_600] {
                self.pending.push_back(PendingOutcome {
                    symbol: snapshot.symbol.clone(),
                    timestamp: snapshot.timestamp,
                    base_price,
                    direction: snapshot.direction.clone(),
                    strength: snapshot.strength,
                    horizon_seconds,
                    entry_bid: quote.map(|(_, bid, _)| bid),
                    entry_ask: quote.map(|(_, _, ask)| ask),
                    trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                    features: feature_vector(&snapshot.components),
                    score: snapshot.score,
                    ranging_probability: snapshot.ranging_probability,
                    entry_quote_timestamp: quote.map(|(quote_time, _, _)| quote_time),
                    taker_fee_rate: configured_signal_taker_fee_rate(),
                    first_long_positive_net_timestamp: None,
                    first_short_positive_net_timestamp: None,
                });
            }
        }
    }

    pub fn on_trade(&mut self, timestamp: u64, price: f64, size: f64, is_buy: bool) {
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size <= 0.0 {
            return;
        }
        let aggressor_cost_bps = self
            .best_quote
            .filter(|(quote_time, bid, ask)| {
                *quote_time <= timestamp
                    && timestamp.saturating_sub(*quote_time) <= MAX_BOOK_AGE_MS
                    && bid.is_finite()
                    && ask.is_finite()
                    && *bid > 0.0
                    && *ask >= *bid
            })
            .map(|(_, bid, ask)| {
                let midpoint = (bid + ask) / 2.0;
                let paid_vs_mid = if is_buy {
                    price - midpoint
                } else {
                    midpoint - price
                };
                paid_vs_mid / midpoint * 10_000.0
            })
            .filter(|cost| cost.is_finite());
        self.trades
            .push_back((timestamp, price, size, is_buy, aggressor_cost_bps));
        if let Some((_quote_time, bid, ask)) = self.best_quote.filter(|(qt, bid, ask)| {
            *qt <= timestamp
                && timestamp.saturating_sub(*qt) <= MAX_BOOK_AGE_MS
                && bid.is_finite()
                && ask.is_finite()
                && *bid > 0.0
                && *ask >= *bid
        }) {
            let depth_at = |levels: &[(f64, f64)], target: f64| {
                levels
                    .iter()
                    .find(|(level, _)| (*level - target).abs() <= target * 1e-8)
                    .map(|(_, quantity)| *quantity)
                    .unwrap_or(0.0)
            };
            self.absorption_trades.push_back(AbsorptionTrade {
                timestamp,
                price,
                size,
                is_buy,
                bid,
                ask,
                bid_depth: depth_at(&self.book_bid_levels, bid),
                ask_depth: depth_at(&self.book_ask_levels, ask),
            });
        }
        self.prune(timestamp);
    }

    pub fn on_book(
        &mut self,
        timestamp: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: &[f64],
        ask_sizes: &[f64],
        bid_levels: &[(f64, f64)],
        ask_levels: &[(f64, f64)],
    ) {
        if best_bid.is_finite() && best_ask.is_finite() && best_bid > 0.0 && best_ask >= best_bid {
            self.best_quote = Some((timestamp, best_bid, best_ask));
        }
        self.book_bid_levels.clear();
        self.book_bid_levels
            .extend(bid_levels.iter().copied().filter(|(price, size)| {
                price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
            }));
        self.book_ask_levels.clear();
        self.book_ask_levels
            .extend(ask_levels.iter().copied().filter(|(price, size)| {
                price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
            }));
        if best_bid > 0.0 && best_ask >= best_bid {
            let mut bid_levels = [(0.0, 0.0); 7];
            let mut ask_levels = [(0.0, 0.0); 7];
            for (target, source) in bid_levels.iter_mut().zip(self.book_bid_levels.iter()) {
                *target = *source;
            }
            for (target, source) in ask_levels.iter_mut().zip(self.book_ask_levels.iter()) {
                *target = *source;
            }
            self.book_history.push_back(BookHistorySample {
                timestamp,
                bid: best_bid,
                ask: best_ask,
                bid_levels,
                ask_levels,
            });
            while self
                .book_history
                .front()
                .is_some_and(|sample| sample.timestamp < timestamp.saturating_sub(60_000))
            {
                self.book_history.pop_front();
            }
        }
        let bid: f64 = bid_sizes
            .iter()
            .copied()
            .filter(|n| n.is_finite() && *n > 0.0)
            .sum();
        let ask: f64 = ask_sizes
            .iter()
            .copied()
            .filter(|n| n.is_finite() && *n > 0.0)
            .sum();
        if bid + ask > 0.0 {
            self.book_imbalance = Some((bid - ask) / (bid + ask));
        }
    }

    pub fn price_state(&self) -> (usize, Option<(u64, f64)>) {
        (self.prices.len(), self.prices.back().copied())
    }

    pub fn input_diagnostic(&self, timestamp: u64) -> String {
        let latest = self.prices.back().copied();
        let recent_trades = self
            .trades
            .iter()
            .filter(|(trade_time, ..)| *trade_time >= timestamp.saturating_sub(TRADE_WINDOW_MS) && *trade_time <= timestamp)
            .count();
        let quote_age = self
            .best_quote
            .map(|(quote_time, _, _)| timestamp.saturating_sub(quote_time));
        format!(
            "price_samples={} latest_price_timestamp={:?} trend_1m={} momentum_10s={} recent_trades_60s={} book_imbalance={:?} quote_age_ms={quote_age:?}",
            self.prices.len(),
            latest.map(|sample| sample.0),
            self.price_return(timestamp.saturating_sub(60_000)).is_some(),
            self.price_return(timestamp.saturating_sub(10_000)).is_some(),
            recent_trades,
            self.book_imbalance,
        )
    }

    pub fn snapshot(&mut self, symbol: &str, now: u64) -> Option<SignalSnapshot> {
        self.prune(now);
        let latest = self.prices.back().copied()?;
        let range_cutoff = now.saturating_sub(300_000);
        let mut range_count = 0usize;
        let mut range_first_price = None;
        let mut range_low = f64::INFINITY;
        let mut range_high = f64::NEG_INFINITY;
        for &(timestamp, price) in &self.prices {
            if timestamp < range_cutoff {
                continue;
            }
            range_count += 1;
            range_first_price.get_or_insert(price);
            range_low = range_low.min(price);
            range_high = range_high.max(price);
        }
        let range_bounds = if range_count >= 12
            && range_low.is_finite()
            && range_high.is_finite()
            && range_low > 0.0
            && range_high > range_low
        {
            Some((range_low, range_high))
        } else {
            None
        };
        let ranging_probability = range_bounds.map(|(low, high)| {
            let width_percent = (high / low - 1.0) * 100.0;
            let net_move_percent =
                (latest.1 / range_first_price.unwrap_or(latest.1) - 1.0).abs() * 100.0;
            // Heuristic only: range likelihood rises when price explores both
            // edges but makes little net progress over the five-minute window.
            let mut low_touches = 0usize;
            let mut high_touches = 0usize;
            for &(timestamp, price) in &self.prices {
                if timestamp < range_cutoff {
                    continue;
                }
                if (price / low - 1.0) * 100.0 <= width_percent * 0.15 {
                    low_touches += 1;
                }
                if (high / price - 1.0) * 100.0 <= width_percent * 0.15 {
                    high_touches += 1;
                }
            }
            let exploration =
                ((low_touches.min(3) + high_touches.min(3)) as f64 / 6.0).clamp(0.0, 1.0);
            let containment =
                (1.0 - net_move_percent / (width_percent * 0.6).max(0.01)).clamp(0.0, 1.0);
            (0.65 * containment + 0.35 * exploration).clamp(0.0, 1.0)
        });
        let trend = self.price_return(now.saturating_sub(60_000));
        let momentum = self.price_return(now.saturating_sub(10_000));
        let trend_5m = self.price_return(now.saturating_sub(300_000));
        let volatility_5m = self.realized_volatility(now.saturating_sub(300_000));
        let trend_5m_volatility_units = trend_5m
            .zip(volatility_5m)
            .and_then(|(trend, volatility)| (volatility > 1e-9).then_some(trend / volatility));
        let trade_cutoff = now.saturating_sub(60_000);
        let (mut buy, mut sell) = (0.0, 0.0);
        let (mut recent_notional, mut recent_quantity) = (0.0, 0.0);
        let (mut recent_count, mut recent_first, mut recent_last) = (0usize, None, None);
        let (mut weighted_cost, mut cost_notional) = (0.0, 0.0);
        for &(timestamp, price, size, is_buy, aggressor_cost) in &self.trades {
            let notional = price * size;
            if is_buy {
                buy += notional;
            } else {
                sell += notional;
            }
            if timestamp >= trade_cutoff {
                recent_count += 1;
                recent_first.get_or_insert(timestamp);
                recent_last = Some(timestamp);
                recent_notional += notional;
                recent_quantity += size;
                if let Some(cost) = aggressor_cost {
                    weighted_cost += cost * notional;
                    cost_notional += notional;
                }
            }
        }
        let flow = (buy + sell > 0.0).then_some((buy - sell) / (buy + sell));
        let mispricing_vs_vwap_bps = if recent_count >= 5
            && recent_last
                .zip(recent_first)
                .is_some_and(|(last, first)| last.saturating_sub(first) >= 10_000)
            && recent_quantity > 0.0
        {
            let vwap = recent_notional / recent_quantity;
            (vwap.is_finite() && vwap > 0.0).then_some((latest.1 / vwap - 1.0) * 10_000.0)
        } else {
            None
        };
        let flow_without_move = flow.zip(momentum).and_then(|(imbalance, movement)| {
            (imbalance.abs() >= 0.55 && movement.abs() <= 0.025).then_some(-imbalance)
        });
        let aggressor_cost_bps = (cost_notional > 0.0).then_some(weighted_cost / cost_notional);
        // Count only market sells hitting the bid or buys lifting the ask when
        // the displayed quantity at that exact level later replenishes.
        let (mut absorbed_sell, mut absorbed_buy) = (0.0, 0.0);
        for trade in self
            .absorption_trades
            .iter()
            .filter(|t| t.timestamp >= now.saturating_sub(ABSORPTION_WINDOW_MS))
        {
            let (touch, prior_depth, bid_side) = if trade.is_buy {
                (trade.ask, trade.ask_depth, false)
            } else {
                (trade.bid, trade.bid_depth, true)
            };
            if prior_depth <= 0.0 || (trade.price - touch).abs() > touch * 2.0 / 10_000.0 {
                continue;
            }
            let replenished = self.book_history.iter().any(|book| {
                let (level, levels) = if bid_side {
                    (book.bid, &book.bid_levels)
                } else {
                    (book.ask, &book.ask_levels)
                };
                book.timestamp >= trade.timestamp
                    && book.timestamp <= trade.timestamp.saturating_add(2_000)
                    && (level - touch).abs() <= touch * 1e-8
                    && levels
                        .iter()
                        .find(|(px, _)| (*px - touch).abs() <= touch * 1e-8)
                        .is_some_and(|(_, qty)| *qty >= prior_depth + trade.size * 0.5)
            });
            if replenished {
                if bid_side {
                    absorbed_sell += trade.price * trade.size;
                } else {
                    absorbed_buy += trade.price * trade.size;
                }
            }
        }
        let absorption_imbalance = if momentum.is_some_and(|m| m.abs() <= 0.025)
            && absorbed_sell + absorbed_buy > 0.0
        {
            Some(((absorbed_sell - absorbed_buy) / (absorbed_sell + absorbed_buy)).clamp(-1.0, 1.0))
        } else {
            None
        };
        // Reuse the existing serialized field to keep persisted bincode records compatible.
        let pressure_without_price_move = absorption_imbalance.or(flow_without_move);
        let components = SignalComponents {
            trend_1m_percent: trend,
            momentum_10s_percent: momentum,
            trade_flow_imbalance: flow,
            book_imbalance: self.book_imbalance,
            volatility_1m_percent: self
                .volatility_1m
                .or_else(|| self.realized_volatility(now.saturating_sub(60_000))),
            trend_5m_percent: trend_5m,
            trend_5m_volatility_units,
            mispricing_vs_vwap_bps,
            pressure_without_price_move,
            aggressor_cost_bps,
        };
        let mispricing_signal =
            mispricing_vs_vwap_bps.map(|value| (-value / 20.0).clamp(-1.0, 1.0));
        // In a confidently ranging market, price near a range edge is a
        // contrarian mean-reversion opportunity. Keep this deliberately
        // gated so the edge vote does not fade moves in a trending regime.
        let mean_reversion_signal =
            range_bounds
                .zip(ranging_probability)
                .and_then(|((low, high), probability)| {
                    if probability < 0.70 {
                        return None;
                    }
                    let width = high - low;
                    let location = (latest.1 - low) / width;
                    if location <= 0.12 {
                        Some(0.7)
                    } else if location >= 0.88 {
                        Some(-0.7)
                    } else {
                        None
                    }
                });
        // Treat book pressure as actionable only when recent aggressive trades
        // confirm the same side. This reduces signals from transient displayed
        // liquidity that is not being consumed or supported by trade flow.
        let order_book_pressure_signal = self.book_imbalance.zip(flow).and_then(|(book, flow)| {
            if book >= 0.60 && flow >= 0.25 {
                Some(0.8)
            } else if book <= -0.60 && flow <= -0.25 {
                Some(-0.8)
            } else {
                None
            }
        });
        // Compare the latest price with an earlier five-minute range that
        // excludes the most recent ten seconds, so a new breakout can actually
        // cross the boundary. Require both short-term momentum and trade flow
        // to confirm direction, plus a volatility-scaled minimum clearance.
        let breakout_cutoff = now.saturating_sub(10_000);
        let mut prior_low = f64::INFINITY;
        let mut prior_high = f64::NEG_INFINITY;
        let mut prior_count = 0usize;
        for &(timestamp, price) in &self.prices {
            if timestamp >= range_cutoff && timestamp < breakout_cutoff {
                prior_low = prior_low.min(price);
                prior_high = prior_high.max(price);
                prior_count += 1;
            }
        }
        let breakout_clearance_percent = self
            .volatility_1m
            .or_else(|| self.realized_volatility(now.saturating_sub(60_000)))
            .map(|volatility| (volatility.abs() * 0.20).max(0.02))
            .unwrap_or(0.02);
        let volatility_breakout_signal = if prior_count >= 12 {
            momentum.zip(flow).and_then(|(move_percent, imbalance)| {
                if latest.1 > prior_high * (1.0 + breakout_clearance_percent / 100.0)
                    && move_percent > 0.0
                    && imbalance >= 0.30
                {
                    Some(0.8)
                } else if latest.1 < prior_low * (1.0 - breakout_clearance_percent / 100.0)
                    && move_percent < 0.0
                    && imbalance <= -0.30
                {
                    Some(-0.8)
                } else {
                    None
                }
            })
        } else {
            None
        };
        let inputs = [
            trend.map(|v| (v / 0.25).clamp(-1.0, 1.0)),
            momentum.map(|v| (v / 0.12).clamp(-1.0, 1.0)),
            flow,
            self.book_imbalance,
            mispricing_signal,
            pressure_without_price_move,
            mean_reversion_signal,
            order_book_pressure_signal,
            volatility_breakout_signal,
        ];
        let count = inputs.iter().filter(|v| v.is_some()).count();
        if count == 0 {
            return None;
        }
        let score = inputs.iter().flatten().sum::<f64>() / count as f64;
        let direction = if score > 0.2 {
            "bullish"
        } else if score < -0.2 {
            "bearish"
        } else if ranging_probability.is_some_and(|probability| probability >= 0.65) {
            "ranging"
        } else {
            "neutral"
        };
        let mut reasons = Vec::new();
        if let Some(v) = trend {
            reasons.push(format!("1m price change {v:.3}%"));
        }
        if let Some(v) = momentum {
            reasons.push(format!("10s price change {v:.3}%"));
        }
        if let Some(v) = flow {
            reasons.push(format!("buy/sell trade imbalance {v:.2}"));
        }
        if let Some(v) = self.book_imbalance {
            reasons.push(format!("top-book imbalance {v:.2}"));
        }
        if let Some(vote) = order_book_pressure_signal {
            reasons.push(format!(
                "strategy_vote:order_book_pressure;direction={};vote={vote:.2};book_imbalance={:.3};trade_flow={:.3};thresholds=abs_book>=0.600,abs_flow>=0.250",
                if vote > 0.0 { "bullish" } else { "bearish" },
                self.book_imbalance.unwrap_or_default(),
                flow.unwrap_or_default()
            ));
        }
        if let Some(vote) = volatility_breakout_signal {
            reasons.push(format!(
                "strategy_vote:volatility_breakout;direction={};vote={vote:.2};price={:.8};prior_low={prior_low:.8};prior_high={prior_high:.8};clearance_percent={breakout_clearance_percent:.4};momentum_10s_percent={:.4};trade_flow={:.3};thresholds=prior_samples>=12,clearance>=max(0.20*volatility_1m,0.0200%),abs_trade_flow>=0.300",
                if vote > 0.0 { "bullish" } else { "bearish" },
                latest.1,
                momentum.unwrap_or_default(),
                flow.unwrap_or_default()
            ));
        }
        if let Some(v) = mispricing_vs_vwap_bps.filter(|value| value.abs() >= 8.0) {
            let relation = if v > 0.0 { "above" } else { "below" };
            reasons.push(format!(
                "Price {:.1} bps {relation} 60s trade VWAP",
                v.abs()
            ));
        }
        if let Some((low, high)) = range_bounds {
            if let Some(vote) = mean_reversion_signal {
                let location = (latest.1 - low) / (high - low);
                reasons.push(format!(
                    "strategy_vote:mean_reversion;direction={};vote={vote:.2};price={:.8};range_low={low:.8};range_high={high:.8};range_location={location:.3};ranging_probability={:.3};thresholds=range_probability>=0.700,edge_fraction=0.120",
                    if vote > 0.0 { "bullish" } else { "bearish" },
                    latest.1,
                    ranging_probability.unwrap_or_default()
                ));
            }
        }
        if let Some(v) = absorption_imbalance.filter(|value| value.abs() >= 0.25) {
            reasons.push(format!(
                "{} aggression met replenishing touch liquidity while price stayed flat",
                if v > 0.0 { "Sell" } else { "Buy" }
            ));
        } else if let Some(v) = flow_without_move {
            let pressure = if v < 0.0 { "buying" } else { "selling" };
            reasons.push(format!(
                "Strong {pressure} pressure met little price movement"
            ));
        }
        if let Some(v) = aggressor_cost_bps.filter(|value| *value >= 5.0) {
            reasons.push(format!(
                "Aggressive traders paid an average {:.1} bps versus midpoint",
                v
            ));
        }
        if direction == "ranging" {
            if let Some((low, high)) = range_bounds {
                reasons.push(format!(
                    "Price is ranging between {:.8} and {:.8} over five minutes",
                    low, high
                ));
                if let Some((side, pressure)) =
                    breakout_watch(&components, range_bounds, ranging_probability)
                {
                    reasons.push(format!(
                        "Breakout watch: {side} · {:.0}% pressure (heuristic, not calibrated)",
                        pressure * 100.0
                    ));
                }
            }
        }
        Some(SignalSnapshot {
            symbol: symbol.to_owned(),
            timestamp: now,
            direction: direction.to_owned(),
            score,
            strength: (score.abs() * count as f64 / 9.0).clamp(0.0, 1.0),
            components,
            reasons,
            stale: now.saturating_sub(latest.0) > 5_000,
            ranging_probability,
            range_lower_bound: range_bounds.map(|(low, _)| low),
            range_upper_bound: range_bounds.map(|(_, high)| high),
        })
    }

    fn realized_volatility(&self, cutoff: u64) -> Option<f64> {
        let mut previous: Option<f64> = None;
        let mut count = 0usize;
        let mut sum = 0.0f64;
        for &(timestamp, price) in &self.prices {
            if timestamp < cutoff {
                continue;
            }
            count += 1;
            if let Some(previous_price) = previous {
                let change = (price / previous_price).ln();
                sum += change * change;
            }
            previous = Some(price);
        }
        (count >= 2).then_some(sum.sqrt() * 100.0)
    }

    fn price_return(&self, cutoff: u64) -> Option<f64> {
        let latest = self.prices.back()?.1;
        let index = self.prices.partition_point(|sample| sample.0 <= cutoff);
        if index == 0 {
            return None;
        }
        let (sample_time, first) = self.prices[index - 1];
        if cutoff.saturating_sub(sample_time) > MAX_PRICE_LOOKBACK_GAP_MS {
            return None;
        }
        Some((latest / first - 1.0) * 100.0)
    }

    fn prune(&mut self, now: u64) {
        while self
            .prices
            .front()
            .is_some_and(|sample| sample.0 < now.saturating_sub(PRICE_WINDOW_MS))
        {
            self.prices.pop_front();
        }
        while self
            .trades
            .front()
            .is_some_and(|sample| sample.0 < now.saturating_sub(TRADE_WINDOW_MS))
        {
            self.trades.pop_front();
        }
        while self
            .absorption_trades
            .front()
            .is_some_and(|sample| sample.timestamp < now.saturating_sub(ABSORPTION_WINDOW_MS))
        {
            self.absorption_trades.pop_front();
        }
    }
}

fn select_strength_threshold(training: &[SignalOutcome], evaluated_by: u64) -> Option<f64> {
    STRENGTH_THRESHOLDS
        .iter()
        .copied()
        .filter_map(|threshold| {
            let (net_total, selected_count) = training
                .iter()
                .filter(|sample| {
                    sample.evaluated_timestamp <= evaluated_by && sample.strength >= threshold
                })
                .fold((0.0, 0usize), |(total, count), sample| {
                    (
                        total + sample.net_return_percent.unwrap_or_default(),
                        count + 1,
                    )
                });
            if selected_count < WALK_FORWARD_MIN_THRESHOLD_SAMPLES {
                return None;
            }
            let mean = net_total / selected_count as f64;
            (mean > 0.0).then_some((threshold, mean))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1).then(left.0.total_cmp(&right.0)))
        .map(|(threshold, _)| threshold)
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
    #[serde(rename = "missedOpportunityBands", default)]
    pub missed_opportunity_bands: Vec<MissedOpportunityGateBand>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MissedOpportunityGateBand {
    pub gate: String,
    pub threshold: f64,
    #[serde(default)]
    pub horizon_seconds: u64,
    pub sample_count: usize,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    #[serde(default)]
    pub time_to_profit_sample_count: usize,
    #[serde(default)]
    pub time_to_profit_rate: Option<f64>,
    #[serde(default)]
    pub average_time_to_profit_seconds: Option<f64>,
    #[serde(default)]
    pub median_time_to_profit_seconds: Option<f64>,
    #[serde(default)]
    pub stop_target_sample_count: usize,
    #[serde(default)]
    pub stop_target_average_net_return_percent: Option<f64>,
    #[serde(default)]
    pub stop_target_positive_net_rate: Option<f64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoTraderExecutionQualitySummary {
    pub order_samples: usize,
    pub filled_order_samples: usize,
    pub partial_fill_samples: usize,
    pub unfilled_order_samples: usize,
    pub measured_fill_samples: usize,
    pub reference_price_coverage_percent: Option<f64>,
    pub average_fill_rate: Option<f64>,
    pub average_implementation_shortfall_bps: Option<f64>,
    pub average_fee_bps: Option<f64>,
    pub average_all_in_cost_bps: Option<f64>,
    pub average_submit_response_ms: Option<f64>,
    pub response_latency_samples: usize,
    pub history_truncated: bool,
    pub last_updated_at: Option<u64>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct AutoTraderDecisionQualitySummary {
    pub decision_samples: usize,
    pub filled_entry_samples: usize,
    pub unresolved_trade_samples: usize,
    pub unpriced_closed_trade_samples: usize,
    pub completed_trade_samples: usize,
    pub scout_filled_entry_samples: usize,
    pub scout_unresolved_trade_samples: usize,
    pub scout_unpriced_closed_trade_samples: usize,
    pub scout_completed_trade_samples: usize,
    pub history_truncated: bool,
    pub cohorts: Vec<AutoTraderDecisionQualityCohort>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AutoTraderDecisionQualityCohort {
    pub dimension: String,
    pub value: String,
    pub completed_trade_samples: usize,
    pub average_net_return_percent: f64,
    pub win_rate: f64,
    pub total_net_pnl: f64,
    pub preliminary: bool,
}

#[derive(Default)]
struct DecisionTradeAudit {
    entry_quantity: f64,
    entry_price: Option<f64>,
    exploratory: Option<bool>,
    scout: bool,
    direction: Option<String>,
    strength: Option<f64>,
    market_context: Option<String>,
    entry_timestamp: Option<u64>,
    signal_components: Option<serde_json::Value>,
    exit_quantity: f64,
    realized_pnl: f64,
}

#[derive(Deserialize)]
struct EvaluationQuery {
    since: Option<u64>,
    limit: Option<usize>,
}

#[derive(Clone)]
pub struct SignalStore {
    path: PathBuf,
    writer: mpsc::Sender<WriteRequest>,
    recent_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>,
    best_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>,
    performance_cache: Arc<Mutex<Option<(Instant, Vec<SignalPerformance>)>>>,
    walk_forward_cache: Arc<Mutex<Option<(Instant, Vec<SignalWalkForward>)>>>,
    missed_opportunity_cache: Arc<Mutex<Option<(Instant, Vec<MissedOpportunityGateBand>)>>>,
    walk_forward_refreshing: Arc<AtomicBool>,
}

struct WriteRequest {
    operation: WriteOperation,
    ack: oneshot::Sender<std::result::Result<(), String>>,
}

enum WriteOperation {
    Records(Vec<StoredSignalRecord>),
    Audit {
        decision_id: String,
        stage: String,
        timestamp: i64,
        symbol: String,
        payload: Vec<u8>,
    },
}

#[derive(Serialize, Deserialize)]
pub enum StoredSignalRecord {
    Snapshot(SignalSnapshot),
    Outcome(SignalOutcome),
    Evaluation(SignalEvaluation),
    BookSnapshot(SignalBookSnapshot),
    ResearchFeatures(SignalResearchFeatures),
    ExternalMarket(ExternalMarketFeatures),
}

// A short-lived development build added this trailing component before it was
// removed for bincode compatibility. Accept those rows too so the dashboard
// can read data written during that build.
#[derive(Deserialize)]
struct ExtendedSignalComponents {
    trend_1m_percent: Option<f64>,
    momentum_10s_percent: Option<f64>,
    trade_flow_imbalance: Option<f64>,
    book_imbalance: Option<f64>,
    volatility_1m_percent: Option<f64>,
    trend_5m_percent: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
    mispricing_vs_vwap_bps: Option<f64>,
    pressure_without_price_move: Option<f64>,
    aggressor_cost_bps: Option<f64>,
    absorption_imbalance: Option<f64>,
}
#[derive(Deserialize)]
struct ExtendedSignalSnapshot {
    symbol: String,
    timestamp: u64,
    direction: String,
    score: f64,
    strength: f64,
    components: ExtendedSignalComponents,
    reasons: Vec<String>,
    stale: bool,
    ranging_probability: Option<f64>,
    range_lower_bound: Option<f64>,
    range_upper_bound: Option<f64>,
}
#[derive(Deserialize)]
enum ExtendedSnapshotRecord {
    Snapshot(ExtendedSignalSnapshot),
}

fn deserialize_record(kind: i64, payload: &[u8]) -> Result<StoredSignalRecord> {
    if let Ok(record) = bincode::deserialize(payload) {
        return Ok(record);
    }
    if kind == 0 {
        let ExtendedSnapshotRecord::Snapshot(snapshot) = bincode::deserialize(payload)?;
        return Ok(StoredSignalRecord::Snapshot(SignalSnapshot {
            symbol: snapshot.symbol,
            timestamp: snapshot.timestamp,
            direction: snapshot.direction,
            score: snapshot.score,
            strength: snapshot.strength,
            components: SignalComponents {
                trend_1m_percent: snapshot.components.trend_1m_percent,
                momentum_10s_percent: snapshot.components.momentum_10s_percent,
                trade_flow_imbalance: snapshot.components.trade_flow_imbalance,
                book_imbalance: snapshot.components.book_imbalance,
                volatility_1m_percent: snapshot.components.volatility_1m_percent,
                trend_5m_percent: snapshot.components.trend_5m_percent,
                trend_5m_volatility_units: snapshot.components.trend_5m_volatility_units,
                mispricing_vs_vwap_bps: snapshot.components.mispricing_vs_vwap_bps,
                pressure_without_price_move: snapshot
                    .components
                    .pressure_without_price_move
                    .or(snapshot.components.absorption_imbalance),
                aggressor_cost_bps: snapshot.components.aggressor_cost_bps,
            },
            reasons: snapshot.reasons,
            stale: snapshot.stale,
            ranging_probability: snapshot.ranging_probability,
            range_lower_bound: snapshot.range_lower_bound,
            range_upper_bound: snapshot.range_upper_bound,
        }));
    }
    Ok(bincode::deserialize(payload)?)
}

fn compress_record(record: &StoredSignalRecord) -> Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(&bincode::serialize(record)?)?;
    Ok(encoder.finish()?)
}

fn record_kind(record: &StoredSignalRecord) -> i64 {
    match record {
        StoredSignalRecord::Snapshot(_) => 0,
        StoredSignalRecord::Outcome(_) => 1,
        StoredSignalRecord::Evaluation(_) => 2,
        StoredSignalRecord::BookSnapshot(_) => 3,
        StoredSignalRecord::ResearchFeatures(_) => 4,
        StoredSignalRecord::ExternalMarket(_) => 5,
    }
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

fn accuracy_bucket(snapshot: &SignalSnapshot) -> u8 {
    let value = if snapshot.direction == "ranging" {
        snapshot.ranging_probability.unwrap_or_default()
    } else {
        snapshot.strength
    };
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
            StoredSignalRecord::Snapshot(snapshot) => history
                .entry(snapshot.symbol.clone())
                .or_default()
                .push(snapshot),
            StoredSignalRecord::Outcome(outcome) => outcomes.push(outcome),
            StoredSignalRecord::Evaluation(_)
            | StoredSignalRecord::BookSnapshot(_)
            | StoredSignalRecord::ResearchFeatures(_)
            | StoredSignalRecord::ExternalMarket(_) => {}
        }
    }
    for items in history.values_mut() {
        items.sort_by_key(|snapshot| snapshot.timestamp);
    }

    for (symbol, items) in &history {
        for prediction in items.iter().filter(|item| item.direction == "ranging") {
            let (Some(lower), Some(upper)) =
                (prediction.range_lower_bound, prediction.range_upper_bound)
            else {
                continue;
            };
            let target = prediction.timestamp.saturating_add(300_000);
            let index = items.partition_point(|item| item.timestamp < target);
            let Some(observed) = items
                .get(index)
                .filter(|item| item.timestamp <= target.saturating_add(15_000))
            else {
                continue;
            };
            let (Some(future_low), Some(future_high)) =
                (observed.range_lower_bound, observed.range_upper_bound)
            else {
                continue;
            };
            let key = (
                symbol.clone(),
                "ranging".to_owned(),
                accuracy_bucket(prediction),
            );
            let entry = accuracies.entry(key).or_default();
            entry.0 += 1;
            if future_low >= lower && future_high <= upper {
                entry.1 += 1;
            }
        }
    }
    for outcome in outcomes.iter().filter(|outcome| {
        outcome.horizon_seconds == 60 && matches!(outcome.direction.as_str(), "bullish" | "bearish")
    }) {
        let bucket = (outcome.strength.clamp(0.0, 0.999) * 10.0) as u8;
        let entry = outcomes_by_key
            .entry((outcome.symbol.clone(), outcome.direction.clone(), bucket))
            .or_default();
        entry.0 += 1;
        if outcome.direction_match {
            entry.1 += 1;
        }
    }

    // Combine adjacent score buckets so each estimate uses more historical examples.
    fn estimate(
        map: &HashMap<(String, String, u8), (usize, usize)>,
        snapshot: &SignalSnapshot,
    ) -> f64 {
        let bucket = accuracy_bucket(snapshot);
        let mut total = (0usize, 0usize);
        for adjacent in bucket.saturating_sub(1)..=bucket.saturating_add(1).min(9) {
            if let Some((samples, hits)) = map.get(&(
                snapshot.symbol.clone(),
                snapshot.direction.clone(),
                adjacent,
            )) {
                total.0 += samples;
                total.1 += hits;
            }
        }
        (total.1 as f64 + PRIOR_SAMPLES * 0.5) / (total.0 as f64 + PRIOR_SAMPLES)
    }

    // The beta prior keeps small samples near 50% until enough resolved forecasts exist.
    for snapshot in snapshots {
        snapshot.strength = if snapshot.direction == "ranging" {
            estimate(&accuracies, snapshot)
        } else {
            estimate(&outcomes_by_key, snapshot)
        };
    }
}

impl SignalStore {
    pub fn from_environment() -> Result<Self> {
        let path = std::env::var_os("SIGNALS_DB_FILE")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("SIGNALS_FILE")
                    .map(|path| PathBuf::from(path).with_extension("sqlite"))
            })
            .unwrap_or(std::env::current_dir()?.join("signals.sqlite"));
        let connection = Self::open_database_at(&path)?;
        let (writer, receiver) = mpsc::channel(4096);
        tokio::task::spawn_blocking(move || run_database_writer(connection, receiver));
        Ok(Self {
            path,
            writer,
            recent_cache: Arc::new(Mutex::new(None)),
            best_cache: Arc::new(Mutex::new(None)),
            performance_cache: Arc::new(Mutex::new(None)),
            walk_forward_cache: Arc::new(Mutex::new(None)),
            missed_opportunity_cache: Arc::new(Mutex::new(None)),
            walk_forward_refreshing: Arc::new(AtomicBool::new(false)),
        })
    }

    fn open_database_at(path: &PathBuf) -> Result<Connection> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL; CREATE TABLE IF NOT EXISTS signal_records (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, kind INTEGER NOT NULL, payload BLOB NOT NULL); CREATE INDEX IF NOT EXISTS signal_records_timestamp ON signal_records(timestamp); CREATE INDEX IF NOT EXISTS signal_records_kind_timestamp ON signal_records(kind, timestamp); CREATE TABLE IF NOT EXISTS auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage)); CREATE INDEX IF NOT EXISTS auto_trader_audit_timestamp ON auto_trader_audit(timestamp);")?;
        Ok(connection)
    }

    fn open_database_path(path: &PathBuf) -> Result<Connection> {
        let connection = Connection::open(path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch("PRAGMA synchronous=NORMAL;")?;
        Ok(connection)
    }

    async fn read_records(
        &self,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<StoredSignalRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || Self::read_records_sync(&path, kind, since, limit))
            .await
            .map_err(|error| anyhow::anyhow!("signal read task failed: {error}"))?
    }

    fn read_records_sync(
        path: &PathBuf,
        kind: i64,
        since: Option<u64>,
        limit: Option<usize>,
    ) -> Result<Vec<StoredSignalRecord>> {
        let connection = Self::open_database_path(path)?;
        let mut sql = String::from("SELECT payload FROM signal_records WHERE kind = ?");
        let mut values = vec![rusqlite::types::Value::Integer(kind)];
        if let Some(timestamp) = since {
            sql.push_str(" AND timestamp >= ?");
            values.push(rusqlite::types::Value::Integer(
                timestamp.min(i64::MAX as u64) as i64,
            ));
        }
        sql.push_str(" ORDER BY timestamp DESC, id DESC");
        if let Some(limit) = limit {
            sql.push_str(" LIMIT ?");
            values.push(rusqlite::types::Value::Integer(
                limit.min(i64::MAX as usize) as i64,
            ));
        }
        let compressed_records = {
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(rusqlite::params_from_iter(values.iter()), |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };
        let compressed_archives = if (0..=5).contains(&kind) {
            let since_day = since.map(|timestamp| (timestamp / 86_400_000) as i64 * 86_400_000);
            let mut statement = connection.prepare(
                "SELECT training_payload FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL AND (?2 IS NULL OR day_start_ms >= ?2)",
            )?;
            statement
                .query_map(params![kind, since_day], |row| row.get::<_, Vec<u8>>(0))?
                .collect::<rusqlite::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        drop(connection);
        let mut records = Vec::new();
        for compressed in compressed_records {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            decoder.read_to_end(&mut payload)?;
            records.push(deserialize_record(kind, &payload)?);
        }
        for compressed in compressed_archives {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut payload = Vec::new();
            decoder.read_to_end(&mut payload)?;
            let mut archived: Vec<StoredSignalRecord> = bincode::deserialize(&payload)?;
            if let Some(timestamp) = since {
                archived.retain(|record| record_timestamp(record) >= timestamp);
            }
            records.extend(archived);
        }
        records.sort_by_key(record_timestamp);
        if let Some(limit) = limit {
            let start = records.len().saturating_sub(limit);
            records.drain(..start);
        }
        Ok(records)
    }

    async fn read_recent_records(
        &self,
        kind: i64,
        limit: usize,
    ) -> Result<Vec<StoredSignalRecord>> {
        self.read_recent_records_with_corrupt_rows(kind, limit, false)
            .await
    }

    async fn read_recent_records_with_corrupt_rows(
        &self,
        kind: i64,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::read_recent_records_sync(&path, kind, limit, skip_corrupt_rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("recent signal read task failed: {error}"))?
    }

    fn read_recent_records_sync(
        path: &PathBuf,
        kind: i64,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<StoredSignalRecord>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare(
            "SELECT payload FROM signal_records WHERE kind = ?1 ORDER BY timestamp DESC, id DESC LIMIT ?2",
        )?;
        let rows = statement
            .query_map(params![kind, limit.min(i64::MAX as usize) as i64], |row| {
                row.get::<_, Vec<u8>>(0)
            })?;
        let compressed_records = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut records = Vec::new();
        for compressed in compressed_records {
            let mut payload = Vec::new();
            let decoded = MultiGzDecoder::new(compressed.as_slice())
                .read_to_end(&mut payload)
                .map_err(anyhow::Error::from)
                .and_then(|_| deserialize_record(kind, &payload));
            match decoded {
                Ok(record) => records.push(record),
                Err(_) if skip_corrupt_rows => continue,
                Err(error) => return Err(error),
            }
        }
        if records.len() < limit && (0..=5).contains(&kind) {
            let mut statement = connection.prepare(
                "SELECT training_payload FROM signal_daily_summary WHERE kind = ?1 AND training_payload IS NOT NULL ORDER BY day_start_ms DESC",
            )?;
            let archives = statement.query_map([kind], |row| row.get::<_, Vec<u8>>(0))?;
            let archives = archives.collect::<rusqlite::Result<Vec<_>>>()?;
            drop(statement);
            for compressed in archives {
                if records.len() >= limit {
                    break;
                }
                let mut payload = Vec::new();
                let decoded = MultiGzDecoder::new(compressed.as_slice())
                    .read_to_end(&mut payload)
                    .map_err(anyhow::Error::from)
                    .and_then(|_| {
                        bincode::deserialize::<Vec<StoredSignalRecord>>(&payload)
                            .map_err(anyhow::Error::from)
                    });
                match decoded {
                    Ok(mut archived) => records.append(&mut archived),
                    Err(_) if skip_corrupt_rows => continue,
                    Err(error) => return Err(error),
                }
            }
        }
        drop(connection);
        records.sort_by_key(record_timestamp);
        let start = records.len().saturating_sub(limit);
        records.drain(..start);
        Ok(records)
    }

    async fn read_calibration_records(&self) -> Result<Vec<StoredSignalRecord>> {
        // The display needs recent signal history, not a decompression pass over
        // every archived outcome each time its cache expires.
        let mut records = self.read_recent_records(0, 10_000).await?;
        records.extend(self.read_recent_records(1, 100_000).await?);
        records.sort_by_key(record_timestamp);
        Ok(records)
    }

    async fn save_auto_trader_audit(&self, payload: serde_json::Value) -> Result<()> {
        let decision_id = payload
            .get("decision_id")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing decision_id"))?;
        let stage = payload
            .get("stage")
            .and_then(|value| value.as_str())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing stage"))?;
        let timestamp = payload
            .get("timestamp")
            .and_then(|value| value.as_u64())
            .ok_or_else(|| anyhow::anyhow!("audit record is missing timestamp"))?;
        let symbol = payload
            .get("symbol")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let json = serde_json::to_vec(&payload)?;
        self.send_write(WriteOperation::Audit {
            decision_id: decision_id.to_owned(),
            stage: stage.to_owned(),
            timestamp: timestamp as i64,
            symbol: symbol.to_owned(),
            payload: json,
        })
        .await
    }

    async fn load_auto_trader_audit(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        self.load_auto_trader_audit_with_corrupt_rows(since, limit, false)
            .await
    }

    async fn load_auto_trader_audit_with_corrupt_rows(
        &self,
        since: Option<u64>,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_audit_sync(&path, since, limit, skip_corrupt_rows)
        })
        .await
        .map_err(|error| anyhow::anyhow!("audit read task failed: {error}"))?
    }

    fn load_auto_trader_audit_sync(
        path: &PathBuf,
        since: Option<u64>,
        limit: usize,
        skip_corrupt_rows: bool,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare("SELECT payload FROM auto_trader_audit WHERE (?1 IS NULL OR timestamp >= ?1) ORDER BY timestamp DESC LIMIT ?2")?;
        let rows = statement.query_map(
            params![
                since.map(|value| value as i64),
                limit.clamp(1, 50_000) as i64
            ],
            |row| row.get::<_, Vec<u8>>(0),
        )?;
        let compressed_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(connection);
        let mut result = Vec::new();
        for compressed in compressed_rows {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut json = Vec::new();
            let decoded = decoder
                .read_to_end(&mut json)
                .map_err(anyhow::Error::from)
                .and_then(|_| serde_json::from_slice(&json).map_err(anyhow::Error::from));
            match decoded {
                Ok(value) => result.push(value),
                Err(_) if skip_corrupt_rows => continue,
                Err(error) => return Err(error),
            }
        }
        result.reverse();
        Ok(result)
    }

    async fn load_auto_trader_execution_audit(
        &self,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || {
            Self::load_auto_trader_execution_audit_sync(&path, limit)
        })
        .await
        .map_err(|error| anyhow::anyhow!("execution audit read task failed: {error}"))?
    }

    fn load_auto_trader_execution_audit_sync(
        path: &PathBuf,
        limit: usize,
    ) -> Result<Vec<serde_json::Value>> {
        let connection = Self::open_database_path(path)?;
        let mut statement = connection.prepare("SELECT payload FROM auto_trader_audit WHERE stage = 'entry' OR stage LIKE 'exit%' ORDER BY timestamp DESC LIMIT ?1")?;
        let rows = statement.query_map(params![limit.clamp(1, 50_000) as i64], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        let compressed_rows = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        drop(connection);
        let mut result = Vec::new();
        for compressed in compressed_rows {
            let mut decoder = MultiGzDecoder::new(compressed.as_slice());
            let mut json = Vec::new();
            decoder.read_to_end(&mut json)?;
            result.push(serde_json::from_slice(&json)?);
        }
        result.reverse();
        Ok(result)
    }

    async fn auto_trader_execution_quality(&self) -> Result<AutoTraderExecutionQualitySummary> {
        // Filter before applying the cap so decision events cannot crowd fills out.
        let records = self.load_auto_trader_execution_audit(50_000).await?;
        let mut summary = AutoTraderExecutionQualitySummary::default();
        summary.history_truncated = records.len() == 50_000;
        let (mut requested_quantity, mut filled_quantity, mut measured_notional) = (0.0, 0.0, 0.0);
        let (mut shortfall_total, mut fee_total, mut all_in_total, mut fee_notional) =
            (0.0, 0.0, 0.0, 0.0);
        let (mut response_latency_total, mut response_latency_samples) = (0u64, 0usize);

        for record in records {
            let stage = record
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if stage != "entry" && !stage.starts_with("exit") {
                continue;
            }
            summary.order_samples += 1;
            summary.last_updated_at = summary.last_updated_at.max(
                record
                    .get("response_received_at")
                    .or_else(|| record.get("observed_at"))
                    .or_else(|| record.get("timestamp"))
                    .and_then(serde_json::Value::as_u64),
            );
            if let Some(latency_ms) = record
                .get("submit_response_ms")
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                response_latency_total += (latency_ms * 1_000.0) as u64;
                response_latency_samples += 1;
            }
            let quantity = (if stage == "entry" {
                record.get("order_quantity")
            } else {
                record.get("exit_order_quantity")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
            let filled = (if stage == "entry" {
                record.get("filled_quantity")
            } else {
                record.get("exit_filled_quantity")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, quantity);
            requested_quantity += quantity;
            filled_quantity += filled;
            if filled <= 0.0 {
                summary.unfilled_order_samples += 1;
                continue;
            }
            summary.filled_order_samples += 1;
            if quantity > 0.0 && filled + 1e-12 < quantity {
                summary.partial_fill_samples += 1;
            }

            let average_price = (if stage == "entry" {
                record.get("average_price")
            } else {
                record.get("exit_average_price")
            })
            .and_then(serde_json::Value::as_f64)
            .filter(|price| price.is_finite() && *price > 0.0);
            let fee = (if stage == "entry" {
                record.get("fees")
            } else {
                record.get("exit_fees")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .abs();
            let Some(execution_price) = average_price else {
                continue;
            };
            let notional = filled * execution_price;
            if notional <= 0.0 || !notional.is_finite() {
                continue;
            }
            let fee_bps = fee / notional * 10_000.0;
            fee_total += fee_bps * notional;
            fee_notional += notional;

            let Some(mid) = record
                .get("decision_mid_price")
                .and_then(serde_json::Value::as_f64)
                .filter(|price| price.is_finite() && *price > 0.0)
            else {
                continue;
            };
            let Some(side) = record.get("side").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let shortfall_bps = if side == "buy" {
                (execution_price / mid - 1.0) * 10_000.0
            } else {
                (1.0 - execution_price / mid) * 10_000.0
            };
            measured_notional += notional;
            shortfall_total += shortfall_bps * notional;
            all_in_total += (shortfall_bps + fee_bps) * notional;
            summary.measured_fill_samples += 1;
        }

        summary.average_fill_rate =
            (requested_quantity > 0.0).then_some(filled_quantity / requested_quantity);
        summary.reference_price_coverage_percent = (summary.filled_order_samples > 0).then_some(
            summary.measured_fill_samples as f64 / summary.filled_order_samples as f64 * 100.0,
        );
        summary.average_implementation_shortfall_bps =
            (measured_notional > 0.0).then_some(shortfall_total / measured_notional);
        summary.average_all_in_cost_bps =
            (measured_notional > 0.0).then_some(all_in_total / measured_notional);
        summary.average_fee_bps = (fee_notional > 0.0).then_some(fee_total / fee_notional);
        summary.response_latency_samples = response_latency_samples;
        summary.average_submit_response_ms = (response_latency_samples > 0)
            .then_some(response_latency_total as f64 / response_latency_samples as f64 / 1_000.0);
        Ok(summary)
    }

    async fn auto_trader_decision_quality(&self) -> Result<AutoTraderDecisionQualitySummary> {
        const MIN_COHORT_SAMPLES: usize = 10;
        let records = self.load_auto_trader_audit(None, 50_000).await?;
        let mut summary = AutoTraderDecisionQualitySummary {
            history_truncated: records.len() == 50_000,
            ..Default::default()
        };
        let mut trades: HashMap<String, DecisionTradeAudit> = HashMap::new();
        let mut seen_decisions = HashSet::new();
        for record in records {
            let Some(decision_id) = record
                .get("decision_id")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let stage = record
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if stage == "decision" {
                seen_decisions.insert(decision_id.to_owned());
            }
            let trade = trades.entry(decision_id.to_owned()).or_default();
            if stage == "entry" {
                trade.entry_quantity = record
                    .get("filled_quantity")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0);
                trade.entry_price = record
                    .get("average_price")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite() && *v > 0.0);
                trade.exploratory = record
                    .get("exploratory")
                    .and_then(serde_json::Value::as_bool);
                trade.scout = record
                    .get("scout")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                trade.direction = record
                    .get("direction")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                trade.strength = record
                    .get("strength")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite());
                trade.market_context = record
                    .get("market_context")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                trade.entry_timestamp = record.get("timestamp").and_then(serde_json::Value::as_u64);
                trade.signal_components = record.get("signal_components").cloned();
            } else if stage.starts_with("exit") {
                trade.exit_quantity += record
                    .get("exit_filled_quantity")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0);
                trade.realized_pnl += record
                    .get("realized_pnl")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0);
            }
        }
        summary.decision_samples = seen_decisions.len();
        let mut groups: BTreeMap<(String, String), (usize, usize, f64, f64)> = BTreeMap::new();
        for trade in trades.values() {
            if trade.entry_quantity <= 0.0 {
                continue;
            }
            summary.filled_entry_samples += 1;
            if trade.scout {
                summary.scout_filled_entry_samples += 1;
            }
            if trade.exit_quantity + 1e-12 < trade.entry_quantity {
                summary.unresolved_trade_samples += 1;
                if trade.scout {
                    summary.scout_unresolved_trade_samples += 1;
                }
                continue;
            }
            let Some(entry_price) = trade.entry_price else {
                summary.unpriced_closed_trade_samples += 1;
                if trade.scout {
                    summary.scout_unpriced_closed_trade_samples += 1;
                }
                continue;
            };
            let entry_notional = trade.entry_quantity * entry_price;
            if entry_notional <= 0.0 || !entry_notional.is_finite() {
                continue;
            }
            summary.completed_trade_samples += 1;
            if trade.scout {
                summary.scout_completed_trade_samples += 1;
            }
            let net_return = trade.realized_pnl / entry_notional * 100.0;
            let strength_band = match trade.strength.unwrap_or(0.0) {
                v if v < 0.50 => "<0.50",
                v if v < 0.60 => "0.50–0.59",
                v if v < 0.70 => "0.60–0.69",
                _ => "0.70+",
            };
            let components = trade.signal_components.as_ref();
            let component_value = |name: &str| {
                components
                    .and_then(|values| values.get(name))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite())
            };
            let momentum_band = component_value("momentum_10s_percent")
                .map(|v| {
                    if v < -0.12 {
                        "<-0.12%"
                    } else if v < -0.04 {
                        "-0.12 to -0.04%"
                    } else if v < 0.04 {
                        "-0.04 to 0.04%"
                    } else if v < 0.12 {
                        "0.04 to 0.12%"
                    } else {
                        "≥0.12%"
                    }
                })
                .unwrap_or("unavailable");
            let trend_band = component_value("trend_5m_volatility_units")
                .map(|v| {
                    if v < -1.5 {
                        "<-1.5σ"
                    } else if v < -0.5 {
                        "-1.5 to -0.5σ"
                    } else if v < 0.5 {
                        "-0.5 to 0.5σ"
                    } else if v < 1.5 {
                        "0.5 to 1.5σ"
                    } else {
                        "≥1.5σ"
                    }
                })
                .unwrap_or("unavailable");
            let volatility_band = component_value("volatility_1m_percent")
                .map(|v| {
                    if v < 0.10 {
                        "<0.10%"
                    } else if v < 0.30 {
                        "0.10 to 0.30%"
                    } else if v < 1.0 {
                        "0.30 to 1.00%"
                    } else {
                        "≥1.00%"
                    }
                })
                .unwrap_or("unavailable");
            let vwap_band = component_value("mispricing_vs_vwap_bps")
                .map(|v| {
                    if v < -8.0 {
                        "<-8 bps"
                    } else if v < -3.0 {
                        "-8 to -3 bps"
                    } else if v < 3.0 {
                        "-3 to 3 bps"
                    } else if v < 8.0 {
                        "3 to 8 bps"
                    } else {
                        "≥8 bps"
                    }
                })
                .unwrap_or("unavailable");
            let pressure_band = component_value("pressure_without_price_move")
                .map(|v| {
                    if v < -0.75 {
                        "<-0.75"
                    } else if v < 0.0 {
                        "-0.75 to 0"
                    } else if v < 0.75 {
                        "0 to 0.75"
                    } else {
                        "≥0.75"
                    }
                })
                .unwrap_or("unavailable");
            let aggressor_cost_band = component_value("aggressor_cost_bps")
                .map(|v| {
                    if v < 0.0 {
                        "<0 bps"
                    } else if v < 2.0 {
                        "0 to 2 bps"
                    } else if v < 5.0 {
                        "2 to 5 bps"
                    } else {
                        "≥5 bps"
                    }
                })
                .unwrap_or("unavailable");
            let momentum_flow_confirmation = match (
                component_value("momentum_10s_percent"),
                component_value("trade_flow_imbalance"),
                trade.direction.as_deref(),
            ) {
                (Some(momentum), Some(flow), Some(direction)) => {
                    let direction_sign = if direction == "bullish" { 1.0 } else { -1.0 };
                    let momentum_confirms = momentum * direction_sign > 0.0;
                    let flow_confirms = flow * direction_sign > 0.0;
                    match (momentum_confirms, flow_confirms) {
                        (true, true) => "both confirm",
                        (true, false) => "momentum only",
                        (false, true) => "flow only",
                        (false, false) => "neither confirms",
                    }
                }
                _ => "unavailable",
            };
            let market_regime = trade
                .market_context
                .as_deref()
                .and_then(|context| context.split_once(':').map(|(_, regime)| regime))
                .unwrap_or("unavailable");
            let entry_time_utc = trade
                .entry_timestamp
                .map(|timestamp| {
                    let hour = (timestamp / 3_600_000) % 24;
                    let start = (hour / 6) * 6;
                    format!("{start:02}:00–{:02}:59 UTC", start + 5)
                })
                .unwrap_or_else(|| "unavailable".to_owned());
            let dimensions = [
                (
                    "direction",
                    trade.direction.as_deref().unwrap_or("unknown").to_owned(),
                ),
                (
                    "selection",
                    if trade.scout {
                        "scout"
                    } else {
                        match trade.exploratory {
                            Some(true) => "exploratory",
                            Some(false) => "strict",
                            None => "unknown",
                        }
                    }
                    .to_owned(),
                ),
                (
                    "market_context",
                    trade
                        .market_context
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_owned(),
                ),
                ("market_regime", market_regime.to_owned()),
                ("entry_time_utc", entry_time_utc),
                ("strength", strength_band.to_owned()),
                ("10s_momentum", momentum_band.to_owned()),
                ("5m_trend", trend_band.to_owned()),
                ("1m_volatility", volatility_band.to_owned()),
                ("VWAP_mispricing", vwap_band.to_owned()),
                ("pressure_without_move", pressure_band.to_owned()),
                ("aggressor_cost", aggressor_cost_band.to_owned()),
                (
                    "momentum_flow_confirmation",
                    momentum_flow_confirmation.to_owned(),
                ),
            ];
            for (dimension, value) in dimensions {
                let group = groups.entry((dimension.to_owned(), value)).or_default();
                group.0 += 1;
                if net_return > 0.0 {
                    group.1 += 1;
                }
                group.2 += net_return;
                group.3 += trade.realized_pnl;
            }
        }
        summary.cohorts = groups
            .into_iter()
            .map(
                |((dimension, value), (samples, wins, return_total, pnl_total))| {
                    AutoTraderDecisionQualityCohort {
                        dimension,
                        value,
                        completed_trade_samples: samples,
                        average_net_return_percent: return_total / samples as f64,
                        win_rate: wins as f64 / samples as f64,
                        total_net_pnl: pnl_total,
                        preliminary: samples < MIN_COHORT_SAMPLES,
                    }
                },
            )
            .collect();
        Ok(summary)
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
        let audit_store = self.clone();
        let best_store = self;
        Router::new()
            .route(
                "/signals/dashboard",
                get(move || {
                    let store = dashboard_store.clone();
                    async move {
                        match store.dashboard().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/history",
                get(move || {
                    let store = history_store.clone();
                    async move {
                        match store.recent(200).await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/performance",
                get(move || {
                    let store = performance_store.clone();
                    async move {
                        match store.performance().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-dashboard",
                get(move || {
                    let store = auto_trader_store.clone();
                    async move {
                        match store.auto_trader_dashboard().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-decision-quality",
                get({
                    let store = audit_store.clone();
                    move || {
                        let store = store.clone();
                        async move {
                            match store.auto_trader_decision_quality().await {
                                Ok(summary) => Ok(Json(summary)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-execution-quality",
                get({
                    let store = audit_store.clone();
                    move || {
                        let store = store.clone();
                        async move {
                            match store.auto_trader_execution_quality().await {
                                Ok(summary) => Ok(Json(summary)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-audit",
                get({
                    let store = audit_store.clone();
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = store.clone();
                        async move {
                            match store
                                .load_auto_trader_audit(query.since, query.limit.unwrap_or(10_000))
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                })
                .post({
                    let store = audit_store.clone();
                    move |Json(payload): Json<serde_json::Value>| {
                        let store = store.clone();
                        async move {
                            match store.save_auto_trader_audit(payload).await {
                                Ok(()) => Ok(StatusCode::NO_CONTENT),
                                Err(error) => Err((StatusCode::BAD_REQUEST, error.to_string())),
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/best",
                get(move || {
                    let store = best_store.clone();
                    async move {
                        match store.best(10).await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/walk-forward",
                get(move || {
                    let store = evaluation_store.clone();
                    async move {
                        match store.walk_forward().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/evaluations",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = raw_evaluation_store.clone();
                        async move {
                            match store
                                .evaluations(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/book-snapshots",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = book_snapshot_store.clone();
                        async move {
                            match store
                                .book_snapshots(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/research-features",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = research_feature_store.clone();
                        async move {
                            match store
                                .research_features(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/external-market",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = external_market_store.clone();
                        async move {
                            match store
                                .external_market_features(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
    }

    pub async fn book_snapshots(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalBookSnapshot>> {
        let mut snapshots = self
            .read_records(3, since, Some(limit))
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::BookSnapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = snapshots.len().saturating_sub(limit);
        Ok(snapshots.drain(start..).collect())
    }

    pub async fn append_book_snapshot(&self, snapshot: &SignalBookSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::BookSnapshot(snapshot.clone()))
            .await
    }

    pub async fn append_external_market_features(
        &self,
        features: &ExternalMarketFeatures,
    ) -> Result<()> {
        self.append_record(StoredSignalRecord::ExternalMarket(features.clone()))
            .await
    }

    pub async fn external_market_features(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<ExternalMarketFeatures>> {
        let mut features = self
            .read_records(5, since, Some(limit))
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::ExternalMarket(features) => Some(features),
                _ => None,
            })
            .collect::<Vec<_>>();
        features.sort_by(|a, b| {
            a.timestamp
                .cmp(&b.timestamp)
                .then(a.underlying.cmp(&b.underlying))
        });
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn append_research_features(&self, features: &SignalResearchFeatures) -> Result<()> {
        self.append_record(StoredSignalRecord::ResearchFeatures(features.clone()))
            .await
    }

    pub async fn research_features(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalResearchFeatures>> {
        let records = if since.is_none() {
            self.read_recent_records(4, limit.min(100_000)).await?
        } else {
            self.read_records(4, since, Some(limit.min(100_000)))
                .await?
        };
        let mut features = records
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::ResearchFeatures(features) => Some(features),
                _ => None,
            })
            .collect::<Vec<_>>();
        features.sort_by(|a, b| a.timestamp.cmp(&b.timestamp).then(a.symbol.cmp(&b.symbol)));
        let start = features.len().saturating_sub(limit);
        Ok(features.drain(start..).collect())
    }

    pub async fn evaluations(
        &self,
        since: Option<u64>,
        limit: usize,
    ) -> Result<Vec<SignalEvaluation>> {
        let lower_bound = since.map(|timestamp| timestamp.saturating_sub(3_600_000));
        let mut evaluations = self
            .read_records(2, lower_bound, None)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(evaluation)
                    if since
                        .is_none_or(|timestamp| evaluation.evaluated_timestamp >= timestamp) =>
                {
                    Some(evaluation)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        evaluations.sort_by(|a, b| {
            a.evaluated_timestamp
                .cmp(&b.evaluated_timestamp)
                .then(a.symbol.cmp(&b.symbol))
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        let start = evaluations.len().saturating_sub(limit);
        Ok(evaluations.drain(start..).collect())
    }

    pub async fn auto_trader_dashboard(&self) -> Result<AutoTraderDashboard> {
        let (best, walk_forward) = tokio::try_join!(self.best(100), self.walk_forward())?;
        let missed_opportunity_bands = self.missed_opportunity_bands().await?;
        Ok(AutoTraderDashboard {
            best,
            walk_forward,
            missed_opportunity_bands,
        })
    }

    pub async fn dashboard(&self) -> Result<SignalDashboard> {
        let (best, history, performance, walk_forward) = tokio::try_join!(
            self.best(10),
            self.recent(200),
            self.performance(),
            self.walk_forward(),
        )?;
        Ok(SignalDashboard {
            best,
            history,
            performance,
            walk_forward,
        })
    }

    pub async fn recent(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        if limit > 200 {
            return self.load_recent(limit).await;
        }
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
        let records = self.read_calibration_records().await?;
        let mut items = records
            .iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Snapshot(snapshot) => Some(snapshot.clone()),
                _ => None,
            })
            .collect::<Vec<_>>();
        calibrate_display_strength(&mut items, &records);
        let start = items.len().saturating_sub(limit);
        Ok(items.drain(start..).collect())
    }

    pub async fn best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        // The AutoTrader asks for more than the UI's ten rows. Keep a wider
        // cache so its five-second cycle does not repeatedly decompress and
        // recalibrate the entire signal history.
        if limit > 100 {
            return self.load_best(limit).await;
        }
        let mut cache = self.best_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(8) {
                return Ok(items.iter().take(limit).cloned().collect());
            }
        }
        let items = self.load_best(100).await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items.into_iter().take(limit).collect())
    }

    async fn load_best(&self, limit: usize) -> Result<Vec<SignalSnapshot>> {
        let records = self.read_calibration_records().await?;
        let mut latest: HashMap<String, SignalSnapshot> = HashMap::new();
        for record in &records {
            let StoredSignalRecord::Snapshot(snapshot) = record else {
                continue;
            };
            let snapshot = snapshot.clone();
            // Retain the latest ranging setup from disk across a service restart.
            // It remains visible until fresh feed data replaces it, with staleness
            // set below so clients can distinguish it from a live signal.
            if snapshot.stale && snapshot.direction != "ranging" {
                continue;
            }
            let replace = latest
                .get(&snapshot.symbol)
                .is_none_or(|current| current.timestamp < snapshot.timestamp);
            if replace {
                latest.insert(snapshot.symbol.clone(), snapshot);
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let mut items = latest
            .into_values()
            .filter_map(|mut item| {
                let age = now.saturating_sub(item.timestamp);
                if age > 60_000 && item.direction != "ranging" {
                    return None;
                }
                item.stale |= age > 60_000;
                Some(item)
            })
            .collect::<Vec<_>>();
        calibrate_display_strength(&mut items, &records);
        items.sort_by(|a, b| a.symbol.cmp(&b.symbol));
        items.truncate(limit);
        Ok(items)
    }

    pub async fn performance(&self) -> Result<Vec<SignalPerformance>> {
        let mut cache = self.performance_cache.lock().await;
        if let Some((created, items)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) {
                return Ok(items.clone());
            }
        }
        let items = self.compute_performance().await?;
        *cache = Some((Instant::now(), items.clone()));
        Ok(items)
    }

    async fn compute_performance(&self) -> Result<Vec<SignalPerformance>> {
        let outcomes = self.read_outcomes().await?;
        let mut totals: HashMap<(String, u64), PerformanceTotals> = HashMap::new();
        for outcome in outcomes {
            let total = totals
                .entry((outcome.symbol.clone(), outcome.horizon_seconds))
                .or_default();
            total.sample_count += 1;
            total.raw_return_total += outcome.return_percent;
            match outcome.direction.as_str() {
                "bullish" => {
                    total.directional_count += 1;
                    total.bullish_count += 1;
                    if outcome.direction_match {
                        total.bullish_matches += 1;
                        total.directional_matches += 1;
                    }
                }
                "bearish" => {
                    total.directional_count += 1;
                    total.bearish_count += 1;
                    if outcome.direction_match {
                        total.bearish_matches += 1;
                        total.directional_matches += 1;
                    }
                }
                "neutral" => total.neutral_count += 1,
                _ => {}
            }
            if let Some(net_return) = outcome.net_return_percent {
                total.net_count += 1;
                total.net_return_total += net_return;
            }
        }
        let mut result = totals
            .into_iter()
            .map(|((symbol, horizon_seconds), total)| SignalPerformance {
                symbol,
                horizon_seconds,
                sample_count: total.sample_count,
                directional_sample_count: total.directional_count,
                neutral_sample_count: total.neutral_count,
                bullish_match_rate: (total.bullish_count > 0)
                    .then_some(total.bullish_matches as f64 / total.bullish_count as f64),
                bearish_match_rate: (total.bearish_count > 0)
                    .then_some(total.bearish_matches as f64 / total.bearish_count as f64),
                direction_match_rate: (total.directional_count > 0)
                    .then_some(total.directional_matches as f64 / total.directional_count as f64),
                average_return_percent: total.raw_return_total / total.sample_count as f64,
                net_sample_count: total.net_count,
                average_net_return_percent: (total.net_count > 0)
                    .then_some(total.net_return_total / total.net_count as f64),
            })
            .collect::<Vec<_>>();
        result.sort_by(|a, b| {
            a.symbol
                .cmp(&b.symbol)
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        Ok(result)
    }

    pub fn cached_walk_forward(&self) -> Vec<SignalWalkForward> {
        self.walk_forward_cache
            .try_lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|(_, value)| value.clone()))
            .unwrap_or_default()
    }

    pub fn cached_missed_opportunity_bands(&self) -> Vec<MissedOpportunityGateBand> {
        self.missed_opportunity_cache
            .try_lock()
            .ok()
            .and_then(|cache| cache.as_ref().map(|(_, value)| value.clone()))
            .unwrap_or_default()
    }

    pub async fn missed_opportunity_bands(&self) -> Result<Vec<MissedOpportunityGateBand>> {
        let cache = self.missed_opportunity_cache.lock().await;
        if let Some((created, value)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(60) {
                return Ok(value.clone());
            }
            let stale = value.clone();
            if self
                .walk_forward_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let store = self.clone();
                tokio::spawn(async move {
                    if let Ok(value) = store.compute_missed_opportunity_bands().await {
                        *store.missed_opportunity_cache.lock().await =
                            Some((Instant::now(), value));
                    }
                    store
                        .walk_forward_refreshing
                        .store(false, Ordering::Release);
                });
            }
            return Ok(stale);
        }
        drop(cache);
        let value = self.compute_missed_opportunity_bands().await?;
        *self.missed_opportunity_cache.lock().await = Some((Instant::now(), value.clone()));
        Ok(value)
    }

    async fn compute_missed_opportunity_bands(&self) -> Result<Vec<MissedOpportunityGateBand>> {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;
        let since = now_ms.saturating_sub(30 * 24 * 60 * 60 * 1_000);
        let decisions = self
            .load_auto_trader_audit_with_corrupt_rows(Some(since), 50_000, true)
            .await?;
        let evaluations = self
            .read_recent_records_with_corrupt_rows(2, 100_000, true)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Evaluation(evaluation)
                    if evaluation.evaluated_timestamp >= since
                        && evaluation.evaluated_timestamp <= now_ms =>
                {
                    Some(evaluation)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let evaluation_by_signal = evaluations
            .into_iter()
            .map(|evaluation| {
                (
                    (
                        evaluation.symbol.clone(),
                        evaluation.signal_timestamp,
                        evaluation.horizon_seconds,
                    ),
                    evaluation,
                )
            })
            .collect::<HashMap<_, _>>();
        let mut resolved_skips = Vec::new();
        for decision in decisions {
            if decision.get("stage").and_then(serde_json::Value::as_str) != Some("decision")
                || decision.get("status").and_then(serde_json::Value::as_str) != Some("skipped")
                || !decision
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|reason| reason.starts_with("No tradeable bias:"))
            {
                continue;
            }
            let Some(symbol) = decision.get("symbol").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(timestamp) = decision
                .get("signal_timestamp")
                .and_then(serde_json::Value::as_u64)
            else {
                continue;
            };
            let Some(direction) = decision
                .get("direction")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let Some(score) = decision
                .get("score")
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
            else {
                continue;
            };
            let Some(strength) = decision
                .get("strength")
                .and_then(serde_json::Value::as_f64)
                .filter(|v| v.is_finite())
            else {
                continue;
            };
            let stop_loss = decision
                .get("stop_loss_percent")
                .and_then(serde_json::Value::as_f64);
            let take_profit = decision
                .get("take_profit_percent")
                .and_then(serde_json::Value::as_f64);
            let mut samples = Vec::new();
            for horizon in [60_u64, 300, 900, 3_600] {
                let Some(evaluation) =
                    evaluation_by_signal.get(&(symbol.to_owned(), timestamp, horizon))
                else {
                    continue;
                };
                if evaluation.evaluated_timestamp > now_ms {
                    continue;
                }
                let (terminal_net, time_to_profit) = match direction {
                    "bullish" => (
                        evaluation.long_net_percent,
                        evaluation.long_time_to_positive_net_seconds,
                    ),
                    "bearish" => (
                        evaluation.short_net_percent,
                        evaluation.short_time_to_positive_net_seconds,
                    ),
                    _ => (None, None),
                };
                let terminal_net = terminal_net.filter(|value| value.is_finite());
                let stop_target_net = stop_loss.zip(take_profit).and_then(|(stop, target)| {
                    stop_target_counterfactual_return(evaluation, direction, stop, target)
                });
                if terminal_net.is_some() || stop_target_net.is_some() {
                    samples.push((horizon, terminal_net, time_to_profit, stop_target_net));
                }
            }
            if samples.iter().any(|sample| sample.0 == 60) {
                resolved_skips.push((
                    symbol.to_owned(),
                    direction.to_owned(),
                    timestamp,
                    score.abs(),
                    strength,
                    samples,
                ));
            }
        }
        resolved_skips.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });
        let mut next_allowed = HashMap::<(String, String), u64>::new();
        let mut aggregates =
            HashMap::<(String, usize, u64), Vec<(Option<f64>, Option<u64>, Option<f64>)>>::new();
        for (symbol, direction, timestamp, score, strength, samples) in resolved_skips {
            let key = (symbol, direction);
            if timestamp < *next_allowed.get(&key).unwrap_or(&0) {
                continue;
            }
            next_allowed.insert(key, timestamp.saturating_add(60_000));
            for (horizon, net, time_to_profit, stop_target_net) in samples {
                for (index, threshold) in MISSED_OPPORTUNITY_SCORE_THRESHOLDS.iter().enumerate() {
                    if score >= *threshold {
                        aggregates
                            .entry(("score".to_owned(), index, horizon))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net));
                    }
                }
                for (index, threshold) in MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS.iter().enumerate()
                {
                    if strength >= *threshold {
                        aggregates
                            .entry(("strength".to_owned(), index, horizon))
                            .or_default()
                            .push((net, time_to_profit, stop_target_net));
                    }
                }
            }
        }
        let mut bands = Vec::new();
        for (gate, thresholds) in [
            ("score", MISSED_OPPORTUNITY_SCORE_THRESHOLDS.as_slice()),
            (
                "strength",
                MISSED_OPPORTUNITY_STRENGTH_THRESHOLDS.as_slice(),
            ),
        ] {
            for (index, threshold) in thresholds.iter().enumerate() {
                for horizon in [60_u64, 300, 900, 3_600] {
                    let samples = aggregates
                        .get(&(gate.to_owned(), index, horizon))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]);
                    let evaluation_count = samples.len();
                    let net_samples = samples
                        .iter()
                        .filter_map(|(net, _, _)| *net)
                        .collect::<Vec<_>>();
                    let count = net_samples.len();
                    let positive_count = net_samples.iter().filter(|net| **net > 0.0).count();
                    let barrier_samples = samples
                        .iter()
                        .filter_map(|(_, _, net)| *net)
                        .collect::<Vec<_>>();
                    let barrier_count = barrier_samples.len();
                    let barrier_positive_count =
                        barrier_samples.iter().filter(|net| **net > 0.0).count();
                    let mut times = samples
                        .iter()
                        .filter_map(|(_, time, _)| *time)
                        .collect::<Vec<_>>();
                    times.sort_unstable();
                    let time_count = times.len();
                    let average_time = (time_count > 0)
                        .then(|| times.iter().sum::<u64>() as f64 / time_count as f64);
                    let average_barrier_net = (barrier_count > 0)
                        .then(|| barrier_samples.iter().sum::<f64>() / barrier_count as f64);
                    let median_time = (time_count > 0).then(|| {
                        if time_count % 2 == 0 {
                            (times[time_count / 2 - 1] + times[time_count / 2]) as f64 / 2.0
                        } else {
                            times[time_count / 2] as f64
                        }
                    });
                    bands.push(MissedOpportunityGateBand {
                        gate: gate.to_owned(),
                        threshold: *threshold,
                        horizon_seconds: horizon,
                        sample_count: count,
                        average_net_return_percent: (count > 0)
                            .then(|| net_samples.iter().sum::<f64>() / count as f64),
                        positive_net_rate: (count > 0)
                            .then_some(positive_count as f64 / count as f64),
                        time_to_profit_sample_count: time_count,
                        time_to_profit_rate: (evaluation_count > 0)
                            .then_some(time_count as f64 / evaluation_count as f64),
                        average_time_to_profit_seconds: average_time,
                        median_time_to_profit_seconds: median_time,
                        stop_target_sample_count: barrier_count,
                        stop_target_average_net_return_percent: average_barrier_net,
                        stop_target_positive_net_rate: (barrier_count > 0)
                            .then_some(barrier_positive_count as f64 / barrier_count as f64),
                    });
                }
            }
        }
        Ok(bands)
    }

    pub async fn walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let mut cache = self.walk_forward_cache.lock().await;
        if let Some((created, value)) = cache.as_ref() {
            if created.elapsed() < Duration::from_secs(10) {
                return Ok(value.clone());
            }
            // Walk-forward training is deliberately expensive. Keep serving
            // the last complete result while a background refresh runs, so
            // a cache expiry cannot stall the AutoTrader's signal path.
            let stale = value.clone();
            if self
                .walk_forward_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let store = self.clone();
                tokio::spawn(async move {
                    let refreshed = store.compute_walk_forward().await;
                    if let Ok(value) = refreshed {
                        *store.walk_forward_cache.lock().await = Some((Instant::now(), value));
                    }
                    store
                        .walk_forward_refreshing
                        .store(false, Ordering::Release);
                });
            }
            return Ok(stale);
        }
        let value = self.compute_walk_forward().await?;
        *cache = Some((Instant::now(), value.clone()));
        Ok(value)
    }

    async fn compute_walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let outcomes = self.read_outcomes().await?;
        let snapshots = self.read_snapshots().await?;
        let snapshots_by_key = snapshots
            .into_iter()
            .map(|snapshot| ((snapshot.symbol.clone(), snapshot.timestamp), snapshot))
            .collect::<HashMap<_, _>>();
        let research_features = self.research_features(None, usize::MAX).await?;
        let research_features_by_key = research_features
            .into_iter()
            .filter_map(|features| {
                Some((
                    (features.symbol.clone(), features.timestamp),
                    research_feature_vector(&features)?,
                ))
            })
            .collect::<HashMap<_, _>>();
        let mut grouped: HashMap<(String, u64), Vec<SignalOutcome>> = HashMap::new();
        for outcome in outcomes {
            grouped
                .entry((outcome.symbol.clone(), outcome.horizon_seconds))
                .or_default()
                .push(outcome);
        }
        let mut result = Vec::new();
        for ((symbol, horizon_seconds), mut samples) in grouped {
            samples.sort_by_key(|sample| sample.signal_timestamp);
            let mut non_overlapping = Vec::new();
            let mut next_timestamp = 0;
            for sample in samples {
                if sample.signal_timestamp < next_timestamp {
                    continue;
                }
                next_timestamp = sample.evaluated_timestamp.max(
                    sample
                        .signal_timestamp
                        .saturating_add(horizon_seconds.saturating_mul(1000)),
                );
                if sample.net_return_percent.is_some()
                    && matches!(sample.direction.as_str(), "bullish" | "bearish")
                {
                    non_overlapping.push(sample);
                }
            }
            if non_overlapping.len() <= WALK_FORWARD_MIN_TRAINING_SAMPLES {
                continue;
            }

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
            let mut research_logistic_long = SideWalkForwardAccumulator::default();
            let mut research_logistic_short = SideWalkForwardAccumulator::default();
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
            let mut research_logistic_weights = None;
            for index in WALK_FORWARD_MIN_TRAINING_SAMPLES..non_overlapping.len() {
                let start = index.saturating_sub(WALK_FORWARD_TRAINING_WINDOW);
                let sample = &non_overlapping[index];
                let sample_net = sample.net_return_percent.unwrap_or_default();
                if sample.direction == "bullish" {
                    all_signal_long.record(sample.direction_match, sample_net);
                } else if sample.direction == "bearish" {
                    all_signal_short.record(sample.direction_match, sample_net);
                }
                if sample.horizon_seconds == 60 {
                    if let (Some(snapshot), Some(cost)) = (
                        snapshots_by_key.get(&(sample.symbol.clone(), sample.signal_timestamp)),
                        sample.execution_cost_percent,
                    ) {
                        let can_explore = snapshot.strength >= 0.05
                            && snapshot.ranging_probability.unwrap_or(0.0) < 0.75
                            && matches!(
                                snapshot.direction.as_str(),
                                "bullish" | "bearish" | "neutral"
                            );
                        if can_explore {
                            let up = snapshot.score >= 0.0;
                            let matches = (up && sample.return_percent > 0.0)
                                || (!up && sample.return_percent < 0.0);
                            let net = if up {
                                sample.return_percent
                            } else {
                                -sample.return_percent
                            } - cost;
                            for (band, threshold) in EXPLORATION_SCORE_THRESHOLDS.iter().enumerate()
                            {
                                if snapshot.score.abs() >= *threshold {
                                    exploration_selected[band] += 1;
                                    if matches {
                                        exploration_correct[band] += 1;
                                    }
                                    if net > 0.0 {
                                        exploration_positive[band] += 1;
                                    }
                                    exploration_net[band] += net;
                                }
                            }
                        }
                    }
                }
                let training_window = &non_overlapping[start..index];
                let training_count = training_window
                    .iter()
                    .filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp)
                    .count();
                if training_count < WALK_FORWARD_MIN_TRAINING_SAMPLES {
                    continue;
                }
                max_training_samples = max_training_samples.max(training_count);
                evaluated_signals += 1;
                if sample.direction_match {
                    baseline_correct += 1;
                }
                baseline_net_total += sample.net_return_percent.unwrap_or_default();
                if (index - WALK_FORWARD_MIN_TRAINING_SAMPLES)
                    % WALK_FORWARD_LOGISTIC_REFRESH_INTERVAL
                    == 0
                {
                    let training = training_window
                        .iter()
                        .filter(|prior| prior.evaluated_timestamp <= sample.signal_timestamp)
                        .cloned()
                        .collect::<Vec<_>>();
                    let logistic_training = training
                        .iter()
                        .filter(|prior| prior.features.is_some())
                        .cloned()
                        .collect::<Vec<_>>();
                    logistic_weights = train_logistic(&logistic_training);
                    trend_logistic_weights = train_trend_logistic(&logistic_training);
                    research_logistic_weights =
                        train_research_logistic(&training, &research_features_by_key);
                }
                if let (Some(weights), Some(features), Some(trend), Some(cost)) = (
                    trend_logistic_weights.as_ref(),
                    sample.features.as_ref(),
                    sample.trend_5m_volatility_units,
                    sample.execution_cost_percent,
                ) {
                    let probability = trend_logistic_up_probability(weights, features, trend);
                    let up = probability >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    trend_logistic_selected += 1;
                    if matches {
                        trend_logistic_correct += 1;
                    }
                    if net > 0.0 {
                        trend_logistic_positive += 1;
                    }
                    trend_logistic_net_total += net;
                    let contrarian_up = probability < 0.5;
                    let contrarian_matches = (contrarian_up && sample.return_percent > 0.0)
                        || (!contrarian_up && sample.return_percent < 0.0);
                    let contrarian_net = if contrarian_up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    contrarian_selected += 1;
                    if contrarian_matches {
                        contrarian_correct += 1;
                    }
                    if contrarian_net > 0.0 {
                        contrarian_positive += 1;
                    }
                    contrarian_net_total += contrarian_net;
                    let confidence = probability.max(1.0 - probability);
                    for (band, threshold) in TREND_LOGISTIC_CONFIDENCE_THRESHOLDS.iter().enumerate()
                    {
                        if confidence >= *threshold {
                            trend_logistic_confident_selected[band] += 1;
                            if matches {
                                trend_logistic_confident_correct[band] += 1;
                            }
                            if net > 0.0 {
                                trend_logistic_confident_positive[band] += 1;
                            }
                            trend_logistic_confident_net[band] += net;
                            contrarian_confident_selected[band] += 1;
                            if contrarian_matches {
                                contrarian_confident_correct[band] += 1;
                            }
                            if contrarian_net > 0.0 {
                                contrarian_confident_positive[band] += 1;
                            }
                            contrarian_confident_net[band] += contrarian_net;
                        }
                    }
                }
                if let (Some(weights), Some(features), Some(cost)) = (
                    research_logistic_weights.as_ref(),
                    research_features_by_key.get(&(sample.symbol.clone(), sample.signal_timestamp)),
                    sample.execution_cost_percent,
                ) {
                    let up = research_logistic_up_probability(weights, features) >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    if up {
                        research_logistic_long.record(matches, net);
                    } else {
                        research_logistic_short.record(matches, net);
                    }
                }
                if let (Some(weights), Some(features), Some(cost)) = (
                    logistic_weights.as_ref(),
                    sample.features.as_ref(),
                    sample.execution_cost_percent,
                ) {
                    let up = logistic_up_probability(&weights, features) >= 0.5;
                    let matches =
                        (up && sample.return_percent > 0.0) || (!up && sample.return_percent < 0.0);
                    let net = if up {
                        sample.return_percent
                    } else {
                        -sample.return_percent
                    } - cost;
                    logistic_selected += 1;
                    if matches {
                        logistic_correct += 1;
                    }
                    if net > 0.0 {
                        logistic_positive += 1;
                    }
                    logistic_net_total += net;
                    if up {
                        logistic_long.record(matches, net);
                    } else {
                        logistic_short.record(matches, net);
                    }
                }
                if let (Some(matches), Some(net_return)) = (
                    sample.trend_5m_direction_match,
                    sample.trend_5m_net_return_percent,
                ) {
                    five_minute_trend_samples += 1;
                    if matches {
                        five_minute_trend_correct += 1;
                    }
                    five_minute_trend_net_total += net_return;
                }
                let Some(threshold) =
                    select_strength_threshold(training_window, sample.signal_timestamp)
                else {
                    continue;
                };
                last_threshold = Some(threshold);
                if sample.strength < threshold {
                    continue;
                }
                selected_signals += 1;
                if sample.direction_match {
                    correct += 1;
                }
                let net = sample.net_return_percent.unwrap_or_default();
                net_total += net;
                if net > 0.0 {
                    positive_net += 1;
                }
                if sample.direction == "bullish" {
                    signal_long.record(sample.direction_match, net);
                } else if sample.direction == "bearish" {
                    signal_short.record(sample.direction_match, net);
                }
            }
            if evaluated_signals == 0 {
                continue;
            }
            result.push(SignalWalkForward {
                symbol,
                horizon_seconds,
                training_samples: max_training_samples,
                evaluated_signals,
                selected_signals,
                selected_strength_threshold: last_threshold,
                direction_match_rate: (selected_signals > 0)
                    .then_some(correct as f64 / selected_signals as f64),
                average_net_return_percent: (selected_signals > 0)
                    .then_some(net_total / selected_signals as f64),
                positive_net_rate: (selected_signals > 0)
                    .then_some(positive_net as f64 / selected_signals as f64),
                baseline_direction_match_rate: (evaluated_signals > 0)
                    .then_some(baseline_correct as f64 / evaluated_signals as f64),
                baseline_average_net_return_percent: (evaluated_signals > 0)
                    .then_some(baseline_net_total / evaluated_signals as f64),
                five_minute_trend_sample_count: five_minute_trend_samples,
                five_minute_trend_match_rate: (five_minute_trend_samples > 0)
                    .then_some(five_minute_trend_correct as f64 / five_minute_trend_samples as f64),
                five_minute_trend_average_net_return_percent: (five_minute_trend_samples > 0)
                    .then_some(five_minute_trend_net_total / five_minute_trend_samples as f64),
                logistic_direction_match_rate: (logistic_selected > 0)
                    .then_some(logistic_correct as f64 / logistic_selected as f64),
                logistic_average_net_return_percent: (logistic_selected > 0)
                    .then_some(logistic_net_total / logistic_selected as f64),
                logistic_positive_net_rate: (logistic_selected > 0)
                    .then_some(logistic_positive as f64 / logistic_selected as f64),
                logistic_selected_signals: logistic_selected,
                trend_logistic_direction_match_rate: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_correct as f64 / trend_logistic_selected as f64),
                trend_logistic_average_net_return_percent: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_net_total / trend_logistic_selected as f64),
                trend_logistic_positive_net_rate: (trend_logistic_selected > 0)
                    .then_some(trend_logistic_positive as f64 / trend_logistic_selected as f64),
                trend_logistic_selected_signals: trend_logistic_selected,
                contrarian_trend_logistic_direction_match_rate: (contrarian_selected > 0)
                    .then_some(contrarian_correct as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_average_net_return_percent: (contrarian_selected > 0)
                    .then_some(contrarian_net_total / contrarian_selected as f64),
                contrarian_trend_logistic_positive_net_rate: (contrarian_selected > 0)
                    .then_some(contrarian_positive as f64 / contrarian_selected as f64),
                contrarian_trend_logistic_selected_signals: contrarian_selected,
                contrarian_trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = contrarian_confident_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0).then_some(
                                contrarian_confident_correct[index] as f64 / selected as f64,
                            ),
                            average_net_return_percent: (selected > 0)
                                .then_some(contrarian_confident_net[index] / selected as f64),
                            positive_net_rate: (selected > 0).then_some(
                                contrarian_confident_positive[index] as f64 / selected as f64,
                            ),
                        }
                    })
                    .collect(),
                exploration_score_bands: EXPLORATION_SCORE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = exploration_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0)
                                .then_some(exploration_correct[index] as f64 / selected as f64),
                            average_net_return_percent: (selected > 0)
                                .then_some(exploration_net[index] / selected as f64),
                            positive_net_rate: (selected > 0)
                                .then_some(exploration_positive[index] as f64 / selected as f64),
                        }
                    })
                    .collect(),
                trend_logistic_confidence_bands: TREND_LOGISTIC_CONFIDENCE_THRESHOLDS
                    .iter()
                    .enumerate()
                    .map(|(index, threshold)| {
                        let selected = trend_logistic_confident_selected[index];
                        ProbabilityConfidenceMetrics {
                            threshold: *threshold,
                            selected_signals: selected,
                            direction_match_rate: (selected > 0).then_some(
                                trend_logistic_confident_correct[index] as f64 / selected as f64,
                            ),
                            average_net_return_percent: (selected > 0)
                                .then_some(trend_logistic_confident_net[index] / selected as f64),
                            positive_net_rate: (selected > 0).then_some(
                                trend_logistic_confident_positive[index] as f64 / selected as f64,
                            ),
                        }
                    })
                    .collect(),
                signal_long: signal_long.metrics(),
                signal_short: signal_short.metrics(),
                all_signal_long: all_signal_long.metrics(),
                all_signal_short: all_signal_short.metrics(),
                logistic_long: logistic_long.metrics(),
                logistic_short: logistic_short.metrics(),
                research_logistic_long: research_logistic_long.metrics(),
                research_logistic_short: research_logistic_short.metrics(),
            });
        }
        result.sort_by(|a, b| {
            a.symbol
                .cmp(&b.symbol)
                .then(a.horizon_seconds.cmp(&b.horizon_seconds))
        });
        Ok(result)
    }

    async fn read_outcomes(&self) -> Result<Vec<SignalOutcome>> {
        Ok(self
            .read_recent_records(1, 100_000)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Outcome(outcome) => Some(outcome),
                _ => None,
            })
            .collect())
    }

    async fn read_snapshots(&self) -> Result<Vec<SignalSnapshot>> {
        Ok(self
            .read_recent_records(0, 100_000)
            .await?
            .into_iter()
            .filter_map(|record| match record {
                StoredSignalRecord::Snapshot(snapshot) => Some(snapshot),
                _ => None,
            })
            .collect())
    }

    pub async fn append(&self, snapshot: &SignalSnapshot) -> Result<()> {
        self.append_record(StoredSignalRecord::Snapshot(snapshot.clone()))
            .await
    }

    pub async fn append_outcome(&self, outcome: &SignalOutcome) -> Result<()> {
        self.append_record(StoredSignalRecord::Outcome(outcome.clone()))
            .await
    }

    pub async fn append_evaluation(&self, evaluation: &SignalEvaluation) -> Result<()> {
        self.append_record(StoredSignalRecord::Evaluation(evaluation.clone()))
            .await
    }

    async fn append_record(&self, record: StoredSignalRecord) -> Result<()> {
        self.append_records(vec![record]).await
    }

    pub async fn append_records(&self, records: Vec<StoredSignalRecord>) -> Result<()> {
        if records.is_empty() {
            return Ok(());
        }
        self.send_write(WriteOperation::Records(records)).await
    }

    async fn send_write(&self, operation: WriteOperation) -> Result<()> {
        let (ack, result) = oneshot::channel();
        self.writer
            .send(WriteRequest { operation, ack })
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer stopped"))?;
        result
            .await
            .map_err(|_| anyhow::anyhow!("signal database writer dropped acknowledgement"))?
            .map_err(|error| anyhow::anyhow!(error))
    }
}

fn run_database_writer(mut connection: Connection, mut receiver: mpsc::Receiver<WriteRequest>) {
    const MAX_BATCH_REQUESTS: usize = 128;
    while let Some(first) = receiver.blocking_recv() {
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH_REQUESTS {
            match receiver.try_recv() {
                Ok(request) => batch.push(request),
                Err(_) => break,
            }
        }
        let result = (|| -> Result<()> {
            // Do serialization and compression before taking SQLite's write lock.
            let mut signal_rows = Vec::new();
            let mut audit_rows = Vec::new();
            for request in &batch {
                match &request.operation {
                    WriteOperation::Records(records) => {
                        for record in records {
                            signal_rows.push((
                                record_timestamp(record) as i64,
                                record_kind(record),
                                compress_record(record)?,
                            ));
                        }
                    }
                    WriteOperation::Audit { decision_id, stage, timestamp, symbol, payload } => {
                        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
                        encoder.write_all(payload)?;
                        audit_rows.push((decision_id, stage, timestamp, symbol, encoder.finish()?));
                    }
                }
            }
            let transaction = connection.transaction()?;
            {
                let mut insert = transaction.prepare(
                    "INSERT INTO signal_records(timestamp, kind, payload) VALUES (?1, ?2, ?3)",
                )?;
                for (timestamp, kind, payload) in signal_rows {
                    insert.execute(params![timestamp, kind, payload])?;
                }
            }
            {
                let mut insert = transaction.prepare(
                    "INSERT INTO auto_trader_audit(decision_id, stage, timestamp, symbol, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(decision_id, stage) DO UPDATE SET timestamp=excluded.timestamp, symbol=excluded.symbol, payload=excluded.payload",
                )?;
                for (decision_id, stage, timestamp, symbol, payload) in audit_rows {
                    insert.execute(params![decision_id, stage, timestamp, symbol, payload])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })()
        .map_err(|error| error.to_string());
        for request in batch {
            let _ = request.ack.send(result.clone());
        }
    }
}
