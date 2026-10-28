#[derive(Deserialize)]

struct PositionSourceQuery {
    source: Option<String>,
}
async fn get_realized_pnl(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<RealizedPnlQuery>,
) -> Result<Json<RealizedPnlResponse>, (StatusCode, String)> {
    if query
        .source
        .as_deref()
        .is_some_and(|source| !matches!(source, "manual" | "auto"))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Position source must be manual or auto".into(),
        ));
    }
    let since_ms = query.since_ms.unwrap_or(0);
    let ledger = store.ledger.lock().await;
    let mut trades = ledger
        .trades
        .iter()
        .filter(|trade| {
            query
                .source
                .as_deref()
                .is_none_or(|source| trade.source == source)
        })
        .cloned()
        .collect::<Vec<_>>();
    trades.sort_by_key(|trade| trade.timestamp);
    let mut positions = BTreeMap::<String, Position>::new();
    let mut today_by_quote = BTreeMap::<String, f64>::new();
    let mut today_by_parent_order = BTreeMap::<String, f64>::new();
    let barrier_created_at = ledger
        .barrier_orders
        .iter()
        .map(|barrier| (barrier.id.clone(), barrier.created_at.to_string()))
        .collect::<HashMap<_, _>>();
    let mut today_by_barrier_created_at = BTreeMap::<String, f64>::new();
    let mut all_time_by_quote = BTreeMap::<String, f64>::new();
    for trade in &trades {
        let position = positions.entry(trade.symbol.clone()).or_insert(Position {
            symbol: trade.symbol.clone(),
            quantity: 0.0,
            average_entry_price: 0.0,
            realized_pnl: 0.0,
            entry_fees: 0.0,
            fees_paid: 0.0,
            estimated_exit_fee_rate: 0.0,
            last_trade_timestamp: trade.timestamp,
        });
        let prior_realized = position.realized_pnl;
        apply_trade(position, trade);
        let realized_delta = position.realized_pnl - prior_realized;
        if realized_delta.abs() <= EPSILON {
            continue;
        }
        let quote = trade
            .symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or("USD");
        *all_time_by_quote.entry(quote.to_owned()).or_default() += realized_delta;
        if trade.timestamp >= since_ms {
            *today_by_quote.entry(quote.to_owned()).or_default() += realized_delta;
            if let Some(parent_order_id) = trade.parent_order_id.as_deref() {
                *today_by_parent_order
                    .entry(parent_order_id.to_owned())
                    .or_default() += realized_delta;
                if let Some(created_at) = barrier_created_at.get(parent_order_id) {
                    *today_by_barrier_created_at
                        .entry(created_at.clone())
                        .or_default() += realized_delta;
                }
            }
        }
    }
    drop(ledger);
    let rates = get_usd_rates().await?.0.rates;
    let convert_usd = |totals: &BTreeMap<String, f64>| -> Option<f64> {
        totals.iter().try_fold(0.0, |sum, (quote, amount)| {
            rates.get(quote).map(|rate| sum + amount * rate)
        })
    };
    Ok(Json(RealizedPnlResponse {
        realized_pnl_usd: convert_usd(&today_by_quote),
        realized_pnl_by_quote: today_by_quote,
        realized_pnl_by_parent_order: today_by_parent_order,
        realized_pnl_by_barrier_created_at: today_by_barrier_created_at,
        realized_pnl_all_time_usd: convert_usd(&all_time_by_quote),
        realized_pnl_all_time_by_quote: all_time_by_quote,
    }))
}

async fn get_positions(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PositionSourceQuery>,
) -> Result<Json<PositionsResponse>, (StatusCode, String)> {
    if query
        .source
        .as_deref()
        .is_some_and(|source| !matches!(source, "manual" | "auto"))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Position source must be manual or auto".into(),
        ));
    }
    let ledger = store.ledger.lock().await;
    let trades = ledger
        .trades
        .iter()
        .filter(|trade| {
            query
                .source
                .as_deref()
                .is_none_or(|source| trade.source == source)
        })
        .cloned()
        .collect::<Vec<_>>();
    Ok(Json(summarize_positions(&trades)))
}

