async fn get_orders(State(store): State<Arc<PositionStore>>) -> Json<OrdersResponse> {
    let ledger = store.ledger.lock().await;
    Json(OrdersResponse {
        orders: ledger.orders.clone(),
        barrier_orders: ledger.barrier_orders.clone(),
    })
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct AdjustBarrierTriggerRequest {
    request_id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    expected_parent_order_id: RequiredNullableString,
    expected_lifecycle_trade_id: RequiredNullableString,
    profile_id: String,
    profile_version: u64,
    policy_source: String,
    reason: TriggerAdjustmentReason,
    target_net_return_percent: f64,
    expected_trigger_price: f64,
    new_trigger_price: f64,
}

#[derive(Clone, Debug, PartialEq)]
struct RequiredNullableString(Option<String>);

impl<'de> Deserialize<'de> for RequiredNullableString {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Option::<String>::deserialize(deserializer).map(Self)
    }
}

#[derive(Serialize)]
struct AdjustBarrierTriggerResponse {
    barrier_id: String,
    request_id: String,
    adjusted_at_ms: u64,
    trigger_price: f64,
    idempotent_replay: bool,
}

async fn adjust_barrier_trigger(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
    Json(request): Json<AdjustBarrierTriggerRequest>,
) -> Result<Json<AdjustBarrierTriggerResponse>, (StatusCode, String)> {
    if !valid_adjustment_label(&request.request_id, 128)
        || !valid_adjustment_label(&request.symbol, 64)
        || !valid_adjustment_label(&request.profile_id, 128)
        || !valid_adjustment_label(&request.policy_source, 64)
        || !request.quantity.is_finite()
        || request.quantity <= 0.0
        || !request.target_net_return_percent.is_finite()
        || !request.expected_trigger_price.is_finite()
        || request.expected_trigger_price <= 0.0
        || !request.new_trigger_price.is_finite()
        || request.new_trigger_price <= 0.0
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Trigger adjustment identity and prices must be valid".into(),
        ));
    }

    // Match the persistence writer's lock order: writer lock, then ledger.
    // Holding both through the durable transaction prevents the matcher and
    // trigger poller from observing an unpersisted or partially rolled back
    // threshold.
    let _write_guard = store.persistence_write_lock.lock().await;
    let mut ledger = store.ledger.lock().await;
    let mut proposed_record = TriggerAdjustmentRecord {
        request_id: request.request_id.clone(),
        adjusted_at_ms: 0,
        barrier_id: id.clone(),
        symbol: request.symbol.clone(),
        side: request.side.clone(),
        quantity: request.quantity,
        parent_order_id: request.expected_parent_order_id.0.clone(),
        lifecycle_trade_id: request.expected_lifecycle_trade_id.0.clone(),
        profile_id: request.profile_id.clone(),
        profile_version: request.profile_version,
        policy_source: request.policy_source.clone(),
        reason: request.reason.clone(),
        target_net_return_percent: request.target_net_return_percent,
        expected_trigger_price: request.expected_trigger_price,
        new_trigger_price: request.new_trigger_price,
    };

    if let Some(existing) = ledger
        .barrier_orders
        .iter()
        .flat_map(|barrier| barrier.trigger_adjustments.iter())
        .find(|record| record.request_id == request.request_id)
        .cloned()
    {
        if !same_trigger_adjustment_request(&existing, &proposed_record) {
            return Err((
                StatusCode::CONFLICT,
                "Trigger adjustment request_id was already used with different values".into(),
            ));
        }
        let Some(barrier) = ledger
            .barrier_orders
            .iter()
            .find(|barrier| barrier.id == id)
        else {
            return Err((StatusCode::NOT_FOUND, "Barrier order not found".into()));
        };
        let response_trigger = barrier.trigger_price;
        let response_barrier_id = barrier.id.clone();
        let adjusted_at_ms = existing.adjusted_at_ms;
        persist_trigger_adjustment_snapshot(&store.path, ledger.clone())
            .await
            .map_err(|error| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Unable to confirm durable trigger adjustment retry: {error}"),
                )
            })?;
        return Ok(Json(AdjustBarrierTriggerResponse {
            barrier_id: response_barrier_id,
            request_id: request.request_id,
            adjusted_at_ms,
            trigger_price: response_trigger,
            idempotent_replay: true,
        }));
    }

    let Some(index) = ledger
        .barrier_orders
        .iter()
        .position(|barrier| barrier.id == id)
    else {
        return Err((StatusCode::NOT_FOUND, "Barrier order not found".into()));
    };
    let barrier = &ledger.barrier_orders[index];
    if barrier.status != "armed" {
        return Err((
            StatusCode::CONFLICT,
            "Only an armed protective barrier can be adjusted".into(),
        ));
    }
    if barrier.order_kind != "stop-market"
        || barrier.execution_type != "market"
        || barrier.limit_price.is_some()
        || barrier.trailing_amount.is_some()
        || barrier.trailing_percent.is_some()
    {
        return Err((
            StatusCode::CONFLICT,
            "Only an armed stop-market barrier can be adjusted".into(),
        ));
    }
    let lifecycle_trade_id = barrier
        .lifecycle
        .as_ref()
        .map(|lifecycle| lifecycle.trade_id.as_str());
    let lifecycle_profile = barrier.lifecycle.as_ref();
    if barrier.symbol != request.symbol
        || barrier.side != request.side
        || barrier.quantity.to_bits() != request.quantity.to_bits()
        || barrier.parent_order_id != request.expected_parent_order_id.0
        || lifecycle_trade_id != request.expected_lifecycle_trade_id.0.as_deref()
        || lifecycle_profile.and_then(|lifecycle| lifecycle.profile_id.as_deref())
            != Some(request.profile_id.as_str())
        || lifecycle_profile.and_then(|lifecycle| lifecycle.profile_version)
            != Some(request.profile_version)
    {
        return Err((
            StatusCode::CONFLICT,
            "Trigger adjustment identity does not match the armed barrier".into(),
        ));
    }
    let current_trigger = barrier.trigger_price;
    if current_trigger.to_bits() != request.expected_trigger_price.to_bits() {
        return Err((
            StatusCode::CONFLICT,
            "Expected trigger price is stale".into(),
        ));
    }
    let protective = match (&barrier.side, &barrier.trigger_direction) {
        (TradeSide::Sell, BarrierDirection::Below) => request.new_trigger_price > current_trigger,
        (TradeSide::Buy, BarrierDirection::Above) => request.new_trigger_price < current_trigger,
        _ => false,
    };
    if !protective {
        return Err((
            StatusCode::CONFLICT,
            "Trigger movement must tighten protection in the barrier's risk-reducing direction"
                .into(),
        ));
    }
    if barrier.trigger_adjustments.len() >= MAX_TRIGGER_ADJUSTMENTS_PER_BARRIER {
        return Err((
            StatusCode::CONFLICT,
            "Barrier trigger adjustment history reached its durable limit".into(),
        ));
    }

    let observed_at_ms = now_ms().map_err(internal_error)?;
    proposed_record.adjusted_at_ms = observed_at_ms;
    ledger.barrier_orders[index].trigger_price = request.new_trigger_price;
    ledger.barrier_orders[index]
        .trigger_adjustments
        .push(proposed_record);
    let snapshot = ledger.clone();
    let durable_result =
        persist_trigger_adjustment_snapshot_at(&store.path, snapshot, observed_at_ms).await;
    if let Err(error) = durable_result {
        ledger.barrier_orders[index].trigger_price = current_trigger;
        ledger.barrier_orders[index].trigger_adjustments.pop();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to durably persist barrier trigger adjustment: {error}"),
        ));
    }
    let _ = store.state_updates.send(());
    Ok(Json(AdjustBarrierTriggerResponse {
        barrier_id: id,
        request_id: request.request_id,
        adjusted_at_ms: observed_at_ms,
        trigger_price: request.new_trigger_price,
        idempotent_replay: false,
    }))
}

