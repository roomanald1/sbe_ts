#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
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
    #[serde(default = "default_usd_rates")]
    usd_rates: BTreeMap<String, f64>,
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
    #[serde(default = "default_closed_trade_window")]
    closed_trade_window: usize,
    #[serde(default)]
    closed_trade_returns_percent: Vec<f64>,
    #[serde(default)]
    closed_trade_timestamps_ms: Vec<u64>,
    #[serde(default)]
    closed_trade_realized_usd: Vec<f64>,
    #[serde(default)]
    closed_trade_fees_usd: Vec<f64>,
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
fn default_risk_appetite() -> f64 {
    50.0
}
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
fn default_closed_trade_window() -> usize {
    200
}
fn default_daily_limit() -> f64 {
    DEFAULT_DAILY_NOTIONAL
}
fn default_usd_rates() -> BTreeMap<String, f64> {
    BTreeMap::from([
        ("USD".to_owned(), 1.0),
        ("USDT".to_owned(), 1.0),
        ("USDC".to_owned(), 1.0),
    ])
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            max_order_notional: default_order_limit(),
            daily_notional_limit: default_daily_limit(),
            used_notional_today: 0.0,
            usd_rates: default_usd_rates(),
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
            closed_trade_window: default_closed_trade_window(),
            closed_trade_returns_percent: Vec::new(),
            closed_trade_timestamps_ms: Vec::new(),
            closed_trade_realized_usd: Vec::new(),
            closed_trade_fees_usd: Vec::new(),
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
