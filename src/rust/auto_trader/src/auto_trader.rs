//! Signal-driven intraday paper trader with bounded, configurable order sizing.
use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    task::JoinSet,
    time::{Duration, interval},
};

const DEFAULT_MAX_ORDER_NOTIONAL: f64 = 100.0;
const DEFAULT_DAILY_NOTIONAL: f64 = 1_000.0;
const MIN_STRENGTH: f64 = 0.35;
const MIN_EXPLORATION_STRENGTH: f64 = 0.05;
const MIN_EXPLORATION_SCORE: f64 = 0.30;
const NEUTRAL_SCORE_DIRECTION_FLOOR: f64 = 0.15;
const PORTFOLIO_SHOCK_PERCENT: f64 = 0.01;
const MIN_HEDGE_CORRELATION_SAMPLES: usize = 30;
const MIN_HEDGE_CORRELATION: f64 = 0.70;
const MIN_SIDE_WALK_FORWARD_SAMPLES: usize = 30;
const DEEP_BOOK_BAND_BPS: f64 = 10.0;
const DEEP_BOOK_MAX_PARTICIPATION: f64 = 0.05;
const MIN_DEEP_BOOK_ORDER_NOTIONAL: f64 = 10.0;
const MIN_SCOUT_DEEP_BOOK_ORDER_NOTIONAL: f64 = 1.0;
const MIN_SIDE_DIRECTION_MATCH_RATE: f64 = 0.55;
const MIN_SIDE_POSITIVE_NET_RATE: f64 = 0.55;
const MIN_SIDE_POSITIVE_NET_LOWER_95: f64 = 0.45;
const SIGNAL_COOLDOWN_MS: u64 = 30_000;
const POST_STOP_REENTRY_COOLDOWN_MS: u64 = 120_000;
const EXPLORATION_NOTIONAL_FRACTION: f64 = 0.05;
const MIN_EXPLORATION_EVIDENCE_SAMPLES: usize = 10;
const SCOUT_MIN_STRENGTH: f64 = 0.15;
const SCOUT_MIN_ABS_SCORE: f64 = 0.20;
const SCOUT_MAX_ORDER_FRACTION: f64 = 0.05;
const SCOUT_MAX_ACTIVE_POSITIONS: usize = 5;
const SCOUT_MIN_STRENGTH_FLOOR: f64 = 0.0;
const SCOUT_MIN_SCORE_FLOOR: f64 = 0.01;
const SCOUT_NEUTRAL_SCORE_FLOOR: f64 = 0.01;
const SHADOW_MIN_ABS_SCORE: f64 = 0.05;
const DEFAULT_MAX_ENTRIES_PER_AUTO_POSITION: u32 = 1;
const NEGATIVE_EVIDENCE_FLOOR: f64 = 0.45;
const MATERIAL_STRENGTH_CHANGE: f64 = 0.02;
const MIN_EXECUTION_LEARNING_SAMPLES: usize = 10;
const PARAMETER_PROFILE_COUNT: usize = 3;
const DEFAULT_DAILY_REALIZED_LOSS_LIMIT: f64 = 10.0;
const MIN_DAILY_REALIZED_LOSS_LIMIT: f64 = 5.0;
const MIN_STOP_LOSS_PERCENT: f64 = 0.25;
const MAX_STOP_LOSS_PERCENT: f64 = 1.50;
const MIN_TAKE_PROFIT_PERCENT: f64 = 0.40;
const MAX_TAKE_PROFIT_PERCENT: f64 = 2.00;
const EXIT_POLICY_ADJUSTMENT_STEP: f64 = 0.05;
const MIN_EXIT_POLICY_SAMPLES: usize = 10;
const MIN_EXIT_REWARD_RISK_RATIO: f64 = 1.5;
const MAX_SIGNAL_AGE_SECONDS: u64 = 60;
const NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY: f64 = 0.75;
const LEARNED_STRENGTH_ADJUSTMENT_SAMPLES: usize = 5;
const LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE: f64 = 0.45;
const LEARNED_STRENGTH_FLOOR: f64 = 0.50;
const EXPLORATION_SCORE_MID_FLOOR: f64 = 0.40;
const EXPLORATION_SCORE_HIGH_FLOOR: f64 = 0.50;
const EXPLORATION_SCORE_MID_SAMPLES: usize = 10;
const EXPLORATION_SCORE_HIGH_SAMPLES: usize = 20;
const EXPLORATION_SCORE_MID_WIN_RATE: f64 = 0.45;
const EXPLORATION_SCORE_HIGH_WIN_RATE: f64 = 0.30;
const COUNTERFACTUAL_RELAXATION_SAMPLES: usize = 20;
const COUNTERFACTUAL_RELAXATION_POSITIVE_RATE: f64 = 0.55;
const COUNTERFACTUAL_SCORE_MIN_THRESHOLD: f64 = 0.20;
const COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD: f64 = 0.25;
const CONTEXT_RANGE_MIN_PROBABILITY: f64 = 0.65;
const CONTEXT_TREND_MIN_VOLATILITY_UNITS: f64 = 1.5;
const CONTEXT_VOLATILE_MIN_PERCENT: f64 = 1.0;
const ORDER_POST_ONLY_RANGE: f64 = 0.75;
const ORDER_POST_ONLY_AGGRESSOR_COST_BPS: f64 = 4.0;
const ORDER_GTD_RANGE: f64 = 0.60;
const ORDER_GTD_MIN_MISPRICING_BPS: f64 = 5.0;
const ORDER_GTD_MAX_MISPRICING_BPS: f64 = 8.0;
const ORDER_LIMIT_RANGE: f64 = 0.65;
const ORDER_STOP_LIMIT_VOLATILITY_PERCENT: f64 = 1.0;
const ORDER_STOP_LIMIT_TREND_UNITS: f64 = 1.5;
const ORDER_STOP_MARKET_TREND_UNITS: f64 = 2.5;
const ORDER_STOP_MARKET_STRENGTH: f64 = 0.65;
const ORDER_MARKET_PRESSURE: f64 = 0.60;
const ORDER_MARKET_STRENGTH: f64 = 0.60;
const ORDER_IOC_MOMENTUM_PERCENT: f64 = 0.12;
const ORDER_IOC_STRENGTH: f64 = 0.65;
const ORDER_VWAP_STRENGTH: f64 = 0.60;
const ORDER_TWAP_RANGE: f64 = 0.50;
const ORDER_TRAILING_VOLATILITY_PERCENT: f64 = 0.70;
const ORDER_FOK_STRENGTH: f64 = 0.50;

fn configured_taker_fee_rate() -> f64 {
    std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1)
        .unwrap_or(0.001)
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard {
    best: Vec<Signal>,
    walk_forward: Arc<Vec<WalkForward>>,
    #[serde(default)]
    missed_opportunity_bands: Vec<MissedOpportunityGateBand>,
    #[serde(default)]
    markets: Arc<BTreeMap<String, serde_json::Value>>,
}
#[derive(Clone, Deserialize)]
struct Signal {
    symbol: String,
    direction: String,
    strength: f64,
    timestamp: u64,
    stale: bool,
    #[serde(default)]
    score: f64,
    #[serde(default)]
    ranging_probability: Option<f64>,
    #[serde(default)]
    components: SignalComponents,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct SignalComponents {
    momentum_10s_percent: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
    #[serde(default)]
    trade_flow_imbalance: Option<f64>,
    #[serde(default)]
    book_imbalance: Option<f64>,
    volatility_1m_percent: Option<f64>,
    mispricing_vs_vwap_bps: Option<f64>,
    pressure_without_price_move: Option<f64>,
    aggressor_cost_bps: Option<f64>,
}
#[derive(Clone, Deserialize, Serialize)]
struct WalkForward {
    symbol: String,
    horizon_seconds: u64,
    #[serde(default)]
    all_signal_long: SideWalkForwardMetrics,
    #[serde(default)]
    all_signal_short: SideWalkForwardMetrics,
    #[serde(default)]
    research_logistic_long: SideWalkForwardMetrics,
    #[serde(default)]
    research_logistic_short: SideWalkForwardMetrics,
}
#[derive(Clone, Default, Deserialize, Serialize)]
struct SideWalkForwardMetrics {
    sample_count: usize,
    direction_match_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
    positive_net_rate: Option<f64>,
    positive_net_rate_lower_95: Option<f64>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MissedOpportunityGateBand {
    gate: String,
    threshold: f64,
    #[serde(default)]
    horizon_seconds: u64,
    #[serde(default)]
    sample_count: usize,
    #[serde(default)]
    average_net_return_percent: Option<f64>,
    #[serde(default)]
    positive_net_rate: Option<f64>,
    #[serde(default)]
    time_to_profit_sample_count: usize,
    #[serde(default)]
    time_to_profit_rate: Option<f64>,
    #[serde(default)]
    average_time_to_profit_seconds: Option<f64>,
    #[serde(default)]
    stop_target_sample_count: usize,
    #[serde(default)]
    stop_target_average_net_return_percent: Option<f64>,
    #[serde(default)]
    stop_target_positive_net_rate: Option<f64>,
}

#[derive(Clone, Copy)]
struct DeepBookAssessment {
    available_within_band: f64,
    notional_cap: f64,
    impact_bps: f64,
    bid_depth_10bps: f64,
    ask_depth_10bps: f64,
    depth_imbalance_10bps: f64,
}

fn market_levels(market: &serde_json::Value, side: &str) -> Vec<(f64, f64)> {
    market
        .get(side)
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|level| {
            let price = level.get("price")?.as_f64()?;
            let size = level.get("size")?.as_f64()?;
            (price.is_finite() && size.is_finite() && price > 0.0 && size > 0.0)
                .then_some((price, size))
        })
        .collect()
}

fn quote_depth_within_band(levels: &[(f64, f64)], mid: f64, is_bid: bool) -> f64 {
    let band_price = if is_bid {
        mid * (1.0 - DEEP_BOOK_BAND_BPS / 10_000.0)
    } else {
        mid * (1.0 + DEEP_BOOK_BAND_BPS / 10_000.0)
    };
    levels
        .iter()
        .filter(|(price, _)| {
            if is_bid {
                *price >= band_price
            } else {
                *price <= band_price
            }
        })
        .map(|(price, size)| price * size)
        .sum()
}

fn assess_deep_book(
    market: &serde_json::Value,
    symbol: &str,
    is_buy: bool,
    target_notional: f64,
    minimum_order_notional: f64,
) -> Option<DeepBookAssessment> {
    let quote = symbol.rsplit('-').next().unwrap_or_default();
    if !matches!(quote, "USDT" | "USDC") {
        return None;
    }
    let bids = market_levels(market, "bids");
    let asks = market_levels(market, "asks");
    let best_bid = bids.first()?.0;
    let best_ask = asks.first()?.0;
    if best_ask < best_bid {
        return None;
    }
    let mid = (best_bid + best_ask) / 2.0;
    let bid_depth_10bps = quote_depth_within_band(&bids, mid, true);
    let ask_depth_10bps = quote_depth_within_band(&asks, mid, false);
    let total_depth = bid_depth_10bps + ask_depth_10bps;
    let depth_imbalance_10bps = if total_depth > 0.0 {
        (bid_depth_10bps - ask_depth_10bps) / total_depth
    } else {
        0.0
    };
    let levels = if is_buy { &asks } else { &bids };
    let available_within_band = if is_buy {
        ask_depth_10bps
    } else {
        bid_depth_10bps
    };
    let notional_cap = available_within_band * DEEP_BOOK_MAX_PARTICIPATION;
    let planned_notional = target_notional.min(notional_cap);
    if planned_notional < minimum_order_notional {
        return Some(DeepBookAssessment {
            available_within_band,
            notional_cap,
            impact_bps: f64::INFINITY,
            bid_depth_10bps,
            ask_depth_10bps,
            depth_imbalance_10bps,
        });
    }
    let mut remaining = planned_notional / mid;
    let mut filled = 0.0;
    let mut quote_value = 0.0;
    for (price, size) in levels {
        let quantity = remaining.min(*size);
        filled += quantity;
        quote_value += quantity * price;
        remaining -= quantity;
        if remaining <= 1e-12 {
            break;
        }
    }
    let impact_bps = if remaining > 1e-12 || filled <= 0.0 {
        f64::INFINITY
    } else {
        let average = quote_value / filled;
        if is_buy {
            (average / mid - 1.0) * 10_000.0
        } else {
            (1.0 - average / mid) * 10_000.0
        }
    };
    Some(DeepBookAssessment {
        available_within_band,
        notional_cap,
        impact_bps,
        bid_depth_10bps,
        ask_depth_10bps,
        depth_imbalance_10bps,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum AutoOrderType {
    Market,
    Ioc,
    Fok,
    Limit,
    Gtd,
    PostOnlyLimit,
    Barrier,
    StopMarket,
    StopLimit,
    TrailingStop,
    Vwap,
    Twap,
}
impl Default for AutoOrderType {
    fn default() -> Self {
        Self::Ioc
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    enabled: bool,
    #[serde(default = "default_order_limit")]
    max_order_notional: f64,
    #[serde(default = "default_daily_limit")]
    daily_notional_limit: f64,
    #[serde(default)]
    used_notional_today: f64,
    #[serde(default)]
    realized_pnl: f64,
    #[serde(default = "default_daily_loss_limit")]
    daily_realized_loss_limit: f64,
    #[serde(default = "default_exploration_percent")]
    exploration_percent: f64,
    #[serde(default = "default_risk_appetite")]
    risk_appetite: f64,
    #[serde(default = "default_max_open_positions")]
    max_open_positions: usize,
    #[serde(default = "default_max_entries_per_auto_position")]
    max_entries_per_auto_position: u32,
    #[serde(default)]
    daily_loss_limit_reduced: bool,
    #[serde(default = "default_stop_loss_percent")]
    stop_loss_percent: f64,
    #[serde(default = "default_take_profit_percent")]
    take_profit_percent: f64,
    #[serde(default = "default_vwap_duration")]
    vwap_duration_minutes: u64,
    #[serde(default = "default_vwap_participation")]
    vwap_participation_percent: f64,
    #[serde(default = "default_limit_offset")]
    limit_offset_bps: f64,
    #[serde(default = "default_trigger_offset")]
    trigger_offset_bps: f64,
    #[serde(default = "default_trailing_distance")]
    trailing_distance_percent: f64,
    #[serde(default)]
    closed_trade_returns_percent: Vec<f64>,
    #[serde(default)]
    daily_realized_pnl: f64,
    #[serde(default)]
    pnl_utc_day: String,
    #[serde(default)]
    utc_day: String,
    #[serde(default)]
    orders: Vec<TraderOrder>,
    #[serde(default)]
    positions: BTreeMap<String, TraderPosition>,
    #[serde(default)]
    learning: BTreeMap<String, LearningStats>,
    #[serde(default)]
    last_hedge_at: BTreeMap<String, u64>,
    #[serde(default)]
    last_decision: String,
    #[serde(default)]
    last_decision_at: u64,
    #[serde(default)]
    last_risk_note: String,
    #[serde(default)]
    risk_limited_sizing: bool,
    #[serde(default)]
    last_signal_at: BTreeMap<String, u64>,
    #[serde(default)]
    last_signal_direction: BTreeMap<String, String>,
    #[serde(default)]
    last_signal_strength: BTreeMap<String, f64>,
}
fn default_order_limit() -> f64 {
    DEFAULT_MAX_ORDER_NOTIONAL
}
fn default_daily_loss_limit() -> f64 {
    DEFAULT_DAILY_REALIZED_LOSS_LIMIT
}
fn default_exploration_percent() -> f64 {
    EXPLORATION_NOTIONAL_FRACTION * 100.0
}
fn default_risk_appetite() -> f64 { 50.0 }
fn default_max_open_positions() -> usize {
    20
}
fn default_max_entries_per_auto_position() -> u32 {
    DEFAULT_MAX_ENTRIES_PER_AUTO_POSITION
}
fn default_stop_loss_percent() -> f64 {
    0.50
}
fn default_take_profit_percent() -> f64 {
    0.80
}
fn default_vwap_duration() -> u64 {
    5
}
fn default_vwap_participation() -> f64 {
    20.0
}
fn default_limit_offset() -> f64 {
    0.0
}
fn default_trigger_offset() -> f64 {
    20.0
}
fn default_trailing_distance() -> f64 {
    0.10
}
fn default_daily_limit() -> f64 {
    DEFAULT_DAILY_NOTIONAL
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_order_notional: default_order_limit(),
            daily_notional_limit: default_daily_limit(),
            used_notional_today: 0.0,
            realized_pnl: 0.0,
            daily_realized_loss_limit: default_daily_loss_limit(),
            exploration_percent: default_exploration_percent(),
            risk_appetite: default_risk_appetite(),
            max_open_positions: default_max_open_positions(),
            max_entries_per_auto_position: default_max_entries_per_auto_position(),
            daily_loss_limit_reduced: false,
            stop_loss_percent: default_stop_loss_percent(),
            take_profit_percent: default_take_profit_percent(),
            vwap_duration_minutes: default_vwap_duration(),
            vwap_participation_percent: default_vwap_participation(),
            limit_offset_bps: default_limit_offset(),
            trigger_offset_bps: default_trigger_offset(),
            trailing_distance_percent: default_trailing_distance(),
            closed_trade_returns_percent: Vec::new(),
            daily_realized_pnl: 0.0,
            pnl_utc_day: utc_day(),
            utc_day: utc_day(),
            orders: Vec::new(),
            positions: BTreeMap::new(),
            learning: BTreeMap::new(),
            last_hedge_at: BTreeMap::new(),
            last_decision: String::new(),
            last_decision_at: 0,
            last_risk_note: String::new(),
            risk_limited_sizing: false,
            last_signal_at: BTreeMap::new(),
            last_signal_direction: BTreeMap::new(),
            last_signal_strength: BTreeMap::new(),
        }
    }
}
#[derive(Serialize)]
struct StatusResponse {
    enabled: bool,
    max_order_notional: f64,
    daily_notional_limit: f64,
    daily_realized_loss_limit: f64,
    exploration_percent: f64,
    risk_appetite: f64,
    max_open_positions: usize,
    max_entries_per_auto_position: u32,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    vwap_duration_minutes: u64,
    vwap_participation_percent: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
    used_notional_today: f64,
    last_decision: String,
    last_decision_at: u64,
    decisions_since_trade: u64,
    gap_reasons: Vec<GapReason>,
    gate_hits: Vec<GateHit>,
    symbol_gate_hits: Vec<SymbolGateHit>,
    latest_signal_diagnostic: String,
}
#[derive(Serialize)]
struct GapReason {
    reason: String,
    count: u64,
}
#[derive(Serialize)]
struct GateHit {
    gate: String,
    count: u64,
}
#[derive(Serialize)]
struct SymbolGateHit {
    symbol: String,
    direction: String,
    gate: String,
    count: u64,
    last_detail: String,
    distance_to_pass: Option<f64>,
}
#[derive(Default)]
struct SymbolGateCount {
    count: u64,
    last_detail: String,
    distance_to_pass: Option<f64>,
}
#[derive(Clone, Deserialize)]
struct SettingsRequest {
    max_order_notional: f64,
    daily_notional_limit: f64,
    daily_realized_loss_limit: f64,
    exploration_percent: f64,
    #[serde(default)]
    risk_appetite: Option<f64>,
    max_open_positions: usize,
    #[serde(default)]
    max_entries_per_auto_position: Option<u32>,
    #[serde(default)]
    stop_loss_percent: Option<f64>,
    #[serde(default)]
    take_profit_percent: Option<f64>,
    #[serde(default)]
    vwap_duration_minutes: Option<u64>,
    #[serde(default)]
    vwap_participation_percent: Option<f64>,
    #[serde(default)]
    limit_offset_bps: Option<f64>,
    #[serde(default)]
    trigger_offset_bps: Option<f64>,
    #[serde(default)]
    trailing_distance_percent: Option<f64>,
}
#[derive(Clone, Deserialize)]
struct EnabledRequest {
    enabled: bool,
}
#[derive(Clone, Serialize)]
struct DecisionParameter {
    section: String,
    name: String,
    value: f64,
    unit: String,
}
#[derive(Clone, Serialize)]
struct ContextDecisionParameters {
    context: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    strength_floor: f64,
    exploration_score_floor: f64,
    exploration_allowed: bool,
}
#[derive(Clone, Serialize)]
struct LatestExecutionParameters {
    timestamp: u64,
    symbol: String,
    context: String,
    baseline_order_type: String,
    selected_order_type: String,
    profile: usize,
    vwap_duration_minutes: u64,
    vwap_participation_percent: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}
#[derive(Clone, Serialize)]
struct ActiveExitPolicy {
    stop_loss_percent: f64,
    take_profit_percent: f64,
}
#[derive(Serialize)]
struct DecisionParametersResponse {
    refreshed_at: u64,
    thresholds: Vec<DecisionParameter>,
    contexts: Vec<ContextDecisionParameters>,
    active_exit_policy: ActiveExitPolicy,
    latest_execution: Option<LatestExecutionParameters>,
}

#[derive(Default)]
struct GapSummary {
    decisions: u64,
    reasons: BTreeMap<String, u64>,
    gate_hits: BTreeMap<String, u64>,
    symbol_gate_hits: BTreeMap<(String, String, String), SymbolGateCount>,
    latest_detail: String,
}

#[derive(Clone)]
struct AutoTraderState {
    client: reqwest::Client,
    enabled: Arc<AtomicBool>,
    settings: Arc<Mutex<Settings>>,
    latest_missed_opportunity_bands: Arc<Mutex<Vec<MissedOpportunityGateBand>>>,
    latest_execution_parameters: Arc<Mutex<Option<LatestExecutionParameters>>>,
    cycle_lock: Arc<Mutex<()>>,
    pending_signals: Arc<Mutex<BTreeMap<String, Dashboard>>>,
    signal_notify: Arc<Notify>,
    audit_tx: mpsc::Sender<serde_json::Value>,
    execution_audit_tx: mpsc::UnboundedSender<serde_json::Value>,
    gap_summary: Arc<std::sync::Mutex<GapSummary>>,
    insight_processing_ms: Arc<Mutex<Vec<f64>>>,
    cycle_lock_wait_ms: Arc<Mutex<BTreeMap<String, Vec<f64>>>>,
    cycle_lock_hold_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    settings_path: PathBuf,
}

#[derive(Clone, Serialize, Deserialize)]
struct TraderOrder {
    timestamp: u64,
    #[serde(default)]
    triggered_at: Option<u64>,
    symbol: String,
    side: String,
    order_type: String,
    #[serde(default)]
    purpose: String,
    #[serde(default)]
    decision_id: Option<String>,
    quantity: f64,
    filled_quantity: f64,
    #[serde(default)]
    fill_rate: f64,
    average_price: Option<f64>,
    #[serde(default)]
    fees_paid: f64,
    #[serde(default)]
    reserved_notional: f64,
    #[serde(default)]
    execution_order_id: Option<String>,
    #[serde(default)]
    processed_fill_trade_ids: Vec<String>,
    #[serde(default)]
    learning_context: Option<String>,
    #[serde(default)]
    book_bid_depth_10bps: Option<f64>,
    #[serde(default)]
    book_ask_depth_10bps: Option<f64>,
    #[serde(default)]
    book_depth_imbalance_10bps: Option<f64>,
    #[serde(default)]
    estimated_sweep_bps: Option<f64>,
    #[serde(default)]
    estimated_ioc_all_in_bps: Option<f64>,
    #[serde(default)]
    decision_bid: Option<f64>,
    #[serde(default)]
    decision_ask: Option<f64>,
    #[serde(default)]
    decision_mid: Option<f64>,
    #[serde(default)]
    submitted_limit_price: Option<f64>,
    #[serde(default)]
    target_price: Option<f64>,
    #[serde(default)]
    limit_to_mid_bps: Option<f64>,
    #[serde(default)]
    limit_to_crossing_quote_bps: Option<f64>,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    failure_reason: Option<String>,
    #[serde(default)]
    stage_timings_ms: BTreeMap<String, f64>,
    status: String,
    realized_pnl: f64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct TraderPosition {
    quantity: f64,
    average_entry_price: f64,
    entry_fees: f64,
    realized_pnl: f64,
    #[serde(default)]
    entries: u32,
    #[serde(default)]
    entry_order_type: String,
    #[serde(default)]
    entry_context: String,
    #[serde(default)]
    entry_decision_id: Option<String>,
    #[serde(default)]
    scout_entry: bool,
    #[serde(default)]
    protective_stops: Vec<ProtectiveStop>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct ProtectiveStop {
    barrier_id: String,
    order_timestamp: u64,
    quantity: f64,
    processed_quantity: f64,
    processed_notional: f64,
    processed_fee: f64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct LearningStats {
    samples: usize,
    positive: usize,
    realized_pnl: f64,
}
#[derive(Serialize)]
struct PnlBreakdown {
    symbol: String,
    direction: String,
    purpose: String,
    realized_pnl: f64,
    outcomes: usize,
}
#[derive(Serialize)]
struct ActivityResponse {
    orders: Vec<TraderOrder>,
    open_position_symbols: Vec<String>,
    pnl_breakdown: Vec<PnlBreakdown>,
    realized_pnl: f64,
    unrealized_pnl: Option<f64>,
    exit_learning_samples: usize,
    closed_trade_average_return_percent: Option<f64>,
    closed_trade_wins: usize,
    closed_trade_losses: usize,
    closed_trade_flat: usize,
    learning_samples: usize,
    learning_win_rate: Option<f64>,
    preferred_order_type: Option<String>,
    parameter_profile_samples: [usize; 3],
    scout_open_positions: usize,
    scout_max_active_positions: usize,
    scout_learning_samples: usize,
    scout_learning_win_rate: Option<f64>,
    scout_learning_realized_pnl: f64,
    insight_processing_latest_ms: Option<f64>,
    insight_processing_average_ms: Option<f64>,
    insight_processing_p95_ms: Option<f64>,
    insight_processing_samples: usize,
    cycle_lock_wait_ms: BTreeMap<String, CycleLockWaitSummary>,
    cycle_lock_hold_ms: BTreeMap<String, CycleLockWaitSummary>,
}
#[derive(Clone, Serialize)]
struct CycleLockWaitSummary {
    latest_ms: f64,
    average_ms: f64,
    p95_ms: f64,
    samples: usize,
}
struct CycleLockGuard {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    hold_samples: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    worker: String,
    acquired_at: Instant,
}

impl Drop for CycleLockGuard {
    fn drop(&mut self) {
        if let Ok(mut samples) = self.hold_samples.lock() {
            let values = samples.entry(self.worker.clone()).or_default();
            values.push(self.acquired_at.elapsed().as_secs_f64() * 1000.0);
            if values.len() > 500 {
                values.remove(0);
            }
        }
    }
}

#[derive(Serialize)]
struct CycleResult {
    status: &'static str,
    reason: Option<String>,
    symbol: Option<String>,
    side: Option<String>,
    quantity: Option<f64>,
    order_type: Option<String>,
    order_response: Option<serde_json::Value>,
}

pub fn router() -> Router {
    let settings_path = settings_path();
    let stored_state = std::fs::read(&settings_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let mut settings = stored_state
        .as_ref()
        .and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok())
        .unwrap_or_default();
    // The previous release defaulted this new cap to three, which can strand existing books above the cap.
    if settings.max_open_positions == 3 {
        settings.max_open_positions = default_max_open_positions();
    }
    if stored_state
        .as_ref()
        .is_some_and(|value| value.get("realized_pnl").is_none())
    {
        settings.realized_pnl = settings
            .positions
            .values()
            .map(|position| position.realized_pnl)
            .sum();
    }
    for position in settings.positions.values_mut() {
        if position.quantity.abs() > 1e-12 && position.entries == 0 {
            position.entries = 1;
        }
    }
    settings.used_notional_today = allocated_notional(&settings);
    if settings.utc_day != utc_day() {
        settings.utc_day = utc_day();
    }
    let enabled = std::env::var("AUTO_TRADER_ENABLED")
        .map(|value| value == "true")
        .unwrap_or(settings.enabled);
    settings.enabled = enabled;
    let (audit_tx, mut audit_rx) = mpsc::channel(256);
    let (execution_audit_tx, mut execution_audit_rx) = mpsc::unbounded_channel();
    let audit_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                Some(payload) = execution_audit_rx.recv() => store_execution_audit_until_saved(&audit_client, &payload).await,
                Some(payload) = audit_rx.recv() => { store_auto_trader_audit(&audit_client, &payload).await; },
                else => break,
            }
        }
    });
    let state = AutoTraderState {
        // Every worker shares the cycle lock. Bound upstream calls so an
        // unresponsive position manager cannot stall signal decisions forever.
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new()),
        enabled: Arc::new(AtomicBool::new(enabled)),
        settings: Arc::new(Mutex::new(settings)),
        latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
        latest_execution_parameters: Arc::new(Mutex::new(None)),
        cycle_lock: Arc::new(Mutex::new(())),
        pending_signals: Arc::new(Mutex::new(BTreeMap::new())),
        signal_notify: Arc::new(Notify::new()),
        audit_tx,
        execution_audit_tx,
        gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
        insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
        cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
        cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
        settings_path,
    };
    let signal_worker_state = state.clone();
    tokio::spawn(async move {
        loop {
            let signal_ready = signal_worker_state.signal_notify.notified();
            tokio::pin!(signal_ready);
            let pending = {
                let mut pending_signals = signal_worker_state.pending_signals.lock().await;
                let symbol = pending_signals.keys().next().cloned();
                symbol.and_then(|symbol| pending_signals.remove(&symbol))
            };
            let dashboard = match pending {
                Some(dashboard) => dashboard,
                None => {
                    signal_ready.await;
                    continue;
                }
            };
            if signal_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_cycle(&signal_worker_state, dashboard).await {
                    crate::service_log!("ERROR", "Auto trader signal cycle failed: {}", error.1);
                }
            }
        }
    });