async fn persist_trigger_adjustment_snapshot(
    path: &std::path::Path,
    snapshot: Ledger,
) -> anyhow::Result<()> {
    let observed_at_ms = now_ms()?;
    persist_trigger_adjustment_snapshot_at(path, snapshot, observed_at_ms).await
}

async fn persist_trigger_adjustment_snapshot_at(
    path: &std::path::Path,
    snapshot: Ledger,
    observed_at_ms: u64,
) -> anyhow::Result<()> {
    let path = path.to_owned();
    match tokio::task::spawn_blocking(move || {
        save_sqlite_ledger_at(&path, &snapshot, observed_at_ms, false)
    })
    .await
    {
        Ok(result) => result,
        Err(error) => Err(anyhow::anyhow!(
            "Position Manager persistence task failed: {error}"
        )),
    }
}

fn valid_adjustment_label(value: &str, max_bytes: usize) -> bool {
    !value.trim().is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn same_trigger_adjustment_request(
    stored: &TriggerAdjustmentRecord,
    proposed: &TriggerAdjustmentRecord,
) -> bool {
    stored.request_id == proposed.request_id
        && stored.barrier_id == proposed.barrier_id
        && stored.symbol == proposed.symbol
        && stored.side == proposed.side
        && stored.quantity.to_bits() == proposed.quantity.to_bits()
        && stored.parent_order_id == proposed.parent_order_id
        && stored.lifecycle_trade_id == proposed.lifecycle_trade_id
        && stored.profile_id == proposed.profile_id
        && stored.profile_version == proposed.profile_version
        && stored.policy_source == proposed.policy_source
        && stored.reason == proposed.reason
        && stored.target_net_return_percent.to_bits()
            == proposed.target_net_return_percent.to_bits()
        && stored.expected_trigger_price.to_bits() == proposed.expected_trigger_price.to_bits()
        && stored.new_trigger_price.to_bits() == proposed.new_trigger_price.to_bits()
}

const DEFAULT_ORDERS_HISTORY_LIMIT: usize = 50;
const MAX_ORDERS_HISTORY_LIMIT: usize = 200;

#[derive(Deserialize)]
struct OrdersHistoryQuery {
    #[serde(default = "default_orders_history_limit")]
    limit: usize,
    #[serde(default)]
    orders_cursor: Option<String>,
    #[serde(default)]
    barrier_orders_cursor: Option<String>,
}

fn default_orders_history_limit() -> usize {
    DEFAULT_ORDERS_HISTORY_LIMIT
}

#[derive(Serialize)]
struct OrdersHistoryPage {
    orders: Vec<RestingOrder>,
    barrier_orders: Vec<BarrierOrder>,
    execution_paths: Vec<ExecutionPathTerminalRecord>,
    execution_paths_truncated: bool,
    active_orders: Vec<RestingOrder>,
    active_barrier_orders: Vec<BarrierOrder>,
    limit: usize,
    next_orders_cursor: Option<String>,
    has_more_orders: bool,
    next_barrier_orders_cursor: Option<String>,
    has_more_barrier_orders: bool,
}

async fn get_orders_history(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<OrdersHistoryQuery>,
) -> Result<Json<OrdersHistoryPage>, (StatusCode, String)> {
    if !valid_orders_history_limit(query.limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Orders history limit must be between 1 and {MAX_ORDERS_HISTORY_LIMIT}"),
        ));
    }
    let order_cursor = query
        .orders_cursor
        .as_deref()
        .map(decode_history_cursor)
        .transpose()?;
    let barrier_cursor = query
        .barrier_orders_cursor
        .as_deref()
        .map(decode_history_cursor)
        .transpose()?;

    let ledger = store.ledger.lock().await;
    let active_orders = active_resting_orders(&ledger.orders);
    let active_barrier_orders = active_barrier_orders(&ledger.barrier_orders);

    let order_page = page_order_history(&ledger.orders, order_cursor, query.limit);
    let barrier_page = page_barrier_history(&ledger.barrier_orders, barrier_cursor, query.limit);
    Ok(Json(OrdersHistoryPage {
        orders: order_page.items,
        barrier_orders: barrier_page.items,
        execution_paths: ledger.execution_path_history.clone(),
        execution_paths_truncated: ledger.execution_path_history_truncated,
        active_orders,
        active_barrier_orders,
        limit: query.limit,
        next_orders_cursor: order_page
            .next_cursor
            .map(|cursor| encode_history_cursor(&cursor)),
        has_more_orders: order_page.has_more,
        next_barrier_orders_cursor: barrier_page
            .next_cursor
            .map(|cursor| encode_history_cursor(&cursor)),
        has_more_barrier_orders: barrier_page.has_more,
    }))
}

