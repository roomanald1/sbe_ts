//! Signal-driven intraday paper trader with bounded, configurable order sizing.
use axum::{extract::State, http::StatusCode, routing::{get, post}, Json, Router};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}, time::{SystemTime, UNIX_EPOCH}};
use tokio::{sync::Mutex, time::{Duration, interval}};

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
const MIN_SIDE_DIRECTION_MATCH_RATE: f64 = 0.55;
const MIN_SIDE_POSITIVE_NET_RATE: f64 = 0.55;
const MIN_SIDE_POSITIVE_NET_LOWER_95: f64 = 0.45;
const SIGNAL_COOLDOWN_MS: u64 = 30_000;
const POST_STOP_REENTRY_COOLDOWN_MS: u64 = 120_000;
const EXPLORATION_NOTIONAL_FRACTION: f64 = 0.10;
const MIN_EXPLORATION_EVIDENCE_SAMPLES: usize = 10;
const MAX_ENTRIES_PER_AUTO_POSITION: u32 = 1;
const NEGATIVE_EVIDENCE_FLOOR: f64 = 0.45;
const MATERIAL_STRENGTH_CHANGE: f64 = 0.02;
const MIN_EXECUTION_LEARNING_SAMPLES: usize = 10;
const DEFAULT_DAILY_REALIZED_LOSS_LIMIT: f64 = 10.0;
const MIN_DAILY_REALIZED_LOSS_LIMIT: f64 = 5.0;
const MIN_STOP_LOSS_PERCENT: f64 = 0.25;
const MAX_STOP_LOSS_PERCENT: f64 = 1.50;
const MIN_TAKE_PROFIT_PERCENT: f64 = 0.40;
const MAX_TAKE_PROFIT_PERCENT: f64 = 2.00;
const EXIT_POLICY_ADJUSTMENT_STEP: f64 = 0.05;
const MIN_EXIT_POLICY_SAMPLES: usize = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Dashboard { best: Vec<Signal>, walk_forward: Vec<WalkForward> }
#[derive(Deserialize)]
struct Signal { symbol: String, direction: String, strength: f64, timestamp: u64, stale: bool,
    #[serde(default)] score: f64,
    #[serde(default)] ranging_probability: Option<f64>,
    #[serde(default)] components: SignalComponents,
}
#[derive(Default, Deserialize)]
struct SignalComponents {
    momentum_10s_percent: Option<f64>, trend_5m_volatility_units: Option<f64>,
    volatility_1m_percent: Option<f64>, mispricing_vs_vwap_bps: Option<f64>,
    pressure_without_price_move: Option<f64>, aggressor_cost_bps: Option<f64>,
}
#[derive(Deserialize)]
struct WalkForward {
    symbol: String, horizon_seconds: u64,
    #[serde(default)] all_signal_long: SideWalkForwardMetrics,
    #[serde(default)] all_signal_short: SideWalkForwardMetrics,
}
#[derive(Default, Deserialize)]
struct SideWalkForwardMetrics {
    sample_count: usize,
    direction_match_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
    positive_net_rate: Option<f64>,
    positive_net_rate_lower_95: Option<f64>,
}

#[derive(Deserialize)]
struct OkxBooksResponse { code: String, msg: String, data: Vec<OkxBook> }
#[derive(Deserialize)]
struct OkxBook { bids: Vec<Vec<String>>, asks: Vec<Vec<String>> }
#[derive(Clone, Copy)]
struct DeepBookAssessment {
    available_within_band: f64,
    notional_cap: f64,
    impact_bps: f64,
    bid_depth_10bps: f64,
    ask_depth_10bps: f64,
    depth_imbalance_10bps: f64,
}

fn quote_depth_within_band(levels: &[Vec<String>], mid: f64, is_bid: bool) -> f64 {
    let band_price = if is_bid { mid * (1.0 - DEEP_BOOK_BAND_BPS / 10_000.0) } else { mid * (1.0 + DEEP_BOOK_BAND_BPS / 10_000.0) };
    levels.iter().filter_map(|row| {
        let price = row.first()?.parse::<f64>().ok()?;
        let size = row.get(1)?.parse::<f64>().ok()?;
        (price.is_finite() && size.is_finite() && price > 0.0 && size > 0.0 && if is_bid { price >= band_price } else { price <= band_price }).then_some(price * size)
    }).sum()
}

async fn assess_deep_book(client: &reqwest::Client, symbol: &str, is_buy: bool, target_notional: f64) -> Result<Option<DeepBookAssessment>, String> {
    let quote = symbol.rsplit('-').next().unwrap_or_default();
    if !matches!(quote, "USDT" | "USDC") { return Ok(None); }
    let response = client.get(format!("https://www.okx.com/api/v5/market/books?instId={symbol}&sz=50")).send().await
        .map_err(|error| format!("deep-book request failed: {error}"))?
        .error_for_status().map_err(|error| format!("deep-book HTTP error: {error}"))?;
    let response: OkxBooksResponse = response.json().await.map_err(|error| format!("deep-book response could not be decoded: {error}"))?;
    if response.code != "0" { return Err(format!("OKX deep-book error: {}", response.msg)); }
    let book = response.data.first().ok_or_else(|| "OKX returned no deep-book snapshot".to_owned())?;
    let best_bid = book.bids.first().and_then(|row| row.first()).and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value > 0.0).ok_or_else(|| "deep book has no valid bid".to_owned())?;
    let best_ask = book.asks.first().and_then(|row| row.first()).and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value >= best_bid).ok_or_else(|| "deep book has no valid ask".to_owned())?;
    let mid = (best_bid + best_ask) / 2.0;
    let bid_depth_10bps = quote_depth_within_band(&book.bids, mid, true);
    let ask_depth_10bps = quote_depth_within_band(&book.asks, mid, false);
    let total_depth = bid_depth_10bps + ask_depth_10bps;
    let depth_imbalance_10bps = if total_depth > 0.0 { (bid_depth_10bps - ask_depth_10bps) / total_depth } else { 0.0 };
    let levels = if is_buy { &book.asks } else { &book.bids };
    let available_within_band = if is_buy { ask_depth_10bps } else { bid_depth_10bps };
    let notional_cap = available_within_band * DEEP_BOOK_MAX_PARTICIPATION;
    let planned_notional = target_notional.min(notional_cap);
    if planned_notional < MIN_DEEP_BOOK_ORDER_NOTIONAL { return Ok(Some(DeepBookAssessment { available_within_band, notional_cap, impact_bps: f64::INFINITY, bid_depth_10bps, ask_depth_10bps, depth_imbalance_10bps })); }
    let mut remaining = planned_notional / mid;
    let mut filled = 0.0;
    let mut quote_value = 0.0;
    for row in levels {
        let (Some(price), Some(size)) = (row.first().and_then(|value| value.parse::<f64>().ok()), row.get(1).and_then(|value| value.parse::<f64>().ok())) else { continue; };
        if !price.is_finite() || !size.is_finite() || price <= 0.0 || size <= 0.0 { continue; }
        let quantity = remaining.min(size);
        filled += quantity;
        quote_value += quantity * price;
        remaining -= quantity;
        if remaining <= 1e-12 { break; }
    }
    let impact_bps = if remaining > 1e-12 || filled <= 0.0 { f64::INFINITY } else {
        let average = quote_value / filled;
        if is_buy { (average / mid - 1.0) * 10_000.0 } else { (1.0 - average / mid) * 10_000.0 }
    };
    Ok(Some(DeepBookAssessment { available_within_band, notional_cap, impact_bps, bid_depth_10bps, ask_depth_10bps, depth_imbalance_10bps }))
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum AutoOrderType { Market, Ioc, Fok, Limit, Gtd, PostOnlyLimit, Barrier, StopMarket, StopLimit, TrailingStop, Vwap, Twap }
impl Default for AutoOrderType { fn default() -> Self { Self::Ioc } }
#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default = "default_order_limit")] max_order_notional: f64,
    #[serde(default = "default_daily_limit")] daily_notional_limit: f64,
    #[serde(default)] used_notional_today: f64,
    #[serde(default)] realized_pnl: f64,
    #[serde(default = "default_daily_loss_limit")] daily_realized_loss_limit: f64,
    #[serde(default = "default_exploration_percent")] exploration_percent: f64,
    #[serde(default = "default_max_open_positions")] max_open_positions: usize,
    #[serde(default)] daily_loss_limit_reduced: bool,
    #[serde(default = "default_stop_loss_percent")] stop_loss_percent: f64,
    #[serde(default = "default_take_profit_percent")] take_profit_percent: f64,
    #[serde(default)] closed_trade_returns_percent: Vec<f64>,
    #[serde(default)] daily_realized_pnl: f64,
    #[serde(default)] pnl_utc_day: String,
    #[serde(default)] utc_day: String,
    #[serde(default)] orders: Vec<TraderOrder>,
    #[serde(default)] positions: BTreeMap<String, TraderPosition>,
    #[serde(default)] learning: BTreeMap<String, LearningStats>,
    #[serde(default)] last_hedge_at: BTreeMap<String, u64>,
    #[serde(default)] last_decision: String,
    #[serde(default)] last_decision_at: u64,
    #[serde(default)] last_risk_note: String,
    #[serde(default)] risk_limited_sizing: bool,
    #[serde(default)] last_signal_at: BTreeMap<String, u64>,
    #[serde(default)] last_signal_direction: BTreeMap<String, String>,
    #[serde(default)] last_signal_strength: BTreeMap<String, f64>,
}
fn default_order_limit() -> f64 { DEFAULT_MAX_ORDER_NOTIONAL }
fn default_daily_loss_limit() -> f64 { DEFAULT_DAILY_REALIZED_LOSS_LIMIT }
fn default_exploration_percent() -> f64 { EXPLORATION_NOTIONAL_FRACTION * 100.0 }
fn default_max_open_positions() -> usize { 20 }
fn default_stop_loss_percent() -> f64 { 0.50 }
fn default_take_profit_percent() -> f64 { 0.80 }
fn default_daily_limit() -> f64 { DEFAULT_DAILY_NOTIONAL }
impl Default for Settings {
    fn default() -> Self { Self { max_order_notional: default_order_limit(), daily_notional_limit: default_daily_limit(), used_notional_today: 0.0, realized_pnl: 0.0, daily_realized_loss_limit: default_daily_loss_limit(), exploration_percent: default_exploration_percent(), max_open_positions: default_max_open_positions(), daily_loss_limit_reduced: false, stop_loss_percent: default_stop_loss_percent(), take_profit_percent: default_take_profit_percent(), closed_trade_returns_percent: Vec::new(), daily_realized_pnl: 0.0, pnl_utc_day: utc_day(), utc_day: utc_day(), orders: Vec::new(), positions: BTreeMap::new(), learning: BTreeMap::new(), last_hedge_at: BTreeMap::new(), last_decision: String::new(), last_decision_at: 0, last_risk_note: String::new(), risk_limited_sizing: false, last_signal_at: BTreeMap::new(), last_signal_direction: BTreeMap::new(), last_signal_strength: BTreeMap::new() } }
}
#[derive(Serialize)]
struct StatusResponse { enabled: bool, max_order_notional: f64, daily_notional_limit: f64, daily_realized_loss_limit: f64, exploration_percent: f64, max_open_positions: usize, stop_loss_percent: f64, take_profit_percent: f64, used_notional_today: f64, last_decision: String, last_decision_at: u64 }
#[derive(Deserialize)]
struct SettingsRequest {
    max_order_notional: f64, daily_notional_limit: f64, daily_realized_loss_limit: f64,
    exploration_percent: f64, max_open_positions: usize,
    #[serde(default)] stop_loss_percent: Option<f64>,
    #[serde(default)] take_profit_percent: Option<f64>,
}
#[derive(Deserialize)]
struct EnabledRequest { enabled: bool }
#[derive(Clone)]
struct AutoTraderState { enabled: Arc<AtomicBool>, settings: Arc<Mutex<Settings>>, cycle_lock: Arc<Mutex<()>>, settings_path: PathBuf }

