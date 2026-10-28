#[derive(Deserialize, Default)]

struct PositionTradesQuery {
    symbol: Option<String>,
    source: Option<String>,
    since_ms: Option<u64>,
}
async fn get_position_trades(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PositionTradesQuery>,
) -> Json<TradesResponse> {
    let symbol = query.symbol.map(|value| value.to_ascii_uppercase());
    let ledger = store.ledger.lock().await;
    Json(TradesResponse {
        trades: ledger
            .trades
            .iter()
            .filter(|trade| symbol.as_ref().is_none_or(|value| trade.symbol == *value))
            .filter(|trade| query.source.as_ref().is_none_or(|value| trade.source == *value))
            .filter(|trade| query.since_ms.is_none_or(|value| trade.timestamp >= value))
            .cloned()
            .map(|mut trade| {
                trade.fee = signed_fee_cashflow(trade.fee);
                trade
            })
            .collect(),
    })
}
