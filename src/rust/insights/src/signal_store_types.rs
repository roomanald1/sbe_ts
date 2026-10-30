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
    pub total_net_pnl: Option<f64>,
    pub pnl_quote_currency: Option<String>,
    pub preliminary: bool,
}

#[derive(Default)]
struct DecisionTradeAudit {
    entry_quantity: f64,
    entry_notional: f64,
    position_entry_quantity: Option<f64>,
    trade_closed: bool,
    entry_price: Option<f64>,
    entry_side: Option<String>,
    entry_fees: f64,
    strategy_parameters: Option<serde_json::Value>,
    exploratory: Option<bool>,
    scout: bool,
    direction: Option<String>,
    strength: Option<f64>,
    market_context: Option<String>,
    strategy: Option<String>,
    quote_currency: Option<String>,
    entry_timestamp: Option<u64>,
    signal_components: Option<serde_json::Value>,
    exit_records: Vec<serde_json::Value>,
}

#[derive(Deserialize)]
struct EvaluationQuery {
    since: Option<u64>,
    limit: Option<usize>,
    cursor: Option<i64>,
}

#[derive(Clone)]
pub struct SignalStore {
    path: PathBuf,
    writer: mpsc::Sender<WriteRequest>,
    recent_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>,
    best_cache: Arc<Mutex<Option<(Instant, Vec<SignalSnapshot>)>>>,
    performance_cache: Arc<Mutex<Option<(Instant, Vec<SignalPerformance>)>>>,
    calibration_records_cache: Arc<Mutex<Option<(Instant, Arc<Vec<StoredSignalRecord>>)>>>,
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