#[derive(Clone, Serialize, Deserialize)]
struct TraderOrder {
    timestamp: u64,
    #[serde(default)] triggered_at: Option<u64>,
    symbol: String,
    side: String,
    order_type: String,
    #[serde(default)] purpose: String,
    quantity: f64,
    filled_quantity: f64,
    average_price: Option<f64>,
    #[serde(default)] reserved_notional: f64,
    #[serde(default)] book_bid_depth_10bps: Option<f64>,
    #[serde(default)] book_ask_depth_10bps: Option<f64>,
    #[serde(default)] book_depth_imbalance_10bps: Option<f64>,
    #[serde(default)] estimated_sweep_bps: Option<f64>,
    status: String,
    realized_pnl: f64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct TraderPosition { quantity: f64, average_entry_price: f64, entry_fees: f64, realized_pnl: f64, #[serde(default)] entries: u32, #[serde(default)] entry_order_type: String, #[serde(default)] entry_context: String, #[serde(default)] protective_stops: Vec<ProtectiveStop> }
#[derive(Clone, Default, Serialize, Deserialize)]
struct ProtectiveStop { barrier_id: String, order_timestamp: u64, quantity: f64, processed_quantity: f64, processed_notional: f64, processed_fee: f64 }
#[derive(Clone, Default, Serialize, Deserialize)]
struct LearningStats { samples: usize, positive: usize, realized_pnl: f64 }
#[derive(Serialize)]
struct PnlBreakdown { symbol: String, direction: String, purpose: String, realized_pnl: f64, outcomes: usize }
#[derive(Serialize)]
struct ActivityResponse { orders: Vec<TraderOrder>, pnl_breakdown: Vec<PnlBreakdown>, realized_pnl: f64, unrealized_pnl: Option<f64>, exit_learning_samples: usize, learning_samples: usize, learning_win_rate: Option<f64>, preferred_order_type: Option<String> }
#[derive(Serialize)]
struct CycleResult { status: &'static str, reason: Option<String>, symbol: Option<String>, side: Option<String>, quantity: Option<f64>, order_type: Option<String>, order_response: Option<serde_json::Value> }

pub fn router() -> Router {
    let settings_path = settings_path();
    let stored_state = std::fs::read(&settings_path).ok().and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok());
    let mut settings = stored_state.as_ref().and_then(|value| serde_json::from_value::<Settings>(value.clone()).ok()).unwrap_or_default();
    // The previous release defaulted this new cap to three, which can strand existing books above the cap.
    if settings.max_open_positions == 3 { settings.max_open_positions = default_max_open_positions(); }
    if stored_state.as_ref().is_some_and(|value| value.get("realized_pnl").is_none()) {
        settings.realized_pnl = settings.positions.values().map(|position| position.realized_pnl).sum();
    }
    for position in settings.positions.values_mut() {
        if position.quantity.abs() > 1e-12 && position.entries == 0 { position.entries = 1; }
    }
    settings.used_notional_today = allocated_notional(&settings);
    if settings.utc_day != utc_day() { settings.utc_day = utc_day(); }
    let state = AutoTraderState {
        enabled: Arc::new(AtomicBool::new(std::env::var("AUTO_TRADER_ENABLED").as_deref() == Ok("true"))),
        settings: Arc::new(Mutex::new(settings)), cycle_lock: Arc::new(Mutex::new(())), settings_path,
    };
    let worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(5));
        loop { ticker.tick().await; if worker_state.enabled.load(Ordering::Relaxed) {
            if let Err(error) = run_cycle(&worker_state).await { eprintln!("Auto trader cycle failed: {}", error.1); }
        } }
    });
    Router::new()
        .route("/positions/auto-trader", get(get_status).put(set_enabled))
        .route("/positions/auto-trader/settings", axum::routing::put(update_settings))
        .route("/positions/auto-trader/run-once", post(run_once))
        .route("/positions/auto-trader/activity", get(get_activity))
        .with_state(state)
}

async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}
async fn set_enabled(State(state): State<AutoTraderState>, Json(request): Json<EnabledRequest>) -> Json<StatusResponse> {
    state.enabled.store(request.enabled, Ordering::Relaxed);
    let settings = state.settings.lock().await;
    Json(status(&state, &settings))
}
async fn update_settings(State(state): State<AutoTraderState>, Json(request): Json<SettingsRequest>) -> Result<Json<StatusResponse>, (StatusCode, String)> {
    if !request.max_order_notional.is_finite() || request.max_order_notional <= 0.0
        || !request.daily_notional_limit.is_finite() || request.daily_notional_limit < request.max_order_notional
        || !request.daily_realized_loss_limit.is_finite() || request.daily_realized_loss_limit <= 0.0
        || !request.exploration_percent.is_finite() || !(0.0..=100.0).contains(&request.exploration_percent)
        || request.stop_loss_percent.is_some_and(|value| !value.is_finite() || !(MIN_STOP_LOSS_PERCENT..=MAX_STOP_LOSS_PERCENT).contains(&value))
        || request.take_profit_percent.is_some_and(|value| !value.is_finite() || !(MIN_TAKE_PROFIT_PERCENT..=MAX_TAKE_PROFIT_PERCENT).contains(&value))
        || !(1..=20).contains(&request.max_open_positions) {
        return Err((StatusCode::BAD_REQUEST, "Order and total notional limits must be positive with total notional at least the per-order limit; daily loss cap must be positive, exploration must be between 0% and 100%, and max open positions must be between 1 and 20".into()));
    }
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if request.daily_notional_limit < settings.used_notional_today {
        return Err((StatusCode::CONFLICT, "Total notional limit cannot be less than the Auto Trader's current allocation".into()));
    }
    settings.max_order_notional = request.max_order_notional;
    settings.daily_notional_limit = request.daily_notional_limit;
    settings.daily_realized_loss_limit = request.daily_realized_loss_limit;
    settings.exploration_percent = request.exploration_percent;
    settings.max_open_positions = request.max_open_positions;
    if let Some(value) = request.stop_loss_percent { settings.stop_loss_percent = value; }
    if let Some(value) = request.take_profit_percent { settings.take_profit_percent = value; }
    persist_settings(&state.settings_path, &settings)?;
    Ok(Json(status(&state, &settings)))
}
fn status(state: &AutoTraderState, settings: &Settings) -> StatusResponse {
    StatusResponse { enabled: state.enabled.load(Ordering::Relaxed), max_order_notional: settings.max_order_notional,
        daily_notional_limit: settings.daily_notional_limit, daily_realized_loss_limit: settings.daily_realized_loss_limit, exploration_percent: settings.exploration_percent, max_open_positions: settings.max_open_positions, stop_loss_percent: settings.stop_loss_percent, take_profit_percent: settings.take_profit_percent, used_notional_today: settings.used_notional_today,
        last_decision: settings.last_decision.clone(), last_decision_at: settings.last_decision_at }
}
async fn get_activity(State(state): State<AutoTraderState>) -> Json<ActivityResponse> {
    let settings = state.settings.lock().await;
    let realized_pnl = settings.realized_pnl;
    let mut grouped_pnl = BTreeMap::<(String, String, String), (f64, usize)>::new();
    for order in &settings.orders {
        if order.realized_pnl.abs() > 1e-12 {
            let direction = if order.side.eq_ignore_ascii_case("sell") { "LONG" } else { "SHORT" };
            let key = (order.symbol.clone(), direction.to_owned(), if order.purpose.is_empty() { "other".to_owned() } else { order.purpose.clone() });
            let total = grouped_pnl.entry(key).or_default();
            total.0 += order.realized_pnl;
            total.1 += 1;
        }
    }
    let mut pnl_breakdown = grouped_pnl.into_iter().map(|((symbol, side, purpose), (realized_pnl, outcomes))| PnlBreakdown { symbol, direction: side, purpose, realized_pnl, outcomes }).collect::<Vec<_>>();
    pnl_breakdown.sort_by(|left, right| right.realized_pnl.abs().total_cmp(&left.realized_pnl.abs()));
    let orders = settings.orders.iter().rev().take(50).cloned().map(|mut order| {
        if order.quantity > 0.0 && order.filled_quantity >= order.quantity && matches!(order.status.as_str(), "armed" | "resting") { order.status = "filled".into(); }
        order
    }).collect();
    let auto_positions = settings.positions.iter().filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| (symbol.clone(), position.clone())).collect::<Vec<_>>();
    drop(settings);
    let unrealized_pnl = if auto_positions.is_empty() { Some(0.0) } else {
        let base = std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
        let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build().unwrap_or_default();
        let mut total = 0.0;
        let mut complete = true;
        for (symbol, position) in auto_positions {
            let response = client.get(format!("{base}/internal/insights-inputs?symbol={symbol}")).send().await;
            let Ok(response) = response else { complete = false; break; };
            let Ok(inputs) = response.error_for_status() else { complete = false; break; };
            let Ok(inputs) = inputs.json::<serde_json::Value>().await else { complete = false; break; };
            let market = &inputs["markets"][&symbol];
            let mark = if position.quantity > 0.0 { market["bids"][0]["price"].as_f64() } else { market["asks"][0]["price"].as_f64() };
            let Some(mark) = mark.filter(|price| price.is_finite() && *price > 0.0) else { complete = false; break; };
            total += (mark - position.average_entry_price) * position.quantity - position.entry_fees
                - position.quantity.abs() * mark * crate::positions::configured_taker_fee_rate();
        }
        complete.then_some(total)
    };
    let settings = state.settings.lock().await;
    let exit_learning_samples = settings.closed_trade_returns_percent.len();
    let learning_samples = settings.learning.values().map(|stats| stats.samples).sum();
    let learning_positive = settings.learning.values().map(|stats| stats.positive).sum::<usize>();
    let preferred_order_type = ALL_ORDER_TYPES.iter().filter_map(|kind| {
        let suffix = format!(":{kind:?}");
        let stats = settings.learning.iter().filter(|(key, _)| key.ends_with(&suffix)).map(|(_, stats)| stats).fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples; total.positive += stats.positive; total.realized_pnl += stats.realized_pnl; total
        });
        (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES && stats.positive as f64 / stats.samples as f64 >= 0.55)
            .then_some((format!("{kind:?}"), stats.realized_pnl / stats.samples as f64))
    }).filter(|(_, pnl)| *pnl > 0.0).max_by(|a, b| a.1.total_cmp(&b.1)).map(|(kind, _)| kind);
    Json(ActivityResponse { orders, pnl_breakdown, realized_pnl, unrealized_pnl, exit_learning_samples, learning_samples,
        learning_win_rate: (learning_samples > 0).then_some(learning_positive as f64 / learning_samples as f64), preferred_order_type })
}

