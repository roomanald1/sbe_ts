#[derive(Clone)]
struct AutoTraderState {
    client: reqwest::Client,
    latest_daily_strategy_snapshot: Arc<Mutex<Option<DailyStrategySnapshot>>>,
    strategy_diagnostic: Arc<std::sync::Mutex<Vec<StrategyDiagnostic>>>,
    enabled: Arc<AtomicBool>,
    settings: Arc<Mutex<Settings>>,
    settings_persist_notify: Arc<Notify>,
    settings_persist_timings_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    latest_missed_opportunity_bands: Arc<Mutex<Vec<MissedOpportunityGateBand>>>,
    latest_execution_parameters: Arc<Mutex<Option<LatestExecutionParameters>>>,
    cycle_lock: Arc<Mutex<()>>,
    pending_signals: Arc<Mutex<PendingSignalQueue>>,
    signal_notify: Arc<Notify>,
    signal_book_prewarms: Arc<Mutex<BTreeMap<String, Instant>>>,
    audit_tx: mpsc::Sender<serde_json::Value>,
    execution_audit_tx: mpsc::UnboundedSender<serde_json::Value>,
    gap_summary: Arc<std::sync::Mutex<GapSummary>>,
    insight_processing_ms: Arc<Mutex<Vec<f64>>>,
    cycle_lock_wait_ms: Arc<Mutex<BTreeMap<String, Vec<f64>>>>,
    cycle_lock_hold_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    settings_path: PathBuf,
    dated_carry: Arc<Mutex<DatedCarryBook>>,
    dated_carry_work_lock: Arc<Mutex<()>>,
    dated_carry_path: PathBuf,
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
    #[serde(alias = "research_strategy")]
    strategy: Option<String>,
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
    entries_by_category: BTreeMap<String, u32>,
    #[serde(default)]
    entry_order_type: String,
    #[serde(default)]
    entry_context: String,
    #[serde(default)]
    entry_decision_id: Option<String>,
    #[serde(default)]
    #[serde(alias = "research_strategy")]
    strategy: Option<String>,
    #[serde(default)]
    strategy_decision: Option<serde_json::Value>,
    #[serde(default)]
    scout_entry: bool,
    #[serde(default)]
    horizon_take_profit_percent: Option<f64>,
    #[serde(default)]
    stop_loss_percent: Option<f64>,
    #[serde(default)]
    take_profit_percent: Option<f64>,
    #[serde(default)]
    take_profit_deferred_at_ms: Option<u64>,
    #[serde(default, alias = "shadow_profit_peak_return_percent")]
    profit_protection_peak_return_percent: Option<f64>,
    #[serde(default)]
    protective_stops: Vec<ProtectiveStop>,
    #[serde(default)]
    price_path: Vec<TradePriceSample>,
    #[serde(default)]
    entry_timestamp_ms: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct TradePriceSample {
    timestamp: u64,
    bid: f64,
    ask: f64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct ProtectiveStop {
    barrier_id: String,
    #[serde(default)]
    strategy: String,
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

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum PendingSignalKind {
    Daily,
    Live,
}

type PendingSignalQueue = BTreeMap<(PendingSignalKind, String), Dashboard>;

fn queue_pending_signal(
    queue: &mut PendingSignalQueue,
    kind: PendingSignalKind,
    symbol: String,
    mut dashboard: Dashboard,
) {
    dashboard.queued_at = Some(Instant::now());
    queue.insert((kind, symbol), dashboard);
}

fn take_next_pending_signal(queue: &mut PendingSignalQueue) -> Option<Dashboard> {
    // The worker submits one symbol at a time, and an order round trip can take
    // close to a second. Process the freshest queued snapshot first so backlog
    // does not make the most actionable signals stale before they reach sizing.
    let key = queue
        .iter()
        .max_by_key(|((kind, symbol), dashboard)| {
            let timestamp = dashboard
                .best
                .iter()
                .map(|signal| signal.timestamp)
                .max()
                .unwrap_or_default();
            (
                timestamp,
                std::cmp::Reverse(*kind),
                std::cmp::Reverse(symbol.as_str()),
            )
        })
        .map(|(key, _)| key.clone())?;
    let mut dashboard = queue.remove(&key)?;
    dashboard.queue_wait_ms = dashboard
        .queued_at
        .take()
        .map(|queued_at| queued_at.elapsed().as_secs_f64() * 1_000.0);
    Some(dashboard)
}

#[cfg(test)]
mod pending_signal_queue_tests {
    use super::*;

    fn dashboard(daily: bool) -> Dashboard {
        Dashboard {
            best: vec![Signal {
                symbol: "BNB-USDT".into(),
                direction: "bullish".into(),
                strength: 1.0,
                timestamp: 1,
                stale: false,
                score: 1.0,
                ranging_probability: None,
                components: SignalComponents::default(),
                strategy_candidate: daily,
                strategy: None,
                strategy_decision: None,
                market_regime: None,
            }],
            walk_forward: Arc::new(Vec::new()),
            missed_opportunity_bands: Vec::new(),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        }
    }

    #[test]
    fn daily_signal_survives_live_update_for_same_symbol() {
        let mut queue = PendingSignalQueue::new();
        queue_pending_signal(
            &mut queue,
            PendingSignalKind::Daily,
            "BNB-USDT".into(),
            dashboard(true),
        );
        queue_pending_signal(
            &mut queue,
            PendingSignalKind::Live,
            "BNB-USDT".into(),
            dashboard(false),
        );
        queue_pending_signal(
            &mut queue,
            PendingSignalKind::Live,
            "BNB-USDT".into(),
            dashboard(false),
        );
        assert_eq!(queue.len(), 2);
        assert!(take_next_pending_signal(&mut queue).unwrap().best[0].strategy_candidate);
        assert!(!take_next_pending_signal(&mut queue).unwrap().best[0].strategy_candidate);
    }
}
