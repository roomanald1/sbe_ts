#[derive(Clone)]
struct AutoTraderState {
    latest_daily_strategy_snapshot: Arc<Mutex<Option<DailyStrategySnapshot>>>,
    latest_live_strategy_signals: Arc<Mutex<BTreeMap<(String, String), Vec<Signal>>>>,
    strategy_diagnostic: Arc<std::sync::Mutex<Vec<StrategyDiagnostic>>>,
    latest_strategy_blockers: Arc<std::sync::Mutex<BTreeMap<String, StrategyBlocker>>>,
    enabled: Arc<AtomicBool>,
    settings: Arc<Mutex<Settings>>,
    settings_persist_notify: Arc<Notify>,
    price_path_last_checkpoint_requested_at: Arc<std::sync::Mutex<Option<Instant>>>,
    state_updates: broadcast::Sender<()>,
    settings_persist_timings_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    latest_missed_opportunity_bands: Arc<Mutex<Arc<Vec<MissedOpportunityGateBand>>>>,
    latest_execution_parameters: Arc<Mutex<Option<LatestExecutionParameters>>>,
    learning_response_cache: Arc<Mutex<LearningResponseCache>>,
    cycle_lock: Arc<Mutex<()>>,
    pending_daily_signals: Arc<Mutex<PendingSignalQueue>>,
    daily_signal_notify: Arc<Notify>,
    pending_live_signals: Arc<Mutex<PendingSignalQueue>>,
    live_signal_notify: Arc<Notify>,
    signal_book_prewarms: Arc<Mutex<BTreeMap<String, Instant>>>,
    audit_tx: AuditQueue,
    execution_audit_tx: AuditQueue,
    // Bit 0 tracks decision audit persistence; bit 1 tracks execution audit persistence.
    audit_health: Arc<AtomicU64>,
    audit_failure_latched: Arc<AtomicBool>,
    gap_summary: Arc<std::sync::Mutex<GapSummary>>,
    stale_signal_candidates_total: Arc<AtomicU64>,
    stale_signal_cycles_total: Arc<AtomicU64>,
    insight_processing_ms: Arc<Mutex<Vec<f64>>>,
    cycle_lock_wait_ms: Arc<Mutex<BTreeMap<String, Vec<f64>>>>,
    cycle_lock_hold_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    cycle_lock_scope_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    settings_path: PathBuf,
    dated_carry: Arc<Mutex<DatedCarryBook>>,
    dated_carry_work_lock: Arc<Mutex<()>>,
    dated_carry_path: PathBuf,
}

#[derive(Default)]
struct LearningResponseCache {
    responses: BTreeMap<String, (Instant, serde_json::Value)>,
    refreshing: BTreeSet<String>,
}

// Context thresholds are advisory and are requested for many symbol/strategy
// contexts. Give Learning enough time to service a cold request, while keeping
// refreshes sparse enough that a busy Learning process is not flooded.
const LEARNING_RESPONSE_REFRESH_AFTER: Duration = Duration::from_secs(5);
const LEARNING_RESPONSE_MAX_STALE: Duration = Duration::from_secs(30);
const LEARNING_RESPONSE_CACHE_CAPACITY: usize = 128;
const LATEST_STRATEGY_SIGNAL_MAX_AGE_MS: u64 = 10_000;
const SIGNAL_BOOK_PREWARM_CAPACITY: usize = 4_096;

fn bound_latest_strategy_signals(
    latest: &mut BTreeMap<(String, String), Vec<Signal>>,
    active_position_keys: &BTreeSet<(String, String)>,
    now_ms: u64,
) {
    latest.retain(|key, _| active_position_keys.contains(key));
    for signals in latest.values_mut() {
        signals.retain(|signal| now_ms.saturating_sub(signal.timestamp) <= LATEST_STRATEGY_SIGNAL_MAX_AGE_MS);
        if signals.len() > 1 {
            let newest = signals.iter().max_by_key(|signal| signal.timestamp).cloned();
            signals.clear();
            if let Some(signal) = newest { signals.push(signal); }
        }
    }
    latest.retain(|_, signals| !signals.is_empty());
}