    let position_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(5));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if position_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_position_maintenance(&position_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader position maintenance failed: {}",
                        error.1
                    );
                }
            }
        }
    });

    let hedge_worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(30));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if hedge_worker_state.enabled.load(Ordering::Relaxed) {
                if let Err(error) = run_hedge_evaluation(&hedge_worker_state).await {
                    crate::service_log!(
                        "ERROR",
                        "Auto trader hedge evaluation failed: {}",
                        error.1
                    );
                }
            }
        }
    });
    Router::new()
        .route("/positions/auto-trader", get(get_status).put(set_enabled))
        .route(
            "/positions/auto-trader/parameters",
            get(get_decision_parameters),
        )
        .route(
            "/internal/auto-trader/signal",
            axum::routing::post(signal_ready),
        )
        .route(
            "/positions/auto-trader/settings",
            axum::routing::put(update_settings),
        )
        .route("/positions/auto-trader/activity", get(get_activity))
        .with_state(state)
}

async fn signal_ready(
    State(state): State<AutoTraderState>,
    Json(dashboard): Json<Dashboard>,
) -> StatusCode {
    static INGRESS_REQUESTS: AtomicU64 = AtomicU64::new(0);
    static INGRESS_SYMBOLS: AtomicU64 = AtomicU64::new(0);
    static LAST_INGRESS_LOG_MS: AtomicU64 = AtomicU64::new(0);

    let enabled = state.enabled.load(Ordering::Relaxed);
    let incoming_symbols = dashboard.best.len() as u64;
    let requests = INGRESS_REQUESTS.fetch_add(1, Ordering::Relaxed) + 1;
    let symbols = INGRESS_SYMBOLS.fetch_add(incoming_symbols, Ordering::Relaxed) + incoming_symbols;
    let pending_symbols = if enabled {
        let mut pending_signals = state.pending_signals.lock().await;
        for signal in dashboard.best {
            let symbol = signal.symbol.clone();
            pending_signals.insert(
                symbol,
                Dashboard {
                    best: vec![signal],
                    walk_forward: dashboard.walk_forward.clone(),
                    missed_opportunity_bands: dashboard.missed_opportunity_bands.clone(),
                    markets: dashboard.markets.clone(),
                },
            );
        }
        let pending_symbols = pending_signals.len();
        drop(pending_signals);
        state.signal_notify.notify_one();
        pending_symbols
    } else {
        0
    };

    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last_logged = LAST_INGRESS_LOG_MS.load(Ordering::Relaxed);
    if now_ms.saturating_sub(last_logged) >= 60_000
        && LAST_INGRESS_LOG_MS
            .compare_exchange(last_logged, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    {
        crate::service_log!(
            if enabled { "INFO" } else { "WARN" },
            "AutoTrader signal ingress: enabled={enabled} cumulative_requests={requests} cumulative_symbols={symbols} pending_symbols={pending_symbols}"
        );
    }
    StatusCode::ACCEPTED
}

async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DecisionParametersResponse> {
    let (learning, active_exit_policy, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        (
            settings.learning.clone(),
            ActiveExitPolicy {
                stop_loss_percent: settings.stop_loss_percent,
                take_profit_percent: settings.take_profit_percent,
            },
            settings.max_entries_per_auto_position,
        )
    };
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
    let contexts = ["long", "short"]
        .into_iter()
        .flat_map(|side| {
            ["range", "trend", "volatile", "mixed"].map(move |regime| format!("{side}:{regime}"))
        })
        .map(|context| {
            let stats = context_learning_stats(&learning, &context);
            let positive_rate =
                (stats.samples > 0).then_some(stats.positive as f64 / stats.samples as f64);
            let strength_floor = adaptive_missed_opportunity_floor(
                adaptive_strength_floor(&learning, &context),
                "strength",
                &bands,
            );
            let exploration_score_floor =
                adaptive_exploration_score_floor(&learning, &context, &bands);
            ContextDecisionParameters {
                exploration_allowed: !context.ends_with(":mixed"),
                context,
                learning_samples: stats.samples,
                positive_rate,
                strength_floor,
                exploration_score_floor,
            }
        })
        .collect();
    let thresholds = vec![
        parameter(
            "Signal qualification",
            "Maximum signal age",
            MAX_SIGNAL_AGE_SECONDS as f64,
            "seconds",
        ),
        parameter(
            "Signal qualification",
            "Neutral score direction floor",
            NEUTRAL_SCORE_DIRECTION_FLOOR,
            "score",
        ),
        parameter(
            "Signal qualification",
            "Neutral inference maximum range probability",
            NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY,
            "probability",
        ),
        parameter(
            "Signal qualification",
            "Base strict strength floor",
            MIN_STRENGTH,
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Strict strength floor after poor learning",
            LEARNED_STRENGTH_FLOOR,
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Samples before strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Positive rate triggering strict floor adjustment",
            LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Minimum exploration strength",
            MIN_EXPLORATION_STRENGTH,
            "strength",
        ),
        parameter(
            "Signal qualification",
            "Base exploration score floor",
            MIN_EXPLORATION_SCORE,
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 10 weak samples",
            EXPLORATION_SCORE_MID_FLOOR,
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score floor after 20 weak samples",
            EXPLORATION_SCORE_HIGH_FLOOR,
            "score",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 10-sample adjustment",
            EXPLORATION_SCORE_MID_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration score samples for 20-sample adjustment",
            EXPLORATION_SCORE_HIGH_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Signal qualification",
            "Exploration adjustment positive-rate boundaries",
            EXPLORATION_SCORE_MID_WIN_RATE,
            "rate",
        ),
        parameter(
            "Signal qualification",
            "Exploration stronger adjustment boundary",
            EXPLORATION_SCORE_HIGH_WIN_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum strict walk-forward samples",
            MIN_SIDE_WALK_FORWARD_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum direction match rate",
            MIN_SIDE_DIRECTION_MATCH_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum positive net rate",
            MIN_SIDE_POSITIVE_NET_RATE,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum 95% positive-rate lower bound",
            MIN_SIDE_POSITIVE_NET_LOWER_95,
            "rate",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration evidence samples",
            MIN_EXPLORATION_EVIDENCE_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Historical evidence",
            "Minimum exploration positive rate",
            NEGATIVE_EVIDENCE_FLOOR,
            "rate",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum resolved stop/target samples",
            COUNTERFACTUAL_RELAXATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum positive stop/target rate",
            COUNTERFACTUAL_RELAXATION_POSITIVE_RATE,
            "rate",
        ),
        parameter(
            "Order selection",
            "Range probability for post-only limit",
            ORDER_POST_ONLY_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "Aggressor cost for post-only limit",
            ORDER_POST_ONLY_AGGRESSOR_COST_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for GTD",
            ORDER_GTD_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for GTD lower bound",
            ORDER_GTD_MIN_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for limit",
            ORDER_LIMIT_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for limit",
            ORDER_GTD_MAX_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Volatility for stop-limit",
            ORDER_STOP_LIMIT_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Trend for stop-limit",
            ORDER_STOP_LIMIT_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Trend for stop-market",
            ORDER_STOP_MARKET_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Strength for stop-market",
            ORDER_STOP_MARKET_STRENGTH,
            "strength",
        ),
        parameter(
            "Order selection",
            "Pressure for market order",
            ORDER_MARKET_PRESSURE,
            "pressure",
        ),
        parameter(
            "Order selection",
            "Strength for market order",
            ORDER_MARKET_STRENGTH,
            "strength",
        ),
        parameter(
            "Order selection",
            "Momentum for IOC",
            ORDER_IOC_MOMENTUM_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for IOC",
            ORDER_IOC_STRENGTH,
            "strength",
        ),
        parameter(
            "Order selection",
            "Strength for VWAP",
            ORDER_VWAP_STRENGTH,
            "strength",
        ),
        parameter(
            "Order selection",
            "Range probability for TWAP",
            ORDER_TWAP_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "Volatility for trailing stop",
            ORDER_TRAILING_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for FOK",
            ORDER_FOK_STRENGTH,
            "strength",
        ),
        parameter(
            "Market context",
            "Range regime probability",
            CONTEXT_RANGE_MIN_PROBABILITY,
            "probability",
        ),
        parameter(
            "Market context",
            "Trend regime volatility units",
            CONTEXT_TREND_MIN_VOLATILITY_UNITS,
            "volatility units",
        ),
        parameter(
            "Market context",
            "Volatile regime threshold",
            CONTEXT_VOLATILE_MIN_PERCENT,
            "%",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed strength threshold",
            COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD,
            "strength",
        ),
        parameter(
            "Counterfactual relaxation",
            "Minimum relaxed score threshold",
            COUNTERFACTUAL_SCORE_MIN_THRESHOLD,
            "score",
        ),
        parameter(
            "Learning",
            "Minimum execution learning samples",
            MIN_EXECUTION_LEARNING_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Learning",
            "Minimum profitable order-type hit rate",
            0.55,
            "rate",
        ),
        parameter(
            "Learning",
            "Number of parameter profiles",
            PARAMETER_PROFILE_COUNT as f64,
            "profiles",
        ),
        parameter(
            "Risk and hedging",
            "Portfolio shock threshold",
            PORTFOLIO_SHOCK_PERCENT * 100.0,
            "%",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation samples",
            MIN_HEDGE_CORRELATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation",
            MIN_HEDGE_CORRELATION,
            "correlation",
        ),
        parameter(
            "Risk and re-entry",
            "Signal cooldown",
            SIGNAL_COOLDOWN_MS as f64 / 1000.0,
            "seconds",
        ),
        parameter(
            "Risk and re-entry",
            "Cooldown after losing stop",
            POST_STOP_REENTRY_COOLDOWN_MS as f64 / 1000.0,
            "seconds",
        ),
        parameter(
            "Risk and re-entry",
            "Required re-entry strength improvement",
            MATERIAL_STRENGTH_CHANGE,
            "strength",
        ),
        parameter(
            "Risk and re-entry",
            "Entries per position",
            max_entries_per_auto_position as f64,
            "entries",
        ),
        parameter(
            "Risk and re-entry",
            "Deep-book impact ceiling",
            DEEP_BOOK_BAND_BPS,
            "bps",
        ),
        parameter(
            "Risk and re-entry",
            "Maximum depth participation",
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            "%",
        ),
        parameter(
            "Risk and re-entry",
            "Minimum deep-book order",
            MIN_DEEP_BOOK_ORDER_NOTIONAL,
            "USD",
        ),
        parameter(
            "Exit learning",
            "Minimum exit samples",
            MIN_EXIT_POLICY_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Exit learning",
            "Reward/risk floor",
            MIN_EXIT_REWARD_RISK_RATIO,
            "ratio",
        ),
        parameter(
            "Exit learning",
            "Exit adjustment step",
            EXIT_POLICY_ADJUSTMENT_STEP,
            "%",
        ),
        parameter(
            "Exit learning",
            "Stop-loss minimum",
            MIN_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit learning",
            "Stop-loss maximum",
            MAX_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit learning",
            "Take-profit minimum",
            MIN_TAKE_PROFIT_PERCENT,
            "%",
        ),
        parameter(
            "Exit learning",
            "Take-profit maximum",
            MAX_TAKE_PROFIT_PERCENT,
            "%",
        ),
    ];
    let latest_execution = state.latest_execution_parameters.lock().await.clone();
    let refreshed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Json(DecisionParametersResponse {
        refreshed_at,
        thresholds,
        contexts,
        active_exit_policy,
        latest_execution,
    })
}

fn parameter(section: &str, name: &str, value: f64, unit: &str) -> DecisionParameter {
    DecisionParameter {
        section: section.to_owned(),
        name: name.to_owned(),
        value,
        unit: unit.to_owned(),
    }
}
async fn set_enabled(
    State(state): State<AutoTraderState>,
    Json(request): Json<EnabledRequest>,
) -> Json<StatusResponse> {
    state.enabled.store(request.enabled, Ordering::Relaxed);
    let mut settings = state.settings.lock().await;
    settings.enabled = request.enabled;
    if let Err(error) = persist_settings(&state.settings_path, &settings) {
        crate::service_log!(
            "ERROR",
            "Could not persist Auto Trader enabled state: {}",
            error.1
        );
    }
    Json(status(&state, &settings))
}
async fn update_settings(
    State(state): State<AutoTraderState>,
    Json(request): Json<SettingsRequest>,
) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if !request.max_order_notional.is_finite()
        || request.max_order_notional <= 0.0
        || !request.daily_notional_limit.is_finite()
        || request.daily_notional_limit < request.max_order_notional
        || !request.daily_realized_loss_limit.is_finite()
        || request.daily_realized_loss_limit <= 0.0
        || !request.exploration_percent.is_finite()
        || !(0.0..=100.0).contains(&request.exploration_percent)
        || request.risk_appetite.is_some_and(|value| !value.is_finite() || !(0.0..=100.0).contains(&value))
        || request.stop_loss_percent.is_some_and(|value| {
            !value.is_finite() || !(MIN_STOP_LOSS_PERCENT..=MAX_STOP_LOSS_PERCENT).contains(&value)
        })
        || request.take_profit_percent.is_some_and(|value| {
            !value.is_finite()
                || !(MIN_TAKE_PROFIT_PERCENT..=MAX_TAKE_PROFIT_PERCENT).contains(&value)
        })
        || request
            .vwap_duration_minutes
            .is_some_and(|value| !(5..=390).contains(&value))
        || request
            .vwap_participation_percent
            .is_some_and(|value| !value.is_finite() || !(1.0..=50.0).contains(&value))
        || request
            .limit_offset_bps
            .is_some_and(|value| !value.is_finite() || !(-10.0..=10.0).contains(&value))
        || request
            .trigger_offset_bps
            .is_some_and(|value| !value.is_finite() || !(5.0..=100.0).contains(&value))
        || request
            .trailing_distance_percent
            .is_some_and(|value| !value.is_finite() || !(0.05..=1.0).contains(&value))
        || request
            .max_entries_per_auto_position
            .is_some_and(|value| !(1..=10).contains(&value))
        || !(1..=20).contains(&request.max_open_positions)
    {
        return Err((StatusCode::BAD_REQUEST, "Order and total notional limits must be positive with total notional at least the per-order limit; daily loss cap must be positive, exploration must be between 0% and 100%, and max open positions must be between 1 and 20, and entries per position must be between 1 and 10".into()));
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if request.daily_notional_limit < settings.used_notional_today {
        return Err((
            StatusCode::CONFLICT,
            "Total notional limit cannot be less than the Auto Trader's current allocation".into(),
        ));
    }
    settings.max_order_notional = request.max_order_notional;
    settings.daily_notional_limit = request.daily_notional_limit;
    settings.daily_realized_loss_limit = request.daily_realized_loss_limit;
    settings.exploration_percent = request.exploration_percent;
    if let Some(value) = request.risk_appetite { settings.risk_appetite = value; }
    settings.max_open_positions = request.max_open_positions;
    if let Some(value) = request.max_entries_per_auto_position {
        settings.max_entries_per_auto_position = value;
    }
    if let Some(value) = request.stop_loss_percent {
        settings.stop_loss_percent = value;
    }
    if let Some(value) = request.take_profit_percent {
        settings.take_profit_percent = value;
    }
    if let Some(value) = request.vwap_duration_minutes {
        settings.vwap_duration_minutes = value;
    }
    if let Some(value) = request.vwap_participation_percent {
        settings.vwap_participation_percent = value;
    }
    if let Some(value) = request.limit_offset_bps {
        settings.limit_offset_bps = value;
    }
    if let Some(value) = request.trigger_offset_bps {
        settings.trigger_offset_bps = value;
    }
    if let Some(value) = request.trailing_distance_percent {
        settings.trailing_distance_percent = value;
    }
    persist_settings(&state.settings_path, &settings)?;
    Ok(Json(status(&state, &settings)))
}
fn status(state: &AutoTraderState, settings: &Settings) -> StatusResponse {
    StatusResponse {
        enabled: state.enabled.load(Ordering::Relaxed),
        max_order_notional: settings.max_order_notional,
        daily_notional_limit: settings.daily_notional_limit,
        daily_realized_loss_limit: settings.daily_realized_loss_limit,
        exploration_percent: settings.exploration_percent,
        risk_appetite: settings.risk_appetite,
        max_open_positions: settings.max_open_positions,
        max_entries_per_auto_position: settings.max_entries_per_auto_position,
        stop_loss_percent: settings.stop_loss_percent,
        take_profit_percent: settings.take_profit_percent,
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_percent: settings.vwap_participation_percent,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
        used_notional_today: settings.used_notional_today,
        last_decision: settings.last_decision.clone(),
        last_decision_at: settings.last_decision_at,
        decisions_since_trade: state
            .gap_summary
            .lock()
            .map(|summary| summary.decisions)
            .unwrap_or_default(),
        gap_reasons: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .reasons
                    .iter()
                    .map(|(reason, count)| GapReason {
                        reason: reason.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .gate_hits
                    .iter()
                    .map(|(gate, count)| GateHit {
                        gate: gate.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        symbol_gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .symbol_gate_hits
                    .iter()
                    .map(|((symbol, direction, gate), count)| SymbolGateHit {
                        symbol: symbol.clone(),
                        direction: direction.clone(),
                        gate: gate.clone(),
                        count: count.count,
                        last_detail: count.last_detail.clone(),
                        distance_to_pass: count.distance_to_pass,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        latest_signal_diagnostic: state
            .gap_summary
            .lock()
            .map(|summary| summary.latest_detail.clone())
            .unwrap_or_default(),
    }
}
async fn get_activity(State(state): State<AutoTraderState>) -> Json<ActivityResponse> {
    let settings = state.settings.lock().await;
    let realized_pnl = settings.realized_pnl;
    let mut grouped_pnl = BTreeMap::<(String, String, String), (f64, usize)>::new();
    for order in &settings.orders {
        if order.realized_pnl.abs() > 1e-12 {
            let direction = if order.side.eq_ignore_ascii_case("sell") {
                "LONG"
            } else {
                "SHORT"
            };
            let key = (
                order.symbol.clone(),
                direction.to_owned(),
                if order.purpose.is_empty() {
                    "other".to_owned()
                } else {
                    order.purpose.clone()
                },
            );
            let total = grouped_pnl.entry(key).or_default();
            total.0 += order.realized_pnl;
            total.1 += 1;
        }
    }
    let mut pnl_breakdown = grouped_pnl
        .into_iter()
        .map(
            |((symbol, side, purpose), (realized_pnl, outcomes))| PnlBreakdown {
                symbol,
                direction: side,
                purpose,
                realized_pnl,
                outcomes,
            },
        )
        .collect::<Vec<_>>();
    pnl_breakdown
        .sort_by(|left, right| right.realized_pnl.abs().total_cmp(&left.realized_pnl.abs()));
    let orders = settings
        .orders
        .iter()
        .rev()
        .take(50)
        .cloned()
        .map(|mut order| {
            if order.target_price.is_none() && order.purpose == "stop-loss" {
                if let Some(position) = settings.positions.get(&order.symbol).filter(|position| position.quantity.abs() > 1e-12) {
                    let quantity = position.quantity.abs();
                    let loss_fraction = settings.stop_loss_percent / 100.0;
                    let fee_rate = configured_taker_fee_rate();
                    order.target_price = Some(if position.quantity > 0.0 {
                        (quantity * position.average_entry_price * (1.0 - loss_fraction) + position.entry_fees)
                            / (quantity * (1.0 - fee_rate))
                    } else {
                        (quantity * position.average_entry_price * (1.0 + loss_fraction) - position.entry_fees)
                            / (quantity * (1.0 + fee_rate))
                    });
                }
            }
            if order.quantity > 0.0
                && order.filled_quantity >= order.quantity
                && matches!(order.status.as_str(), "armed" | "resting")
            {
                order.status = "filled".into();
            }
            order
        })
        .collect();
    let auto_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| (symbol.clone(), position.clone()))
        .collect::<Vec<_>>();
    let open_position_symbols = auto_positions.iter().map(|(symbol, _)| symbol.clone()).collect::<Vec<_>>();
    drop(settings);
    let unrealized_pnl = if auto_positions.is_empty() {
        Some(0.0)
    } else {
        let base = std::env::var("POSITION_MANAGEMENT_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .unwrap_or_default();
        let mut total = 0.0;
        let mut complete = true;
        for (symbol, position) in auto_positions {
            let response = client
                .get(format!("{base}/internal/insights-inputs?symbol={symbol}"))
                .send()
                .await;
            let Ok(response) = response else {
                complete = false;
                break;
            };
            let Ok(inputs) = response.error_for_status() else {
                complete = false;
                break;
            };
            let Ok(inputs) = inputs.json::<serde_json::Value>().await else {
                complete = false;
                break;
            };
            let market = &inputs["markets"][&symbol];
            let mark = if position.quantity > 0.0 {
                market["bids"][0]["price"].as_f64()
            } else {
                market["asks"][0]["price"].as_f64()
            };
            let Some(mark) = mark.filter(|price| price.is_finite() && *price > 0.0) else {
                complete = false;
                break;
            };
            total += (mark - position.average_entry_price) * position.quantity
                - position.entry_fees
                - position.quantity.abs() * mark * configured_taker_fee_rate();
        }
        complete.then_some(total)
    };
    let settings = state.settings.lock().await;
    let closed_returns = &settings.closed_trade_returns_percent;
    let exit_learning_samples = closed_returns.len();
    let closed_trade_average_return_percent = (exit_learning_samples > 0)
        .then(|| closed_returns.iter().sum::<f64>() / exit_learning_samples as f64);
    let closed_trade_wins = closed_returns
        .iter()
        .filter(|value| **value > 1e-12)
        .count();
    let closed_trade_losses = closed_returns
        .iter()
        .filter(|value| **value < -1e-12)
        .count();
    let closed_trade_flat = exit_learning_samples - closed_trade_wins - closed_trade_losses;
    let learning_samples = settings.learning.values().map(|stats| stats.samples).sum();
    let scout_open_positions = settings
        .positions
        .values()
        .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
        .count();
    let scout_learning = settings
        .learning
        .iter()
        .filter(|(key, _)| key.starts_with("scout:"))
        .map(|(_, stats)| stats)
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        });
    let scout_learning_win_rate = (scout_learning.samples > 0)
        .then_some(scout_learning.positive as f64 / scout_learning.samples as f64);
    let learning_positive = settings
        .learning
        .values()
        .map(|stats| stats.positive)
        .sum::<usize>();
    let preferred_order_type = ALL_ORDER_TYPES
        .iter()
        .filter_map(|kind| {
            let suffix = format!(":{kind:?}");
            let stats = settings
                .learning
                .iter()
                .filter(|(key, _)| key.ends_with(&suffix))
                .map(|(_, stats)| stats)
                .fold(LearningStats::default(), |mut total, stats| {
                    total.samples += stats.samples;
                    total.positive += stats.positive;
                    total.realized_pnl += stats.realized_pnl;
                    total
                });
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && stats.positive as f64 / stats.samples as f64 >= 0.55)
                .then_some((
                    format!("{kind:?}"),
                    stats.realized_pnl / stats.samples as f64,
                ))
        })
        .filter(|(_, pnl)| *pnl > 0.0)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind);
    let parameter_profile_samples = std::array::from_fn(|profile| {
        settings
            .learning
            .iter()
            .filter(|(key, _)| key.contains(&format!(":P{profile}:")))
            .map(|(_, stats)| stats.samples)
            .sum()
    });
    let processing_times = state.insight_processing_ms.lock().await.clone();
    let insight_processing_latest_ms = processing_times.last().copied();
    let mut sorted_processing_times = processing_times;
    sorted_processing_times.sort_by(f64::total_cmp);
    let insight_processing_samples = sorted_processing_times.len();
    let insight_processing_average_ms = (insight_processing_samples > 0)
        .then(|| sorted_processing_times.iter().sum::<f64>() / insight_processing_samples as f64);
    let insight_processing_p95_ms = (insight_processing_samples > 0).then(|| {
        sorted_processing_times[(95 * insight_processing_samples)
            .div_ceil(100)
            .saturating_sub(1)]
    });
    let lock_wait_samples = state.cycle_lock_wait_ms.lock().await.clone();
    let cycle_lock_wait_ms = lock_wait_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    let lock_hold_samples = state
        .cycle_lock_hold_ms
        .lock()
        .ok()
        .map(|samples| samples.clone())
        .unwrap_or_default();
    let cycle_lock_hold_ms = lock_hold_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    Json(ActivityResponse {
        orders,
        open_position_symbols,
        pnl_breakdown,
        realized_pnl,
        unrealized_pnl,
        exit_learning_samples,
        closed_trade_average_return_percent,
        closed_trade_wins,
        closed_trade_losses,
        closed_trade_flat,
        learning_samples,
        learning_win_rate: (learning_samples > 0)
            .then_some(learning_positive as f64 / learning_samples as f64),
        preferred_order_type,
        parameter_profile_samples,
        scout_open_positions,
        scout_max_active_positions: SCOUT_MAX_ACTIVE_POSITIONS,
        scout_learning_samples: scout_learning.samples,
        scout_learning_win_rate,
        scout_learning_realized_pnl: scout_learning.realized_pnl,
        insight_processing_latest_ms,
        insight_processing_average_ms,
        insight_processing_p95_ms,
        insight_processing_samples,
        cycle_lock_wait_ms,
        cycle_lock_hold_ms,
    })
}

async fn acquire_cycle_lock(state: &AutoTraderState, worker: &str) -> CycleLockGuard {
    let wait_started = Instant::now();
    let guard = state.cycle_lock.clone().lock_owned().await;
    record_cycle_lock_wait(state, worker, wait_started.elapsed()).await;
    CycleLockGuard {
        _guard: guard,
        hold_samples: state.cycle_lock_hold_ms.clone(),
        worker: worker.to_owned(),
        acquired_at: Instant::now(),
    }
}

async fn record_cycle_lock_wait(state: &AutoTraderState, worker: &str, elapsed: Duration) {
    let mut samples = state.cycle_lock_wait_ms.lock().await;
    let values = samples.entry(worker.to_owned()).or_default();
    values.push(elapsed.as_secs_f64() * 1000.0);
    if values.len() > 500 {
        values.remove(0);
    }
}

fn summarize_lock_wait(values: &[f64]) -> Option<CycleLockWaitSummary> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(CycleLockWaitSummary {
        latest_ms: *values.last()?,
        average_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p95_ms: sorted[(95 * sorted.len()).div_ceil(100).saturating_sub(1)],
        samples: sorted.len(),
    })
}

async fn run_cycle(
    state: &AutoTraderState,
    dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let processing_started = Instant::now();
    let result = run_cycle_inner(state, dashboard).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut message = match &result {
        Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted" => {
            format!(
                "{}: {} {} {}",
                cycle.status,
                cycle.side.as_deref().unwrap_or(""),
                cycle.symbol.as_deref().unwrap_or(""),
                cycle.order_type.as_deref().unwrap_or("")
            )
        }
        Ok(cycle) => cycle
            .reason
            .clone()
            .unwrap_or_else(|| cycle.status.to_owned()),
        Err((_, error)) => format!("Cycle error: {error}"),
    };
    let traded = matches!(&result, Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted");
    if let Ok(mut summary) = state.gap_summary.lock() {
        if traded {
            *summary = GapSummary::default();
            if let Ok(cycle) = &result {
                if cycle.status == "order_submitted"
                    && cycle.order_type.as_deref() != Some("Ioc exit")
                {
                    if let (Some(symbol), Some(side)) = (&cycle.symbol, &cycle.side) {
                        let direction = match side.as_str() {
                            "buy" => "BULLISH",
                            "sell" => "BEARISH",
                            other => other,
                        };
                        summary.symbol_gate_hits.insert(
                            (symbol.clone(), direction.to_owned(), "Readiness".to_owned()),
                            SymbolGateCount {
                                count: 0,
                                last_detail:
                                    "All entry, risk, and execution gates passed; order submitted."
                                        .to_owned(),
                                distance_to_pass: Some(0.0),
                            },
                        );
                    }
                }
            }
        } else {
            summary
                .symbol_gate_hits
                .retain(|(_, _, gate), _| gate != "Readiness");
            summary.decisions = summary.decisions.saturating_add(1);
            let raw_reason = match &result {
                Ok(cycle) => cycle.reason.as_deref().unwrap_or(cycle.status),
                Err((_, error)) => error.as_str(),
            };
            let reason = gap_reason_category(raw_reason);
            *summary.reasons.entry(reason).or_default() += 1;
            for gate in gate_hits_for_reason(raw_reason) {
                *summary.gate_hits.entry(gate).or_default() += 1;
            }
            for (symbol, direction, gate, detail, distance_to_pass) in
                symbol_gate_hits_for_reason(raw_reason)
            {
                let entry = summary
                    .symbol_gate_hits
                    .entry((symbol, direction, gate))
                    .or_default();
                entry.count = entry.count.saturating_add(1);
                entry.last_detail = detail;
                entry.distance_to_pass = distance_to_pass;
            }
            if let Ok(cycle) = &result {
                if let (Some(symbol), Some(direction)) = (&cycle.symbol, &cycle.side) {
                    for gate in gate_hits_for_reason(raw_reason) {
                        let distance_to_pass = gate_distance_to_pass(&gate, raw_reason);
                        let entry = summary
                            .symbol_gate_hits
                            .entry((symbol.clone(), direction.clone(), gate))
                            .or_default();
                        entry.count = entry.count.saturating_add(1);
                        entry.last_detail = raw_reason.to_owned();
                        entry.distance_to_pass = distance_to_pass;
                    }
                }
            }
            summary.latest_detail = raw_reason.to_owned();
        }
    }
    let mut settings = state.settings.lock().await;
    if !settings.last_risk_note.is_empty() {
        message.push_str(" · ");
        message.push_str(&settings.last_risk_note);
    }
    settings.last_decision = message;
    settings.last_decision_at = now;
    // The last decision is informational. Keep it in memory immediately and
    // let the next durable settings write include it, rather than writing the
    // full state file on every signal cycle.
    drop(settings);
    let mut processing_times = state.insight_processing_ms.lock().await;
    processing_times.push(processing_started.elapsed().as_secs_f64() * 1000.0);
    if processing_times.len() > 500 {
        processing_times.remove(0);
    }
    result
}

async fn run_position_maintenance(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    reconcile_maker_fills(&client, &position_url, state).await?;
    let _ = maybe_exit_auto_position(&client, &position_url, state).await?;
    Ok(())
}

#[derive(Clone, Deserialize)]
struct MakerFillRecord {
    id: String,
    symbol: String,
    side: String,
    quantity: f64,
    execution_price: f64,
    fee: f64,
    fee_liquidity: String,
    resting_order_id: Option<String>,
}

async fn reconcile_maker_fills(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let has_pending = state.settings.lock().await.orders.iter().any(|order| {
        order.execution_order_id.is_some()
            && order.order_type.starts_with("PostOnlyLimit")
            && order.status != "filled"
            && order.status != "cancelled"
    });
    if !has_pending {
        return Ok(());
    }
    let fills = client
        .get(format!("{base}/positions/trades"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch maker fills: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Maker fill lookup failed: {error}"),
            )
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid maker fill response: {error}"),
            )
        })?;
    let records = serde_json::from_value::<Vec<MakerFillRecord>>(fills["trades"].clone()).map_err(
        |error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid maker trade records: {error}"),
            )
        },
    )?;
    let open = client
        .get(format!("{base}/positions/orders"))
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Unable to fetch resting orders: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Resting order lookup failed: {error}"),
            )
        })?
        .json::<serde_json::Value>()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Invalid resting order response: {error}"),
            )
        })?;
    let active_ids = open["orders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| item["id"].as_str().map(str::to_owned))
        .collect::<std::collections::HashSet<_>>();
    let mut settings = state.settings.lock().await;
    let mut changed = false;
    let mut learning_updates = Vec::new();
    let Settings {
        orders, positions, ..
    } = &mut *settings;
    for order in orders.iter_mut().filter(|order| {
        order.order_type.starts_with("PostOnlyLimit")
            && matches!(
                order.status.as_str(),
                "resting" | "partially_filled" | "accepted"
            )
    }) {
        let Some(id) = order.execution_order_id.as_deref() else {
            continue;
        };
        let pending_fills = records
            .iter()
            .filter(|fill| {
                fill.resting_order_id.as_deref() == Some(id)
                    && fill.symbol == order.symbol
                    && fill.side == order.side
                    && fill.fee_liquidity == "maker"
                    && !order
                        .processed_fill_trade_ids
                        .iter()
                        .any(|processed| processed == &fill.id)
            })
            .cloned()
            .collect::<Vec<_>>();
        for fill in pending_fills {
            if !fill.quantity.is_finite()
                || fill.quantity <= 0.0
                || !fill.execution_price.is_finite()
                || fill.execution_price <= 0.0
            {
                continue;
            }
            let (realized, learned) = apply_auto_fill(
                positions,
                &order.symbol,
                &order.side,
                fill.quantity,
                fill.execution_price,
                fill.fee.abs(),
                "PostOnlyLimit",
                order.learning_context.as_deref().unwrap_or(&order.symbol),
            );
            order.filled_quantity += fill.quantity;
            order.fill_rate = (order.filled_quantity / order.quantity).clamp(0.0, 1.0);
            order.fees_paid += fill.fee.abs();
            let total_notional = order.average_price.unwrap_or(0.0)
                * (order.filled_quantity - fill.quantity)
                + fill.execution_price * fill.quantity;
            order.average_price = Some(total_notional / order.filled_quantity);
            order.realized_pnl += realized;
            order.processed_fill_trade_ids.push(fill.id.clone());
            if let Some(kind) = learned {
                learning_updates.push((kind, realized));
            }
            changed = true;
        }
        let prior_status = order.status.clone();
        let mut unfilled_post_only_removal = false;
        if order.filled_quantity + 1e-12 >= order.quantity {
            order.status = "filled".into();
        } else if active_ids.contains(id) {
            order.status = if order.filled_quantity > 0.0 {
                "partially_filled"
            } else {
                "resting"
            }
            .into();
        } else if !active_ids.contains(id) {
            order.status = if order.filled_quantity > 0.0 {
                "partially_filled_closed"
            } else {
                "expired_or_cancelled"
            }
            .into();
            order.failure_reason = Some(
                if order.expires_at.is_some_and(|expires| {
                    expires
                        <= SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                }) {
                    "expired_without_full_fill".to_owned()
                } else {
                    "removed_before_expiry".to_owned()
                },
            );
            unfilled_post_only_removal = order.filled_quantity <= 1e-12
                && prior_status != "expired_or_cancelled";
        }
        if unfilled_post_only_removal {
            if let Some(context) = order.learning_context.as_deref() {
                learning_updates.push((format!("{context}:PostOnlyLimit"), 0.0));
            }
        }
        changed |= order.status != prior_status;
    }
    for (kind, realized) in learning_updates {
        let stats = settings.learning.entry(kind).or_default();
        stats.samples += 1;
        if realized > 0.0 {
            stats.positive += 1;
        }
        stats.realized_pnl += realized;
        settings.realized_pnl += realized;
        settings.daily_realized_pnl += realized;
    }
    if changed {
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(())
}

async fn run_hedge_evaluation(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    let _ = maybe_place_portfolio_hedge(&client, &position_url, state).await?;
    Ok(())
}

async fn fresh_subscribed_market_book(
    client: &reqwest::Client,
    position_url: &str,
    symbol: &str,
) -> Option<serde_json::Value> {
    let subscribe_url = format!("{position_url}/internal/market-book/{symbol}/subscribe");
    let _ = client
        .post(subscribe_url)
        .timeout(Duration::from_secs(1))
        .send()
        .await;
    let market_url = format!("{position_url}/internal/market-book?symbol={symbol}");
    for _ in 0..4 {
        if let Ok(response) = client
            .get(&market_url)
            .timeout(Duration::from_millis(500))
            .send()
            .await
            && let Ok(response) = response.error_for_status()
            && let Ok(market) = response.json::<serde_json::Value>().await
        {
            let updated_at = market
                .get("updated_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_millis() as u64;
            if updated_at > 0 && now_ms.saturating_sub(updated_at) <= 5_000 {
                return Some(market);
            }
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    None
}

async fn run_cycle_inner(
    state: &AutoTraderState,
    dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let cycle_started = Instant::now();
    let _cycle = acquire_cycle_lock(state, "signal").await;
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note.clear();
        settings.risk_limited_sizing = false;
    }
    *state.latest_missed_opportunity_bands.lock().await =
        dashboard.missed_opportunity_bands.clone();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "System clock is before Unix epoch".into(),
            )
        })?
        .as_secs();
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = state.client.clone();
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
    }
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        if settings.daily_realized_pnl <= -settings.daily_realized_loss_limit {
            if !settings.daily_loss_limit_reduced {
                settings.daily_realized_loss_limit = (settings.daily_realized_loss_limit * 0.9)
                    .max(MIN_DAILY_REALIZED_LOSS_LIMIT.min(settings.daily_realized_loss_limit));
                settings.daily_loss_limit_reduced = true;
                let _ = persist_settings(&state.settings_path, &settings);
            }
            return Ok(skipped(&format!(
                "Daily realized-loss limit reached at USD {:.2}; learned daily cap is now USD {:.2}",
                settings.daily_realized_pnl, settings.daily_realized_loss_limit
            )));
        }
    }
    let dashboard_ms = 0.0;
    let selection_started = Instant::now();
    let (
        learning,
        active_scout_count,
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
    ) = {
        let settings = state.settings.lock().await;
        (
            settings.learning.clone(),
            settings
                .positions
                .values()
                .filter(|position| position.quantity.abs() > 1e-12 && position.scout_entry)
                .count(),
            settings.stop_loss_percent,
            settings.take_profit_percent,
        )
    };
    let mut signals = dashboard.best;
    for signal in &mut signals {
        let multiplier = longer_horizon_score_multiplier(
            signal.score,
            &dashboard.missed_opportunity_bands,
        );
        if multiplier > 1.0 {
            signal.score *= multiplier;
        }
    }
    let mut inferred_direction_count = 0;
    for signal in &mut signals {
        if signal.direction == "neutral"
            && signal.ranging_probability.unwrap_or(0.0) < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            && signal.score.is_finite()
            && signal.score.abs() >= NEUTRAL_SCORE_DIRECTION_FLOOR
        {
            signal.direction = if signal.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            inferred_direction_count += 1;
        }
    }
    // Record borderline neutral scores as directional shadow probes only. These
    // are evaluated on every scan and never feed the order-selection path.
    for candidate in signals.iter().filter(|candidate| {
        !candidate.stale
            && candidate.direction == "neutral"
            && now.saturating_sub(candidate.timestamp / 1000) <= MAX_SIGNAL_AGE_SECONDS
            && candidate.score.is_finite()
            && candidate.score.abs() >= SHADOW_MIN_ABS_SCORE
            && candidate.score.abs() < NEUTRAL_SCORE_DIRECTION_FLOOR
            && candidate.ranging_probability.unwrap_or(0.0)
                < NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
    }) {
        let mut shadow = candidate.clone();
        shadow.direction = if shadow.score > 0.0 {
            "bullish"
        } else {
            "bearish"
        }
        .into();
        let evidence = dashboard.walk_forward.iter().find(|item| {
            item.symbol == shadow.symbol && (60..=300).contains(&item.horizon_seconds)
        });
        let payload = signal_audit_payload(
            &shadow,
            "decision",
            now.saturating_mul(1000),
            "skipped",
            Some(
                "No tradeable bias: shadow-only neutral score probe below live direction floor; no order submitted",
            ),
            Some(true),
            counterfactual_stop_loss_percent,
            counterfactual_take_profit_percent,
            evidence,
        );
        queue_auto_trader_audit(state, payload);
    }
    let (open_positions, max_entries_per_auto_position) = {
        let settings = state.settings.lock().await;
        (
            settings
                .positions
                .iter()
                .filter(|(_, position)| position.quantity.abs() > 1e-12)
                .map(|(symbol, position)| (symbol.clone(), position.clone()))
                .collect::<BTreeMap<_, _>>(),
            settings.max_entries_per_auto_position,
        )
    };
    let mut stale_signal_count = 0;
    let mut stalest_signal_age_seconds = 0u64;
    let mut stale_symbols = Vec::new();
    let mut non_directional_count = 0;
    let mut position_blocked_count = 0;
    let fresh_directional = signals
        .iter()
        .filter(|signal| {
            if signal.stale || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
            {
                stale_signal_count += 1;
                let age = now.saturating_sub(signal.timestamp / 1000);
                stalest_signal_age_seconds = stalest_signal_age_seconds.max(age);
                if !stale_symbols.iter().any(|symbol| symbol == &signal.symbol) {
                    stale_symbols.push(signal.symbol.clone());
                }
                return false;
            }
            if !matches!(signal.direction.as_str(), "bullish" | "bearish") {
                non_directional_count += 1;
                return false;
            }
            let allowed_by_position = open_positions.get(&signal.symbol).is_none_or(|position| {
                let signal_quantity = if signal.direction == "bullish" {
                    1.0
                } else {
                    -1.0
                };
                position.quantity.signum() == signal_quantity
                    && position.entries < max_entries_per_auto_position
            });
            if !allowed_by_position {
                position_blocked_count += 1;
            }
            allowed_by_position
        })
        .collect::<Vec<_>>();
    let neutral_scout_candidates = signals
        .iter()
        .filter_map(|signal| {
            if signal.direction != "neutral"
                || signal.stale
                || now.saturating_sub(signal.timestamp / 1000) > MAX_SIGNAL_AGE_SECONDS
                || !signal.score.is_finite()
                || signal.score.abs() < SCOUT_NEUTRAL_SCORE_FLOOR
                || signal.ranging_probability.unwrap_or(0.0)
                    >= NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY
            {
                return None;
            }
            let mut candidate = signal.clone();
            candidate.direction = if candidate.score > 0.0 {
                "bullish".into()
            } else {
                "bearish".into()
            };
            let allowed_by_position =
                open_positions
                    .get(&candidate.symbol)
                    .is_none_or(|position| {
                        let direction = if candidate.direction == "bullish" {
                            1.0
                        } else {
                            -1.0
                        };
                        position.quantity.signum() == direction
                            && position.entries < max_entries_per_auto_position
                    });
            allowed_by_position.then_some(candidate)
        })
        .collect::<Vec<_>>();
    let strength_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            let context = market_context(signal);
            let learned_floor = adaptive_strength_floor(&learning, &context);
            let floor = adaptive_missed_opportunity_floor(
                learned_floor,
                "strength",
                &dashboard.missed_opportunity_bands,
            );
            signal.strength.is_finite() && signal.strength >= floor
        })
        .copied()
        .collect::<Vec<_>>();
    let exploration_qualified = fresh_directional
        .iter()
        .filter(|signal| {
            let context = market_context(signal);
            !context.ends_with(":mixed")
                && signal.strength.is_finite()
                && signal.strength >= MIN_EXPLORATION_STRENGTH
                && signal.score.is_finite()
                && signal.score.abs()
                    >= adaptive_exploration_score_floor(
                        &learning,
                        &context,
                        &dashboard.missed_opportunity_bands,
                    )
        })
        .copied()
        .collect::<Vec<_>>();
    let strict_candidate = strength_qualified
        .iter()
        .copied()
        .filter(|signal| {
            let walk_forward = dashboard.walk_forward.iter().find(|item| {
                item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
            });
            let side_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.all_signal_long
                } else {
                    &item.all_signal_short
                }
            });
            let research_evidence = walk_forward.map(|item| {
                if signal.direction == "bullish" {
                    &item.research_logistic_long
                } else {
                    &item.research_logistic_short
                }
            });
            side_evidence.is_some_and(|evidence| {
                let research_supports = research_evidence.is_none_or(|research| {
                    research.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || research
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                evidence.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                    && evidence
                        .direction_match_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                    && evidence
                        .average_net_return_percent
                        .is_some_and(|net| net > 0.0)
                    && evidence
                        .positive_net_rate
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_RATE)
                    && evidence
                        .positive_net_rate_lower_95
                        .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
                    && research_supports
            })
        })
        .max_by(|a, b| a.strength.total_cmp(&b.strength));
    let (signal, exploratory, scout) = if let Some(signal) = strict_candidate {
        (signal, false, false)
    } else {
        let exploratory = exploration_qualified
            .iter()
            .copied()
            .filter(|signal| {
                let walk_forward = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let side_evidence = walk_forward.map(|item| {
                    if signal.direction == "bullish" {
                        &item.all_signal_long
                    } else {
                        &item.all_signal_short
                    }
                });
                let research_evidence = walk_forward.map(|item| {
                    if signal.direction == "bullish" {
                        &item.research_logistic_long
                    } else {
                        &item.research_logistic_short
                    }
                });
                let research_supports = research_evidence.is_none_or(|evidence| {
                    evidence.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES
                        || evidence
                            .average_net_return_percent
                            .is_some_and(|net| net > 0.0)
                });
                research_supports
                    && side_evidence.is_none_or(|evidence| {
                        evidence.sample_count < MIN_EXPLORATION_EVIDENCE_SAMPLES
                            || (evidence
                                .average_net_return_percent
                                .is_some_and(|net| net > 0.0)
                                && evidence
                                    .positive_net_rate
                                    .is_some_and(|rate| rate >= NEGATIVE_EVIDENCE_FLOOR))
                    })
            })
            .max_by(|a, b| a.strength.total_cmp(&b.strength));
        let scout_candidate = if exploratory.is_none() {
            fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .filter(|signal| {
                    let context = market_context(signal);
                    let (strength_floor, score_floor) =
                        adaptive_scout_thresholds(&learning, &context, active_scout_count);
                    if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS
                        || !signal.strength.is_finite()
                        || signal.strength < strength_floor
                        || !signal.score.is_finite()
                        || signal.score.abs() < score_floor
                    {
                        return false;
                    }
                    let evidence = dashboard.walk_forward.iter().find(|item| {
                        item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds)
                    });
                    let side = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.all_signal_long
                        } else {
                            &item.all_signal_short
                        }
                    });
                    let research = evidence.map(|item| {
                        if signal.direction == "bullish" {
                            &item.research_logistic_long
                        } else {
                            &item.research_logistic_short
                        }
                    });
                    let measured_negative = side.is_some_and(|m| {
                        m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                            && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                                || m.positive_net_rate
                                    .is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR))
                    }) || research.is_some_and(|m| {
                        m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                            && m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                    });
                    !measured_negative || active_scout_count == 0
                })
                .max_by(|a, b| a.strength.total_cmp(&b.strength))
        } else {
            None
        };
        let Some(signal) = exploratory.or(scout_candidate) else {
            let blocked_by_negative = exploration_qualified
                .iter()
                .filter(|signal| {
                    dashboard
                        .walk_forward
                        .iter()
                        .find(|item| {
                            item.symbol == signal.symbol
                                && (60..=300).contains(&item.horizon_seconds)
                        })
                        .map(|item| {
                            if signal.direction == "bullish" {
                                &item.all_signal_long
                            } else {
                                &item.all_signal_short
                            }
                        })
                        .is_some_and(|evidence| {
                            evidence.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                                && (evidence
                                    .average_net_return_percent
                                    .is_none_or(|net| net <= 0.0)
                                    || evidence
                                        .positive_net_rate
                                        .is_none_or(|rate| rate < NEGATIVE_EVIDENCE_FLOOR))
                        })
                })
                .count();
            let mut reason = format!(
                "No tradeable bias: fresh directional {}/{} ({} inferred from neutral score; {} neutral scout probes; filtered: {} stale, {} non-directional, {} blocked by existing positions); contextual learned strength floors passed {}; exploration strength floor {:.2}; contextual raw-score floors passed {}; {} candidate(s) blocked by negative walk-forward history",
                fresh_directional.len(),
                signals.len(),
                neutral_scout_candidates.len(),
                inferred_direction_count,
                stale_signal_count,
                non_directional_count,
                position_blocked_count,
                strength_qualified.len(),
                MIN_EXPLORATION_STRENGTH,
                exploration_qualified.len(),
                blocked_by_negative
            );
            if stale_signal_count > 0 {
                reason.push_str(&format!(
                    "; oldest stale signal {}s; symbols {}",
                    stalest_signal_age_seconds,
                    stale_symbols.join(", ")
                ));
            }
            let early_filter_details = signals.iter().filter_map(|candidate| {
                let age = now.saturating_sub(candidate.timestamp / 1000);
                if candidate.stale {
                    Some(format!("{} {} · Signal freshness (stale flag true; age {}s; max age {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if age > MAX_SIGNAL_AGE_SECONDS {
                    Some(format!("{} {} · Signal freshness (age {}s > max {}s)", candidate.symbol, candidate.direction, age, MAX_SIGNAL_AGE_SECONDS))
                } else if !matches!(candidate.direction.as_str(), "bullish" | "bearish") {
                    let ranging = candidate.ranging_probability.map(|value| format!("{:.1}%", value * 100.0)).unwrap_or_else(|| "n/a (defaults to 0%)".into());
                    Some(format!("{} {} · Directional signal (|score| {:.3} vs floor {:.2}; range probability {} must be below {:.0}% to infer direction)", candidate.symbol, candidate.direction, candidate.score.abs(), NEUTRAL_SCORE_DIRECTION_FLOOR, ranging, NEUTRAL_INFERENCE_MAX_RANGE_PROBABILITY * 100.0))
                } else {
                    open_positions.get(&candidate.symbol).and_then(|position| {
                        let signal_quantity = if candidate.direction == "bullish" { 1.0 } else { -1.0 };
                        let matching_side = position.quantity.signum() == signal_quantity;
                        if !matching_side {
                            Some(format!("{} {} · Existing position / re-entry limit (opposite position qty {:.8}; signal side {})", candidate.symbol, candidate.direction, position.quantity, candidate.direction))
                        } else if position.entries >= max_entries_per_auto_position {
                            Some(format!("{} {} · Existing position / re-entry limit (entries {} >= max {})", candidate.symbol, candidate.direction, position.entries, max_entries_per_auto_position))
                        } else { None }
                    })
                }
            }).collect::<Vec<_>>();
            if !early_filter_details.is_empty() {
                reason.push_str("; early filters: ");
                reason.push_str(&early_filter_details.join(" | "));
            }
            let candidate_details = fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
                .map(|candidate| {
                let context = market_context(candidate);
                let learned_floor = adaptive_strength_floor(&learning, &context);
                let strength_floor = adaptive_missed_opportunity_floor(
                    learned_floor, "strength", &dashboard.missed_opportunity_bands,
                );
                let score_floor = adaptive_exploration_score_floor(
                    &learning, &context, &dashboard.missed_opportunity_bands,
                );
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let side = evidence.map(|item| if candidate.direction == "bullish" {
                    &item.all_signal_long
                } else {
                    &item.all_signal_short
                });
                let pct = |value: Option<f64>| value.map(|v| format!("{:.1}%", v * 100.0)).unwrap_or_else(|| "n/a".into());
                let metrics = side.map(|m| format!(
                    "walk-forward {}s n={} hit={} net={} positive={} lower95={}",
                    evidence.map(|item| item.horizon_seconds).unwrap_or_default(), m.sample_count,
                    pct(m.direction_match_rate),
                    m.average_net_return_percent.map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()),
                    pct(m.positive_net_rate), pct(m.positive_net_rate_lower_95)
                )).unwrap_or_else(|| "walk-forward evidence unavailable".into());
                let research_metrics = evidence.map(|item| {
                    let research = if candidate.direction == "bullish" { &item.research_logistic_long } else { &item.research_logistic_short };
                    format!("research-model n={} net={}", research.sample_count,
                        research.average_net_return_percent.map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()))
                }).unwrap_or_else(|| "research-model evidence unavailable".into());
                let weak = candidate.strength < strength_floor;
                let mixed_context = context.ends_with(":mixed");
                let score_weak = candidate.score.abs() < score_floor;
                let evidence_missing = !weak && side.is_none_or(|m| m.sample_count < MIN_SIDE_WALK_FORWARD_SAMPLES);
                let exploration_path_qualified = !mixed_context
                    && candidate.strength.is_finite()
                    && candidate.strength >= MIN_EXPLORATION_STRENGTH
                    && candidate.score.is_finite()
                    && candidate.score.abs() >= score_floor;
                let evidence_bad = exploration_path_qualified && side.is_some_and(|m| m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                    && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                        || m.positive_net_rate.is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR)));
                let research = evidence.map(|item| if candidate.direction == "bullish" {
                    &item.research_logistic_long
                } else {
                    &item.research_logistic_short
                });
                let research_bad = (!weak || exploration_path_qualified) && research.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                    && m.average_net_return_percent.is_none_or(|v| v <= 0.0));
                let strict_evidence_bad = !weak && side.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES && (
                    m.direction_match_rate.is_none_or(|v| v < MIN_SIDE_DIRECTION_MATCH_RATE)
                    || m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                    || m.positive_net_rate.is_none_or(|v| v < MIN_SIDE_POSITIVE_NET_RATE)
                    || m.positive_net_rate_lower_95.is_none_or(|v| v < MIN_SIDE_POSITIVE_NET_LOWER_95)
                ));
                let (scout_strength_floor, scout_score_floor) =
                    adaptive_scout_thresholds(&learning, &context, active_scout_count);
                let scout_strength_pass = candidate.strength.is_finite() && candidate.strength >= scout_strength_floor;
                let scout_score_pass = candidate.score.is_finite() && candidate.score.abs() >= scout_score_floor;
                let scout_negative = side.is_some_and(|m| m.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                    && (m.average_net_return_percent.is_none_or(|v| v <= 0.0)
                        || m.positive_net_rate.is_none_or(|v| v < NEGATIVE_EVIDENCE_FLOOR)))
                    || research.is_some_and(|m| m.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                        && m.average_net_return_percent.is_none_or(|v| v <= 0.0));
                let mut failed = Vec::new();
                if weak { failed.push(format!("strength {:.2} < {:.2}", candidate.strength, strength_floor)); }
                if mixed_context { failed.push("normal exploration blocked for mixed context".into()); }
                else if score_weak { failed.push(format!("score {:.2} < {:.2}", candidate.score.abs(), score_floor)); }
                if evidence_bad { failed.push(format!("exploration needs net return > 0 and positive rate ≥ {:.0}%", NEGATIVE_EVIDENCE_FLOOR * 100.0)); }
                if evidence_missing { failed.push(format!("normal entry needs {} side samples", MIN_SIDE_WALK_FORWARD_SAMPLES)); }
                if strict_evidence_bad { failed.push(format!("normal entry needs hit ≥ {:.0}%, net > 0, positive rate ≥ {:.0}%, lower-95 ≥ {:.0}%", MIN_SIDE_DIRECTION_MATCH_RATE * 100.0, MIN_SIDE_POSITIVE_NET_RATE * 100.0, MIN_SIDE_POSITIVE_NET_LOWER_95 * 100.0)); }
                if research_bad { failed.push("research-model side return is not positive".into()); }
                if active_scout_count >= SCOUT_MAX_ACTIVE_POSITIONS { failed.push(format!("scout inventory cap reached ({active_scout_count}/{SCOUT_MAX_ACTIVE_POSITIONS} open scout positions)")); }
                else if !scout_strength_pass { failed.push(format!("scout strength {:.2} < learned {:.2}", candidate.strength, scout_strength_floor)); }
                else if !scout_score_pass { failed.push(format!("scout |score| {:.2} < learned {:.2}", candidate.score.abs(), scout_score_floor)); }
                else if scout_negative && active_scout_count > 0 { failed.push(format!("measured negative-history scout probe waits for zero open scouts ({}; net {}; positive {})", side.map(|m| m.sample_count).unwrap_or_default(), side.and_then(|m| m.average_net_return_percent).map(|v| format!("{v:+.3}%")).unwrap_or_else(|| "n/a".into()), side.and_then(|m| m.positive_net_rate).map(|v| format!("{:.1}%", v * 100.0)).unwrap_or_else(|| "n/a".into()))); }
                format!("{} {} · score {:.2} · strength {:.2} · context {} · {} · {} · {}",
                    candidate.symbol, candidate.direction, candidate.score, candidate.strength, context,
                    metrics, research_metrics, if failed.is_empty() { "did not pass combined entry checks".into() } else { failed.join(", ") })
            }).collect::<Vec<_>>();
            if !candidate_details.is_empty() {
                reason.push_str("; candidate checks: ");
                reason.push_str(&candidate_details.join(" | "));
            }
            for candidate in fresh_directional
                .iter()
                .copied()
                .chain(neutral_scout_candidates.iter())
            {
                let evidence = dashboard.walk_forward.iter().find(|item| {
                    item.symbol == candidate.symbol && (60..=300).contains(&item.horizon_seconds)
                });
                let payload = signal_audit_payload(
                    candidate,
                    "decision",
                    now.saturating_mul(1000),
                    "skipped",
                    Some(&reason),
                    Some(true),
                    counterfactual_stop_loss_percent,
                    counterfactual_take_profit_percent,
                    evidence,
                );
                queue_auto_trader_audit(state, payload);
            }
            return Ok(skipped(&reason));
        };
        (signal, true, exploratory.is_none())
    };
    let signal_selection_ms = selection_started.elapsed().as_secs_f64() * 1000.0;
    let evidence = dashboard
        .walk_forward
        .iter()
        .find(|item| item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds));
    let selected_payload = signal_audit_payload(
        signal,
        "decision",
        now.saturating_mul(1000),
        "selected",
        Some(if scout {
            "scout candidate selected; capped paper exploration"
        } else if exploratory {
            "exploratory candidate selected"
        } else {
            "strict walk-forward candidate selected"
        }),
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    let mut selected_payload = selected_payload;
    selected_payload["scout"] = serde_json::json!(scout);
    queue_auto_trader_audit(state, selected_payload);
    let decision_id = signal_decision_id(signal);
    if !signal
        .symbol
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Ok(skipped_for_signal(
            signal,
            "Selected insight has an invalid instrument symbol",
        ));
    }
    let market_inputs_started = Instant::now();
    let cached_market = dashboard.markets.get(&signal.symbol).cloned();
    let cached_market_is_fresh = cached_market.as_ref().is_some_and(|market| {
        let updated_at = market
            .get("updated_at_ms")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        updated_at > 0 && now.saturating_mul(1000).saturating_sub(updated_at) <= 5_000
    });
    let market = if cached_market_is_fresh {
        cached_market
    } else {
        fresh_subscribed_market_book(&client, &position_url, &signal.symbol).await
    };
    let Some(market) = market else {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Waiting for a fresh subscribed local order book for {}",
                signal.symbol
            ),
        ));
    };
    let market_inputs_ms = market_inputs_started.elapsed().as_secs_f64() * 1000.0;
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid
        .zip(ask)
        .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
    else {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "No current order book for {}; subscribed and waiting for a fresh book",
                signal.symbol
            ),
        ));
    };
    let mid = (bid + ask) / 2.0;
    // Warm the paper manager's book subscription while the remaining order
    // checks run, so the submit path doesn't pay the feed's first-book delay.
    let warm_client = client.clone();
    let warm_url = format!(
        "{position_url}/internal/market-book/{}/subscribe",
        signal.symbol
    );
    tokio::spawn(async move {
        let _ = warm_client.post(warm_url).send().await;
    });
    let signal_key = signal.symbol.clone();
    let now_ms = now.saturating_mul(1000);
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if let Some(stopped_at) = settings
        .orders
        .iter()
        .filter(|order| {
            order.symbol == signal_key
                && order.purpose == "stop-loss"
                && order.status == "filled"
                && order.realized_pnl < 0.0
        })
        .filter_map(|order| order.triggered_at)
        .max()
        .filter(|stopped_at| now_ms.saturating_sub(*stopped_at) < POST_STOP_REENTRY_COOLDOWN_MS)
    {
        let seconds_remaining =
            POST_STOP_REENTRY_COOLDOWN_MS.saturating_sub(now_ms.saturating_sub(stopped_at)) / 1000;
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Post-stop cooldown: {signal_key} paused for {seconds_remaining}s after a losing protective stop"
            ),
        ));
    }
    let open_position_count = settings
        .positions
        .values()
        .filter(|position| position.quantity.abs() > 1e-12)
        .count();
    let already_open = settings
        .positions
        .get(&signal_key)
        .is_some_and(|position| position.quantity.abs() > 1e-12);
    if !already_open && open_position_count >= settings.max_open_positions {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Open-position cap reached ({}/{})",
                open_position_count, settings.max_open_positions
            ),
        ));
    }
    if let Some(position) = settings
        .positions
        .get(&signal_key)
        .filter(|position| position.quantity.abs() > 1e-12)
    {
        let signal_quantity = if signal.direction == "bullish" {
            1.0
        } else {
            -1.0
        };
        if position.quantity.signum() != signal_quantity {
            return Ok(skipped_for_signal(
                signal,
                &format!("Opposite signal for {signal_key}; waiting for its stop or target exit"),
            ));
        }
        if position.entries >= settings.max_entries_per_auto_position {
            return Ok(skipped_for_signal(
                signal,
                &format!(
                    "Add-on limit reached for {signal_key} ({} entries per position)",
                    settings.max_entries_per_auto_position
                ),
            ));
        }
    } else if settings
        .last_signal_direction
        .get(&signal_key)
        .is_some_and(|direction| direction == &signal.direction)
        && settings
            .last_signal_strength
            .get(&signal_key)
            .is_some_and(|strength| signal.strength < strength + MATERIAL_STRENGTH_CHANGE)
    {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Re-entry blocked: {signal_key} signal strength has not improved by {:.2}",
                MATERIAL_STRENGTH_CHANGE
            ),
        ));
    }
    if settings
        .last_signal_at
        .get(&signal_key)
        .is_some_and(|last| now_ms.saturating_sub(*last) < SIGNAL_COOLDOWN_MS)
    {
        return Ok(skipped_for_signal(
            signal,
            &format!(
                "Re-entry cooldown: {signal_key} needs a materially new signal after {} seconds",
                SIGNAL_COOLDOWN_MS / 1000
            ),
        ));
    }
    let risk_fraction = if settings.risk_limited_sizing {
        0.25
    } else {
        1.0
    };
    let exploration_cap = if scout {
        settings.max_order_notional * SCOUT_MAX_ORDER_FRACTION
    } else if exploratory {
        settings.max_order_notional * (settings.exploration_percent / 100.0)
    } else {
        settings.max_order_notional
    };
    let risk_order_cap = exploration_cap.min(settings.max_order_notional * risk_fraction);
    let mut notional =
        risk_order_cap.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 {
        return Ok(skipped_for_signal(
            signal,
            "Total notional capacity is fully allocated",
        ));
    }
    if exploratory {
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings
            .last_risk_note
            .push_str(if scout {
                "scout order capped at 5% of max order; no sufficiently negative same-side walk-forward evidence"
            } else {
                "small paper-exploration size; no reliable same-side walk-forward edge yet"
            });
    }
    let baseline_params = ExecutionParameters {
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_rate: settings.vwap_participation_percent / 100.0,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
    };
    let baseline_type = choose_order_type(&signal);
    let context = market_context(&signal);
    let order_type = if scout {
        baseline_type
    } else if let Some(order_type) = adapt_order_type(baseline_type, &context, &settings.learning) {
        order_type
    } else {
        return Ok(skipped_for_signal(
            signal,
            "Recent closed auto-trades show no profitable order type with enough samples",
        ));
    };
    let profile = choose_parameter_profile(order_type, &context, &settings.learning, exploratory);
    let execution_params = parameter_profile(baseline_params, profile);
    *state.latest_execution_parameters.lock().await = Some(LatestExecutionParameters {
        timestamp: now_ms,
        symbol: signal.symbol.clone(),
        context: context.clone(),
        baseline_order_type: format!("{baseline_type:?}"),
        selected_order_type: format!("{order_type:?}"),
        profile,
        vwap_duration_minutes: execution_params.vwap_duration_minutes,
        vwap_participation_percent: execution_params.vwap_participation_rate * 100.0,
        limit_offset_bps: execution_params.limit_offset_bps,
        trigger_offset_bps: execution_params.trigger_offset_bps,
        trailing_distance_percent: execution_params.trailing_distance_percent,
    });
    let learning_context = if scout {
        format!("scout:{context}:P{profile}")
    } else {
        format!("{context}:P{profile}")
    };
    drop(settings);
    let side = if signal.direction == "bullish" {
        "buy"
    } else {
        "sell"
    };
    let deep_book_started = Instant::now();
    let minimum_book_order = if scout {
        MIN_SCOUT_DEEP_BOOK_ORDER_NOTIONAL
    } else {
        MIN_DEEP_BOOK_ORDER_NOTIONAL
    };
    let deep_book = assess_deep_book(
        &market,
        &signal.symbol,
        side == "buy",
        notional,
        minimum_book_order,
    );
    let deep_book_ms = deep_book_started.elapsed().as_secs_f64() * 1000.0;
    // Walk-forward net returns include configured taker fees, but not the live
    // spread/sweep cost of this particular order. Avoid trading when that
    // incremental round-trip cost would consume the measured edge.
    if let Some(evidence) = evidence {
        let side_edge = if signal.direction == "bullish" {
            evidence.all_signal_long.average_net_return_percent
        } else {
            evidence.all_signal_short.average_net_return_percent
        };
        if let (Some(edge_percent), Some(assessment)) = (side_edge, deep_book) {
            let spread_bps = (ask - bid) / mid * 10_000.0;
            let incremental_cost_percent = (spread_bps + 2.0 * assessment.impact_bps) / 100.0;
            if edge_percent.is_finite()
                && incremental_cost_percent.is_finite()
                && incremental_cost_percent >= edge_percent
            {
                return Ok(skipped_for_signal(
                    signal,
                    &format!(
                        "Estimated spread and sweep cost ({incremental_cost_percent:.3}%) consumes the measured walk-forward edge ({edge_percent:.3}%)"
                    ),
                ));
            }
        }
    }
    let deep_book_note = if let Some(assessment) = deep_book {
        if !assessment.impact_bps.is_finite() || assessment.impact_bps > DEEP_BOOK_BAND_BPS {
            return Ok(skipped(&format!(
                "Deep book cannot support a {} {side} order: estimated impact {:.2} bps exceeds {:.0} bps",
                signal.symbol, assessment.impact_bps, DEEP_BOOK_BAND_BPS
            )));
        }
        notional = notional.min(assessment.notional_cap);
        if notional < minimum_book_order {
            return Ok(skipped(&format!(
                "Deep-book capacity for {} ({:.2}) is below the minimum paper order ({:.2})",
                signal.symbol, assessment.notional_cap, minimum_book_order
            )));
        }
        Some(format!(
            "deep book: {:.0} quote liquidity within {:.0} bps; order cap {:.0} at {:.0}% participation; estimated sweep {:.2} bps",
            assessment.available_within_band,
            DEEP_BOOK_BAND_BPS,
            assessment.notional_cap,
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            assessment.impact_bps
        ))
    } else {
        None
    };
    if let Some(note) = deep_book_note {
        let mut settings = state.settings.lock().await;
        if !settings.last_risk_note.is_empty() {
            settings.last_risk_note.push_str(" · ");
        }
        settings.last_risk_note.push_str(&note);
    }
    let quantity = notional / mid;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut stage_timings_ms = BTreeMap::new();
    stage_timings_ms.insert("dashboard_fetch".to_owned(), dashboard_ms);
    stage_timings_ms.insert("signal_selection".to_owned(), signal_selection_ms);
    stage_timings_ms.insert("local_market_book".to_owned(), market_inputs_ms);
    stage_timings_ms.insert("deep_book_local".to_owned(), deep_book_ms);
    stage_timings_ms.insert(
        "cycle_to_order_record".to_owned(),
        cycle_started.elapsed().as_secs_f64() * 1000.0,
    );
    stage_timings_ms.insert(
        "signal_age_at_order_record".to_owned(),
        timestamp.saturating_sub(signal.timestamp) as f64,
    );
    let is_post_only = matches!(order_type, AutoOrderType::PostOnlyLimit);
    let submitted_limit_price = is_post_only.then_some(if side == "buy" { bid } else { ask });
    let limit_to_mid_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let limit_to_crossing_quote_bps = submitted_limit_price.map(|price| {
        if side == "buy" {
            (ask - price) / mid * 10_000.0
        } else {
            (price - bid) / mid * 10_000.0
        }
    });
    let expires_at = is_post_only.then_some(timestamp.saturating_add(300_000));
    let estimated_ioc_all_in_bps =
        deep_book.map(|assessment| assessment.impact_bps + configured_taker_fee_rate() * 10_000.0);
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        settings.used_notional_today = allocated_notional(&settings);
        settings
            .last_signal_at
            .insert(signal_key.clone(), timestamp);
        settings
            .last_signal_direction
            .insert(signal_key.clone(), signal.direction.clone());
        settings
            .last_signal_strength
            .insert(signal_key, signal.strength);
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: signal.symbol.clone(),
            side: side.to_owned(),
            order_type: format!("{order_type:?} · P{profile}"),
            purpose: if scout { "scout" } else { "signal" }.into(),
            decision_id: Some(decision_id.clone()),
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: notional,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: Some(learning_context.clone()),
            book_bid_depth_10bps: deep_book.map(|assessment| assessment.bid_depth_10bps),
            book_ask_depth_10bps: deep_book.map(|assessment| assessment.ask_depth_10bps),
            book_depth_imbalance_10bps: deep_book
                .map(|assessment| assessment.depth_imbalance_10bps),
            estimated_sweep_bps: deep_book.map(|assessment| assessment.impact_bps),
            estimated_ioc_all_in_bps,
            decision_bid: Some(bid),
            decision_ask: Some(ask),
            decision_mid: Some(mid),
            submitted_limit_price,
            target_price: None,
            limit_to_mid_bps,
            limit_to_crossing_quote_bps,
            expires_at,
            failure_reason: None,
            stage_timings_ms: stage_timings_ms.clone(),
            status: "submitting".to_owned(),
            realized_pnl: 0.0,
        });
        if settings.orders.len() > 500 {
            let excess = settings.orders.len() - 500;
            settings.orders.drain(..excess);
        }
        persist_settings(&state.settings_path, &settings)?;
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(skipped_for_signal(
            signal,
            "Auto Trader was disabled before order submission",
        ));
    }
    let submit_started = Instant::now();
    let response = match submit_configured_order(
        &client,
        &position_url,
        &signal.symbol,
        side,
        quantity,
        notional,
        bid,
        ask,
        order_type,
        execution_params,
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".into();
                order.failure_reason = Some(error.1.clone());
            }
            persist_settings(&state.settings_path, &settings)?;
            return Err(error);
        }
    };
    let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
    stage_timings_ms.insert("order_submit_response".to_owned(), submit_response_ms);
    if let Some(handler_ms) = response
        .get("position_manager_handler_ms")
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
    {
        stage_timings_ms.insert("position_manager_handler".to_owned(), handler_ms);
        stage_timings_ms.insert(
            "client_transport_and_decode".to_owned(),
            (submit_response_ms - handler_ms).max(0.0),
        );
        if let Some(phases) = response
            .get("processing_timings_ms")
            .and_then(serde_json::Value::as_object)
        {
            for (phase, value) in phases {
                if let Some(milliseconds) = value
                    .as_f64()
                    .filter(|value| value.is_finite() && *value >= 0.0)
                {
                    stage_timings_ms.insert(format!("position_manager_{phase}"), milliseconds);
                }
            }
        }
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let filled = response
        .get("filled_quantity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let average_price = response
        .get("average_execution_price")
        .and_then(serde_json::Value::as_f64);
    let fee = response
        .get("estimated_fee")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        .abs();
    let (realized_delta, learned_type) = if filled > 0.0 {
        let result = average_price
            .map(|price| {
                apply_auto_fill(
                    &mut settings.positions,
                    &signal.symbol,
                    side,
                    filled,
                    price,
                    fee,
                    &format!("{order_type:?}"),
                    &learning_context,
                )
            })
            .unwrap_or_default();
        if let Some(position) = settings.positions.get_mut(&signal.symbol) {
            position.entry_decision_id = Some(decision_id.clone());
        }
        result
    } else {
        (0.0, None)
    };
    settings.realized_pnl += realized_delta;
    settings.daily_realized_pnl += realized_delta;
    if let Some(kind) = learned_type {
        let stats = settings.learning.entry(kind).or_default();
        stats.samples += 1;
        if realized_delta > 0.0 {
            stats.positive += 1;
        }
        stats.realized_pnl += realized_delta;
    }
    let status = if response
        .get("barrier_order")
        .is_some_and(|value| !value.is_null())
    {
        "armed"
    } else if response.get("order").is_some_and(|value| !value.is_null()) {
        "resting"
    } else if filled > 0.0 {
        "filled"
    } else {
        "accepted"
    };
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.fill_rate = (filled / quantity).clamp(0.0, 1.0);
        order.average_price = average_price;
        order.fees_paid = fee;
        order.execution_order_id = response
            .get("order")
            .and_then(|order| order.get("id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        order.status = status.to_owned();
        order.realized_pnl = realized_delta;
        order.stage_timings_ms = stage_timings_ms;
        order.failure_reason = None;
    }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    let mut audit_payload = signal_audit_payload(
        signal,
        "entry",
        timestamp,
        status,
        None,
        Some(exploratory),
        counterfactual_stop_loss_percent,
        counterfactual_take_profit_percent,
        evidence,
    );
    audit_payload["scout"] = serde_json::json!(scout);
    audit_payload["order_type"] = serde_json::json!(format!("{order_type:?}"));
    audit_payload["order_quantity"] = serde_json::json!(quantity);
    audit_payload["filled_quantity"] = serde_json::json!(filled);
    audit_payload["average_price"] = serde_json::json!(average_price);
    audit_payload["fees"] = serde_json::json!(fee);
    audit_payload["realized_pnl"] = serde_json::json!(realized_delta);
    let response_received_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let implementation_shortfall_bps = average_price.map(|price| {
        if side == "buy" {
            (price / mid - 1.0) * 10_000.0
        } else {
            (1.0 - price / mid) * 10_000.0
        }
    });
    let fee_bps = (filled > 0.0 && mid > 0.0).then(|| fee / (filled * mid) * 10_000.0);
    audit_payload["side"] = serde_json::json!(side);
    audit_payload["decision_mid_price"] = serde_json::json!(mid);
    audit_payload["submitted_at"] = serde_json::json!(timestamp);
    audit_payload["response_received_at"] = serde_json::json!(response_received_at);
    audit_payload["submit_response_ms"] = serde_json::json!(submit_response_ms);
    audit_payload["fill_rate"] = serde_json::json!((filled / quantity).clamp(0.0, 1.0));
    audit_payload["implementation_shortfall_bps"] = serde_json::json!(implementation_shortfall_bps);
    audit_payload["fee_bps"] = serde_json::json!(fee_bps);
    audit_payload["all_in_cost_bps"] = serde_json::json!(
        implementation_shortfall_bps
            .zip(fee_bps)
            .map(|(shortfall, fees)| shortfall + fees)
    );
    audit_payload["bid"] = serde_json::json!(bid);
    audit_payload["ask"] = serde_json::json!(ask);
    audit_payload["spread_bps"] = serde_json::json!((ask - bid) / mid * 10_000.0);
    audit_payload["book_bid_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.bid_depth_10bps));
    audit_payload["book_ask_depth_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.ask_depth_10bps));
    audit_payload["book_depth_imbalance_10bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.depth_imbalance_10bps));
    audit_payload["estimated_sweep_bps"] =
        serde_json::json!(deep_book.map(|assessment| assessment.impact_bps));
    audit_payload["estimated_ioc_all_in_bps"] = serde_json::json!(estimated_ioc_all_in_bps);
    audit_payload["submitted_limit_price"] = serde_json::json!(submitted_limit_price);
    audit_payload["limit_to_mid_bps"] = serde_json::json!(limit_to_mid_bps);
    audit_payload["limit_to_crossing_quote_bps"] = serde_json::json!(limit_to_crossing_quote_bps);
    audit_payload["expires_at"] = serde_json::json!(expires_at);
    audit_payload["execution_response"] = response.clone();
    queue_auto_trader_audit(state, audit_payload);
    let stop_armed = if filled > 0.0 {
        if let Some(entry_price) = average_price {
            match arm_protective_stop(
                &client,
                &position_url,
                state,
                &signal.symbol,
                side,
                filled,
                entry_price,
                fee,
            )
            .await
            {
                Ok(()) => true,
                Err((_, error)) => {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!(
                        "Protective stop could not be armed: {error}; five-second exit monitor remains as fallback"
                    );
                    let _ = persist_settings(&state.settings_path, &settings);
                    false
                }
            }
        } else {
            false
        }
    } else {
        false
    };
    Ok(CycleResult {
        status: "order_submitted",
        reason: stop_armed.then(|| "Protective stop-market armed".into()),
        symbol: Some(signal.symbol.clone()),
        side: Some(side.into()),
        quantity: Some(quantity),
        order_type: Some(format!("{order_type:?}")),
        order_response: Some(response),
    })
}

async fn maybe_exit_auto_position(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    // Reconciliation performs position-manager HTTP calls and may persist
    // settings. Keep it outside the shared cycle lock so this periodic
    // maintenance work does not inflate cycle-lock hold time. Each in-memory
    // update is serialized by the settings mutex, and the exit path below
    // still takes the cycle lock before mutating orders or positions.
    sync_protective_stops(client, base, state).await?;
    ensure_protective_stops(client, base, state).await?;
    let positions = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<Vec<_>>()
    };
    // These reads do not depend on each other. Fetch them concurrently, then
    // retain the existing deterministic position processing and serialized
    // stop/order handling below.
    let mut input_tasks = JoinSet::new();
    for (symbol, _) in &positions {
        let client = client.clone();
        let base = base.to_owned();
        let symbol = symbol.clone();
        input_tasks.spawn(async move {
            let inputs: serde_json::Value = client
                .get(format!("{base}/internal/insights-inputs?symbol={symbol}"))
                .send()
                .await
                .map_err(upstream_error)?
                .error_for_status()
                .map_err(upstream_error)?
                .json()
                .await
                .map_err(upstream_error)?;
            Ok::<_, (StatusCode, String)>((symbol, inputs))
        });
    }
    let mut inputs_by_symbol = BTreeMap::new();
    while let Some(result) = input_tasks.join_next().await {
        let (symbol, inputs) = result.map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Market input task failed: {error}"),
            )
        })??;
        inputs_by_symbol.insert(symbol, inputs);
    }
    // Market input requests do not mutate trader state. Keep them outside the
    // shared cycle lock, then serialize exit decisions and order mutations.
    let _cycle = acquire_cycle_lock(state, "position_maintenance").await;
    for (symbol, position) in positions {
        let position_is_current = {
            let settings = state.settings.lock().await;
            settings.positions.get(&symbol).is_some_and(|current| {
                (current.quantity - position.quantity).abs() <= 1e-12
                    && (current.average_entry_price - position.average_entry_price).abs() <= 1e-12
            })
        };
        if !position_is_current {
            continue;
        }
        let Some(inputs) = inputs_by_symbol.remove(&symbol) else {
            continue;
        };
        let market = &inputs["markets"][&symbol];
        let bid = market["bids"][0]["price"].as_f64();
        let ask = market["asks"][0]["price"].as_f64();
        let Some((bid, ask)) = bid
            .zip(ask)
            .filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid)
        else {
            continue;
        };
        let is_long = position.quantity > 0.0;
        let exit_price = if is_long { bid } else { ask };
        let entry_notional = position.quantity.abs() * position.average_entry_price;
        let net_open_pnl = (exit_price - position.average_entry_price) * position.quantity
            - position.entry_fees
            - position.quantity.abs() * exit_price * configured_taker_fee_rate();
        let return_fraction = if entry_notional > 0.0 {
            net_open_pnl / entry_notional
        } else {
            0.0
        };
        let (stop_loss_percent, take_profit_percent) = {
            let settings = state.settings.lock().await;
            (settings.stop_loss_percent, settings.take_profit_percent)
        };
        let reason = if return_fraction <= -stop_loss_percent / 100.0 {
            Some("stop-loss")
        } else if return_fraction >= take_profit_percent / 100.0 {
            Some("take-profit")
        } else {
            None
        };
        let Some(reason) = reason else {
            continue;
        };
        let entry_decision_id = position.entry_decision_id.clone();
        let protected_quantity = position
            .protective_stops
            .iter()
            .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
            .sum::<f64>();
        if reason == "stop-loss" && protected_quantity + 1e-12 >= position.quantity.abs() {
            // The server-side barrier is already watching this price; don't race it with a second IOC.
            continue;
        }
        if !cancel_protective_stops(client, base, state, &symbol).await? {
            // A stop is already triggering or could not be cancelled. Let its server-side execution settle.
            continue;
        }
        let side = if is_long { "sell" } else { "buy" };
        let quantity = position.quantity.abs();
        let notional = quantity * (bid + ask) / 2.0;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        {
            let mut settings = state.settings.lock().await;
            reset_daily_budget(&mut settings);
            settings.used_notional_today = allocated_notional(&settings);
            settings.orders.push(TraderOrder {
                timestamp,
                triggered_at: None,
                symbol: symbol.clone(),
                side: side.into(),
                order_type: "Ioc".into(),
                purpose: reason.into(),
                decision_id: None,
                quantity,
                filled_quantity: 0.0,
                fill_rate: 0.0,
                average_price: None,
                fees_paid: 0.0,
                reserved_notional: notional,
                execution_order_id: None,
                processed_fill_trade_ids: Vec::new(),
                learning_context: None,
                book_bid_depth_10bps: None,
                book_ask_depth_10bps: None,
                book_depth_imbalance_10bps: None,
                estimated_sweep_bps: None,
                estimated_ioc_all_in_bps: None,
                decision_bid: None,
                decision_ask: None,
                decision_mid: None,
                submitted_limit_price: None,
                target_price: None,
                limit_to_mid_bps: None,
                limit_to_crossing_quote_bps: None,
                expires_at: None,
                failure_reason: None,
                stage_timings_ms: BTreeMap::new(),
                status: "submitting".into(),
                realized_pnl: 0.0,
            });
        }
        let submit_started = Instant::now();
        let response = match submit_configured_order(
            client,
            base,
            &symbol,
            side,
            quantity,
            notional,
            bid,
            ask,
            AutoOrderType::Ioc,
            ExecutionParameters::default(),
        )
        .await
        {
            Ok(response) => response,
            Err(error) => {
                let mut settings = state.settings.lock().await;
                if let Some(order) = settings
                    .orders
                    .iter_mut()
                    .find(|order| order.timestamp == timestamp)
                {
                    order.status = "failed".into();
                }
                persist_settings(&state.settings_path, &settings)?;
                return Err(error);
            }
        };
        let submit_response_ms = submit_started.elapsed().as_secs_f64() * 1000.0;
        let mut settings = state.settings.lock().await;
        let filled = response["filled_quantity"].as_f64().unwrap_or(0.0);
        let average_price = response["average_execution_price"].as_f64();
        let fee = response["estimated_fee"].as_f64().unwrap_or(0.0).abs();
        let (realized_pnl, learned_key) = if filled > 0.0 {
            average_price
                .map(|price| {
                    apply_auto_fill(
                        &mut settings.positions,
                        &symbol,
                        side,
                        filled,
                        price,
                        fee,
                        "Ioc",
                        reason,
                    )
                })
                .unwrap_or_default()
        } else {
            (0.0, None)
        };
        settings.realized_pnl += realized_pnl;
        settings.daily_realized_pnl += realized_pnl;
        if filled + 1e-12 >= quantity {
            let closed_quantity = filled.min(quantity);
            let entry_notional = closed_quantity * position.average_entry_price;
            if entry_notional > 0.0 {
                settings
                    .closed_trade_returns_percent
                    .push(realized_pnl / entry_notional * 100.0);
                if settings.closed_trade_returns_percent.len() > 200 {
                    settings.closed_trade_returns_percent.remove(0);
                }
                adapt_exit_policy(&mut settings);
            }
        }
        if let Some(key) = learned_key {
            let stats = settings.learning.entry(key).or_default();
            stats.samples += 1;
            if realized_pnl > 0.0 {
                stats.positive += 1;
            }
            stats.realized_pnl += realized_pnl;
        }
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.filled_quantity = filled;
            order.average_price = average_price;
            order.realized_pnl = realized_pnl;
            order.status = if filled > 0.0 { "filled" } else { "accepted" }.into();
        }
        persist_settings(&state.settings_path, &settings)?;
        drop(settings);
        if let Some(decision_id) = entry_decision_id {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let payload = serde_json::json!({
                "decision_id": decision_id, "stage": format!("exit:{timestamp}"), "timestamp": now, "symbol": symbol,
                "status": if filled > 0.0 { "filled" } else { "accepted" }, "reason": reason,
                "exit_order_type": "Ioc", "exit_order_quantity": quantity, "exit_filled_quantity": filled,
                "exit_average_price": average_price, "exit_fees": fee, "realized_pnl": realized_pnl,
                "net_return_percent": if entry_notional > 0.0 { Some(realized_pnl / entry_notional * 100.0) } else { None },
                "side": side,
                "decision_mid_price": (bid + ask) / 2.0,
                "submitted_at": timestamp,
                "response_received_at": now,
                "submit_response_ms": submit_response_ms,
                "fill_rate": if quantity > 0.0 { (filled / quantity).clamp(0.0, 1.0) } else { 0.0 },
                "implementation_shortfall_bps": average_price.map(|price| if side == "buy" { (price / ((bid + ask) / 2.0) - 1.0) * 10_000.0 } else { (1.0 - price / ((bid + ask) / 2.0)) * 10_000.0 }),
                "fee_bps": if filled > 0.0 { Some(fee / (filled * ((bid + ask) / 2.0)) * 10_000.0) } else { None },
                "exit_bid": bid, "exit_ask": ask
            });
            store_auto_trader_audit(client, &payload).await;
        }
        return Ok(Some(CycleResult {
            status: "order_submitted",
            reason: Some(format!(
                "Auto-trader {reason} at {:.4}%",
                return_fraction * 100.0
            )),
            symbol: Some(symbol),
            side: Some(side.into()),
            quantity: Some(quantity),
            order_type: Some("Ioc exit".into()),
            order_response: Some(response),
        }));
    }
    Ok(None)
}