fn valid_orders_history_limit(limit: usize) -> bool {
    (1..=MAX_ORDERS_HISTORY_LIMIT).contains(&limit)
}

fn active_resting_orders(orders: &[RestingOrder]) -> Vec<RestingOrder> {
    orders
        .iter()
        .filter(|order| order.remaining_quantity > EPSILON)
        .cloned()
        .collect()
}

fn active_barrier_orders(orders: &[BarrierOrder]) -> Vec<BarrierOrder> {
    orders
        .iter()
        .filter(|order| is_active_barrier_status(&order.status))
        .cloned()
        .collect()
}

fn is_active_barrier_status(status: &str) -> bool {
    matches!(status, "armed" | "triggering")
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct HistoryCursor {
    timestamp: u64,
    id: String,
}

struct HistoryPage<T> {
    items: Vec<T>,
    next_cursor: Option<HistoryCursor>,
    has_more: bool,
}

fn encode_history_cursor(cursor: &HistoryCursor) -> String {
    let id_hex = cursor
        .id
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("{}~{id_hex}", cursor.timestamp)
}

fn decode_history_cursor(value: &str) -> Result<HistoryCursor, (StatusCode, String)> {
    let invalid = || {
        (
            StatusCode::BAD_REQUEST,
            "Invalid orders history cursor".to_owned(),
        )
    };
    let (timestamp, id_hex) = value.split_once('~').ok_or_else(invalid)?;
    let timestamp = timestamp.parse::<u64>().map_err(|_| invalid())?;
    if id_hex.is_empty() || id_hex.len() % 2 != 0 {
        return Err(invalid());
    }
    let mut id_bytes = Vec::with_capacity(id_hex.len() / 2);
    for pair in id_hex.as_bytes().chunks_exact(2) {
        let high = (pair[0] as char).to_digit(16).ok_or_else(invalid)? as u8;
        let low = (pair[1] as char).to_digit(16).ok_or_else(invalid)? as u8;
        id_bytes.push((high << 4) | low);
    }
    let id = String::from_utf8(id_bytes).map_err(|_| invalid())?;
    if id.is_empty() {
        return Err(invalid());
    }
    Ok(HistoryCursor { timestamp, id })
}

fn page_order_history(
    orders: &[RestingOrder],
    cursor: Option<HistoryCursor>,
    limit: usize,
) -> HistoryPage<RestingOrder> {
    let mut history = orders
        .iter()
        .filter(|order| order.remaining_quantity <= EPSILON)
        .filter(|order| {
            cursor.as_ref().map_or(true, |cursor| {
                (order.timestamp, order.id.as_str()) < (cursor.timestamp, cursor.id.as_str())
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    history.sort_by(|left, right| {
        (right.timestamp, right.id.as_str()).cmp(&(left.timestamp, left.id.as_str()))
    });
    make_history_page(history, limit, |order| HistoryCursor {
        timestamp: order.timestamp,
        id: order.id.clone(),
    })
}

fn page_barrier_history(
    orders: &[BarrierOrder],
    cursor: Option<HistoryCursor>,
    limit: usize,
) -> HistoryPage<BarrierOrder> {
    let mut history = orders
        .iter()
        .filter(|order| !is_active_barrier_status(&order.status))
        .filter(|order| {
            cursor.as_ref().map_or(true, |cursor| {
                (order.created_at, order.id.as_str()) < (cursor.timestamp, cursor.id.as_str())
            })
        })
        .cloned()
        .collect::<Vec<_>>();
    history.sort_by(|left, right| {
        (right.created_at, right.id.as_str()).cmp(&(left.created_at, left.id.as_str()))
    });
    make_history_page(history, limit, |order| HistoryCursor {
        timestamp: order.created_at,
        id: order.id.clone(),
    })
}

fn make_history_page<T>(
    mut records: Vec<T>,
    limit: usize,
    cursor_for: impl Fn(&T) -> HistoryCursor,
) -> HistoryPage<T> {
    let has_more = records.len() > limit;
    records.truncate(limit);
    let next_cursor = has_more.then(|| records.last().map(cursor_for)).flatten();
    HistoryPage {
        items: records,
        next_cursor,
        has_more,
    }
}

#[cfg(test)]
mod orders_history_tests {
    use super::*;
    use axum::{body::Body, http::Request};
    use tower::ServiceExt;

    fn resting(id: &str, timestamp: u64, remaining_quantity: f64) -> RestingOrder {
        RestingOrder {
            id: id.to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: TradeSide::Buy,
            quantity: 1.0,
            remaining_quantity,
            limit_price: 100.0,
            timestamp,
            maker_fee_rate: 0.0,
            estimated_fee: 0.0,
            expires_at: None,
            time_in_force: "GTC".to_owned(),
            post_only: false,
            source: "manual".to_owned(),
            parent_order_id: None,
            client_order_id: None,
            lifecycle: None,
            reduce_only: false,
            market_path: Vec::new(),
            market_path_completeness: default_market_path_completeness(),
            market_path_dropped_samples: 0,
        }
    }

    fn barrier(id: &str, created_at: u64, status: &str) -> BarrierOrder {
        BarrierOrder {
            id: id.to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: TradeSide::Sell,
            quantity: 1.0,
            remaining_quantity: 1.0,
            filled_quantity: 0.0,
            trigger_price: 110.0,
            trigger_direction: BarrierDirection::Above,
            created_at,
            triggered_at: None,
            retry_after: None,
            status: status.to_owned(),
            average_execution_price: None,
            fee_paid: 0.0,
            error: None,
            order_kind: "barrier".to_owned(),
            execution_type: "market".to_owned(),
            limit_price: None,
            trailing_amount: None,
            trailing_percent: None,
            extreme_price: None,
            source: "manual".to_owned(),
            parent_order_id: None,
            client_order_id: None,
            lifecycle: None,
            trigger_adjustments: Vec::new(),
        }
    }

    #[test]
    fn order_history_is_deterministic_for_tied_timestamps_and_pages_by_key() {
        let orders = vec![
            resting("a", 7, 0.0),
            resting("c", 8, 0.0),
            resting("b", 8, 0.0),
            resting("working", 9, 0.5),
        ];
        let first = page_order_history(&orders, None, 2);
        assert_eq!(
            first
                .items
                .iter()
                .map(|order| order.id.as_str())
                .collect::<Vec<_>>(),
            vec!["c", "b"]
        );
        assert!(first.has_more);
        let cursor = first.next_cursor.clone().unwrap();
        let second = page_order_history(&orders, Some(cursor), 2);
        assert_eq!(
            second
                .items
                .iter()
                .map(|order| order.id.as_str())
                .collect::<Vec<_>>(),
            vec!["a"]
        );
        assert!(!second.has_more);
        assert!(second.next_cursor.is_none());
    }

    #[test]
    fn barrier_history_pages_newest_first_and_stops_at_end() {
        let barriers = vec![
            barrier("old", 2, "cancelled"),
            barrier("armed", 4, "armed"),
            barrier("triggering", 5, "triggering"),
            barrier("new", 3, "filled"),
        ];
        let first = page_barrier_history(&barriers, None, 1);
        assert_eq!(first.items[0].id, "new");
        assert!(first.has_more);
        let last = page_barrier_history(&barriers, first.next_cursor.clone(), 1);
        assert_eq!(last.items[0].id, "old");
        assert!(!last.has_more);
        assert!(last.next_cursor.is_none());
        let end_cursor = HistoryCursor {
            timestamp: last.items[0].created_at,
            id: last.items[0].id.clone(),
        };
        let end = page_barrier_history(&barriers, Some(end_cursor), 1);
        assert!(end.items.is_empty());
        assert!(!end.has_more);
    }

    #[test]
    fn legacy_barrier_rows_default_trigger_adjustment_history() {
        let mut legacy = serde_json::to_value(barrier("legacy", 1, "armed")).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("trigger_adjustments");
        let decoded: BarrierOrder = serde_json::from_value(legacy).unwrap();
        assert!(decoded.trigger_adjustments.is_empty());
    }

    #[test]
    fn history_limit_is_explicitly_bounded() {
        assert!(!valid_orders_history_limit(0));
        assert!(valid_orders_history_limit(1));
        assert!(valid_orders_history_limit(MAX_ORDERS_HISTORY_LIMIT));
        assert!(!valid_orders_history_limit(MAX_ORDERS_HISTORY_LIMIT + 1));
        let empty = page_order_history(&[], None, DEFAULT_ORDERS_HISTORY_LIMIT);
        assert!(empty.items.is_empty());
        assert!(!empty.has_more);
        assert!(empty.next_cursor.is_none());
    }

    #[test]
    fn active_orders_and_barriers_are_returned_outside_history_pages() {
        let orders = vec![resting("working", 3, 1.0), resting("done", 2, 0.0)];
        let barriers = vec![
            barrier("armed", 3, "armed"),
            barrier("triggering", 4, "triggering"),
            barrier("cancelled", 2, "cancelled"),
        ];
        let active_orders = active_resting_orders(&orders);
        let active_barriers = active_barrier_orders(&barriers);
        assert_eq!(
            active_orders
                .iter()
                .map(|order| order.id.as_str())
                .collect::<Vec<_>>(),
            vec!["working"]
        );
        assert_eq!(
            active_barriers
                .iter()
                .map(|order| order.id.as_str())
                .collect::<Vec<_>>(),
            vec!["armed", "triggering"]
        );
        assert_eq!(page_order_history(&orders, None, 1).items[0].id, "done");
        assert_eq!(
            page_barrier_history(&barriers, None, 1).items[0].id,
            "cancelled"
        );
    }

    #[test]
    fn history_cursor_round_trips_and_rejects_invalid_values() {
        let cursor = HistoryCursor {
            timestamp: 42,
            id: "order:α".to_owned(),
        };
        assert_eq!(
            decode_history_cursor(&encode_history_cursor(&cursor)).unwrap(),
            cursor
        );
        assert_eq!(
            decode_history_cursor("broken").unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }

    fn trigger_test_store(barrier: BarrierOrder, path: PathBuf) -> Arc<PositionStore> {
        let (state_updates, _) = broadcast::channel(8);
        let ledger = Ledger {
            barrier_orders: vec![barrier],
            ..Ledger::default()
        };
        Arc::new(PositionStore {
            path,
            ledger: Arc::new(Mutex::new(ledger)),
            persistence_notify: Arc::new(Notify::new()),
            persistence_write_lock: Arc::new(Mutex::new(())),
            pending_client_order_commits: StdMutex::new(HashSet::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            market_marks: StdRwLock::new(HashMap::new()),
            market_mark_pollers: StdMutex::new(HashSet::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        })
    }

    fn trigger_test_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pm-trigger-adjustment-{}-{label}-{nonce}.sqlite",
            std::process::id()
        ))
    }

    fn stop_market_barrier(id: &str, side: TradeSide) -> BarrierOrder {
        let mut barrier = barrier(id, 1, "armed");
        barrier.order_kind = "stop-market".to_owned();
        barrier.execution_type = "market".to_owned();
        barrier.parent_order_id = Some("parent-1".to_owned());
        barrier.client_order_id = Some("parent-1".to_owned());
        barrier.lifecycle = Some(CanonicalLifecycleLink {
            schema_version: 1,
            opportunity_id: "opportunity-1".to_owned(),
            episode_id: "episode-1".to_owned(),
            trade_id: "trade-1".to_owned(),
            profile_id: Some("profile-1".to_owned()),
            profile_hash: Some("hash-1".to_owned()),
            profile_version: Some(3),
            execution_mode: "paper".to_owned(),
            entry_policy: "strict".to_owned(),
            source: "auto".to_owned(),
        });
        barrier.side = side.clone();
        match side {
            TradeSide::Sell => {
                barrier.trigger_price = 90.0;
                barrier.trigger_direction = BarrierDirection::Below;
            }
            TradeSide::Buy => {
                barrier.trigger_price = 110.0;
                barrier.trigger_direction = BarrierDirection::Above;
            }
        }
        barrier
    }

    fn trigger_request(
        request_id: &str,
        barrier: &BarrierOrder,
        symbol: &str,
        parent_order_id: Option<&str>,
        expected: f64,
        new: f64,
    ) -> serde_json::Value {
        serde_json::json!({
            "request_id": request_id,
            "symbol": symbol,
            "side": barrier.side,
            "quantity": barrier.quantity,
            "expected_parent_order_id": parent_order_id,
            "expected_lifecycle_trade_id": "trade-1",
            "profile_id": "profile-1",
            "profile_version": 3,
            "policy_source": "validated_profile",
            "reason": "trailing-stop",
            "target_net_return_percent": 0.25,
            "expected_trigger_price": expected,
            "new_trigger_price": new
        })
    }

    async fn post_trigger_adjustment(
        store: Arc<PositionStore>,
        id: &str,
        request: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = position_router(store)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/positions/orders/{id}/trigger"))
                    .header(axum::http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let value = serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
        (status, value)
    }

    #[tokio::test]
    async fn trigger_adjustment_tightens_both_sides_and_survives_sqlite_restart() {
        for (id, side, old, new) in [
            ("long-stop", TradeSide::Sell, 90.0, 95.0),
            ("short-stop", TradeSide::Buy, 110.0, 105.0),
        ] {
            let barrier = stop_market_barrier(id, side.clone());
            let path = trigger_test_path(id);
            let store = trigger_test_store(barrier.clone(), path.clone());
            let mut request = trigger_request(
                "request-1",
                &barrier,
                "BTC-USDT",
                Some("parent-1"),
                old,
                new,
            );
            if matches!(side, TradeSide::Buy) {
                request["reason"] = serde_json::json!("breakeven-protection");
            }
            let (status, body) = post_trigger_adjustment(Arc::clone(&store), id, request).await;
            assert_eq!(status, StatusCode::OK, "{body}");
            assert_eq!(body["trigger_price"], new);
            assert_eq!(body["idempotent_replay"], false);
            let adjustment_timestamp = body["adjusted_at_ms"].as_u64().unwrap();
            assert!(adjustment_timestamp > 0);

            #[cfg(feature = "test-support")]
            let ledger = {
                let restarted = PositionStore::load_from_path_without_writer(path.clone())
                    .await
                    .unwrap();
                restarted.ledger.lock().await.clone()
            };
            #[cfg(not(feature = "test-support"))]
            let ledger = load_sqlite_ledger(&path).unwrap();
            let persisted = ledger
                .barrier_orders
                .iter()
                .find(|record| record.id == id)
                .unwrap();
            assert_eq!(persisted.trigger_price, new);
            assert_eq!(persisted.id, barrier.id);
            assert_eq!(persisted.quantity, barrier.quantity);
            assert_eq!(persisted.side, barrier.side);
            assert_eq!(persisted.status, "armed");
            assert_eq!(persisted.trigger_adjustments.len(), 1);
            assert_eq!(
                persisted.trigger_adjustments[0].adjusted_at_ms,
                adjustment_timestamp
            );

            let _ = std::fs::remove_file(&path);
            let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
            let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
        }
    }

    #[tokio::test]
    async fn trigger_adjustment_accepts_explicit_zero_profile_version() {
        let mut barrier = stop_market_barrier("stop-profile-zero", TradeSide::Sell);
        barrier.lifecycle.as_mut().unwrap().profile_version = Some(0);
        let path = trigger_test_path("profile-zero");
        let store = trigger_test_store(barrier.clone(), path.clone());
        let mut request = trigger_request(
            "profile-zero-request",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        request["profile_version"] = serde_json::json!(0);
        let (status, _) = post_trigger_adjustment(store, &barrier.id, request).await;
        assert_eq!(status, StatusCode::OK);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trigger_adjustment_retry_is_idempotent_and_stale_or_reused_ids_conflict() {
        let barrier = stop_market_barrier("stop-idempotent", TradeSide::Sell);
        let path = trigger_test_path("idempotent");
        let store = trigger_test_store(barrier.clone(), path.clone());
        let request = trigger_request(
            "stable-request",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        let (first, first_body) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, request.clone()).await;
        let (retry, response) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, request.clone()).await;
        assert_eq!(first, StatusCode::OK);
        assert_eq!(retry, StatusCode::OK);
        assert_eq!(response["idempotent_replay"], true);
        let stored_timestamp = response["adjusted_at_ms"].as_u64().unwrap();
        assert!(stored_timestamp > 0);
        assert_eq!(first_body["adjusted_at_ms"], stored_timestamp);

        let mut reused = request.clone();
        reused["new_trigger_price"] = serde_json::json!(96.0);
        let (reused_status, _) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, reused).await;
        assert_eq!(reused_status, StatusCode::CONFLICT);

        let stale = trigger_request(
            "another-request",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            97.0,
        );
        let (stale_status, _) = post_trigger_adjustment(store, &barrier.id, stale).await;
        assert_eq!(stale_status, StatusCode::CONFLICT);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trigger_adjustment_rejects_identity_state_and_nonprotective_updates() {
        let barrier = stop_market_barrier("stop-reject", TradeSide::Sell);
        let path = trigger_test_path("reject");
        let store = trigger_test_store(barrier.clone(), path.clone());

        for (request_id, symbol, parent, expected, new, status) in [
            (
                "wrong-symbol",
                "ETH-USDT",
                Some("parent-1"),
                90.0,
                95.0,
                "identity",
            ),
            (
                "wrong-parent",
                "BTC-USDT",
                Some("other-parent"),
                90.0,
                95.0,
                "identity",
            ),
            (
                "widen",
                "BTC-USDT",
                Some("parent-1"),
                90.0,
                85.0,
                "direction",
            ),
        ] {
            let request = trigger_request(request_id, &barrier, symbol, parent, expected, new);
            let (actual, _) =
                post_trigger_adjustment(Arc::clone(&store), &barrier.id, request).await;
            assert_eq!(actual, StatusCode::CONFLICT, "{status}");
        }

        for (index, state) in ["triggering", "filled", "cancelled"].iter().enumerate() {
            {
                let mut ledger = store.ledger.lock().await;
                ledger.barrier_orders[0].status = (*state).to_owned();
            }
            let request = trigger_request(
                &format!("non-armed-{index}"),
                &barrier,
                "BTC-USDT",
                Some("parent-1"),
                90.0,
                95.0,
            );
            let (actual, _) =
                post_trigger_adjustment(Arc::clone(&store), &barrier.id, request).await;
            assert_eq!(actual, StatusCode::CONFLICT);
        }

        // The following malformed-request assertions exercise decoding and
        // value validation independently of the terminal-state rejection.
        store.ledger.lock().await.barrier_orders[0].status = "armed".to_owned();

        let mut malformed = trigger_request(
            "malformed",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        malformed["expected_trigger_price"] = serde_json::json!(0.0);
        let (actual, _) = post_trigger_adjustment(Arc::clone(&store), &barrier.id, malformed).await;
        assert_eq!(actual, StatusCode::BAD_REQUEST);

        let missing_identity = serde_json::json!({
            "request_id": "missing-parent-identity",
            "symbol": "BTC-USDT",
            "side": "sell",
            "quantity": 1.0,
            "expected_lifecycle_trade_id": "trade-1",
            "profile_id": "profile-1",
            "profile_version": 3,
            "policy_source": "validated_profile",
            "reason": "trailing-stop",
            "target_net_return_percent": 0.25,
            "expected_trigger_price": 90.0,
            "new_trigger_price": 95.0
        });
        let (actual, _) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, missing_identity).await;
        assert_eq!(
            actual,
            StatusCode::CONFLICT,
            "missing parent identity is rejected"
        );

        {
            let mut ledger = store.ledger.lock().await;
            ledger.barrier_orders[0].status = "armed".to_owned();
            ledger.barrier_orders[0].order_kind = "stop-limit".to_owned();
        }
        let request = trigger_request(
            "stop-limit",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        let (actual, _) = post_trigger_adjustment(store, &barrier.id, request).await;
        assert_eq!(actual, StatusCode::CONFLICT);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trigger_adjustment_limit_keeps_existing_idempotency_records() {
        let mut barrier = stop_market_barrier("stop-cap", TradeSide::Sell);
        let prior = TriggerAdjustmentRecord {
            request_id: "cap-replay".to_owned(),
            adjusted_at_ms: 1,
            barrier_id: barrier.id.clone(),
            symbol: "BTC-USDT".to_owned(),
            side: TradeSide::Sell,
            quantity: 1.0,
            parent_order_id: Some("parent-1".to_owned()),
            lifecycle_trade_id: Some("trade-1".to_owned()),
            profile_id: "profile-1".to_owned(),
            profile_version: 3,
            policy_source: "validated_profile".to_owned(),
            reason: TriggerAdjustmentReason::TrailingStop,
            target_net_return_percent: 0.25,
            expected_trigger_price: 89.0,
            new_trigger_price: 90.0,
        };
        barrier.trigger_adjustments.push(prior.clone());
        for index in 1..MAX_TRIGGER_ADJUSTMENTS_PER_BARRIER {
            barrier.trigger_adjustments.push(TriggerAdjustmentRecord {
                request_id: format!("request-{index}"),
                ..prior.clone()
            });
        }
        let path = trigger_test_path("cap");
        let store = trigger_test_store(barrier.clone(), path.clone());

        let replay = trigger_request(
            "cap-replay",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            89.0,
            90.0,
        );
        let (replay_status, replay_body) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, replay).await;
        assert_eq!(replay_status, StatusCode::OK);
        assert_eq!(replay_body["idempotent_replay"], true);

        let new_request = trigger_request(
            "over-cap",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        let (status, _) =
            post_trigger_adjustment(Arc::clone(&store), &barrier.id, new_request).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let ledger = store.ledger.lock().await;
        assert_eq!(ledger.barrier_orders[0].trigger_price, 90.0);
        assert_eq!(
            ledger.barrier_orders[0].trigger_adjustments.len(),
            MAX_TRIGGER_ADJUSTMENTS_PER_BARRIER
        );
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trigger_adjustment_rolls_back_memory_when_durable_write_fails() {
        let barrier = stop_market_barrier("stop-rollback", TradeSide::Sell);
        let parent = trigger_test_path("not-a-directory");
        std::fs::write(&parent, b"block parent directory").unwrap();
        let path = parent.join("ledger.sqlite");
        let store = trigger_test_store(barrier.clone(), path);
        let request = trigger_request(
            "write-fails",
            &barrier,
            "BTC-USDT",
            Some("parent-1"),
            90.0,
            95.0,
        );
        let (status, _) = post_trigger_adjustment(Arc::clone(&store), &barrier.id, request).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        let ledger = store.ledger.lock().await;
        assert_eq!(ledger.barrier_orders[0].trigger_price, 90.0);
        assert!(ledger.barrier_orders[0].trigger_adjustments.is_empty());
        let _ = std::fs::remove_file(parent);
    }
}
async fn get_barrier_orders(State(store): State<Arc<PositionStore>>) -> Json<Vec<BarrierOrder>> {
    Json(store.ledger.lock().await.barrier_orders.clone())
}

#[derive(Deserialize)]
struct ProfitBarrierRequest {
    symbol: String,
    target_profit_usd: f64,
}

async fn create_profit_barrier(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<ProfitBarrierRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !request.target_profit_usd.is_finite() || request.target_profit_usd <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Target profit must be a finite USD amount greater than zero".into(),
        ));
    }
    let rates = get_usd_rates().await?.0.rates;
    let quote = symbol
        .split_once('-')
        .map(|(_, quote)| quote)
        .unwrap_or_default();
    let rate = *rates.get(quote).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        format!("No OKX USD conversion rate for {quote}"),
    ))?;
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions;
    let position = positions
        .into_iter()
        .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
    let quantity = position.quantity.abs();
    let fee_rate = position.estimated_exit_fee_rate.abs();
    let denominator = position.quantity - quantity * fee_rate;
    if !denominator.is_finite() || denominator.abs() <= EPSILON {
        return Err((
            StatusCode::BAD_REQUEST,
            "Cannot calculate a profit trigger for this position and fee rate".into(),
        ));
    }
    let trigger_price = (request.target_profit_usd / rate
        + position.average_entry_price * position.quantity
        - position.entry_fees)
        / denominator;
    if !trigger_price.is_finite() || trigger_price <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Target profit produces an invalid trigger price".into(),
        ));
    }
    let side = if position.quantity > 0.0 {
        TradeSide::Sell
    } else {
        TradeSide::Buy
    };
    let trigger_direction = if position.quantity > 0.0 {
        BarrierDirection::Above
    } else {
        BarrierDirection::Below
    };
    store.ensure_market_mark_polling(&symbol);
    let timestamp = now_ms().map_err(internal_error)?;
    let capacity_rates = cached_usd_rates_for_order().await?;
    let mut ledger = store.ledger.lock().await;
    ensure_paper_capacity_with_rates(
        &store,
        &ledger,
        &capacity_rates.rates,
        &symbol,
        &side,
        quantity,
        trigger_price,
        configured_fee_rates().taker,
        None,
    )?;
    let barrier_order = BarrierOrder {
        id: format!("barrier-{timestamp}-{}", ledger.barrier_orders.len()),
        symbol,
        side,
        quantity,
        remaining_quantity: quantity,
        filled_quantity: 0.0,
        trigger_price,
        trigger_direction,
        created_at: timestamp,
        triggered_at: None,
        retry_after: None,
        status: "armed".to_owned(),
        average_execution_price: None,
        fee_paid: 0.0,
        error: None,
        order_kind: "profit-target".to_owned(),
        execution_type: "market".to_owned(),
        limit_price: None,
        trailing_amount: None,
        trailing_percent: None,
        extreme_price: None,
        source: default_trade_source(),
        parent_order_id: None,
        client_order_id: None,
        lifecycle: None,
        trigger_adjustments: Vec::new(),
    };
    ledger.barrier_orders.push(barrier_order.clone());
    if let Err(error) = store.persist(&ledger).await {
        ledger.barrier_orders.pop();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist profit barrier: {error}"),
        ));
    }
    Ok(Json(OrderResponse {
        estimated_fee: 0.0,
        fee_complete: true,
        unknown_fee_fills: 0,
        filled_quantity: 0.0,
        remaining_quantity: quantity,
        average_execution_price: None,
        order: None,
        barrier_order: Some(barrier_order),
        fill_evidence: Vec::new(),
        processing_timings_ms: BTreeMap::new(),
    }))
}