fn bound_signal_book_prewarms(prewarms: &mut BTreeMap<String, Instant>, now: Instant) {
    prewarms.retain(|_, requested_at| now.duration_since(*requested_at) < Duration::from_secs(300));
    while prewarms.len() > SIGNAL_BOOK_PREWARM_CAPACITY {
        let oldest = prewarms.iter().min_by_key(|(_, requested_at)| **requested_at)
            .map(|(symbol, _)| symbol.clone());
        let Some(oldest) = oldest else { break };
        prewarms.remove(&oldest);
    }
}

async fn cached_learning_response<T>(
    response_cache: &Arc<Mutex<LearningResponseCache>>,
    cache_key: String,
    path: String,
    request: serde_json::Value,
) -> Option<T>
where
    T: serde::de::DeserializeOwned + Send + 'static,
{
    let (cached, refresh) = {
        let mut cache = response_cache.lock().await;
        let cached = cache.responses.get(&cache_key).and_then(|(updated_at, value)| {
            (updated_at.elapsed() <= LEARNING_RESPONSE_MAX_STALE).then(|| value.clone())
        });
        let should_refresh = cache
            .responses
            .get(&cache_key)
            .is_none_or(|(updated_at, _)| updated_at.elapsed() >= LEARNING_RESPONSE_REFRESH_AFTER);
        let refresh = should_refresh && cache.refreshing.insert(cache_key.clone());
        (cached, refresh)
    };

    if refresh {
        let response_cache = response_cache.clone();
        let key = cache_key.clone();
        tokio::spawn(async move {
            let response = tokio::time::timeout(
                Duration::from_secs(2),
                crate::service_rpc::learning_request(
                    strategy::ScoutWorkerHttpMethod::Post,
                    &path,
                    Some(request),
                ),
            ).await;
            let value = match response {
                Ok(Ok(response)) if (200..300).contains(&response.status) => Some(response.body),
                Ok(Ok(response)) => {
                    crate::service_log!("WARN", "Learning returned status {} for {}", response.status, path);
                    None
                }
                Ok(Err(error)) => {
                    crate::service_log!("WARN", "Learning service refresh failed: {error}");
                    None
                }
                Err(_) => {
                    crate::service_log!("WARN", "Learning service refresh timed out for {}", path);
                    None
                }
            };
            let mut cache = response_cache.lock().await;
            cache.refreshing.remove(&key);
            if let Some(value) = value {
                cache.responses.insert(key, (Instant::now(), value));
                if cache.responses.len() > LEARNING_RESPONSE_CACHE_CAPACITY {
                    cache.responses.retain(|_, (updated_at, _)| {
                        updated_at.elapsed() <= LEARNING_RESPONSE_MAX_STALE
                    });
                    while cache.responses.len() > LEARNING_RESPONSE_CACHE_CAPACITY {
                        let oldest = cache
                            .responses
                            .iter()
                            .min_by_key(|(_, (updated_at, _))| *updated_at)
                            .map(|(key, _)| key.clone());
                        let Some(oldest) = oldest else { break };
                        cache.responses.remove(&oldest);
                    }
                }
            } else if !cache.responses.contains_key(&key) {
                // Back off failed cold misses so a busy signal stream cannot
                // turn an unavailable Learning service into an HTTP storm.
                cache
                    .responses
                    .insert(key, (Instant::now(), serde_json::Value::Null));
            }
        });
    }

    cached.and_then(|value| serde_json::from_value(value).ok())
}

#[cfg(test)]
mod learning_response_cache_tests {
    use super::*;