async fn arm_protective_stop(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
    position_side: &str,
    quantity: f64,
    entry_price: f64,
    entry_fees: f64,
) -> Result<(), (StatusCode, String)> {
    if quantity <= 0.0 || entry_price <= 0.0 {
        return Ok(());
    }
    let (stop_loss_percent, is_enabled) = {
        let settings = state.settings.lock().await;
        (
            settings.stop_loss_percent,
            state.enabled.load(Ordering::Relaxed),
        )
    };
    if !is_enabled {
        return Ok(());
    }
    let is_long = position_side == "buy";
    let side = if is_long { "sell" } else { "buy" };
    let fee_rate = configured_taker_fee_rate();
    let loss_fraction = stop_loss_percent / 100.0;
    let trigger_price = if is_long {
        (quantity * entry_price * (1.0 - loss_fraction) + entry_fees)
            / (quantity * (1.0 - fee_rate))
    } else {
        (quantity * entry_price * (1.0 + loss_fraction) - entry_fees)
            / (quantity * (1.0 + fee_rate))
    };
    let trigger_direction = if is_long { "below" } else { "above" };
    let response = client
        .post(format!("{base}/positions/orders"))
        .json(&serde_json::json!({
            "symbol": symbol, "side": side, "order_type": "stop-market", "quantity": quantity,
            "trigger_price": trigger_price, "trigger_direction": trigger_direction
        }))
        .send()
        .await
        .map_err(upstream_error)?;
    let status = response.status();
    let payload = response
        .json::<serde_json::Value>()
        .await
        .map_err(upstream_error)?;
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Protective stop rejected ({status}): {payload}"),
        ));
    }
    let barrier = &payload["barrier_order"];
    let id = barrier["id"]
        .as_str()
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Protective stop response did not include a barrier id".into(),
        ))?
        .to_owned();
    let timestamp = barrier["created_at"].as_u64().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    });
    let mut settings = state.settings.lock().await;
    if let Some(position) = settings.positions.get_mut(symbol).filter(|position| {
        position.quantity.abs() > 1e-12
            && position.quantity.signum() == if is_long { 1.0 } else { -1.0 }
    }) {
        position.protective_stops.push(ProtectiveStop {
            barrier_id: id,
            order_timestamp: timestamp,
            quantity,
            processed_quantity: 0.0,
            processed_notional: 0.0,
            processed_fee: 0.0,
        });
        settings.orders.push(TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: symbol.to_owned(),
            side: side.to_owned(),
            order_type: "StopMarket".into(),
            purpose: "stop-loss".into(),
            decision_id: None,
            quantity,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: 0.0,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: None,
            book_bid_depth_10bps: None,
            book_ask_depth_10bps: None,
            book_depth_imbalance_10bps: None,
            estimated_sweep_bps: None,
            estimated_ioc_all_in_bps: None,
            decision_bid: None,
            decision_ask: None,
            decision_mid: None,
            submitted_limit_price: None,
            target_price: Some(trigger_price),
            limit_to_mid_bps: None,
            limit_to_crossing_quote_bps: None,
            expires_at: None,
            failure_reason: None,
            stage_timings_ms: BTreeMap::new(),
            status: "armed".into(),
            realized_pnl: 0.0,
        });
        if settings.orders.len() > 500 {
            let excess = settings.orders.len() - 500;
            settings.orders.drain(..excess);
        }
        persist_settings(&state.settings_path, &settings)?;
    } else {
        drop(settings);
        let _ = client
            .delete(format!("{base}/positions/orders/{id}"))
            .send()
            .await;
    }
    Ok(())
}

