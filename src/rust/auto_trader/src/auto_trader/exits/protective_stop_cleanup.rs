async fn cancel_protective_stops(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
    strategy: Option<&str>,
) -> Result<bool, (StatusCode, String)> {
    let stops = {
        let settings = state.settings.lock().await;
        settings.positions.get(symbol)
            .map(|position| position.protective_stops.iter()
                .filter(|stop| strategy.is_none_or(|key| stop.strategy.is_empty() || stop.strategy == key))
                .cloned().collect::<Vec<_>>())
            .unwrap_or_default()
    };
    for stop in stops {
        let response = client
            .delete(format!("{base}/positions/orders/{}", stop.barrier_id))
            .send()
            .await
            .map_err(upstream_error)?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let mut settings = state.settings.lock().await;
        if let Some(position) = settings.positions.get_mut(symbol) {
            position.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
        }
        if let Some(lots) = settings.strategy_positions.get_mut(symbol) {
            for lot in lots.values_mut() {
                lot.protective_stops.retain(|tracked| tracked.barrier_id != stop.barrier_id);
            }
        }
        if let Some(order) = settings.orders.iter_mut().find(|order| {
            order.timestamp == stop.order_timestamp && order.symbol == symbol && order.purpose == "stop-loss"
        }) {
            order.status = "cancelled".into();
        }
        request_settings_persist(state)?;
    }
    Ok(true)
}
