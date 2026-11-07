#[derive(Clone)]
struct AutoTraderState {
    client: reqwest::Client,
    latest_daily_strategy_snapshot: Arc<Mutex<Option<DailyStrategySnapshot>>>,
    strategy_diagnostic: Arc<std::sync::Mutex<Vec<StrategyDiagnostic>>>,
    enabled: Arc<AtomicBool>,
    settings: Arc<Mutex<Settings>>,
    settings_persist_notify: Arc<Notify>,
    price_path_last_checkpoint_requested_at: Arc<std::sync::Mutex<Option<Instant>>>,
    state_updates: broadcast::Sender<()>,
    settings_persist_timings_ms: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    latest_missed_opportunity_bands: Arc<Mutex<Vec<MissedOpportunityGateBand>>>,
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

const LEARNING_RESPONSE_REFRESH_AFTER: Duration = Duration::from_secs(1);
const LEARNING_RESPONSE_MAX_STALE: Duration = Duration::from_secs(30);

async fn cached_learning_response<T>(
    response_cache: &Arc<Mutex<LearningResponseCache>>,
    cache_key: String,
    client: &reqwest::Client,
    url: String,
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
        let client = client.clone();
        let key = cache_key.clone();
        tokio::spawn(async move {
            let response = client
                .post(url)
                .timeout(Duration::from_millis(250))
                .json(&request)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status);
            let value = match response {
                Ok(response) => response.json::<serde_json::Value>().await.ok(),
                Err(error) => {
                    crate::service_log!("WARN", "Learning service refresh failed: {error}");
                    None
                }
            };
            let mut cache = response_cache.lock().await;
            cache.refreshing.remove(&key);
            if let Some(value) = value {
                cache.responses.insert(key, (Instant::now(), value));
                if cache.responses.len() > 512 {
                    cache.responses.retain(|_, (updated_at, _)| {
                        updated_at.elapsed() <= LEARNING_RESPONSE_MAX_STALE
                    });
                    while cache.responses.len() > 512 {
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
            &reqwest::Client::new(),
            "http://127.0.0.1:1/execution-decision".to_owned(),
            serde_json::Value::Null,
        )
        .await;

        assert_eq!(response, Some(expected));
    }
}


const DECISION_AUDIT_QUEUE_CAPACITY: usize = 16_384;
const EXECUTION_AUDIT_QUEUE_CAPACITY: usize = 4_096;

#[derive(Clone)]
struct AuditQueue {
    messages: Arc<std::sync::Mutex<std::collections::VecDeque<serde_json::Value>>>,
    notify: Arc<Notify>,
    capacity: usize,
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
    fn new(capacity: usize) -> Self {
        Self {
            messages: Arc::new(std::sync::Mutex::new(std::collections::VecDeque::new())),
            notify: Arc::new(Notify::new()),
            capacity,
            coalesced: Arc::new(AtomicU64::new(0)),
            dropped: Arc::new(AtomicU64::new(0)),
        }
    }

    fn enqueue(&self, payload: serde_json::Value) -> AuditEnqueueResult {
        let replaceable = is_replaceable_audit(&payload);
        let symbol = payload.get("symbol").and_then(serde_json::Value::as_str);
        let mut messages = self.messages.lock().unwrap_or_else(|error| error.into_inner());

        if replaceable {
            if let Some(symbol) = symbol {
                if let Some(existing) = messages.iter_mut().find(|existing| {
                    is_replaceable_audit(existing)
                        && existing.get("symbol").and_then(serde_json::Value::as_str) == Some(symbol)
                }) {
                    if audit_timestamp(&payload) >= audit_timestamp(existing) {
                        *existing = payload;
                    }
                    self.coalesced.fetch_add(1, Ordering::Relaxed);
                    self.notify.notify_one();
                    return AuditEnqueueResult::Coalesced;
                }
            }
        }

        let mut result = AuditEnqueueResult::Queued;
        if messages.len() >= self.capacity {
            if let Some(index) = messages.iter().position(is_replaceable_audit) {
                messages.remove(index);
                self.dropped.fetch_add(1, Ordering::Relaxed);
                result = AuditEnqueueResult::EvictedOld;
            } else {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                return AuditEnqueueResult::Dropped;
            }
        }
        messages.push_back(payload);
        self.notify.notify_one();
        result
    }

    async fn recv(&self) -> serde_json::Value {
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            if let Some(payload) = self.messages.lock()
                .unwrap_or_else(|error| error.into_inner()).pop_front()
            {
                return payload;
            }
            notified.await;
        }
    }

    fn depth(&self) -> usize {
        self.messages.lock().unwrap_or_else(|error| error.into_inner()).len()
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
#[derive(Clone, Serialize, Deserialize)]
struct StrategyThresholdSample {
    timestamp_ms: u64,
    parameter_key: StrategyParameterKey,
    threshold: f64,
    feature_value: f64,
    net_return_percent: f64,
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
    #[serde(default)]
    threshold_samples: Vec<StrategyThresholdSample>,
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
            walk_forward: Arc::new(Vec::new()),
            missed_opportunity_bands: Vec::new(),
            markets: Arc::new(BTreeMap::new()),
            queued_at: None,
            queue_wait_ms: None,
        }
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
}