    #[tokio::test]
    async fn returns_recent_cached_response_without_a_service_request() {
        let response_cache = Arc::new(Mutex::new(LearningResponseCache::default()));
        let cache_key = "execution-decision:BTC-USDT".to_owned();
        let expected = serde_json::json!({ "learned_order_type": "PostOnlyLimit" });
        response_cache.lock().await.responses.insert(
            cache_key.clone(),
            (Instant::now(), expected.clone()),
        );

        let response = cached_learning_response::<serde_json::Value>(
            &response_cache,
            cache_key,
            "/execution-decision".to_owned(),
            serde_json::Value::Null,
        )
        .await;

        assert_eq!(response, Some(expected));
    }
}


const DECISION_AUDIT_QUEUE_CAPACITY: usize = 16_384;
const EXECUTION_AUDIT_QUEUE_CAPACITY: usize = 4_096;
const DECISION_AUDIT_QUEUE_MAX_BYTES: usize = 8 * 1024 * 1024;
const EXECUTION_AUDIT_QUEUE_MAX_BYTES: usize = 8 * 1024 * 1024;

struct QueuedAudit {
    payload: serde_json::Value,
    estimated_bytes: usize,
}

#[derive(Default)]
struct AuditQueueContents {
    messages: std::collections::VecDeque<QueuedAudit>,
    estimated_bytes: usize,
}

#[derive(Clone)]
struct AuditQueue {
    messages: Arc<std::sync::Mutex<AuditQueueContents>>,
    notify: Arc<Notify>,
    capacity: usize,
    max_bytes: usize,
    coalesced: Arc<AtomicU64>,
    dropped: Arc<AtomicU64>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AuditEnqueueResult {
    Queued,
    Coalesced,
    EvictedOld,
    Dropped,
}

impl AuditQueue {
    fn new(capacity: usize, max_bytes: usize) -> Self {
        Self {
            messages: Arc::new(std::sync::Mutex::new(AuditQueueContents::default())),
            notify: Arc::new(Notify::new()),
            capacity,
            max_bytes,
            coalesced: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    fn enqueue(&self, payload: serde_json::Value) -> AuditEnqueueResult {
        let replaceable = is_replaceable_audit(&payload);
        let symbol = payload.get("symbol").and_then(serde_json::Value::as_str);
        let estimated_bytes = estimate_json_value_bytes(&payload);
        let mut messages = self.messages.lock().unwrap_or_else(|error| error.into_inner());

        if estimated_bytes > self.max_bytes {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return AuditEnqueueResult::Dropped;
        }

        if replaceable {
            if let Some(symbol) = symbol {
                if let Some(index) = messages.messages.iter().position(|existing| {
                    is_replaceable_audit(&existing.payload)
                        && existing.payload.get("symbol").and_then(serde_json::Value::as_str) == Some(symbol)
                }) {
                    let existing_bytes = messages.messages[index].estimated_bytes;
                    if audit_timestamp(&payload) >= audit_timestamp(&messages.messages[index].payload) {
                        let projected = messages.estimated_bytes - existing_bytes + estimated_bytes;
                        if projected <= self.max_bytes {
                            messages.messages[index] = QueuedAudit { payload, estimated_bytes };
                            messages.estimated_bytes = projected;
                        } else {
                            self.dropped.fetch_add(1, Ordering::Relaxed);
                            return AuditEnqueueResult::Dropped;
                        }
                    }
                    self.coalesced.fetch_add(1, Ordering::Relaxed);
                    self.notify.notify_one();
                    return AuditEnqueueResult::Coalesced;
                }
            }
        }

        let mut result = AuditEnqueueResult::Queued;
        while messages.messages.len() >= self.capacity
            || messages.estimated_bytes.saturating_add(estimated_bytes) > self.max_bytes
        {
            if let Some(index) = messages.messages.iter().position(|queued| is_replaceable_audit(&queued.payload)) {
                if let Some(evicted) = messages.messages.remove(index) {
                    messages.estimated_bytes = messages.estimated_bytes.saturating_sub(evicted.estimated_bytes);
                }
                self.dropped.fetch_add(1, Ordering::Relaxed);
                result = AuditEnqueueResult::EvictedOld;
            } else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return AuditEnqueueResult::Dropped;
            }
        }
        messages.estimated_bytes += estimated_bytes;
        messages.messages.push_back(QueuedAudit { payload, estimated_bytes });
        self.notify.notify_one();
        result
    }

    async fn recv(&self) -> serde_json::Value {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            let queued = {
                let mut contents = self.messages.lock().unwrap_or_else(|error| error.into_inner());
                let queued = contents.messages.pop_front();
                if let Some(queued) = &queued {
                    contents.estimated_bytes = contents.estimated_bytes.saturating_sub(queued.estimated_bytes);
                }
                queued
            };
            if let Some(payload) = queued
            {
                return payload.payload;
            }
            notified.await;
        }
    }

