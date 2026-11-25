async fn get_orders(State(store): State<Arc<PositionStore>>) -> Json<OrdersResponse> {
    let ledger = store.ledger.lock().await;
    Json(OrdersResponse {
        orders: ledger.orders.clone(),
        barrier_orders: ledger.barrier_orders.clone(),
    })
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
