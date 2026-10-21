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
    horizon_take_profit_percent: Option<f64>,
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
    horizon_take_profit_percent: Option<f64>,
    #[serde(default)]
    take_profit_deferred_at_ms: Option<u64>,
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
    #[serde(default)]
    updated_at_ms: u64,
    #[serde(default)]
    daily_utc_day: String,
    #[serde(default)]
    daily_samples: usize,
    #[serde(default)]
    daily_positive: usize,
    #[serde(default)]
    daily_realized_pnl: f64,
}