async fn sync_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let tracked = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .flat_map(|(symbol, position)| {
                position
                    .protective_stops
                    .iter()
                    .cloned()
                    .map(move |stop| (symbol.clone(), stop))
            })
            .collect::<Vec<_>>()
    };
    if tracked.is_empty() {
        return Ok(());
    }
    let barriers: Vec<serde_json::Value> = client
        .get(format!("{base}/positions/barriers"))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json()
        .await
        .map_err(upstream_error)?;
    for (symbol, stop) in tracked {
        let barrier = barriers
            .iter()
            .find(|barrier| barrier["id"].as_str() == Some(&stop.barrier_id));
        let Some(barrier) = barrier else {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = "cancelled".into();
            }
            persist_settings(&state.settings_path, &settings)?;
            continue;
        };
        let total_filled = barrier["filled_quantity"].as_f64().unwrap_or(0.0).max(0.0);
        let total_notional =
            barrier["average_execution_price"].as_f64().unwrap_or(0.0) * total_filled;
        let total_fee = barrier["fee_paid"].as_f64().unwrap_or(0.0).abs();
        let delta_quantity = (total_filled - stop.processed_quantity).max(0.0);
        let delta_notional = (total_notional - stop.processed_notional).max(0.0);
        let delta_fee = (total_fee - stop.processed_fee).max(0.0);
        let status = barrier["status"].as_str().unwrap_or("unknown");
        if delta_quantity > 1e-12 && delta_notional > 0.0 {
            let side = if barrier["side"].as_str() == Some("buy") {
                "buy"
            } else {
                "sell"
            };
            let fill_price = delta_notional / delta_quantity;
            let mut settings = state.settings.lock().await;
            let entry_decision_id = settings
                .positions
                .get(&symbol)
                .and_then(|position| position.entry_decision_id.clone());
            let entry_price_before = settings
                .positions
                .get(&symbol)
                .map(|position| position.average_entry_price)
                .filter(|price| *price > 0.0)
                .unwrap_or(fill_price);
            let (realized_delta, learned_key) = apply_auto_fill(
                &mut settings.positions,
                &symbol,
                side,
                delta_quantity,
                fill_price,
                delta_fee,
                "StopMarket",
                "stop-loss",
            );
            settings.realized_pnl += realized_delta;
            settings.daily_realized_pnl += realized_delta;
            settings.used_notional_today = allocated_notional(&settings);
            let entry_notional = delta_quantity * entry_price_before;
            let fully_closed = settings
                .positions
                .get(&symbol)
                .is_none_or(|position| position.quantity.abs() <= 1e-12);
            if fully_closed && entry_notional > 0.0 {
                settings
                    .closed_trade_returns_percent
                    .push(realized_delta / entry_notional * 100.0);
                if settings.closed_trade_returns_percent.len() > 200 {
                    settings.closed_trade_returns_percent.remove(0);
                }
                adapt_exit_policy(&mut settings);
            }
            if let Some(key) = learned_key {
                let stats = settings.learning.entry(key).or_default();
                stats.samples += 1;
                if realized_delta > 0.0 {
                    stats.positive += 1;
                }
                stats.realized_pnl += realized_delta;
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.triggered_at = barrier["triggered_at"].as_u64();
                order.filled_quantity = total_filled;
                order.average_price = barrier["average_execution_price"].as_f64();
                order.realized_pnl += realized_delta;
                order.status = status.to_owned();
            }
            if let Some(position) = settings.positions.get_mut(&symbol) {
                if let Some(tracked) = position
                    .protective_stops
                    .iter_mut()
                    .find(|tracked| tracked.barrier_id == stop.barrier_id)
                {
                    tracked.processed_quantity = total_filled;
                    tracked.processed_notional = total_notional;
                    tracked.processed_fee = total_fee;
                }
            }
            persist_settings(&state.settings_path, &settings)?;
            let cumulative_pnl = settings
                .orders
                .iter()
                .find(|order| order.timestamp == stop.order_timestamp)
                .map(|order| order.realized_pnl)
                .unwrap_or(realized_delta);
            drop(settings);
            if let Some(decision_id) = entry_decision_id {
                let update_timestamp = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let payload = serde_json::json!({
                    "decision_id": decision_id, "stage": format!("exit:stop:{}", stop.order_timestamp), "timestamp": update_timestamp,
                    "observed_at": update_timestamp, "symbol": symbol, "side": barrier["side"],
                    "status": status, "reason": "stop-loss", "exit_order_type": "StopMarket",
                    "submitted_at": barrier["triggered_at"],
                    "exit_order_quantity": stop.quantity, "exit_filled_quantity": total_filled,
                    "exit_average_price": barrier["average_execution_price"],
                    "exit_fees": total_fee, "realized_pnl": cumulative_pnl,
                    "net_return_percent": if entry_price_before > 0.0 { Some(cumulative_pnl / (total_filled * entry_price_before).max(1e-12) * 100.0) } else { None }
                });
                store_auto_trader_audit(client, &payload).await;
            }
        }
        if !matches!(status, "armed" | "triggering") {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) {
                position
                    .protective_stops
                    .retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == stop.order_timestamp)
            {
                order.status = status.to_owned();
            }
            persist_settings(&state.settings_path, &settings)?;
        }
    }
    Ok(())
}