    fn depth(&self) -> usize {
        self.messages.lock().unwrap_or_else(|error| error.into_inner()).messages.len()
    }

    fn estimated_bytes(&self) -> usize {
        self.messages.lock().unwrap_or_else(|error| error.into_inner()).estimated_bytes
    }
}

fn estimate_json_value_bytes(value: &serde_json::Value) -> usize {
    match value {
        serde_json::Value::Null | serde_json::Value::Bool(_) | serde_json::Value::Number(_) => 32,
        serde_json::Value::String(value) => 32 + value.len(),
        serde_json::Value::Array(values) => 24 + values.iter().map(estimate_json_value_bytes).sum::<usize>(),
        serde_json::Value::Object(values) => 24 + values.iter().map(|(key, value)| 32 + key.len() + estimate_json_value_bytes(value)).sum::<usize>(),
    }
}

fn audit_timestamp(payload: &serde_json::Value) -> u64 {
    payload.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or_default()
}

fn is_replaceable_audit(payload: &serde_json::Value) -> bool {
    payload.get("stage").and_then(serde_json::Value::as_str) == Some("decision")
        && matches!(
            payload.get("status").and_then(serde_json::Value::as_str),
            Some("rejected" | "skipped")
        )
}

#[cfg(test)]
mod audit_queue_tests {
    use super::*;

    fn audit(stage: &str, status: &str, symbol: &str, timestamp: u64, detail: &str) -> serde_json::Value {
        serde_json::json!({
            "stage": stage,
            "status": status,
            "symbol": symbol,
            "timestamp": timestamp,
            "trade_id": format!("trade-{symbol}"),
            "detail": detail,
        })
    }