async fn run_once(State(state): State<AutoTraderState>) -> Result<Json<CycleResult>, (StatusCode, String)> {
    if !state.enabled.load(Ordering::Relaxed) { return Ok(Json(skipped("Enable Auto Trader before running a cycle"))); }
    run_cycle(&state).await.map(Json)
}

async fn run_cycle(state: &AutoTraderState) -> Result<CycleResult, (StatusCode, String)> {
    let result = run_cycle_inner(state).await;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut message = match &result {
        Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted" => format!("{}: {} {} {}", cycle.status, cycle.side.as_deref().unwrap_or(""), cycle.symbol.as_deref().unwrap_or(""), cycle.order_type.as_deref().unwrap_or("")),
        Ok(cycle) => cycle.reason.clone().unwrap_or_else(|| cycle.status.to_owned()),
        Err((_, error)) => format!("Cycle error: {error}"),
    };
    let mut settings = state.settings.lock().await;
    if !settings.last_risk_note.is_empty() { message.push_str(" · "); message.push_str(&settings.last_risk_note); }
    settings.last_decision = message;
    settings.last_decision_at = now;
    if let Err(error) = persist_settings(&state.settings_path, &settings) { eprintln!("Could not persist Auto Trader decision: {}", error.1); }
    result
}

async fn run_cycle_inner(state: &AutoTraderState) -> Result<CycleResult, (StatusCode, String)> {
    let _cycle = state.cycle_lock.lock().await;
    { let mut settings = state.settings.lock().await; settings.last_risk_note.clear(); settings.risk_limited_sizing = false; }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| (StatusCode::INTERNAL_SERVER_ERROR, "System clock is before Unix epoch".into()))?.as_secs();
    let insights_url = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let position_url = std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let client = reqwest::Client::new();
    { let mut settings = state.settings.lock().await; reset_daily_budget(&mut settings); }
    if let Some(result) = maybe_exit_auto_position(&client, &position_url, state).await? { return Ok(result); }
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        if settings.daily_realized_pnl <= -settings.daily_realized_loss_limit {
            if !settings.daily_loss_limit_reduced {
                settings.daily_realized_loss_limit = (settings.daily_realized_loss_limit * 0.9).max(MIN_DAILY_REALIZED_LOSS_LIMIT.min(settings.daily_realized_loss_limit));
                settings.daily_loss_limit_reduced = true;
                let _ = persist_settings(&state.settings_path, &settings);
            }
            return Ok(skipped(&format!("Daily realized-loss limit reached at USD {:.2}; learned daily cap is now USD {:.2}", settings.daily_realized_pnl, settings.daily_realized_loss_limit)));
        }
    }
    if let Some(result) = maybe_place_portfolio_hedge(&client, &position_url, state).await? { return Ok(result); }
    let dashboard_response = client.get(format!("{insights_url}/signals/auto-trader-dashboard")).send().await.map_err(upstream_error)?.error_for_status().map_err(upstream_error)?;
    let dashboard_payload: serde_json::Value = dashboard_response.json().await.map_err(upstream_error)?;
    let dashboard: Dashboard = serde_json::from_value(dashboard_payload).map_err(|error| (StatusCode::BAD_GATEWAY, format!("Auto Trader signal payload could not be decoded: {error}")))?;
    let (learned_strength_floor, exploration_score_floor) = {
        let settings = state.settings.lock().await;
        (
            adaptive_strength_floor(&settings.learning),
            adaptive_exploration_score_floor(&settings.learning),
        )
    };
    let mut signals = dashboard.best;
    let mut inferred_direction_count = 0;
    for signal in &mut signals {
        if signal.direction == "neutral" && signal.ranging_probability.unwrap_or(0.0) < 0.75
            && signal.score.is_finite() && signal.score.abs() >= NEUTRAL_SCORE_DIRECTION_FLOOR {
            signal.direction = if signal.score > 0.0 { "bullish".into() } else { "bearish".into() };
            inferred_direction_count += 1;
        }
    }
    let open_positions = state.settings.lock().await.positions.iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12).map(|(symbol, position)| (symbol.clone(), position.clone())).collect::<BTreeMap<_, _>>();
    let now_ms = now.saturating_mul(1000);
    let fresh_directional = signals.iter().filter(|signal| {
        if signal.stale || now.saturating_sub(signal.timestamp / 1000) > 60 || !matches!(signal.direction.as_str(), "bullish" | "bearish") { return false; }
        open_positions.get(&signal.symbol).is_none_or(|position| {
            let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
            position.quantity.signum() == signal_quantity && position.entries < MAX_ENTRIES_PER_AUTO_POSITION
        })
    }).collect::<Vec<_>>();
    let strength_qualified = fresh_directional.iter().filter(|signal| signal.strength.is_finite() && signal.strength >= learned_strength_floor).copied().collect::<Vec<_>>();
    let exploration_qualified = fresh_directional.iter().filter(|signal| signal.strength.is_finite() && signal.strength >= MIN_EXPLORATION_STRENGTH && signal.score.is_finite() && signal.score.abs() >= exploration_score_floor).copied().collect::<Vec<_>>();
    let strict_candidate = strength_qualified.iter().copied().filter(|signal| {
        let side_evidence = dashboard.walk_forward.iter()
            .find(|item| item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds))
            .map(|item| if signal.direction == "bullish" { &item.all_signal_long } else { &item.all_signal_short });
        side_evidence.is_some_and(|evidence| {
            evidence.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                && evidence.direction_match_rate.is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                && evidence.average_net_return_percent.is_some_and(|net| net > 0.0)
                && evidence.positive_net_rate.is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_RATE)
                && evidence.positive_net_rate_lower_95.is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
        })
    }).max_by(|a, b| a.strength.total_cmp(&b.strength));
    let (signal, exploratory) = if let Some(signal) = strict_candidate {
        (signal, false)
    } else {
        let exploratory = exploration_qualified.iter().copied().filter(|signal| {
            let side_evidence = dashboard.walk_forward.iter()
                .find(|item| item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds))
                .map(|item| if signal.direction == "bullish" { &item.all_signal_long } else { &item.all_signal_short });
            side_evidence.is_none_or(|evidence| {
                evidence.sample_count < MIN_EXPLORATION_EVIDENCE_SAMPLES
                    || (evidence.average_net_return_percent.is_some_and(|net| net > 0.0)
                        && evidence.positive_net_rate.is_some_and(|rate| rate >= NEGATIVE_EVIDENCE_FLOOR))
            })
        }).max_by(|a, b| a.strength.total_cmp(&b.strength));
        let Some(signal) = exploratory else {
            let blocked_by_negative = exploration_qualified.iter().filter(|signal| {
                dashboard.walk_forward.iter().find(|item| item.symbol == signal.symbol && (60..=300).contains(&item.horizon_seconds))
                    .map(|item| if signal.direction == "bullish" { &item.all_signal_long } else { &item.all_signal_short })
                    .is_some_and(|evidence| evidence.sample_count >= MIN_EXPLORATION_EVIDENCE_SAMPLES
                        && (evidence.average_net_return_percent.is_none_or(|net| net <= 0.0)
                            || evidence.positive_net_rate.is_none_or(|rate| rate < NEGATIVE_EVIDENCE_FLOOR)))
            }).count();
            return Ok(skipped(&format!("No tradeable bias: fresh directional {}/{} ({} inferred from neutral score); learned floor {:.2} passed {}; exploration strength floor {:.2}; raw score floor {:.2} passed {}; {} candidate(s) blocked by negative walk-forward history", fresh_directional.len(), signals.len(), inferred_direction_count, learned_strength_floor, strength_qualified.len(), MIN_EXPLORATION_STRENGTH, exploration_score_floor, exploration_qualified.len(), blocked_by_negative)));
        };
        (signal, true)
    };
    let position_url = std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    if !signal.symbol.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-') { return Ok(skipped("Selected insight has an invalid instrument symbol")); }
    let inputs: serde_json::Value = client.get(format!("{position_url}/internal/insights-inputs?symbol={}", signal.symbol)).send().await.map_err(upstream_error)?.error_for_status().map_err(upstream_error)?.json().await.map_err(upstream_error)?;
    let market = &inputs["markets"][&signal.symbol];
    let bid = market["bids"][0]["price"].as_f64();
    let ask = market["asks"][0]["price"].as_f64();
    let Some((bid, ask)) = bid.zip(ask).filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid) else { return Ok(skipped(&format!("No current order book for {}; subscribed and waiting for a fresh book", signal.symbol))); };
    let mid = (bid + ask) / 2.0;
    let signal_key = signal.symbol.clone();
    let now_ms = now.saturating_mul(1000);
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if let Some(stopped_at) = settings.orders.iter()
        .filter(|order| order.symbol == signal_key && order.purpose == "stop-loss" && order.status == "filled" && order.realized_pnl < 0.0)
        .filter_map(|order| order.triggered_at)
        .max()
        .filter(|stopped_at| now_ms.saturating_sub(*stopped_at) < POST_STOP_REENTRY_COOLDOWN_MS) {
        let seconds_remaining = POST_STOP_REENTRY_COOLDOWN_MS.saturating_sub(now_ms.saturating_sub(stopped_at)) / 1000;
        return Ok(skipped(&format!("Post-stop cooldown: {signal_key} paused for {seconds_remaining}s after a losing protective stop")));
    }
    let open_position_count = settings.positions.values().filter(|position| position.quantity.abs() > 1e-12).count();
    let already_open = settings.positions.get(&signal_key).is_some_and(|position| position.quantity.abs() > 1e-12);
    if !already_open && open_position_count >= settings.max_open_positions {
        return Ok(skipped(&format!("Open-position cap reached ({}/{})", open_position_count, settings.max_open_positions)));
    }
    if let Some(position) = settings.positions.get(&signal_key).filter(|position| position.quantity.abs() > 1e-12) {
        let signal_quantity = if signal.direction == "bullish" { 1.0 } else { -1.0 };
        if position.quantity.signum() != signal_quantity {
            return Ok(skipped(&format!("Opposite signal for {signal_key}; waiting for its stop or target exit")));
        }
        if position.entries >= MAX_ENTRIES_PER_AUTO_POSITION {
            return Ok(skipped(&format!("Add-on limit reached for {signal_key} ({MAX_ENTRIES_PER_AUTO_POSITION} entries per position)")));
        }
    } else if settings.last_signal_direction.get(&signal_key).is_some_and(|direction| direction == &signal.direction)
        && settings.last_signal_strength.get(&signal_key).is_some_and(|strength| signal.strength < strength + MATERIAL_STRENGTH_CHANGE) {
        return Ok(skipped(&format!("Re-entry blocked: {signal_key} signal strength has not improved by {:.2}", MATERIAL_STRENGTH_CHANGE)));
    }
    if settings.last_signal_at.get(&signal_key).is_some_and(|last| now_ms.saturating_sub(*last) < SIGNAL_COOLDOWN_MS) {
        return Ok(skipped(&format!("Re-entry cooldown: {signal_key} needs a materially new signal after {} seconds", SIGNAL_COOLDOWN_MS / 1000)));
    }
    let risk_fraction = if settings.risk_limited_sizing { 0.25 } else { 1.0 };
    let exploration_cap = if exploratory { settings.max_order_notional * (settings.exploration_percent / 100.0) } else { settings.max_order_notional };
    let risk_order_cap = exploration_cap.min(settings.max_order_notional * risk_fraction);
    let mut notional = risk_order_cap.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 { return Ok(skipped("Total notional capacity is fully allocated")); }
    if exploratory {
        if !settings.last_risk_note.is_empty() { settings.last_risk_note.push_str(" · "); }
        settings.last_risk_note.push_str("small paper-exploration size; no reliable same-side walk-forward edge yet");
    }
    let baseline_type = choose_order_type(&signal);
    let context = market_context(&signal);
    let Some(order_type) = adapt_order_type(baseline_type, &context, &settings.learning) else {
        return Ok(skipped("Recent closed auto-trades show no profitable order type with enough samples"));
    };
    drop(settings);
    let side = if signal.direction == "bullish" { "buy" } else { "sell" };
    let deep_book = assess_deep_book(&client, &signal.symbol, side == "buy", notional).await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))?;
    let deep_book_note = if let Some(assessment) = deep_book {
        if !assessment.impact_bps.is_finite() || assessment.impact_bps > DEEP_BOOK_BAND_BPS {
            return Ok(skipped(&format!("Deep book cannot support a {} {side} order with impact under {:.0} bps", signal.symbol, DEEP_BOOK_BAND_BPS)));
        }
        notional = notional.min(assessment.notional_cap);
        if notional < MIN_DEEP_BOOK_ORDER_NOTIONAL { return Ok(skipped(&format!("Deep-book capacity for {} is below the minimum paper order", signal.symbol))); }
        Some(format!("deep book: {:.0} quote liquidity within {:.0} bps; order cap {:.0} at {:.0}% participation; estimated sweep {:.2} bps", assessment.available_within_band, DEEP_BOOK_BAND_BPS, assessment.notional_cap, DEEP_BOOK_MAX_PARTICIPATION * 100.0, assessment.impact_bps))
    } else { None };
    if let Some(note) = deep_book_note {
        let mut settings = state.settings.lock().await;
        if !settings.last_risk_note.is_empty() { settings.last_risk_note.push_str(" · "); }
        settings.last_risk_note.push_str(&note);
    }
    let quantity = notional / mid;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    {
        let mut settings = state.settings.lock().await;
        reset_daily_budget(&mut settings);
        settings.used_notional_today = allocated_notional(&settings);
        settings.last_signal_at.insert(signal_key.clone(), timestamp);
        settings.last_signal_direction.insert(signal_key.clone(), signal.direction.clone());
        settings.last_signal_strength.insert(signal_key, signal.strength);
        settings.orders.push(TraderOrder { timestamp, triggered_at: None, symbol: signal.symbol.clone(), side: side.to_owned(), order_type: format!("{order_type:?}"), purpose: "signal".into(), quantity, filled_quantity: 0.0, average_price: None, reserved_notional: notional,
            book_bid_depth_10bps: deep_book.map(|assessment| assessment.bid_depth_10bps),
            book_ask_depth_10bps: deep_book.map(|assessment| assessment.ask_depth_10bps),
            book_depth_imbalance_10bps: deep_book.map(|assessment| assessment.depth_imbalance_10bps),
            estimated_sweep_bps: deep_book.map(|assessment| assessment.impact_bps),
            status: "submitting".to_owned(), realized_pnl: 0.0 });
        if settings.orders.len() > 500 { let excess = settings.orders.len() - 500; settings.orders.drain(..excess); }
        persist_settings(&state.settings_path, &settings)?;
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "cancelled_before_submit".into(); }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(skipped("Auto Trader was disabled before order submission"));
    }
    let response = match submit_configured_order(&client, &position_url, &signal.symbol, side, quantity, notional, bid, ask, order_type).await {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "unknown".into(); }
            persist_settings(&state.settings_path, &settings)?;
            return Err(error);
        }
    };
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let filled = response.get("filled_quantity").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let average_price = response.get("average_execution_price").and_then(serde_json::Value::as_f64);
    let fee = response.get("estimated_fee").and_then(serde_json::Value::as_f64).unwrap_or(0.0).abs();
    let (realized_delta, learned_type) = if filled > 0.0 {
        average_price.map(|price| apply_auto_fill(&mut settings.positions, &signal.symbol, side, filled, price, fee, &format!("{order_type:?}"), &context)).unwrap_or_default()
    } else { (0.0, None) };
    settings.realized_pnl += realized_delta;
    settings.daily_realized_pnl += realized_delta;
    if let Some(kind) = learned_type {
        let stats = settings.learning.entry(kind).or_default();
        stats.samples += 1;
        if realized_delta > 0.0 { stats.positive += 1; }
        stats.realized_pnl += realized_delta;
    }
    let status = if response.get("barrier_order").is_some_and(|value| !value.is_null()) { "armed" }
        else if response.get("order").is_some_and(|value| !value.is_null()) { "resting" }
        else if filled > 0.0 { "filled" }
        else { "accepted" };
    if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) {
        order.filled_quantity = filled; order.average_price = average_price; order.status = status.to_owned(); order.realized_pnl = realized_delta;
    }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    let stop_armed = if filled > 0.0 {
        if let Some(entry_price) = average_price {
            match arm_protective_stop(&client, &position_url, state, &signal.symbol, side, filled, entry_price, fee).await {
                Ok(()) => true,
                Err((_, error)) => {
                    let mut settings = state.settings.lock().await;
                    settings.last_risk_note = format!("Protective stop could not be armed: {error}; five-second exit monitor remains as fallback");
                    let _ = persist_settings(&state.settings_path, &settings);
                    false
                }
            }
        } else { false }
    } else { false };
    Ok(CycleResult { status: "order_submitted", reason: stop_armed.then(|| "Protective stop-market armed".into()), symbol: Some(signal.symbol.clone()), side: Some(side.into()), quantity: Some(quantity), order_type: Some(format!("{order_type:?}")), order_response: Some(response) })
}



