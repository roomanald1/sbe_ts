#[derive(Deserialize, Default)]

struct PositionTradesQuery {
    symbol: Option<String>,
    source: Option<String>,
    since_ms: Option<u64>,
    before_ms: Option<u64>,
    limit: Option<usize>,
    offset: Option<usize>,
}
async fn get_position_trades(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PositionTradesQuery>,
) -> Json<TradesResponse> {
    let symbol = query.symbol.map(|value| value.to_ascii_uppercase());
    let ledger = store.ledger.lock().await;
    let matches_query = |trade: &&PositionTrade| {
        symbol.as_ref().is_none_or(|value| trade.symbol == *value)
            && query.source.as_ref().is_none_or(|value| trade.source == *value)
            && query.since_ms.is_none_or(|value| trade.timestamp >= value)
            && query.before_ms.is_none_or(|value| trade.timestamp < value)
    };
    let total = ledger.trades.iter().filter(|trade| matches_query(trade)).count();
    let limit = query.limit.map(|value| value.clamp(1, 500));
    let offset = query.offset.unwrap_or_default();
    let has_older = query.since_ms.map(|since| ledger.trades.iter().any(|trade| {
        trade.timestamp < since && query.source.as_ref().is_none_or(|value| trade.source == *value)
    }));
    let mut page = if let Some(limit) = limit {
        ledger.trades.iter().rev().filter(|trade| matches_query(trade)).skip(offset).take(limit).cloned().collect::<Vec<_>>()
    } else {
        ledger.trades.iter().filter(|trade| matches_query(trade)).cloned().collect::<Vec<_>>()
    };
    for trade in &mut page {
        trade.fee = signed_fee_cashflow(trade.fee);
    }
    let next_offset = limit.filter(|_| offset + page.len() < total).map(|_| offset + page.len());
    Json(TradesResponse { trades: page, total: limit.map(|_| total), next_offset, has_older })
}