    #[test]
    fn decision_queue_coalesces_by_symbol_and_accounts_for_replaced_bytes() {
        let queue = AuditQueue::new(8, 512);
        assert_eq!(queue.enqueue(audit("decision", "rejected", "BTC", 1, "old")), AuditEnqueueResult::Queued);
        let bytes_before = queue.estimated_bytes();

        assert_eq!(queue.enqueue(audit("decision", "rejected", "BTC", 2, "newer")), AuditEnqueueResult::Coalesced);

        assert_eq!(queue.depth(), 1);
        assert!(queue.estimated_bytes() > bytes_before);
        assert!(queue.estimated_bytes() <= 512);
        assert_eq!(queue.coalesced.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn audit_queue_evicts_replaceable_decisions_before_exceeding_byte_budget() {
        let queue = AuditQueue::new(8, 700);
        for index in 0..3 {
            let payload = audit("decision", "rejected", &format!("S{index}"), index, &"x".repeat(40));
            let result = queue.enqueue(payload);
            assert!(matches!(result, AuditEnqueueResult::Queued | AuditEnqueueResult::EvictedOld));
        }
        assert!(queue.estimated_bytes() <= 700);
        assert!(matches!(
            queue.enqueue(audit("decision", "selected", "LIVE", 10, "critical")),
            AuditEnqueueResult::Queued | AuditEnqueueResult::EvictedOld
        ));
        assert!(queue.estimated_bytes() <= 700);
        assert!(queue.dropped.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn execution_queue_never_evicts_critical_records_to_admit_another() {
        let queue = AuditQueue::new(8, 1_000);
        let first = audit("exit_fill", "filled", "BTC", 1, &"x".repeat(30));
        let second = audit("exit_fill", "filled", "ETH", 2, &"y".repeat(30));
        assert_eq!(queue.enqueue(first.clone()), AuditEnqueueResult::Queued);
        assert_eq!(queue.enqueue(second), AuditEnqueueResult::Dropped);
        assert_eq!(queue.depth(), 1);
        assert!(queue.estimated_bytes() <= 1_000);
        assert_eq!(queue.dropped.load(Ordering::Relaxed), 1);
    }
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
    #[serde(default)]
    strategy_realized_pnl: Option<f64>,
    #[serde(default)]
    strategy_realized_pnl_today: Option<f64>,
    #[serde(default)]
    strategy_realized_pnl_day: Option<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
struct TraderPosition {
    quantity: f64,
    average_entry_price: f64,
    entry_fees: f64,
    realized_pnl: f64,
    #[serde(default)]
    initial_entry_notional: f64,
    #[serde(default)]
    total_fees_paid: f64,
    #[serde(default)]
    total_entry_fees_paid: f64,
    #[serde(default)]
    entries: u32,
    #[serde(default)]
    entries_by_category: BTreeMap<String, u32>,
    #[serde(default)]
    entry_order_type: String,
    #[serde(default)]
    entry_order_types: Vec<String>,
    #[serde(default)]
    entry_context: String,
    #[serde(default)]
    entry_decision_id: Option<String>,
    // Stable identifier for the lifetime of this strategy/symbol position.
    // It is initialized from the first entry decision and survives add-on fills.
    #[serde(default)]
    trade_id: Option<String>,
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
    #[serde(default, alias = "scout_profit_peak_return_percent")]
    profit_protection_peak_return_percent: Option<f64>,
    #[serde(default)]
    thesis_failure_confirmation_count: u32,
    #[serde(default)]
    thesis_failure_last_signal_timestamp_ms: u64,
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
    // Ordered recent outcomes used by strategy sizing so old results cannot dominate
    // current win-rate or net-P&L estimates.
    #[serde(default)]
    recent_trade_pnls: Vec<(u64, f64)>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct ExecutionLearningStats {
    samples: usize,
    filled_samples: usize,
    fill_rate_total: f64,
    cost_notional: f64,
    all_in_cost_notional_bps: f64,
    #[serde(default)]
    updated_at_ms: u64,
}

#[derive(Clone, Copy, Eq, PartialEq, Ord, PartialOrd)]
enum PendingSignalKind {
    Daily,
    Live,
}

type PendingSignalQueue = BTreeMap<(PendingSignalKind, String), Dashboard>;

// A feed may report a large universe, while the cycle worker can submit only
// a small batch per pass. Keep the freshest symbols while bounding retained
// dashboards during bursts or malformed/unexpected symbol floods.
const DEFAULT_MAX_PENDING_LIVE_SYMBOLS: usize = 256;

fn max_pending_live_symbols() -> usize {
    static LIMIT: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *LIMIT.get_or_init(|| {
        std::env::var("AUTO_TRADER_MAX_PENDING_LIVE_SYMBOLS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(DEFAULT_MAX_PENDING_LIVE_SYMBOLS)
    })
}

fn queue_pending_signal(
    queue: &mut PendingSignalQueue,
    kind: PendingSignalKind,
    symbol: String,
    dashboard: Dashboard,
) {
    queue_pending_signal_with_cap(queue, kind, symbol, dashboard, max_pending_live_symbols());
}

fn queue_pending_signal_with_cap(
    queue: &mut PendingSignalQueue,
    kind: PendingSignalKind,
    symbol: String,
    mut dashboard: Dashboard,
    live_cap: usize,
) {
    retain_pending_dashboard_market(&mut dashboard, &symbol);
    let key = (kind, symbol);
    let incoming_timestamp = dashboard
        .best
        .iter()
        .map(|signal| signal.timestamp)
        .max()
        .unwrap_or_default();
    match queue.entry(key) {
        std::collections::btree_map::Entry::Occupied(mut entry) => {
            let existing_timestamp = entry
                .get()
                .best
                .iter()
                .map(|signal| signal.timestamp)
                .max()
                .unwrap_or_default();
            if incoming_timestamp >= existing_timestamp {
                dashboard.queued_at = Some(Instant::now());
                entry.insert(dashboard);
            }
        }
        std::collections::btree_map::Entry::Vacant(entry) => {
            dashboard.queued_at = Some(Instant::now());
            entry.insert(dashboard);
        }
    }
    let live_count = queue
        .keys()
        .filter(|(queued_kind, _)| *queued_kind == PendingSignalKind::Live)
        .count();
    if live_count > live_cap {
        let oldest = queue
            .iter()
            .filter(|((queued_kind, _), _)| *queued_kind == PendingSignalKind::Live)
            .min_by_key(|((_, symbol), dashboard)| {
                (
                    dashboard
                        .best
                        .iter()
                        .map(|signal| signal.timestamp)
                        .max()
                        .unwrap_or_default(),
                    symbol.as_str(),
                )
            })
            .map(|(key, _)| key.clone());
        if let Some(oldest) = oldest {
            queue.remove(&oldest);
        }
    }
}

fn retain_pending_dashboard_market(dashboard: &mut Dashboard, symbol: &str) {
    let market = dashboard.markets.get(symbol).cloned();
    dashboard.markets = Arc::new(market.map(|market| BTreeMap::from([(symbol.to_owned(), market)])).unwrap_or_default());
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

const MAX_LIVE_SIGNALS_PER_CYCLE: usize = 32;

fn take_next_pending_signal_batch(
    queue: &mut PendingSignalQueue,
    max_signals: usize,
) -> Option<Dashboard> {
    let mut keys = queue
        .iter()
        .map(|(key, dashboard)| {
            let timestamp = dashboard
                .best
                .iter()
                .map(|signal| signal.timestamp)
                .max()
                .unwrap_or_default();
            (key.clone(), timestamp)
        })
        .collect::<Vec<_>>();
    keys.sort_by(|(left_key, left_timestamp), (right_key, right_timestamp)| {
        right_timestamp
            .cmp(left_timestamp)
            .then_with(|| left_key.cmp(right_key))
    });
    keys.truncate(max_signals.max(1));

    let mut dashboards = keys
        .into_iter()
        .filter_map(|(key, _)| queue.remove(&key))
        .collect::<Vec<_>>();
    if dashboards.is_empty() {
        return None;
    }

    let mut batch = dashboards.remove(0);
    let mut markets = batch.markets.as_ref().clone();
    let mut oldest_queued_at = batch.queued_at.take();
    for dashboard in dashboards {
        batch.best.extend(dashboard.best);
        for (symbol, market) in dashboard.markets.iter() {
            markets
                .entry(symbol.clone())
                .or_insert_with(|| market.clone());
        }
        if let Some(queued_at) = dashboard.queued_at {
            oldest_queued_at = Some(
                oldest_queued_at
                    .map_or(queued_at, |oldest| oldest.min(queued_at)),
            );
        }
    }
    batch.markets = Arc::new(markets);
    batch.queued_at = oldest_queued_at;
    batch.queue_wait_ms = batch
        .queued_at
        .take()
        .map(|queued_at| queued_at.elapsed().as_secs_f64() * 1_000.0);
    Some(batch)
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
                news_attribution: None,
                entry_gate: None,
            }],
            missed_opportunity_bands: Arc::new(Vec::new()),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        }
    }

    fn strategy_signal(symbol: &str, timestamp: u64) -> Signal {
        Signal {
            symbol: symbol.into(), direction: "bullish".into(), strength: 1.0,
            timestamp, stale: false, score: 1.0, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: true,
            strategy: Some("live-momentum".into()), strategy_decision: None,
            market_regime: None, news_attribution: None, entry_gate: None,
        }
    }

    #[test]
    fn latest_strategy_signal_cache_keeps_only_the_freshest_signal_and_expires_old_keys() {
        let key = ("BTC-USDT".to_owned(), "live-momentum".to_owned());
        let mut latest = BTreeMap::from([(key.clone(), vec![
            strategy_signal("BTC-USDT", 198_000), strategy_signal("BTC-USDT", 199_000),
        ])]);
        latest.insert(("OLD-USDT".into(), "live-momentum".into()),
            vec![strategy_signal("OLD-USDT", 1)]);

        bound_latest_strategy_signals(&mut latest, &BTreeSet::from([key.clone()]), 200_000);

        assert_eq!(latest.len(), 1);
        assert_eq!(latest[&key].len(), 1);
        assert_eq!(latest[&key][0].timestamp, 199_000);
    }

    #[test]
    fn signal_book_prewarm_cache_is_capped_and_expires_old_entries() {
        let now = Instant::now();
        let mut prewarms = BTreeMap::new();
        prewarms.insert("EXPIRED".into(), now - Duration::from_secs(301));
        for index in 0..=SIGNAL_BOOK_PREWARM_CAPACITY {
            prewarms.insert(format!("S{index}"), now - Duration::from_millis(index as u64));
        }

        bound_signal_book_prewarms(&mut prewarms, now);

        assert_eq!(prewarms.len(), SIGNAL_BOOK_PREWARM_CAPACITY);
        assert!(!prewarms.contains_key("EXPIRED"));
    }

    #[test]
    fn daily_and_live_signals_use_independent_queues() {
        let mut daily_queue = PendingSignalQueue::new();
        let mut live_queue = PendingSignalQueue::new();
        queue_pending_signal(
            &mut daily_queue,
            PendingSignalKind::Daily,
            "BNB-USDT".into(),
            dashboard(true),
        );
        queue_pending_signal(
            &mut live_queue,
            PendingSignalKind::Live,
            "BNB-USDT".into(),
            dashboard(false),
        );
        queue_pending_signal(
            &mut live_queue,
            PendingSignalKind::Live,
            "BNB-USDT".into(),
            dashboard(false),
        );

        assert_eq!(daily_queue.len(), 1);
        assert_eq!(live_queue.len(), 1);
        assert!(!take_next_pending_signal(&mut live_queue).unwrap().best[0].strategy_candidate);
        assert!(take_next_pending_signal(&mut daily_queue).unwrap().best[0].strategy_candidate);
    }

    #[test]
    fn live_signal_batch_takes_freshest_symbols_and_merges_their_market_books() {
        let mut queue = PendingSignalQueue::new();
        for (symbol, timestamp) in [("BTC-USDT", 10), ("ETH-USDT", 30), ("SOL-USDT", 20)] {
            let mut dashboard = dashboard(false);
            dashboard.best[0].symbol = symbol.into();
            dashboard.best[0].timestamp = timestamp;
            dashboard.markets = Arc::new(BTreeMap::from([(
                symbol.into(),
                serde_json::json!({ "updated_at_ms": timestamp }),
            )]));
            queue_pending_signal(
                &mut queue,
                PendingSignalKind::Live,
                symbol.into(),
                dashboard,
            );
        }

        let batch = take_next_pending_signal_batch(&mut queue, 2).unwrap();
        assert_eq!(
            batch
                .best
                .iter()
                .map(|signal| signal.symbol.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["ETH-USDT", "SOL-USDT"])
        );
        assert!(batch.markets.contains_key("ETH-USDT"));
        assert!(batch.markets.contains_key("SOL-USDT"));
        assert_eq!(queue.len(), 1);
        assert!(queue.contains_key(&(PendingSignalKind::Live, "BTC-USDT".into())));
    }

    #[test]
    fn pending_signal_queue_discards_oldest_live_symbols_at_capacity() {
        let mut queue = PendingSignalQueue::new();
        queue_pending_signal_with_cap(
            &mut queue,
            PendingSignalKind::Daily,
            "DAILY-KEEP".into(),
            dashboard(true),
            2,
        );
        for timestamp in 1..=3 {
            let symbol = format!("ASSET-{timestamp}");
            let mut dashboard = dashboard(false);
            dashboard.best[0].symbol = symbol.clone();
            dashboard.best[0].timestamp = timestamp;
            queue_pending_signal_with_cap(
                &mut queue,
                PendingSignalKind::Live,
                symbol,
                dashboard,
                2,
            );
        }

        assert_eq!(queue.len(), 3);
        assert!(queue.contains_key(&(PendingSignalKind::Daily, "DAILY-KEEP".into())));
        assert!(!queue.contains_key(&(PendingSignalKind::Live, "ASSET-1".into())));
        assert!(queue.contains_key(&(PendingSignalKind::Live, "ASSET-3".into())));
    }

    #[test]
    fn pending_signal_queue_keeps_only_each_symbol_market_and_merges_batch_books() {
        let full_markets = Arc::new(BTreeMap::from([
            ("BTC-USDT".into(), serde_json::json!({"bids":[{"price":100.0,"size":1.0}]})),
            ("ETH-USDT".into(), serde_json::json!({"bids":[{"price":50.0,"size":2.0}]})),
            ("SOL-USDT".into(), serde_json::json!({"bids":[{"price":25.0,"size":3.0}]})),
        ]));
        let mut queue = PendingSignalQueue::new();
        for symbol in ["BTC-USDT", "ETH-USDT"] {
            let mut item = dashboard(false);
            item.best[0].symbol = symbol.into();
            item.markets = Arc::clone(&full_markets);
            queue_pending_signal_with_cap(
                &mut queue,
                PendingSignalKind::Live,
                symbol.into(),
                item,
                256,
            );
            let queued = &queue[&(PendingSignalKind::Live, symbol.into())];
            assert_eq!(queued.markets.len(), 1);
            assert!(queued.markets.contains_key(symbol));
        }

        let batch = take_next_pending_signal_batch(&mut queue, 2).unwrap();
        assert_eq!(batch.markets.len(), 2);
        assert!(batch.markets.contains_key("BTC-USDT"));
        assert!(batch.markets.contains_key("ETH-USDT"));
        assert!(!batch.markets.contains_key("SOL-USDT"));
    }

    #[test]
    fn same_symbol_update_replaces_older_snapshot_without_growing_queue() {
        let mut queue = PendingSignalQueue::new();
        for timestamp in [2, 1] {
            let mut dashboard = dashboard(false);
            dashboard.best[0].timestamp = timestamp;
            queue_pending_signal_with_cap(
                &mut queue,
                PendingSignalKind::Live,
                "BTC-USDT".into(),
                dashboard,
                1,
            );
        }

        assert_eq!(queue.len(), 1);
        assert_eq!(queue.values().next().unwrap().best[0].timestamp, 2);
        assert!(queue.values().next().unwrap().queued_at.is_some());
    }

    #[test]
    fn batched_signal_retains_queue_wait_measurement() {
        let mut queue = PendingSignalQueue::new();
        queue_pending_signal_with_cap(
            &mut queue,
            PendingSignalKind::Live,
            "BTC-USDT".into(),
            dashboard(false),
            1,
        );
        let batch = take_next_pending_signal_batch(&mut queue, 1).unwrap();
        assert!(batch.queue_wait_ms.is_some_and(|wait| wait >= 0.0));
    }
}