async fn ensure_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let positions = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.quantity.abs() > 1e-12)
            .map(|(symbol, position)| (symbol.clone(), position.clone()))
            .collect::<Vec<_>>()
    };
    for (symbol, position) in positions {
        let covered = position
            .protective_stops
            .iter()
            .map(|stop| (stop.quantity - stop.processed_quantity).max(0.0))
            .sum::<f64>();
        let uncovered = (position.quantity.abs() - covered).max(0.0);
        if uncovered > 1e-12 {
            if let Err((_, error)) = arm_protective_stop(
                client,
                base,
                state,
                &symbol,
                if position.quantity > 0.0 {
                    "buy"
                } else {
                    "sell"
                },
                uncovered,
                position.average_entry_price,
                position.entry_fees * uncovered / position.quantity.abs(),
            )
            .await
            {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!(
                    "Protective stop could not be armed for {symbol}: {error}; five-second exit monitor remains as fallback"
                );
            }
        }
    }
    Ok(())
}

async fn cancel_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
) -> Result<bool, (StatusCode, String)> {
    let stops = {
        let settings = state.settings.lock().await;
        settings
            .positions
            .get(symbol)
            .map(|position| position.protective_stops.clone())
            .unwrap_or_default()
    };
    for stop in stops {
        let response = client
            .delete(format!("{base}/positions/orders/{}", stop.barrier_id))
            .send()
            .await
            .map_err(upstream_error)?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let mut settings = state.settings.lock().await;
        if let Some(position) = settings.positions.get_mut(symbol) {
            position
                .protective_stops
                .retain(|tracked| tracked.barrier_id != stop.barrier_id);
        }
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == stop.order_timestamp)
        {
            order.status = "cancelled".into();
        }
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(true)
}