async fn maybe_exit_auto_position(client: &reqwest::Client, base: &str, state: &AutoTraderState) -> Result<Option<CycleResult>, (StatusCode, String)> {
    sync_protective_stops(client, base, state).await?;
    ensure_protective_stops(client, base, state).await?;
    let positions = {
        let settings = state.settings.lock().await;
        settings.positions.iter().filter(|(_, position)| position.quantity.abs() > 1e-12)
            .map(|(symbol, position)| (symbol.clone(), position.clone())).collect::<Vec<_>>()
    };
    for (symbol, position) in positions {
        let inputs: serde_json::Value = client.get(format!("{base}/internal/insights-inputs?symbol={symbol}")).send().await
            .map_err(upstream_error)?.error_for_status().map_err(upstream_error)?.json().await.map_err(upstream_error)?;
        let market = &inputs["markets"][&symbol];
        let bid = market["bids"][0]["price"].as_f64();
        let ask = market["asks"][0]["price"].as_f64();
        let Some((bid, ask)) = bid.zip(ask).filter(|(bid, ask)| bid.is_finite() && ask.is_finite() && *bid > 0.0 && *ask >= *bid) else { continue; };
        let is_long = position.quantity > 0.0;
        let exit_price = if is_long { bid } else { ask };
        let entry_notional = position.quantity.abs() * position.average_entry_price;
        let net_open_pnl = (exit_price - position.average_entry_price) * position.quantity
            - position.entry_fees
            - position.quantity.abs() * exit_price * crate::positions::configured_taker_fee_rate();
        let return_fraction = if entry_notional > 0.0 { net_open_pnl / entry_notional } else { 0.0 };
        let (stop_loss_percent, take_profit_percent) = { let settings = state.settings.lock().await; (settings.stop_loss_percent, settings.take_profit_percent) };
        let reason = if return_fraction <= -stop_loss_percent / 100.0 { Some("stop-loss") }
            else if return_fraction >= take_profit_percent / 100.0 { Some("take-profit") } else { None };
        let Some(reason) = reason else { continue; };
        let protected_quantity = position.protective_stops.iter().map(|stop| (stop.quantity - stop.processed_quantity).max(0.0)).sum::<f64>();
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
        let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
        {
            let mut settings = state.settings.lock().await;
            reset_daily_budget(&mut settings);
            settings.used_notional_today = allocated_notional(&settings);
            settings.orders.push(TraderOrder { timestamp, triggered_at: None, symbol: symbol.clone(), side: side.into(), order_type: "Ioc".into(), purpose: reason.into(), quantity, filled_quantity: 0.0, average_price: None, reserved_notional: notional, book_bid_depth_10bps: None, book_ask_depth_10bps: None, book_depth_imbalance_10bps: None, estimated_sweep_bps: None, status: "submitting".into(), realized_pnl: 0.0 });
        }
        let response = match submit_configured_order(client, base, &symbol, side, quantity, notional, bid, ask, AutoOrderType::Ioc).await {
            Ok(response) => response,
            Err(error) => {
                let mut settings = state.settings.lock().await;
                if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "unknown".into(); }
                persist_settings(&state.settings_path, &settings)?;
                return Err(error);
            }
        };
        let mut settings = state.settings.lock().await;
        let filled = response["filled_quantity"].as_f64().unwrap_or(0.0);
        let average_price = response["average_execution_price"].as_f64();
        let fee = response["estimated_fee"].as_f64().unwrap_or(0.0).abs();
        let (realized_pnl, learned_key) = if filled > 0.0 {
            average_price.map(|price| apply_auto_fill(&mut settings.positions, &symbol, side, filled, price, fee, "Ioc", reason)).unwrap_or_default()
        } else { (0.0, None) };
        settings.realized_pnl += realized_pnl;
        settings.daily_realized_pnl += realized_pnl;
        if filled + 1e-12 >= quantity {
            let closed_quantity = filled.min(quantity);
            let entry_notional = closed_quantity * position.average_entry_price;
            if entry_notional > 0.0 {
                settings.closed_trade_returns_percent.push(realized_pnl / entry_notional * 100.0);
                if settings.closed_trade_returns_percent.len() > 200 { settings.closed_trade_returns_percent.remove(0); }
                adapt_exit_policy(&mut settings);
            }
        }
        if let Some(key) = learned_key {
            let stats = settings.learning.entry(key).or_default();
            stats.samples += 1;
            if realized_pnl > 0.0 { stats.positive += 1; }
            stats.realized_pnl += realized_pnl;
        }
        if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) {
            order.filled_quantity = filled; order.average_price = average_price; order.realized_pnl = realized_pnl;
            order.status = if filled > 0.0 { "filled" } else { "accepted" }.into();
        }
        persist_settings(&state.settings_path, &settings)?;
        return Ok(Some(CycleResult { status: "order_submitted", reason: Some(format!("Auto-trader {reason} at {:.4}%", return_fraction * 100.0)), symbol: Some(symbol), side: Some(side.into()), quantity: Some(quantity), order_type: Some("Ioc exit".into()), order_response: Some(response) }));
    }
    Ok(None)
}