#[derive(Deserialize)]
struct CloseProfitableRequest {
    symbol: String,
    #[serde(default)]
    strategy_instance_id: Option<String>,
}

async fn close_profitable(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<CloseProfitableRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    let (position, source) = {
        let ledger = store.ledger.lock().await;
        let auto_positions =
            summarize_positions_from(ledger.trades.iter().filter(|trade| trade.source == "auto"))
                .positions;
        if let Some(position) = auto_positions
            .into_iter()
            .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        {
            // Attribute a manual close to the AutoTrader inventory it closes.
            // AutoTrader reconciles its state from source=auto; tagging this fill
            // manual would leave a stale open position and block future entries.
            (position, "auto".to_owned())
        } else {
            let positions = summarize_positions(&ledger.trades).positions;
            let position = positions
                .into_iter()
                .find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
                .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
            (position, default_trade_source())
        }
    };
    store.ensure_market_mark_polling(&symbol);
    let side = if position.quantity > 0.0 {
        TradeSide::Sell
    } else {
        TradeSide::Buy
    };
    let quantity = position.quantity.abs();
    let client_order_id = match request.strategy_instance_id {
        Some(instance_id) if !instance_id.trim().is_empty() => Some(format!(
            "auto-manual-close:{instance_id}:{}",
            now_ms().map_err(internal_error)?
        )),
        _ => None,
    };
    submit_order(
        State(store),
        Json(NewOrder {
            symbol: symbol.clone(),
            side: side.clone(),
            order_type: OrderType::Ioc,
            quantity,
            limit_price: None,
            measure_execution_quality: true,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
            source,
            client_order_id,
            lifecycle: None,
            // A manual close must not reserve buying power or open/reverse a
            // position if inventory changed after the close quantity was read.
            reduce_only: true,
        }),
    )
    .await
    .map_err(|(status, error)| {
        (
            status,
            format!(
                "Close IOC {symbol} {} {quantity} failed: {error}",
                match side {
                    TradeSide::Buy => "buy",
                    TradeSide::Sell => "sell",
                }
            ),
        )
    })
}