#[derive(Clone)]
struct RiskHolding {
    symbol: String,
    signed_usd: f64,
}

async fn maybe_place_portfolio_hedge(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let candidates_request = client
        .get(format!("{base}/positions/hedge-candidates"))
        .send();
    let rates_request = client.get(format!("{base}/positions/usd-rates")).send();
    let (candidates_response, rates_response) =
        tokio::try_join!(candidates_request, rates_request).map_err(upstream_error)?;
    let candidates_response = candidates_response
        .error_for_status()
        .map_err(upstream_error)?;
    let rates_response = rates_response.error_for_status().map_err(upstream_error)?;
    let (candidates, rates_response) = tokio::try_join!(
        candidates_response.json::<serde_json::Value>(),
        rates_response.json::<serde_json::Value>(),
    )
    .map_err(upstream_error)?;
    let rates = &rates_response["rates"];
    let positions = candidates["positions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut holdings = Vec::new();
    let mut fallback_marks = 0;
    for position in &positions {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let quantity = position["quantity"].as_f64().unwrap_or(0.0);
        let live_price = position["latest_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let entry_price = position["average_entry_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let quote = symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or_default();
        let rate = rates[quote].as_f64().unwrap_or(0.0);
        if quantity.is_finite() && rate.is_finite() && rate > 0.0 {
            if let Some(price) = live_price.or(entry_price) {
                if live_price.is_none() {
                    fallback_marks += 1;
                }
                holdings.push(RiskHolding {
                    symbol: symbol.to_owned(),
                    signed_usd: quantity * price * rate,
                });
            }
        }
    }
    let unknown_marks = positions.len().saturating_sub(holdings.len());
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note = match (fallback_marks, unknown_marks) {
            (fallback, unknown) if unknown > 0 => format!(
                "risk covers {} open positions; {fallback} use entry-price marks and {unknown} are unmarked, so hedging is paused",
                positions.len()
            ),
            (fallback, _) if fallback > 0 => format!(
                "risk covers {} open positions; {fallback} use entry-price marks",
                positions.len()
            ),
            _ => format!("risk covers {} open positions", positions.len()),
        };
        settings.risk_limited_sizing = unknown_marks > 0;
    }
    if holdings.is_empty() || unknown_marks > 0 {
        return Ok(None);
    }
    let mut portfolio_variance = holdings
        .iter()
        .map(|holding| holding.signed_usd.powi(2))
        .sum::<f64>();
    if let Some(pairs) = candidates["pair_correlations"].as_array() {
        for i in 0..holdings.len() {
            for j in (i + 1)..holdings.len() {
                if let Some(pair) = pairs.iter().find(|pair| {
                    (pair["symbol_a"].as_str() == Some(&holdings[i].symbol)
                        && pair["symbol_b"].as_str() == Some(&holdings[j].symbol))
                        || (pair["symbol_b"].as_str() == Some(&holdings[i].symbol)
                            && pair["symbol_a"].as_str() == Some(&holdings[j].symbol))
                }) {
                    if let Some(correlation) = pair["correlation"]
                        .as_f64()
                        .filter(|value| value.is_finite())
                    {
                        portfolio_variance += 2.0
                            * holdings[i].signed_usd
                            * holdings[j].signed_usd
                            * correlation.clamp(-1.0, 1.0);
                    }
                }
            }
        }
    }
    let portfolio_shock_risk = portfolio_variance.max(0.0).sqrt() * PORTFOLIO_SHOCK_PERCENT;
    let settings_snapshot = state.settings.lock().await.clone();
    let risk_trigger = settings_snapshot.daily_realized_loss_limit;
    if portfolio_shock_risk <= risk_trigger {
        return Ok(None);
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut best: Option<(String, String, f64, f64, f64, f64)> = None;
    // Reduce the largest direct holding first, before considering a correlated proxy.
    if let Some(holding) = holdings
        .iter()
        .max_by(|a, b| a.signed_usd.abs().total_cmp(&b.signed_usd.abs()))
    {
        if holding.signed_usd.abs() > 0.0 {
            let row = positions
                .iter()
                .find(|row| row["symbol"].as_str() == Some(&holding.symbol));
            let price = row
                .and_then(|row| {
                    row["latest_price"]
                        .as_f64()
                        .or_else(|| row["average_entry_price"].as_f64())
                })
                .unwrap_or(0.0);
            let quote = holding
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            let last = settings_snapshot
                .last_hedge_at
                .get(&holding.symbol)
                .copied()
                .unwrap_or(0);
            if price > 0.0 && quote_rate > 0.0 && now_ms.saturating_sub(last) >= 300_000 {
                best = Some((
                    holding.symbol.clone(),
                    if holding.signed_usd > 0.0 {
                        "sell"
                    } else {
                        "buy"
                    }
                    .into(),
                    holding
                        .signed_usd
                        .abs()
                        .min(settings_snapshot.max_order_notional),
                    price,
                    quote_rate,
                    holding.signed_usd.abs(),
                ));
            }
        }
    }
    for position in &positions {
        if best.is_some() {
            break;
        }
        let candidate_rows = position["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for candidate in candidate_rows {
            let hedge_symbol = candidate["symbol"].as_str().unwrap_or_default();
            let hedge_price = candidate["latest_price"].as_f64().unwrap_or(0.0);
            let quote = hedge_symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            if hedge_price <= 0.0 || quote_rate <= 0.0 {
                continue;
            }
            let mut covariance = 0.0;
            let mut usable_correlations = 0;
            let mut strongest = 0.0_f64;
            let mut enough_samples = false;
            let mut complete_sample = true;
            for holding in &holdings {
                let relation = if holding.symbol == hedge_symbol {
                    Some((1.0, usize::MAX))
                } else {
                    positions
                        .iter()
                        .find(|row| row["symbol"].as_str() == Some(&holding.symbol))
                        .and_then(|row| row["candidates"].as_array())
                        .and_then(|rows| {
                            rows.iter()
                                .find(|row| row["symbol"].as_str() == Some(hedge_symbol))
                        })
                        .and_then(|row| {
                            row["correlation"]
                                .as_f64()
                                .zip(row["paired_samples"].as_u64())
                                .map(|(corr, n)| (corr, n as usize))
                        })
                };
                if let Some((correlation, samples)) = relation.filter(|(corr, _)| corr.is_finite())
                {
                    if samples < MIN_HEDGE_CORRELATION_SAMPLES {
                        complete_sample = false;
                    }
                    let correlation = correlation.clamp(-1.0, 1.0);
                    covariance += holding.signed_usd * correlation;
                    usable_correlations += 1;
                    if correlation.abs() > strongest {
                        strongest = correlation.abs();
                        enough_samples = samples >= MIN_HEDGE_CORRELATION_SAMPLES;
                    }
                }
            }
            if usable_correlations != holdings.len()
                || !complete_sample
                || !enough_samples
                || strongest < MIN_HEDGE_CORRELATION
                || !covariance.is_finite()
                || covariance.abs() < settings_snapshot.max_order_notional * 0.1
            {
                continue;
            }
            let last = settings_snapshot
                .last_hedge_at
                .get(hedge_symbol)
                .copied()
                .unwrap_or(0);
            if now_ms.saturating_sub(last) < 300_000 {
                continue;
            }
            let hedge_notional = covariance.abs().min(settings_snapshot.max_order_notional);
            let score = covariance.abs().min(portfolio_shock_risk);
            if best.as_ref().is_none_or(|current| score > current.5) {
                let side = if covariance > 0.0 { "sell" } else { "buy" };
                best = Some((
                    hedge_symbol.to_owned(),
                    side.to_owned(),
                    hedge_notional,
                    hedge_price,
                    quote_rate,
                    score,
                ));
            }
        }
    }
    let Some((symbol, side, requested_notional, price, quote_rate, _)) = best else {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but no liquid hedge has enough reliable correlation data; new exposure is blocked",
        )));
    };
    // Candidate and rate reads, plus correlation/risk calculations above, are
    // read-only. Keep them off the shared cycle lock so a slow position-manager
    // response cannot delay a signal or position-maintenance decision. Acquire
    // the lock only when we are ready to reserve the hedge and run its
    // state-changing stop/order sequence.
    let _cycle = acquire_cycle_lock(state, "hedge_evaluation").await;
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let notional =
        requested_notional.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but the daily hedge budget is exhausted",
        )));
    }
    let timestamp = now_ms;
    let quantity = notional / (price * quote_rate);
    settings.used_notional_today = allocated_notional(&settings);
    settings.last_hedge_at.insert(symbol.clone(), timestamp);
    settings.orders.push(TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.clone(),
        side: side.clone(),
        order_type: "Ioc".into(),
        purpose: "portfolio-hedge".into(),
        decision_id: None,
        quantity,
        filled_quantity: 0.0,
        fill_rate: 0.0,
        average_price: None,
        fees_paid: 0.0,
        reserved_notional: notional,
        execution_order_id: None,
        processed_fill_trade_ids: Vec::new(),
        learning_context: None,
        book_bid_depth_10bps: None,
        book_ask_depth_10bps: None,
        book_depth_imbalance_10bps: None,
        estimated_sweep_bps: None,
        estimated_ioc_all_in_bps: None,
        decision_bid: None,
        decision_ask: None,
        decision_mid: None,
        submitted_limit_price: None,
        target_price: None,
        limit_to_mid_bps: None,
        limit_to_crossing_quote_bps: None,
        expires_at: None,
        failure_reason: None,
        stage_timings_ms: BTreeMap::new(),
        status: "submitting".into(),
        realized_pnl: 0.0,
    });
    if settings.orders.len() > 500 {
        let excess = settings.orders.len() - 500;
        settings.orders.drain(..excess);
    }
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    if !cancel_protective_stops(client, base, state, &symbol).await? {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_stop_active".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(Some(skipped(
            "Could not cancel the existing protective stop; deferred portfolio hedge",
        )));
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        settings.last_hedge_at.remove(&symbol);
        if let Some(order) = settings
            .orders
            .iter_mut()
            .find(|order| order.timestamp == timestamp)
        {
            order.status = "cancelled_before_submit".into();
        }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        drop(settings);
        ensure_protective_stops(client, base, state).await?;
        return Ok(Some(skipped(
            "Auto Trader was disabled before hedge submission",
        )));
    }
    let response = match submit_configured_order(
        client,
        base,
        &symbol,
        &side,
        quantity,
        notional,
        price,
        price,
        AutoOrderType::Ioc,
        ExecutionParameters::default(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings
                .orders
                .iter_mut()
                .find(|order| order.timestamp == timestamp)
            {
                order.status = "failed".into();
            }
            persist_settings(&state.settings_path, &settings)?;
            drop(settings);
            ensure_protective_stops(client, base, state).await?;
            return Err(error);
        }
    };
    let mut settings = state.settings.lock().await;
    let filled = response
        .get("filled_quantity")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);
    let average_price = response
        .get("average_execution_price")
        .and_then(serde_json::Value::as_f64);
    let fee = response
        .get("estimated_fee")
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
        .abs();
    let (realized_pnl, _) = if filled > 0.0 {
        average_price
            .map(|execution| {
                apply_auto_fill(
                    &mut settings.positions,
                    &symbol,
                    &side,
                    filled,
                    execution,
                    fee,
                    "Ioc",
                    "portfolio-hedge",
                )
            })
            .unwrap_or_default()
    } else {
        (0.0, None)
    };
    settings.realized_pnl += realized_pnl;
    settings.daily_realized_pnl += realized_pnl;
    let status = if response
        .get("barrier_order")
        .is_some_and(|value| !value.is_null())
    {
        "armed"
    } else if response.get("order").is_some_and(|value| !value.is_null()) {
        "resting"
    } else if filled > 0.0 {
        "filled"
    } else {
        "accepted"
    };
    if let Some(order) = settings
        .orders
        .iter_mut()
        .find(|order| order.timestamp == timestamp)
    {
        order.filled_quantity = filled;
        order.average_price = average_price;
        order.realized_pnl = realized_pnl;
        order.status = status.into();
    }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    ensure_protective_stops(client, base, state).await?;
    Ok(Some(CycleResult {
        status: "hedge_submitted",
        reason: Some(format!(
            "Portfolio 1% shock risk was USD {:.2}; submitted a risk-reducing hedge",
            portfolio_shock_risk
        )),
        symbol: Some(symbol),
        side: Some(side),
        quantity: Some(quantity),
        order_type: Some("Ioc hedge".into()),
        order_response: Some(response),
    }))
}