async fn arm_protective_stop(client: &reqwest::Client, base: &str, state: &AutoTraderState, symbol: &str, position_side: &str, quantity: f64, entry_price: f64, entry_fees: f64) -> Result<(), (StatusCode, String)> {
    if quantity <= 0.0 || entry_price <= 0.0 { return Ok(()); }
    let (stop_loss_percent, is_enabled) = {
        let settings = state.settings.lock().await;
        (settings.stop_loss_percent, state.enabled.load(Ordering::Relaxed))
    };
    if !is_enabled { return Ok(()); }
    let is_long = position_side == "buy";
    let side = if is_long { "sell" } else { "buy" };
    let fee_rate = crate::positions::configured_taker_fee_rate();
    let loss_fraction = stop_loss_percent / 100.0;
    let trigger_price = if is_long {
        (quantity * entry_price * (1.0 - loss_fraction) + entry_fees) / (quantity * (1.0 - fee_rate))
    } else {
        (quantity * entry_price * (1.0 + loss_fraction) - entry_fees) / (quantity * (1.0 + fee_rate))
    };
    let trigger_direction = if is_long { "below" } else { "above" };
    let response = client.post(format!("{base}/positions/orders")).json(&serde_json::json!({
        "symbol": symbol, "side": side, "order_type": "stop-market", "quantity": quantity,
        "trigger_price": trigger_price, "trigger_direction": trigger_direction
    })).send().await.map_err(upstream_error)?;
    let status = response.status();
    let payload = response.json::<serde_json::Value>().await.map_err(upstream_error)?;
    if !status.is_success() { return Err((StatusCode::BAD_GATEWAY, format!("Protective stop rejected ({status}): {payload}"))); }
    let barrier = &payload["barrier_order"];
    let id = barrier["id"].as_str().ok_or((StatusCode::BAD_GATEWAY, "Protective stop response did not include a barrier id".into()))?.to_owned();
    let timestamp = barrier["created_at"].as_u64().unwrap_or_else(|| SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64);
    let mut settings = state.settings.lock().await;
    if let Some(position) = settings.positions.get_mut(symbol).filter(|position| position.quantity.abs() > 1e-12 && position.quantity.signum() == if is_long { 1.0 } else { -1.0 }) {
        position.protective_stops.push(ProtectiveStop { barrier_id: id, order_timestamp: timestamp, quantity, processed_quantity: 0.0, processed_notional: 0.0, processed_fee: 0.0 });
        settings.orders.push(TraderOrder { timestamp, triggered_at: None, symbol: symbol.to_owned(), side: side.to_owned(), order_type: "StopMarket".into(), purpose: "stop-loss".into(), quantity, filled_quantity: 0.0, average_price: None, reserved_notional: 0.0, book_bid_depth_10bps: None, book_ask_depth_10bps: None, book_depth_imbalance_10bps: None, estimated_sweep_bps: None, status: "armed".into(), realized_pnl: 0.0 });
        if settings.orders.len() > 500 { let excess = settings.orders.len() - 500; settings.orders.drain(..excess); }
        persist_settings(&state.settings_path, &settings)?;
    } else {
        drop(settings);
        let _ = client.delete(format!("{base}/positions/orders/{id}")).send().await;
    }
    Ok(())
}

