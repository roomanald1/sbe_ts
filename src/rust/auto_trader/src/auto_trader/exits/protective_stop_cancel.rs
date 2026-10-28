async fn cancel_protective_barrier(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
    symbol: &str,
    barrier_id: &str,
    created_at: u64,
    strategy: &str,
) -> Result<bool, (StatusCode, String)> {
    let response = client
        .delete(format!("{base}/positions/orders/{barrier_id}"))
        .send()
        .await
        .map_err(upstream_error)?;
    if !response.status().is_success() && response.status() != StatusCode::NOT_FOUND {
        return Ok(false);
    }
    let mut settings = state.settings.lock().await;
    if let Some(position) = settings.positions.get_mut(symbol) {
        position.protective_stops.retain(|stop| stop.barrier_id != barrier_id);
    }
    if let Some(position) = settings.strategy_positions.get_mut(symbol).and_then(|lots| lots.get_mut(strategy)) {
        position.protective_stops.retain(|stop| stop.barrier_id != barrier_id);
    }
    if let Some(order) = settings.orders.iter_mut().find(|order| {
        order.timestamp == created_at && order.symbol == symbol && order.purpose == "stop-loss"
    }) {
        order.status = "cancelled".into();
    }
    request_settings_persist(state)?;
    Ok(true)
}