fn apply_auto_fill(
    positions: &mut BTreeMap<String, TraderPosition>,
    symbol: &str,
    side: &str,
    quantity: f64,
    price: f64,
    fee: f64,
    order_type: &str,
    context: &str,
) -> (f64, Option<String>) {
    let position = positions.entry(symbol.to_owned()).or_default();
    let old = position.quantity;
    let incoming = if side == "buy" { quantity } else { -quantity };
    if old == 0.0 || old.signum() == incoming.signum() {
        position.entries = position.entries.saturating_add(1);
        let total = old.abs() + incoming.abs();
        position.average_entry_price =
            (old.abs() * position.average_entry_price + incoming.abs() * price) / total;
        position.quantity = old + incoming;
        position.entry_fees += fee;
        if position.entry_order_type.is_empty() {
            position.entry_order_type = order_type.to_owned();
            position.entry_context = context.to_owned();
            position.scout_entry = context.starts_with("scout:");
        }
        return (0.0, None);
    }
    let entry_type = if position.entry_order_type.is_empty() {
        order_type.to_owned()
    } else {
        position.entry_order_type.clone()
    };
    let entry_context = if position.entry_context.is_empty() {
        context.to_owned()
    } else {
        position.entry_context.clone()
    };
    let closed = old.abs().min(incoming.abs());
    let entry_fee_closed = position.entry_fees * (closed / old.abs());
    let closing_fee = fee * (closed / incoming.abs());
    let pnl = closed
        * if old > 0.0 {
            price - position.average_entry_price
        } else {
            position.average_entry_price - price
        }
        - entry_fee_closed
        - closing_fee;
    position.realized_pnl += pnl;
    let net = old + incoming;
    if net.abs() < 1e-12 {
        *position = TraderPosition::default();
    } else if net.signum() != old.signum() {
        position.quantity = net;
        position.average_entry_price = price;
        position.entry_fees = fee - closing_fee;
        position.entry_order_type = order_type.to_owned();
        position.entry_context = context.to_owned();
        position.scout_entry = context.starts_with("scout:");
    } else {
        position.quantity = net;
        position.entry_fees -= entry_fee_closed;
    }
    (pnl, Some(format!("{entry_context}:{entry_type}")))
}

fn signal_decision_id(signal: &Signal) -> String {
    format!(
        "{}|{}|{}",
        signal.timestamp, signal.symbol, signal.direction
    )
}

fn signal_audit_payload(
    signal: &Signal,
    stage: &str,
    timestamp: u64,
    status: &str,
    reason: Option<&str>,
    exploratory: Option<bool>,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    evidence: Option<&WalkForward>,
) -> serde_json::Value {
    let evidence_metrics = evidence.map(|item| serde_json::json!({
        "horizon_seconds": item.horizon_seconds,
        "metrics": if signal.direction == "bullish" { &item.all_signal_long } else { &item.all_signal_short },
    }));
    serde_json::json!({
        "decision_id": signal_decision_id(signal),
        "stage": stage,
        "timestamp": timestamp,
        "signal_timestamp": signal.timestamp,
        "symbol": signal.symbol,
        "direction": signal.direction,
        "score": signal.score,
        "strength": signal.strength,
        "ranging_probability": signal.ranging_probability,
        "market_context": market_context(signal),
        "signal_components": signal.components,
        "exploratory": exploratory,
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "walk_forward_evidence": evidence_metrics,
        "status": status,
        "reason": reason,
    })
}

fn queue_auto_trader_audit(state: &AutoTraderState, payload: serde_json::Value) {
    let is_execution = payload
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|stage| stage == "entry" || stage.starts_with("exit"));
    if is_execution {
        // Keep execution events separate so decision bursts cannot crowd them out
        // or delay position protection while the audit service is unavailable.
        if state.execution_audit_tx.send(payload).is_err() {
            crate::service_log!(
                "ERROR",
                "Auto-trader execution audit writer stopped; event could not be queued"
            );
        }
    } else if state.audit_tx.try_send(payload).is_err() {
        crate::service_log!(
            "WARN",
            "Auto-trader audit queue is full; dropping a decision event"
        );
    }
}

async fn store_auto_trader_audit(client: &reqwest::Client, payload: &serde_json::Value) -> bool {
    let base = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let url = format!("{base}/signals/auto-trader-audit");
    for attempt in 0..3 {
        match client
            .post(&url)
            .timeout(Duration::from_secs(2))
            .json(payload)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return true,
            Ok(response) => {
                let status = response.status();
                if attempt == 2 {
                    crate::service_log!(
                        "ERROR",
                        "Auto-trader audit write failed after 3 attempts with status {}",
                        status
                    );
                }
            }
            Err(error) if attempt == 2 => {
                crate::service_log!(
                    "ERROR",
                    "Auto-trader audit write failed after 3 attempts: {}",
                    error
                );
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(150 * (attempt + 1))).await;
    }
    false
}

async fn store_execution_audit_until_saved(client: &reqwest::Client, payload: &serde_json::Value) {
    let mut retry_delay = Duration::from_secs(1);
    while !store_auto_trader_audit(client, payload).await {
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

fn market_context(signal: &Signal) -> String {
    let regime = if signal.ranging_probability.unwrap_or(0.0) >= CONTEXT_RANGE_MIN_PROBABILITY {
        "range"
    } else if signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs()
        >= CONTEXT_TREND_MIN_VOLATILITY_UNITS
    {
        "trend"
    } else if signal.components.volatility_1m_percent.unwrap_or(0.0).abs()
        >= CONTEXT_VOLATILE_MIN_PERCENT
    {
        "volatile"
    } else {
        "mixed"
    };
    format!(
        "{}:{regime}",
        if signal.direction == "bullish" {
            "long"
        } else {
            "short"
        }
    )
}

const ALL_ORDER_TYPES: [AutoOrderType; 12] = [
    AutoOrderType::Market,
    AutoOrderType::Ioc,
    AutoOrderType::Fok,
    AutoOrderType::Limit,
    AutoOrderType::Gtd,
    AutoOrderType::PostOnlyLimit,
    AutoOrderType::Barrier,
    AutoOrderType::StopMarket,
    AutoOrderType::StopLimit,
    AutoOrderType::TrailingStop,
    AutoOrderType::Vwap,
    AutoOrderType::Twap,
];
fn context_learning_stats(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
) -> LearningStats {
    let prefix = format!("{context}:");
    learning
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, stats)| stats)
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        })
}

fn adaptive_scout_thresholds(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    active_scout_count: usize,
) -> (f64, f64) {
    let stats = context_learning_stats(learning, &format!("scout:{context}"));
    let win_rate = stats.positive as f64 / stats.samples.max(1) as f64;
    let density = active_scout_count.min(SCOUT_MAX_ACTIVE_POSITIONS) as f64;
    let mut strength =
        (SCOUT_MIN_STRENGTH - 0.15 + density * 0.025).clamp(SCOUT_MIN_STRENGTH_FLOOR, 0.25);
    let mut score =
        (SCOUT_MIN_ABS_SCORE - 0.15 + density * 0.05).clamp(SCOUT_MIN_SCORE_FLOOR, 0.40);
    if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES {
        if win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE {
            strength = (strength + 0.05).min(0.30);
            score = (score + 0.10).min(0.50);
        } else if win_rate >= 0.60 {
            strength = (strength - 0.02).max(SCOUT_MIN_STRENGTH_FLOOR);
            score = (score - 0.03).max(SCOUT_MIN_SCORE_FLOOR);
        }
    }
    (strength, score)
}

fn adaptive_strength_floor(learning: &BTreeMap<String, LearningStats>, context: &str) -> f64 {
    let stats = context_learning_stats(learning, context);
    if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES
        && stats.positive as f64 / (stats.samples as f64) < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE
    {
        LEARNED_STRENGTH_FLOOR
    } else {
        MIN_STRENGTH
    }
}

fn adaptive_exploration_score_floor(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    missed_opportunity_bands: &[MissedOpportunityGateBand],
) -> f64 {
    let stats = context_learning_stats(learning, context);
    let win_rate = stats.positive as f64 / stats.samples.max(1) as f64;
    let learned_floor = if stats.samples >= EXPLORATION_SCORE_HIGH_SAMPLES
        && win_rate < EXPLORATION_SCORE_HIGH_WIN_RATE
    {
        EXPLORATION_SCORE_HIGH_FLOOR
    } else if stats.samples >= EXPLORATION_SCORE_MID_SAMPLES
        && win_rate < EXPLORATION_SCORE_MID_WIN_RATE
    {
        EXPLORATION_SCORE_MID_FLOOR
    } else {
        MIN_EXPLORATION_SCORE
    };
    // Five-minute counterfactual stop/target outcomes may relax gates when they
    // are profitable. Longer terminal outcomes also inform the score floor when
    // mature cohorts show that otherwise-rejected signals need more time to turn
    // net positive. This changes entry selection, not live position exit timing.
    let short_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.stop_target_positive_net_rate.is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    let longer_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && matches!(band.horizon_seconds, 900 | 3_600)
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.positive_net_rate.is_some_and(|value| value >= 0.55)
                // Reject a later recovery if the path would already have hit
                // the configured stop before the longer horizon elapsed.
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.stop_target_positive_net_rate.is_some_and(|value| value >= 0.55)
                && band.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.time_to_profit_rate.is_some_and(|value| value >= 0.50)
                && band.average_time_to_profit_seconds.is_some_and(|value| {
                    value > 300.0 && value <= band.horizon_seconds as f64
                })
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    short_horizon_floor.min(longer_horizon_floor)
}

fn longer_horizon_score_multiplier(score: f64, bands: &[MissedOpportunityGateBand]) -> f64 {
    if !score.is_finite() {
        return 1.0;
    }
    let absolute_score = score.abs();
    let short_horizon = bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && band.horizon_seconds == 300
                && band.threshold <= absolute_score
        })
        .max_by(|left, right| left.threshold.total_cmp(&right.threshold));
    let Some(short) = short_horizon else {
        return 1.0;
    };
    let Some(short_average) = short.average_net_return_percent else {
        return 1.0;
    };
    let Some(short_positive_rate) = short.positive_net_rate else {
        return 1.0;
    };
    let supported_longer_horizon = bands.iter().any(|long| {
        long.gate == "score"
            && (long.threshold - short.threshold).abs() < 1e-9
            && matches!(long.horizon_seconds, 900 | 3_600)
            && long.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.average_net_return_percent.is_some_and(|average| {
                average >= short_average + 0.10 && average > 0.0
            })
            && long.positive_net_rate.is_some_and(|rate| {
                rate >= 0.70 && rate + 0.05 >= short_positive_rate
            })
            // The stop-target path model treats a stop breach as the exit,
            // even if the terminal mark later recovers above entry.
            && long.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
            && long.stop_target_positive_net_rate.is_some_and(|rate| rate >= 0.70)
            && long.time_to_profit_rate.is_some_and(|rate| rate >= 0.70)
            && long.average_time_to_profit_seconds.is_some_and(|seconds| {
                seconds > 300.0 && seconds <= long.horizon_seconds as f64
            })
    });
    if supported_longer_horizon { 1.5 } else { 1.0 }
}

fn adaptive_missed_opportunity_floor(
    learned_floor: f64,
    gate: &str,
    bands: &[MissedOpportunityGateBand],
) -> f64 {
    bands
        .iter()
        .filter(|band| {
            band.gate == gate
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor)
}
fn order_type_learning_stats(
    context: &str,
    kind: AutoOrderType,
    learning: &BTreeMap<String, LearningStats>,
) -> LearningStats {
    let prefix = format!("{context}:P");
    let suffix = format!(":{kind:?}");
    learning
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(&suffix))
        .map(|(_, stats)| stats)
        .fold(
            learning
                .get(&format!("{context}:{kind:?}"))
                .cloned()
                .unwrap_or_default(),
            |mut total, item| {
                total.samples += item.samples;
                total.positive += item.positive;
                total.realized_pnl += item.realized_pnl;
                total
            },
        )
}

fn adapt_order_type(
    baseline: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, LearningStats>,
) -> Option<AutoOrderType> {
    // A post-only entry is intentionally allowed as a cold-start experiment in
    // calm, costly-to-aggress conditions. Do not let the default IOC fallback
    // erase that signal before the maker cohort has enough outcomes to learn.
    if matches!(baseline, AutoOrderType::PostOnlyLimit) {
        let maker_stats =
            order_type_learning_stats(context, AutoOrderType::PostOnlyLimit, learning);
        if maker_stats.samples < MIN_EXECUTION_LEARNING_SAMPLES {
            return Some(AutoOrderType::PostOnlyLimit);
        }
    }
    let candidates = ALL_ORDER_TYPES
        .iter()
        .copied()
        .filter_map(|kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            let average_pnl = stats.realized_pnl / stats.samples.max(1) as f64;
            let positive_rate = stats.positive as f64 / stats.samples.max(1) as f64;
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && average_pnl > 0.0
                && positive_rate >= 0.55)
                .then_some((kind, average_pnl))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        if matches!(baseline, AutoOrderType::PostOnlyLimit) {
            let maker_stats =
                order_type_learning_stats(context, AutoOrderType::PostOnlyLimit, learning);
            if maker_stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && (maker_stats.realized_pnl / maker_stats.samples as f64 <= 0.0
                    || maker_stats.positive as f64 / (maker_stats.samples as f64) < 0.55)
            {
                return Some(AutoOrderType::Ioc);
            }
        }
        // Falling back to IOC must not bypass a mature losing record for the
        // fallback order type when the signal's preferred type is still unknown.
        let has_mature_unprofitable_fallback =
            [baseline, AutoOrderType::Ioc].into_iter().any(|kind| {
                let stats = order_type_learning_stats(context, kind, learning);
                stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                    && (stats.realized_pnl / stats.samples.max(1) as f64 <= 0.0
                        || stats.positive as f64 / (stats.samples.max(1) as f64) < 0.55)
            });
        return if has_mature_unprofitable_fallback {
            None
        } else {
            Some(AutoOrderType::Ioc)
        };
    }
    candidates
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind)
}

fn choose_parameter_profile(
    kind: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, LearningStats>,
    explore: bool,
) -> usize {
    let profiles = (0..PARAMETER_PROFILE_COUNT)
        .map(|profile| {
            let stats = learning
                .get(&format!("{context}:P{profile}:{kind:?}"))
                .cloned()
                .unwrap_or_default();
            (profile, stats)
        })
        .collect::<Vec<_>>();
    let proven = profiles
        .iter()
        .filter(|(_, stats)| {
            stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && stats.realized_pnl > 0.0
                && stats.positive as f64 / stats.samples as f64 >= 0.55
        })
        .max_by(|a, b| {
            (a.1.realized_pnl / a.1.samples as f64)
                .total_cmp(&(b.1.realized_pnl / b.1.samples as f64))
        });
    if let Some((profile, _)) = proven {
        return *profile;
    }
    if explore {
        return profiles
            .iter()
            .min_by_key(|(_, stats)| stats.samples)
            .map(|(profile, _)| *profile)
            .unwrap_or(0);
    }
    0
}

fn choose_order_type(signal: &Signal) -> AutoOrderType {
    let ranging = signal.ranging_probability.unwrap_or(0.0);
    let volatility = signal.components.volatility_1m_percent.unwrap_or(0.0).abs();
    let trend_units = signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs();
    let momentum = signal.components.momentum_10s_percent.unwrap_or(0.0).abs();
    let mispricing = signal
        .components
        .mispricing_vs_vwap_bps
        .unwrap_or(0.0)
        .abs();
    let pressure = signal
        .components
        .pressure_without_price_move
        .unwrap_or(0.0)
        .abs();
    let aggressor_cost = signal.components.aggressor_cost_bps.unwrap_or(0.0).abs();
    if ranging >= ORDER_POST_ONLY_RANGE && aggressor_cost >= ORDER_POST_ONLY_AGGRESSOR_COST_BPS {
        AutoOrderType::PostOnlyLimit
    } else if ranging >= ORDER_GTD_RANGE
        && mispricing >= ORDER_GTD_MIN_MISPRICING_BPS
        && mispricing < ORDER_GTD_MAX_MISPRICING_BPS
        && aggressor_cost < ORDER_POST_ONLY_AGGRESSOR_COST_BPS
    {
        AutoOrderType::Gtd
    } else if ranging >= ORDER_LIMIT_RANGE && mispricing >= ORDER_GTD_MAX_MISPRICING_BPS {
        AutoOrderType::Limit
    } else if volatility >= ORDER_STOP_LIMIT_VOLATILITY_PERCENT
        && trend_units >= ORDER_STOP_LIMIT_TREND_UNITS
    {
        AutoOrderType::StopLimit
    } else if trend_units >= ORDER_STOP_MARKET_TREND_UNITS
        && signal.strength >= ORDER_STOP_MARKET_STRENGTH
    {
        AutoOrderType::StopMarket
    } else if pressure >= ORDER_MARKET_PRESSURE && signal.strength >= ORDER_MARKET_STRENGTH {
        AutoOrderType::Market
    } else if momentum >= ORDER_IOC_MOMENTUM_PERCENT && signal.strength >= ORDER_IOC_STRENGTH {
        AutoOrderType::Ioc
    } else if signal.strength >= ORDER_VWAP_STRENGTH {
        AutoOrderType::Vwap
    } else if ranging >= ORDER_TWAP_RANGE {
        AutoOrderType::Twap
    } else if volatility >= ORDER_TRAILING_VOLATILITY_PERCENT {
        AutoOrderType::TrailingStop
    } else if signal.strength >= ORDER_FOK_STRENGTH {
        AutoOrderType::Fok
    } else {
        AutoOrderType::Ioc
    }
}

#[derive(Clone, Copy)]
struct ExecutionParameters {
    vwap_duration_minutes: u64,
    vwap_participation_rate: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}
impl Default for ExecutionParameters {
    fn default() -> Self {
        Self {
            vwap_duration_minutes: default_vwap_duration(),
            vwap_participation_rate: default_vwap_participation() / 100.0,
            limit_offset_bps: default_limit_offset(),
            trigger_offset_bps: default_trigger_offset(),
            trailing_distance_percent: default_trailing_distance(),
        }
    }
}
fn parameter_profile(mut params: ExecutionParameters, profile: usize) -> ExecutionParameters {
    match profile {
        1 => {
            params.vwap_duration_minutes = (params.vwap_duration_minutes * 3 / 4).clamp(5, 390);
            params.vwap_participation_rate = (params.vwap_participation_rate * 1.25).min(0.50);
            params.limit_offset_bps = (params.limit_offset_bps - 2.0).max(-10.0);
            params.trigger_offset_bps = (params.trigger_offset_bps * 0.75).max(5.0);
            params.trailing_distance_percent = (params.trailing_distance_percent * 0.75).max(0.05);
        }
        2 => {
            params.vwap_duration_minutes = (params.vwap_duration_minutes * 3 / 2).clamp(5, 390);
            params.vwap_participation_rate = (params.vwap_participation_rate * 0.75).max(0.01);
            params.limit_offset_bps = (params.limit_offset_bps + 2.0).min(10.0);
            params.trigger_offset_bps = (params.trigger_offset_bps * 1.25).min(100.0);
            params.trailing_distance_percent = (params.trailing_distance_percent * 1.25).min(1.0);
        }
        _ => {}
    }
    params
}

fn offset_limit(side: &str, bid: f64, ask: f64, offset_bps: f64) -> f64 {
    let reference = if side == "buy" { bid } else { ask };
    reference
        * (1.0
            + (if side == "buy" {
                -offset_bps
            } else {
                offset_bps
            }) / 10_000.0)
}