async fn sync_protective_stops(client: &reqwest::Client, base: &str, state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let tracked = {
        let settings = state.settings.lock().await;
        settings.positions.iter().flat_map(|(symbol, position)| position.protective_stops.iter().cloned().map(move |stop| (symbol.clone(), stop))).collect::<Vec<_>>()
    };
    if tracked.is_empty() { return Ok(()); }
    let barriers: Vec<serde_json::Value> = client.get(format!("{base}/positions/barriers")).send().await
        .map_err(upstream_error)?.error_for_status().map_err(upstream_error)?.json().await.map_err(upstream_error)?;
    for (symbol, stop) in tracked {
        let barrier = barriers.iter().find(|barrier| barrier["id"].as_str() == Some(&stop.barrier_id));
        let Some(barrier) = barrier else {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) { position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id); }
            if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == stop.order_timestamp) { order.status = "cancelled".into(); }
            persist_settings(&state.settings_path, &settings)?;
            continue;
        };
        let total_filled = barrier["filled_quantity"].as_f64().unwrap_or(0.0).max(0.0);
        let total_notional = barrier["average_execution_price"].as_f64().unwrap_or(0.0) * total_filled;
        let total_fee = barrier["fee_paid"].as_f64().unwrap_or(0.0).abs();
        let delta_quantity = (total_filled - stop.processed_quantity).max(0.0);
        let delta_notional = (total_notional - stop.processed_notional).max(0.0);
        let delta_fee = (total_fee - stop.processed_fee).max(0.0);
        let status = barrier["status"].as_str().unwrap_or("unknown");
        if delta_quantity > 1e-12 && delta_notional > 0.0 {
            let side = if barrier["side"].as_str() == Some("buy") { "buy" } else { "sell" };
            let fill_price = delta_notional / delta_quantity;
            let mut settings = state.settings.lock().await;
            let entry_price_before = settings.positions.get(&symbol).map(|position| position.average_entry_price).filter(|price| *price > 0.0).unwrap_or(fill_price);
            let (realized_delta, learned_key) = apply_auto_fill(&mut settings.positions, &symbol, side, delta_quantity, fill_price, delta_fee, "StopMarket", "stop-loss");
            settings.realized_pnl += realized_delta;
            settings.daily_realized_pnl += realized_delta;
            settings.used_notional_today = allocated_notional(&settings);
            let entry_notional = delta_quantity * entry_price_before;
            let fully_closed = settings.positions.get(&symbol).is_none_or(|position| position.quantity.abs() <= 1e-12);
            if fully_closed && entry_notional > 0.0 {
                settings.closed_trade_returns_percent.push(realized_delta / entry_notional * 100.0);
                if settings.closed_trade_returns_percent.len() > 200 { settings.closed_trade_returns_percent.remove(0); }
                adapt_exit_policy(&mut settings);
            }
            if let Some(key) = learned_key {
                let stats = settings.learning.entry(key).or_default();
                stats.samples += 1;
                if realized_delta > 0.0 { stats.positive += 1; }
                stats.realized_pnl += realized_delta;
            }
            if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == stop.order_timestamp) {
                order.triggered_at = barrier["triggered_at"].as_u64();
                order.filled_quantity = total_filled;
                order.average_price = barrier["average_execution_price"].as_f64();
                order.realized_pnl += realized_delta;
                order.status = status.to_owned();
            }
            if let Some(position) = settings.positions.get_mut(&symbol) {
                if let Some(tracked) = position.protective_stops.iter_mut().find(|tracked| tracked.barrier_id == stop.barrier_id) {
                    tracked.processed_quantity = total_filled;
                    tracked.processed_notional = total_notional;
                    tracked.processed_fee = total_fee;
                }
            }
            persist_settings(&state.settings_path, &settings)?;
        }
        if !matches!(status, "armed" | "triggering") {
            let mut settings = state.settings.lock().await;
            if let Some(position) = settings.positions.get_mut(&symbol) { position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id); }
            if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == stop.order_timestamp) { order.status = status.to_owned(); }
            persist_settings(&state.settings_path, &settings)?;
        }
    }
    Ok(())
}

async fn ensure_protective_stops(client: &reqwest::Client, base: &str, state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let positions = {
        let settings = state.settings.lock().await;
        settings.positions.iter().filter(|(_, position)| position.quantity.abs() > 1e-12).map(|(symbol, position)| (symbol.clone(), position.clone())).collect::<Vec<_>>()
    };
    for (symbol, position) in positions {
        let covered = position.protective_stops.iter().map(|stop| (stop.quantity - stop.processed_quantity).max(0.0)).sum::<f64>();
        let uncovered = (position.quantity.abs() - covered).max(0.0);
        if uncovered > 1e-12 {
            if let Err((_, error)) = arm_protective_stop(client, base, state, &symbol, if position.quantity > 0.0 { "buy" } else { "sell" }, uncovered, position.average_entry_price, position.entry_fees * uncovered / position.quantity.abs()).await {
                let mut settings = state.settings.lock().await;
                settings.last_risk_note = format!("Protective stop could not be armed for {symbol}: {error}; five-second exit monitor remains as fallback");
            }
        }
    }
    Ok(())
}

async fn cancel_protective_stops(client: &reqwest::Client, base: &str, state: &AutoTraderState, symbol: &str) -> Result<bool, (StatusCode, String)> {
    let stops = {
        let settings = state.settings.lock().await;
        settings.positions.get(symbol).map(|position| position.protective_stops.clone()).unwrap_or_default()
    };
    for stop in stops {
        let response = client.delete(format!("{base}/positions/orders/{}", stop.barrier_id)).send().await.map_err(upstream_error)?;
        if !response.status().is_success() { return Ok(false); }
        let mut settings = state.settings.lock().await;
        if let Some(position) = settings.positions.get_mut(symbol) { position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id); }
        if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == stop.order_timestamp) { order.status = "cancelled".into(); }
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(true)
}

#[derive(Clone)]
struct RiskHolding { symbol: String, signed_usd: f64 }

async fn maybe_place_portfolio_hedge(client: &reqwest::Client, base: &str, state: &AutoTraderState) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let candidates: serde_json::Value = client.get(format!("{base}/positions/hedge-candidates")).send().await.map_err(upstream_error)?.error_for_status().map_err(upstream_error)?.json().await.map_err(upstream_error)?;
    let rates_response: serde_json::Value = client.get(format!("{base}/positions/usd-rates")).send().await.map_err(upstream_error)?.error_for_status().map_err(upstream_error)?.json().await.map_err(upstream_error)?;
    let rates = &rates_response["rates"];
    let positions = candidates["positions"].as_array().cloned().unwrap_or_default();
    let mut holdings = Vec::new();
    let mut fallback_marks = 0;
    for position in &positions {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let quantity = position["quantity"].as_f64().unwrap_or(0.0);
        let live_price = position["latest_price"].as_f64().filter(|price| price.is_finite() && *price > 0.0);
        let entry_price = position["average_entry_price"].as_f64().filter(|price| price.is_finite() && *price > 0.0);
        let quote = symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
        let rate = rates[quote].as_f64().unwrap_or(0.0);
        if quantity.is_finite() && rate.is_finite() && rate > 0.0 {
            if let Some(price) = live_price.or(entry_price) {
                if live_price.is_none() { fallback_marks += 1; }
                holdings.push(RiskHolding { symbol: symbol.to_owned(), signed_usd: quantity * price * rate });
            }
        }
    }
    let unknown_marks = positions.len().saturating_sub(holdings.len());
    {
        let mut settings = state.settings.lock().await;
        settings.last_risk_note = match (fallback_marks, unknown_marks) {
            (fallback, unknown) if unknown > 0 => format!("risk covers {} open positions; {fallback} use entry-price marks and {unknown} are unmarked, so hedging is paused", positions.len()),
            (fallback, _) if fallback > 0 => format!("risk covers {} open positions; {fallback} use entry-price marks", positions.len()),
            _ => format!("risk covers {} open positions", positions.len()),
        };
        settings.risk_limited_sizing = unknown_marks > 0;
    }
    if holdings.is_empty() || unknown_marks > 0 { return Ok(None); }
    let mut portfolio_variance = holdings.iter().map(|holding| holding.signed_usd.powi(2)).sum::<f64>();
    if let Some(pairs) = candidates["pair_correlations"].as_array() {
        for i in 0..holdings.len() { for j in (i + 1)..holdings.len() {
            if let Some(pair) = pairs.iter().find(|pair| {
                (pair["symbol_a"].as_str() == Some(&holdings[i].symbol) && pair["symbol_b"].as_str() == Some(&holdings[j].symbol))
                    || (pair["symbol_b"].as_str() == Some(&holdings[i].symbol) && pair["symbol_a"].as_str() == Some(&holdings[j].symbol))
            }) {
                if let Some(correlation) = pair["correlation"].as_f64().filter(|value| value.is_finite()) {
                    portfolio_variance += 2.0 * holdings[i].signed_usd * holdings[j].signed_usd * correlation.clamp(-1.0, 1.0);
                }
            }
        }}
    }
    let portfolio_shock_risk = portfolio_variance.max(0.0).sqrt() * PORTFOLIO_SHOCK_PERCENT;
    let settings_snapshot = state.settings.lock().await.clone();
    let risk_trigger = settings_snapshot.daily_realized_loss_limit;
    if portfolio_shock_risk <= risk_trigger { return Ok(None); }
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let mut best: Option<(String, String, f64, f64, f64, f64)> = None;
    // Reduce the largest direct holding first, before considering a correlated proxy.
    if let Some(holding) = holdings.iter().max_by(|a, b| a.signed_usd.abs().total_cmp(&b.signed_usd.abs())) {
        if holding.signed_usd.abs() > 0.0 {
            let row = positions.iter().find(|row| row["symbol"].as_str() == Some(&holding.symbol));
            let price = row.and_then(|row| row["latest_price"].as_f64().or_else(|| row["average_entry_price"].as_f64())).unwrap_or(0.0);
            let quote = holding.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            let last = settings_snapshot.last_hedge_at.get(&holding.symbol).copied().unwrap_or(0);
            if price > 0.0 && quote_rate > 0.0 && now_ms.saturating_sub(last) >= 300_000 {
                best = Some((holding.symbol.clone(), if holding.signed_usd > 0.0 { "sell" } else { "buy" }.into(), holding.signed_usd.abs().min(settings_snapshot.max_order_notional), price, quote_rate, holding.signed_usd.abs()));
            }
        }
    }
    for position in &positions {
        if best.is_some() { break; }
        let candidate_rows = position["candidates"].as_array().cloned().unwrap_or_default();
        for candidate in candidate_rows {
            let hedge_symbol = candidate["symbol"].as_str().unwrap_or_default();
            let hedge_price = candidate["latest_price"].as_f64().unwrap_or(0.0);
            let quote = hedge_symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            if hedge_price <= 0.0 || quote_rate <= 0.0 { continue; }
            let mut covariance = 0.0;
            let mut usable_correlations = 0;
            let mut strongest = 0.0_f64;
            let mut enough_samples = false;
            let mut complete_sample = true;
            for holding in &holdings {
                let relation = if holding.symbol == hedge_symbol { Some((1.0, usize::MAX)) } else {
                    positions.iter().find(|row| row["symbol"].as_str() == Some(&holding.symbol))
                        .and_then(|row| row["candidates"].as_array())
                        .and_then(|rows| rows.iter().find(|row| row["symbol"].as_str() == Some(hedge_symbol)))
                        .and_then(|row| row["correlation"].as_f64().zip(row["paired_samples"].as_u64()).map(|(corr, n)| (corr, n as usize)))
                };
                if let Some((correlation, samples)) = relation.filter(|(corr, _)| corr.is_finite()) {
                    if samples < MIN_HEDGE_CORRELATION_SAMPLES { complete_sample = false; }
                    let correlation = correlation.clamp(-1.0, 1.0);
                    covariance += holding.signed_usd * correlation;
                    usable_correlations += 1;
                    if correlation.abs() > strongest { strongest = correlation.abs(); enough_samples = samples >= MIN_HEDGE_CORRELATION_SAMPLES; }
                }
            }
            if usable_correlations != holdings.len() || !complete_sample || !enough_samples || strongest < MIN_HEDGE_CORRELATION || !covariance.is_finite() || covariance.abs() < settings_snapshot.max_order_notional * 0.1 { continue; }
            let last = settings_snapshot.last_hedge_at.get(hedge_symbol).copied().unwrap_or(0);
            if now_ms.saturating_sub(last) < 300_000 { continue; }
            let hedge_notional = covariance.abs().min(settings_snapshot.max_order_notional);
            let score = covariance.abs().min(portfolio_shock_risk);
            if best.as_ref().is_none_or(|current| score > current.5) {
                let side = if covariance > 0.0 { "sell" } else { "buy" };
                best = Some((hedge_symbol.to_owned(), side.to_owned(), hedge_notional, hedge_price, quote_rate, score));
            }
        }
    }
    let Some((symbol, side, requested_notional, price, quote_rate, _)) = best else {
        return Ok(Some(skipped("Portfolio risk is elevated, but no liquid hedge has enough reliable correlation data; new exposure is blocked")));
    };
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    let notional = requested_notional.min(settings.daily_notional_limit - settings.used_notional_today);
    if notional <= 0.0 { return Ok(Some(skipped("Portfolio risk is elevated, but the daily hedge budget is exhausted"))); }
    let timestamp = now_ms;
    let quantity = notional / (price * quote_rate);
    settings.used_notional_today = allocated_notional(&settings);
    settings.last_hedge_at.insert(symbol.clone(), timestamp);
    settings.orders.push(TraderOrder { timestamp, triggered_at: None, symbol: symbol.clone(), side: side.clone(), order_type: "Ioc".into(), purpose: "portfolio-hedge".into(), quantity, filled_quantity: 0.0, average_price: None, reserved_notional: notional, book_bid_depth_10bps: None, book_ask_depth_10bps: None, book_depth_imbalance_10bps: None, estimated_sweep_bps: None, status: "submitting".into(), realized_pnl: 0.0 });
    if settings.orders.len() > 500 { let excess = settings.orders.len() - 500; settings.orders.drain(..excess); }
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    if !cancel_protective_stops(client, base, state, &symbol).await? {
        let mut settings = state.settings.lock().await;
        if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "cancelled_stop_active".into(); }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        return Ok(Some(skipped("Could not cancel the existing protective stop; deferred portfolio hedge")));
    }
    if !state.enabled.load(Ordering::Relaxed) {
        let mut settings = state.settings.lock().await;
        settings.last_hedge_at.remove(&symbol);
        if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "cancelled_before_submit".into(); }
        settings.used_notional_today = allocated_notional(&settings);
        persist_settings(&state.settings_path, &settings)?;
        drop(settings);
        ensure_protective_stops(client, base, state).await?;
        return Ok(Some(skipped("Auto Trader was disabled before hedge submission")));
    }
    let response = match submit_configured_order(client, base, &symbol, &side, quantity, notional, price, price, AutoOrderType::Ioc).await {
        Ok(response) => response,
        Err(error) => {
            let mut settings = state.settings.lock().await;
            if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.status = "unknown".into(); }
            persist_settings(&state.settings_path, &settings)?;
            drop(settings);
            ensure_protective_stops(client, base, state).await?;
            return Err(error);
        }
    };
    let mut settings = state.settings.lock().await;
    let filled = response.get("filled_quantity").and_then(serde_json::Value::as_f64).unwrap_or(0.0);
    let average_price = response.get("average_execution_price").and_then(serde_json::Value::as_f64);
    let fee = response.get("estimated_fee").and_then(serde_json::Value::as_f64).unwrap_or(0.0).abs();
    let (realized_pnl, _) = if filled > 0.0 { average_price.map(|execution| apply_auto_fill(&mut settings.positions, &symbol, &side, filled, execution, fee, "Ioc", "portfolio-hedge")).unwrap_or_default() } else { (0.0, None) };
    settings.realized_pnl += realized_pnl;
    settings.daily_realized_pnl += realized_pnl;
    let status = if response.get("barrier_order").is_some_and(|value| !value.is_null()) { "armed" } else if response.get("order").is_some_and(|value| !value.is_null()) { "resting" } else if filled > 0.0 { "filled" } else { "accepted" };
    if let Some(order) = settings.orders.iter_mut().find(|order| order.timestamp == timestamp) { order.filled_quantity = filled; order.average_price = average_price; order.realized_pnl = realized_pnl; order.status = status.into(); }
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    drop(settings);
    ensure_protective_stops(client, base, state).await?;
    Ok(Some(CycleResult { status: "hedge_submitted", reason: Some(format!("Portfolio 1% shock risk was USD {:.2}; submitted a risk-reducing hedge", portfolio_shock_risk)), symbol: Some(symbol), side: Some(side), quantity: Some(quantity), order_type: Some("Ioc hedge".into()), order_response: Some(response) }))
}

fn apply_auto_fill(positions: &mut BTreeMap<String, TraderPosition>, symbol: &str, side: &str, quantity: f64, price: f64, fee: f64, order_type: &str, context: &str) -> (f64, Option<String>) {
    let position = positions.entry(symbol.to_owned()).or_default();
    let old = position.quantity;
    let incoming = if side == "buy" { quantity } else { -quantity };
    if old == 0.0 || old.signum() == incoming.signum() {
        position.entries = position.entries.saturating_add(1);
        let total = old.abs() + incoming.abs();
        position.average_entry_price = (old.abs() * position.average_entry_price + incoming.abs() * price) / total;
        position.quantity = old + incoming;
        position.entry_fees += fee;
        if position.entry_order_type.is_empty() { position.entry_order_type = order_type.to_owned(); position.entry_context = context.to_owned(); }
        return (0.0, None);
    }
    let entry_type = if position.entry_order_type.is_empty() { order_type.to_owned() } else { position.entry_order_type.clone() };
    let entry_context = if position.entry_context.is_empty() { context.to_owned() } else { position.entry_context.clone() };
    let closed = old.abs().min(incoming.abs());
    let entry_fee_closed = position.entry_fees * (closed / old.abs());
    let closing_fee = fee * (closed / incoming.abs());
    let pnl = closed * if old > 0.0 { price - position.average_entry_price } else { position.average_entry_price - price } - entry_fee_closed - closing_fee;
    position.realized_pnl += pnl;
    let net = old + incoming;
    if net.abs() < 1e-12 { *position = TraderPosition::default(); }
    else if net.signum() != old.signum() { position.quantity = net; position.average_entry_price = price; position.entry_fees = fee - closing_fee; position.entry_order_type = order_type.to_owned(); position.entry_context = context.to_owned(); }
    else { position.quantity = net; position.entry_fees -= entry_fee_closed; }
    (pnl, Some(format!("{entry_context}:{entry_type}")))
}


fn market_context(signal: &Signal) -> String {
    let regime = if signal.ranging_probability.unwrap_or(0.0) >= 0.65 { "range" }
    else if signal.components.trend_5m_volatility_units.unwrap_or(0.0).abs() >= 1.5 { "trend" }
    else if signal.components.volatility_1m_percent.unwrap_or(0.0).abs() >= 1.0 { "volatile" }
    else { "mixed" };
    format!("{}:{regime}", if signal.direction == "bullish" { "long" } else { "short" })
}

const ALL_ORDER_TYPES: [AutoOrderType; 12] = [AutoOrderType::Market, AutoOrderType::Ioc, AutoOrderType::Fok, AutoOrderType::Limit, AutoOrderType::Gtd, AutoOrderType::PostOnlyLimit, AutoOrderType::Barrier, AutoOrderType::StopMarket, AutoOrderType::StopLimit, AutoOrderType::TrailingStop, AutoOrderType::Vwap, AutoOrderType::Twap];
fn adaptive_strength_floor(learning: &BTreeMap<String, LearningStats>) -> f64 {
    let totals = learning.values().fold(LearningStats::default(), |mut total, stats| {
        total.samples += stats.samples; total.positive += stats.positive; total.realized_pnl += stats.realized_pnl; total
    });
    if totals.samples >= 5 && totals.positive as f64 / (totals.samples as f64) < 0.45 { 0.5 } else { MIN_STRENGTH }
}