async fn submit_configured_order(
    client: &reqwest::Client,
    base: &str,
    symbol: &str,
    side: &str,
    quantity: f64,
    notional: f64,
    bid: f64,
    ask: f64,
    kind: AutoOrderType,
    params: ExecutionParameters,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut body = serde_json::json!({ "symbol": symbol, "side": side, "quantity": quantity, "measure_execution_quality": true });
    let endpoint = match kind {
        AutoOrderType::Vwap | AutoOrderType::Twap => {
            body["duration_minutes"] = serde_json::json!(params.vwap_duration_minutes);
            if matches!(kind, AutoOrderType::Vwap) {
                body["max_participation_rate"] = serde_json::json!(params.vwap_participation_rate);
            }
            if matches!(kind, AutoOrderType::Twap) {
                body["strategy"] = serde_json::json!("twap");
            }
            "/positions/vwap"
        }
        _ => {
            body["order_type"] = serde_json::json!(match kind {
                AutoOrderType::Market => "market",
                AutoOrderType::Ioc => "ioc",
                AutoOrderType::Fok => "fok",
                AutoOrderType::Limit | AutoOrderType::Gtd => "limit",
                AutoOrderType::PostOnlyLimit => "post-only-limit",
                AutoOrderType::Barrier => "barrier",
                AutoOrderType::StopMarket => "stop-market",
                AutoOrderType::StopLimit => "stop-limit",
                AutoOrderType::TrailingStop => "trailing-stop",
                _ => unreachable!(),
            });
            match kind {
                AutoOrderType::Limit => {
                    body["limit_price"] =
                        serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps))
                }
                AutoOrderType::Gtd => {
                    body["limit_price"] =
                        serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps));
                    body["time_in_force"] = serde_json::json!("gtd");
                    body["expires_at"] = serde_json::json!(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                            + 300_000
                    );
                }
                AutoOrderType::PostOnlyLimit => {
                    // Rest at the touch so a maker order has a realistic chance
                    // of being hit. The normal limit offset moves buys below
                    // the bid and sells above the ask.
                    body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask });
                    body["expires_at"] = serde_json::json!(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                            + 300_000
                    );
                }
                AutoOrderType::Barrier | AutoOrderType::StopMarket => {
                    body["trigger_price"] = serde_json::json!(if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    });
                    body["trigger_direction"] =
                        serde_json::json!(if side == "buy" { "above" } else { "below" });
                }
                AutoOrderType::StopLimit => {
                    let trigger = if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    };
                    body["trigger_price"] = serde_json::json!(trigger);
                    body["trigger_direction"] =
                        serde_json::json!(if side == "buy" { "above" } else { "below" });
                    body["limit_price"] = serde_json::json!(if side == "buy" {
                        trigger * (1.0 + params.limit_offset_bps / 10_000.0)
                    } else {
                        trigger * (1.0 - params.limit_offset_bps / 10_000.0)
                    });
                }
                AutoOrderType::TrailingStop => {
                    body["trailing_percent"] =
                        serde_json::json!(params.trailing_distance_percent / 100.0)
                }
                _ => {}
            }
            "/positions/orders"
        }
    };
    let response = client
        .post(format!("{base}{endpoint}"))
        .json(&body)
        .send()
        .await
        .map_err(upstream_error)?;
    let status = response.status();
    let position_manager_ms = response
        .headers()
        .get("x-position-manager-handler-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok());
    let response_body = response.bytes().await.map_err(upstream_error)?;
    let mut payload = serde_json::from_slice::<serde_json::Value>(&response_body).unwrap_or_else(|_| {
        serde_json::json!({"error": String::from_utf8_lossy(&response_body).trim()})
    });
    if let Some(handler_ms) = position_manager_ms {
        payload["position_manager_handler_ms"] = serde_json::json!(handler_ms);
    }
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Configured paper order rejected ({status}): {payload}"),
        ));
    }
    if !payload.is_object() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Position Manager returned an invalid order response: {payload}"),
        ));
    }
    let _ = notional;
    Ok(payload)
}

fn adapt_exit_policy(settings: &mut Settings) {
    let samples = settings.closed_trade_returns_percent.len();
    if samples < MIN_EXIT_POLICY_SAMPLES || samples % 5 != 0 {
        return;
    }
    let recent = &settings.closed_trade_returns_percent[samples.saturating_sub(20)..];
    let average = recent.iter().sum::<f64>() / recent.len() as f64;
    let positive_rate =
        recent.iter().filter(|value| **value > 0.0).count() as f64 / recent.len() as f64;
    // Poor win rate or negative expectancy is an entry-quality problem. Cutting
    // the target in response shrinks winners while leaving the stop fixed, making
    // the required break-even win rate even higher. Learn exits upward only after
    // recent outcomes are profitable, and preserve at least a 1.5:1 reward/risk.
    if average > 0.0 && positive_rate >= 0.60 {
        settings.take_profit_percent = (settings.take_profit_percent + EXIT_POLICY_ADJUSTMENT_STEP)
            .min(MAX_TAKE_PROFIT_PERCENT);
    }
    settings.stop_loss_percent = settings
        .stop_loss_percent
        .clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    let reward_risk_floor =
        (settings.stop_loss_percent * MIN_EXIT_REWARD_RISK_RATIO).min(MAX_TAKE_PROFIT_PERCENT);
    settings.take_profit_percent = settings
        .take_profit_percent
        .max(reward_risk_floor)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT);
}

fn symbol_gate_hits_for_reason(reason: &str) -> Vec<(String, String, String, String, Option<f64>)> {
    let mut hits = BTreeMap::<(String, String, String), String>::new();
    if let Some(details) = reason
        .split("; early filters: ")
        .nth(1)
        .and_then(|part| part.split("; candidate checks: ").next())
    {
        for filter in details.split(" | ") {
            let Some((identity, gate_details)) = filter.split_once(" · ") else {
                continue;
            };
            let mut identity = identity.split_whitespace();
            let (Some(symbol), Some(direction)) = (identity.next(), identity.next()) else {
                continue;
            };
            let gate = if gate_details.starts_with("Signal freshness") {
                Some("Signal freshness")
            } else if gate_details.starts_with("Directional signal") {
                Some("Directional signal")
            } else if gate_details.starts_with("Existing position") {
                Some("Existing position / re-entry limit")
            } else {
                None
            };
            if let Some(gate) = gate {
                hits.insert(
                    (symbol.to_owned(), direction.to_owned(), gate.to_owned()),
                    gate_details.to_owned(),
                );
            }
        }
    }
    if let Some(details) = reason.split("; candidate checks: ").nth(1) {
        for candidate in details.split(" | ") {
            let Some((identity, checks)) = candidate.rsplit_once(" · ") else {
                continue;
            };
            let mut identity = identity.split_whitespace();
            let (Some(symbol), Some(direction)) = (identity.next(), identity.next()) else {
                continue;
            };
            let mut push = |gate: &str, detail: String| {
                hits.insert(
                    (symbol.to_owned(), direction.to_owned(), gate.to_owned()),
                    detail,
                );
            };
            if checks.contains("strength ") && checks.contains(" < ") {
                push(
                    "Learned strength floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("strength "))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("score ") && checks.contains(" < ") {
                push(
                    "Exploration score floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("score "))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("exploration blocked for mixed context") {
                push(
                    "Exploration disabled for mixed context",
                    format!("{candidate}; failed: mixed market context disables exploration"),
                );
            }
            if checks.contains("exploration needs") {
                push(
                    "Exploration walk-forward return and positive-rate floor",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("exploration needs"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("normal entry needs") && checks.contains("side samples") {
                push(
                    "Strict walk-forward sample minimum",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("normal entry needs")
                                && check.contains("side samples"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("normal entry needs") && !checks.contains("side samples") {
                push(
                    "Strict walk-forward quality thresholds",
                    format!(
                        "{candidate}; failed: {}",
                        checks
                            .split(", ")
                            .find(|check| check.starts_with("normal entry needs")
                                && !check.contains("side samples"))
                            .unwrap_or(checks)
                    ),
                );
            }
            if checks.contains("research-model side return") {
                push(
                    "Research-model side evidence",
                    format!("{candidate}; failed: research-model side return is not positive"),
                );
            }
        }
    }
    hits.into_iter()
        .map(|((symbol, direction, gate), detail)| {
            let distance = gate_distance_to_pass(&gate, &detail);
            (symbol, direction, gate, detail, distance)
        })
        .collect()
}

fn gate_distance_to_pass(gate: &str, detail: &str) -> Option<f64> {
    fn number(token: &str) -> Option<f64> {
        let cleaned = token.trim_matches(|character: char| {
            !character.is_ascii_digit() && character != '.' && character != '-'
        });
        cleaned.parse().ok()
    }
    fn number_at_end(text: &str) -> Option<f64> {
        text.split_whitespace().rev().find_map(number)
    }
    fn number_at_start(text: &str) -> Option<f64> {
        text.split_whitespace().find_map(number)
    }
    fn lower_bound_gap(actual: f64, threshold: f64) -> f64 {
        if threshold == 0.0 {
            actual.abs()
        } else {
            ((threshold - actual) / threshold.abs()).max(0.0)
        }
    }
    fn metric(text: &str, name: &str, section: Option<&str>) -> Option<f64> {
        let scoped = match section {
            Some(section) => text.split_once(section)?.1,
            None => text,
        };
        let value = scoped.split_once(name)?.1.split_whitespace().next()?;
        if value == "n/a" { None } else { number(value) }
    }
    fn positive_rate_gap(detail: &str, threshold: f64) -> Option<f64> {
        let actual_percent = metric(detail, "positive=", None)?;
        Some(lower_bound_gap(actual_percent / 100.0, threshold))
    }
    fn return_to_zero_gap(detail: &str, section: Option<&str>) -> Option<f64> {
        let actual_percent = metric(detail, "net=", section)?;
        // Return values are reported in percent; one percentage point is the normalization unit at a zero threshold.
        Some((-actual_percent).max(0.0))
    }

    let mut gaps = Vec::new();
    if let Some(failed) = detail.split("failed: ").nth(1) {
        if let Some((actual_text, threshold_text)) = failed.split_once(" < ") {
            if let (Some(actual), Some(threshold)) =
                (number_at_end(actual_text), number_at_start(threshold_text))
            {
                gaps.push(lower_bound_gap(actual, threshold));
            }
        }
        let mut parts = failed.split(" > ");
        if let (Some(actual), Some(threshold)) = (
            parts.next().and_then(number_at_end),
            parts.next().and_then(number_at_start),
        ) {
            gaps.push(if threshold == 0.0 {
                (actual - threshold).max(0.0)
            } else {
                ((actual - threshold) / threshold.abs()).max(0.0)
            });
        }
    }
    if gate == "Signal freshness" {
        if let Some((age, max_age)) = detail
            .split_once("age ")
            .and_then(|(_, tail)| tail.split_once("s > max age "))
        {
            if let (Some(actual), Some(threshold)) =
                (number_at_start(age), number_at_start(max_age))
            {
                gaps.push(if threshold == 0.0 {
                    1.0
                } else {
                    ((actual - threshold) / threshold).max(0.0)
                });
            }
        }
    }
    if gate == "Directional signal" {
        if let Some((score, floor)) = detail.split_once("vs floor ") {
            if let (Some(actual), Some(threshold)) = (number_at_end(score), number_at_start(floor))
            {
                gaps.push(lower_bound_gap(actual, threshold));
            }
        }
        if let Some((probability, threshold)) = detail
            .split_once("range probability ")
            .and_then(|(_, tail)| tail.split_once(" must be below "))
        {
            if let (Some(actual), Some(limit)) =
                (number_at_start(probability), number_at_start(threshold))
            {
                if limit != 0.0 {
                    gaps.push(((actual - limit) / limit.abs()).max(0.0));
                }
            }
        }
    }
    if gate == "Strict walk-forward sample minimum" {
        if let (Some(actual), Some(required)) = (
            detail.split("n=").nth(1).and_then(number_at_start),
            detail.split("needs ").nth(1).and_then(number_at_start),
        ) {
            if required != 0.0 {
                gaps.push(((required - actual) / required).max(0.0));
            }
        }
    }
    if gate == "Strict walk-forward quality thresholds" {
        if let Some(hit_percent) = metric(detail, "hit=", None) {
            gaps.push(lower_bound_gap(
                hit_percent / 100.0,
                MIN_SIDE_DIRECTION_MATCH_RATE,
            ));
        }
        if let Some(return_gap) = return_to_zero_gap(detail, None) {
            gaps.push(return_gap);
        }
        if let Some(gap) = positive_rate_gap(detail, MIN_SIDE_POSITIVE_NET_RATE) {
            gaps.push(gap);
        }
        if let Some(lower_percent) = metric(detail, "lower95=", None) {
            gaps.push(lower_bound_gap(
                lower_percent / 100.0,
                MIN_SIDE_POSITIVE_NET_LOWER_95,
            ));
        }
    }
    if gate == "Exploration walk-forward return and positive-rate floor" {
        if let Some(return_gap) = return_to_zero_gap(detail, None) {
            gaps.push(return_gap);
        }
        if let Some(gap) = positive_rate_gap(detail, NEGATIVE_EVIDENCE_FLOOR) {
            gaps.push(gap);
        }
    }
    if gate == "Research-model side evidence" {
        if let Some(return_gap) = return_to_zero_gap(detail, Some("research-model ")) {
            gaps.push(return_gap);
        }
    }
    if gate == "Existing position / re-entry limit" {
        if let Some((entries, max_entries)) = detail
            .split_once("entries ")
            .and_then(|(_, tail)| tail.split_once(" >= max "))
        {
            if let (Some(actual), Some(limit)) =
                (number_at_start(entries), number_at_start(max_entries))
            {
                if limit != 0.0 {
                    gaps.push(((actual - limit) / limit).max(0.0));
                }
            }
        }
    }
    if gate == "Execution cost versus measured edge" {
        if let (Some(cost), Some(edge)) = (
            detail
                .split_once("Estimated spread and sweep cost (")
                .and_then(|(_, tail)| tail.split_once("%) consumes"))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once("measured walk-forward edge (")
                .and_then(|(_, tail)| tail.split_once("%)"))
                .and_then(|(value, _)| number(value)),
        ) {
            let scale = edge.abs().max(0.001);
            gaps.push(((cost - edge) / scale).max(0.0));
        }
    }
    if gate == "Deep-book minimum order capacity" {
        if let (Some(capacity), Some(minimum)) = (
            detail
                .split_once("Deep-book capacity for ")
                .and_then(|(_, tail)| tail.split_once(" ("))
                .and_then(|(_, tail)| tail.split_once(") is below"))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once("minimum paper order (")
                .and_then(|(_, tail)| tail.split_once(')'))
                .and_then(|(value, _)| number(value)),
        ) {
            gaps.push(lower_bound_gap(capacity, minimum));
        }
    }
    if gate == "Deep-book impact limit" {
        if let (Some(impact), Some(limit)) = (
            detail
                .split_once("estimated impact ")
                .and_then(|(_, tail)| tail.split_once(" bps exceeds "))
                .and_then(|(value, _)| number(value)),
            detail
                .split_once(" bps exceeds ")
                .and_then(|(_, tail)| number_at_start(tail)),
        ) {
            gaps.push(if limit == 0.0 {
                impact.max(0.0)
            } else {
                ((impact - limit) / limit.abs()).max(0.0)
            });
        }
    }
    gaps.into_iter()
        .filter(|gap| gap.is_finite())
        .reduce(f64::max)
}

fn gate_hits_for_reason(reason: &str) -> Vec<String> {
    let lower = reason.to_ascii_lowercase();
    let mut gates = Vec::new();
    let count_after = |marker: &str| {
        lower
            .split(marker)
            .nth(1)
            .and_then(|tail| tail.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default()
    };
    let mut add = |condition: bool, label: &str| {
        if condition && !gates.iter().any(|existing| existing == label) {
            gates.push(label.to_owned());
        }
    };
    if lower.starts_with("no tradeable bias") {
        add(count_after("filtered: ") > 0, "Signal freshness");
        add(count_after("stale, ") > 0, "Directional signal");
        add(
            count_after("non-directional, ") > 0,
            "Existing position / re-entry limit",
        );
        add(
            lower.contains("strength ") && lower.contains(" < "),
            "Learned strength floor",
        );
        add(
            lower.contains("score ") && lower.contains(" < "),
            "Exploration score floor",
        );
        add(
            lower.contains("exploration blocked for mixed context"),
            "Exploration disabled for mixed context",
        );
        add(
            lower.contains("exploration needs"),
            "Exploration walk-forward return and positive-rate floor",
        );
        add(
            lower.contains("normal entry needs") && lower.contains("side samples"),
            "Strict walk-forward sample minimum",
        );
        add(
            lower.contains("normal entry needs") && !lower.contains("side samples"),
            "Strict walk-forward quality thresholds",
        );
        add(
            lower.contains("research-model side return"),
            "Research-model side evidence",
        );
        let blocked_by_negative = lower
            .split("candidate(s) blocked by negative walk-forward history")
            .next()
            .and_then(|prefix| prefix.split_whitespace().last())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        add(blocked_by_negative > 0, "Negative walk-forward evidence");
        if gates.is_empty() {
            if count_after("fresh directional ") == 0 {
                gates.push("No fresh directional candidate".into());
            } else {
                gates.push("Candidate selection / combined entry checks".into());
            }
        }
    } else if lower.contains("daily realized-loss limit") {
        gates.push("Daily realized-loss limit".into());
    } else if lower.contains("invalid instrument symbol") {
        gates.push("Instrument symbol validation".into());
    } else if lower.contains("no cached local order book")
        || lower.contains("no current order book")
    {
        gates.push("Local order book availability".into());
    } else if lower.contains("stale local order book") {
        gates.push("Local order book freshness".into());
    } else if lower.contains("post-stop cooldown")
        || lower.contains("re-entry cooldown")
        || lower.contains("re-entry blocked")
        || lower.contains("opposite signal")
        || lower.contains("add-on limit")
        || lower.contains("open-position cap")
    {
        gates.push("Position / re-entry / cooldown".into());
    } else if lower.contains("notional capacity") || lower.contains("capacity is fully allocated") {
        gates.push("Available notional capacity".into());
    } else if lower.contains("daily notional")
        || lower.contains("budget")
        || lower.contains("capacity")
    {
        gates.push("Available budget / position capacity".into());
    } else if lower.contains("no profitable order type") {
        gates.push("Learned order-type profitability".into());
    } else if lower.contains("spread and sweep cost") {
        gates.push("Execution cost versus measured edge".into());
    } else if lower.contains("deep book cannot support") {
        gates.push("Deep-book impact limit".into());
    } else if lower.contains("deep-book capacity") {
        gates.push("Deep-book minimum order capacity".into());
    } else if lower.contains("walk-forward") || lower.contains("negative") {
        gates.push("Walk-forward history".into());
    } else if lower.contains("cycle error") {
        gates.push("Cycle error".into());
    } else {
        gates.push(gap_reason_category(reason));
    }
    gates
}

fn gap_reason_category(reason: &str) -> String {
    let lower = reason.to_ascii_lowercase();
    if lower.starts_with("no tradeable bias") {
        let counts = lower
            .split("fresh directional ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .unwrap_or("0/0");
        let mut counts = counts
            .split('/')
            .filter_map(|value| value.parse::<usize>().ok());
        let usable = counts.next().unwrap_or_default();
        let total = counts.next().unwrap_or_default();
        if total == 0 {
            return "No insights available".into();
        }
        if usable == 0 {
            let filtered_count = |marker: &str| {
                lower
                    .split(marker)
                    .nth(1)
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default()
            };
            let stale = filtered_count("filtered: ");
            let no_direction = filtered_count("non-directional, ");
            let position = filtered_count("blocked by existing positions");
            if position > 0 {
                return "Existing position blocked signals".into();
            }
            if no_direction > 0 {
                return "No directional signal".into();
            }
            if stale > 0 {
                return "Signals were stale".into();
            }
            return "No usable directional signal".into();
        }
        let strength = lower
            .split("contextual learned strength floors passed ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        let exploration = lower
            .split("contextual raw-score floors passed ")
            .nth(1)
            .and_then(|value| value.split_whitespace().next())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        if strength == 0 && exploration == 0 {
            return "Signal too weak for entry".into();
        }
        let blocked = lower
            .split("candidate(s) blocked by negative walk-forward history")
            .next()
            .and_then(|prefix| prefix.split_whitespace().last())
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or_default();
        if blocked > 0 {
            return "Walk-forward history blocked candidates".into();
        }
        return "No candidate passed all entry checks".into();
    }
    if lower.contains("daily realized-loss limit") {
        return "Daily realized-loss limit reached".into();
    }
    if lower.contains("daily notional")
        || lower.contains("budget")
        || lower.contains("capacity is fully allocated")
    {
        return "Available trading budget or position capacity was insufficient".into();
    }
    if lower.contains("walk-forward") || lower.contains("negative") {
        return "Historical walk-forward evidence did not qualify".into();
    }
    if lower.contains("stale local order book") {
        return "Local order book data was stale".into();
    }
    if lower.contains("no cached local order book") {
        return "No cached local order book was available".into();
    }
    if lower.contains("open position") || lower.contains("already") {
        return "Existing position or cooldown blocked another entry".into();
    }
    if lower.starts_with("cycle error:") {
        return "Cycle error".into();
    }
    reason
        .split(|character| character == ':' || character == ';' || character == '·')
        .next()
        .unwrap_or(reason)
        .trim()
        .to_owned()
}

fn skipped_for_signal(signal: &Signal, reason: &str) -> CycleResult {
    let mut cycle = skipped(reason);
    cycle.symbol = Some(signal.symbol.clone());
    cycle.side = Some(signal.direction.clone());
    cycle
}
fn skipped(reason: &str) -> CycleResult {
    CycleResult {
        status: "skipped",
        reason: Some(reason.into()),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
    }
}
fn upstream_error(error: reqwest::Error) -> (StatusCode, String) {
    (
        StatusCode::BAD_GATEWAY,
        format!("Auto trader upstream request failed: {error}"),
    )
}
fn settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AUTO_TRADER_STATE_FILE") {
        return PathBuf::from(path);
    }
    std::env::var_os("POSITIONS_FILE")
        .map(PathBuf::from)
        .and_then(|path| {
            path.parent()
                .map(|parent| parent.join("auto_trader_state.json"))
        })
        .unwrap_or_else(|| PathBuf::from("auto_trader_state.json"))
}
fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), (StatusCode, String)> {
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|_| std::fs::rename(&temporary, path))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Could not save Auto Trader settings: {error}"),
            )
        })
}
fn utc_day() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    days.to_string()
}
fn allocated_notional(settings: &Settings) -> f64 {
    let open_positions = settings
        .positions
        .values()
        .map(|position| position.quantity.abs() * position.average_entry_price.max(0.0))
        .sum::<f64>();
    let mut pending = BTreeMap::<String, f64>::new();
    let mut pending_quantity = BTreeMap::<String, f64>::new();
    for order in &settings.orders {
        if !matches!(order.status.as_str(), "submitting" | "accepted" | "resting")
            || order.quantity <= 0.0
        {
            continue;
        }
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let current_quantity = settings
            .positions
            .get(&order.symbol)
            .map(|position| position.quantity)
            .unwrap_or(0.0)
            + pending_quantity.get(&order.symbol).copied().unwrap_or(0.0);
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let increase_quantity = if current_quantity == 0.0 || current_quantity.signum() == direction
        {
            remaining
        } else {
            (remaining - current_quantity.abs()).max(0.0)
        };
        let reference_notional = if order.reserved_notional > 0.0 {
            order.reserved_notional
        } else {
            order.average_price.unwrap_or(0.0) * order.quantity
        };
        *pending.entry(order.symbol.clone()).or_default() +=
            reference_notional * increase_quantity / order.quantity;
        *pending_quantity.entry(order.symbol.clone()).or_default() += direction * remaining;
    }
    open_positions + pending.values().sum::<f64>()
}
fn reset_daily_budget(settings: &mut Settings) {
    let day = utc_day();
    if settings.utc_day != day {
        settings.utc_day = day.clone();
    }
    settings.used_notional_today = allocated_notional(settings);
    if settings.pnl_utc_day != day {
        settings.pnl_utc_day = day;
        settings.daily_realized_pnl = 0.0;
        settings.daily_loss_limit_reduced = false;
    }
}