fn adaptive_exploration_score_floor(learning: &BTreeMap<String, LearningStats>) -> f64 {
    let totals = learning.values().fold(LearningStats::default(), |mut total, stats| {
        total.samples += stats.samples;
        total.positive += stats.positive;
        total
    });
    let win_rate = totals.positive as f64 / totals.samples.max(1) as f64;
    if totals.samples >= 20 && win_rate < 0.30 {
        0.50
    } else if totals.samples >= 10 && win_rate < 0.45 {
        0.40
    } else {
        MIN_EXPLORATION_SCORE
    }
}
fn adapt_order_type(baseline: AutoOrderType, context: &str, learning: &BTreeMap<String, LearningStats>) -> Option<AutoOrderType> {
    let candidates = ALL_ORDER_TYPES.iter().copied().filter_map(|kind| {
        let stats = learning.get(&format!("{context}:{kind:?}"))?;
        let average_pnl = stats.realized_pnl / stats.samples.max(1) as f64;
        let positive_rate = stats.positive as f64 / stats.samples.max(1) as f64;
        (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES && average_pnl > 0.0 && positive_rate >= 0.55)
            .then_some((kind, average_pnl))
    }).collect::<Vec<_>>();
    if candidates.is_empty() {
        let baseline_stats = learning.get(&format!("{context}:{baseline:?}"));
        return match baseline_stats {
            Some(stats) if stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES => None,
            _ => Some(AutoOrderType::Ioc),
        };
    }
    candidates.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)).map(|(kind, _)| kind)
}

fn choose_order_type(signal: &Signal) -> AutoOrderType {
    let ranging = signal.ranging_probability.unwrap_or(0.0);
    let volatility = signal.components.volatility_1m_percent.unwrap_or(0.0).abs();
    let trend_units = signal.components.trend_5m_volatility_units.unwrap_or(0.0).abs();
    let momentum = signal.components.momentum_10s_percent.unwrap_or(0.0).abs();
    let mispricing = signal.components.mispricing_vs_vwap_bps.unwrap_or(0.0).abs();
    let pressure = signal.components.pressure_without_price_move.unwrap_or(0.0).abs();
    let aggressor_cost = signal.components.aggressor_cost_bps.unwrap_or(0.0).abs();
    if ranging >= 0.75 && aggressor_cost >= 4.0 { AutoOrderType::PostOnlyLimit }
    else if ranging >= 0.6 && mispricing >= 5.0 && mispricing < 8.0 && aggressor_cost < 4.0 { AutoOrderType::Gtd }
    else if ranging >= 0.65 && mispricing >= 8.0 { AutoOrderType::Limit }
    else if volatility >= 1.0 && trend_units >= 1.5 { AutoOrderType::StopLimit }
    else if trend_units >= 2.5 && signal.strength >= 0.65 { AutoOrderType::StopMarket }
    else if pressure >= 0.6 && signal.strength >= 0.6 { AutoOrderType::Market }
    else if momentum >= 0.12 && signal.strength >= 0.65 { AutoOrderType::Ioc }
    else if signal.strength >= 0.6 { AutoOrderType::Vwap }
    else if ranging >= 0.5 { AutoOrderType::Twap }
    else if volatility >= 0.7 { AutoOrderType::TrailingStop }
    else if signal.strength >= 0.5 { AutoOrderType::Fok }
    else { AutoOrderType::Ioc }
}

async fn submit_configured_order(client: &reqwest::Client, base: &str, symbol: &str, side: &str, quantity: f64, notional: f64, bid: f64, ask: f64, kind: AutoOrderType) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut body = serde_json::json!({ "symbol": symbol, "side": side, "quantity": quantity, "measure_execution_quality": true });
    let endpoint = match kind {
        AutoOrderType::Vwap | AutoOrderType::Twap => {
            body["duration_minutes"] = serde_json::json!(5);
            if matches!(kind, AutoOrderType::Vwap) { body["max_participation_rate"] = serde_json::json!(0.2); }
            if matches!(kind, AutoOrderType::Twap) { body["strategy"] = serde_json::json!("twap"); }
            "/positions/vwap"
        }
        _ => {
            body["order_type"] = serde_json::json!(match kind { AutoOrderType::Market => "market", AutoOrderType::Ioc => "ioc", AutoOrderType::Fok => "fok", AutoOrderType::Limit | AutoOrderType::Gtd => "limit", AutoOrderType::PostOnlyLimit => "post-only-limit", AutoOrderType::Barrier => "barrier", AutoOrderType::StopMarket => "stop-market", AutoOrderType::StopLimit => "stop-limit", AutoOrderType::TrailingStop => "trailing-stop", _ => unreachable!() });
            match kind {
                AutoOrderType::Limit => body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask }),
                AutoOrderType::Gtd => { body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask }); body["time_in_force"] = serde_json::json!("gtd"); body["expires_at"] = serde_json::json!(SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64 + 300_000); },
                AutoOrderType::PostOnlyLimit => body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask }),
                AutoOrderType::Barrier | AutoOrderType::StopMarket => {
                    body["trigger_price"] = serde_json::json!(if side == "buy" { ask * 1.002 } else { bid * 0.998 });
                    body["trigger_direction"] = serde_json::json!(if side == "buy" { "above" } else { "below" });
                }
                AutoOrderType::StopLimit => {
                    let trigger = if side == "buy" { ask * 1.002 } else { bid * 0.998 };
                    body["trigger_price"] = serde_json::json!(trigger);
                    body["trigger_direction"] = serde_json::json!(if side == "buy" { "above" } else { "below" });
                    body["limit_price"] = serde_json::json!(if side == "buy" { trigger * 1.001 } else { trigger * 0.999 });
                }
                AutoOrderType::TrailingStop => body["trailing_percent"] = serde_json::json!(0.001),
                _ => {}
            }
            "/positions/orders"
        }
    };
    let response = client.post(format!("{base}{endpoint}")).json(&body).send().await.map_err(upstream_error)?;
    let status = response.status();
    let payload = response.json::<serde_json::Value>().await.map_err(upstream_error)?;
    if !status.is_success() { return Err((StatusCode::BAD_GATEWAY, format!("Configured paper order rejected ({status}): {payload}"))); }
    let _ = notional;
    Ok(payload)
}


fn adapt_exit_policy(settings: &mut Settings) {
    let samples = settings.closed_trade_returns_percent.len();
    if samples < MIN_EXIT_POLICY_SAMPLES || samples % 5 != 0 { return; }
    let recent = &settings.closed_trade_returns_percent[samples.saturating_sub(20)..];
    let average = recent.iter().sum::<f64>() / recent.len() as f64;
    let positive_rate = recent.iter().filter(|value| **value > 0.0).count() as f64 / recent.len() as f64;
    if average < 0.0 {
        // Poor signal outcomes do not justify tightening the stop: that makes the
        // system exit closer to normal quote noise and can turn losses into churn.
        // Keep the risk boundary fixed until we have evidence to change it.
        settings.take_profit_percent = (settings.take_profit_percent - EXIT_POLICY_ADJUSTMENT_STEP).max(MIN_TAKE_PROFIT_PERCENT);
    } else if positive_rate >= 0.60 {
        settings.take_profit_percent = (settings.take_profit_percent + EXIT_POLICY_ADJUSTMENT_STEP).min(MAX_TAKE_PROFIT_PERCENT);
    } else if positive_rate < 0.45 {
        settings.take_profit_percent = (settings.take_profit_percent - EXIT_POLICY_ADJUSTMENT_STEP).max(MIN_TAKE_PROFIT_PERCENT);
    }
    settings.stop_loss_percent = settings.stop_loss_percent.clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    settings.take_profit_percent = settings.take_profit_percent.clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT);
}

fn skipped(reason: &str) -> CycleResult { CycleResult { status: "skipped", reason: Some(reason.into()), symbol: None, side: None, quantity: None, order_type: None, order_response: None } }
fn upstream_error(error: reqwest::Error) -> (StatusCode, String) { (StatusCode::BAD_GATEWAY, format!("Auto trader upstream request failed: {error}")) }
fn settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AUTO_TRADER_STATE_FILE") { return PathBuf::from(path); }
    std::env::var_os("POSITIONS_FILE").map(PathBuf::from).and_then(|path| path.parent().map(|parent| parent.join("auto_trader_state.json"))).unwrap_or_else(|| PathBuf::from("auto_trader_state.json"))
}
fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), (StatusCode, String)> {
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes).and_then(|_| std::fs::rename(&temporary, path)).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("Could not save Auto Trader settings: {error}")))
}
fn utc_day() -> String {
    let days = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() / 86_400;
    days.to_string()
}
fn allocated_notional(settings: &Settings) -> f64 {
    let open_positions = settings.positions.values().map(|position| position.quantity.abs() * position.average_entry_price.max(0.0)).sum::<f64>();
    let mut pending = BTreeMap::<String, f64>::new();
    let mut pending_quantity = BTreeMap::<String, f64>::new();
    for order in &settings.orders {
        if !matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "unknown") || order.quantity <= 0.0 { continue; }
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 { continue; }
        let current_quantity = settings.positions.get(&order.symbol).map(|position| position.quantity).unwrap_or(0.0)
            + pending_quantity.get(&order.symbol).copied().unwrap_or(0.0);
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let increase_quantity = if current_quantity == 0.0 || current_quantity.signum() == direction { remaining }
            else { (remaining - current_quantity.abs()).max(0.0) };
        let reference_notional = if order.reserved_notional > 0.0 { order.reserved_notional } else { order.average_price.unwrap_or(0.0) * order.quantity };
        *pending.entry(order.symbol.clone()).or_default() += reference_notional * increase_quantity / order.quantity;
        *pending_quantity.entry(order.symbol.clone()).or_default() += direction * remaining;
    }
    open_positions + pending.values().sum::<f64>()
}
fn reset_daily_budget(settings: &mut Settings) {
    let day = utc_day();
    if settings.utc_day != day { settings.utc_day = day.clone(); }
    settings.used_notional_today = allocated_notional(settings);
    if settings.pnl_utc_day != day { settings.pnl_utc_day = day; settings.daily_realized_pnl = 0.0; settings.daily_loss_limit_reduced = false; }
}
