async fn cancel_protective_barrier(
    state: &AutoTraderState,
    symbol: &str,
    barrier_id: &str,
    created_at: u64,
    strategy: Option<&str>,
) -> Result<bool, (StatusCode, String)> {
    let response = crate::service_rpc::position_request(
        "DELETE",
        &format!("/positions/orders/{barrier_id}"),
        None,
    ).await.map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    if !(200..300).contains(&response.status) && response.status != StatusCode::NOT_FOUND.as_u16() {
        return Ok(false);
    }
    let mut settings = state.settings.lock().await;
    if let Some(position) = settings.positions.get_mut(symbol) {
        position.protective_stops.retain(|stop| stop.barrier_id != barrier_id);
    }
    if let Some(strategy) = strategy {
        if let Some(position) = settings.strategy_positions.get_mut(symbol).and_then(|lots| lots.get_mut(strategy)) {
            position.protective_stops.retain(|stop| stop.barrier_id != barrier_id);
        }
    } else if let Some(lots) = settings.strategy_positions.get_mut(symbol) {
        for position in lots.values_mut() {
            position.protective_stops.retain(|stop| stop.barrier_id != barrier_id);
        }
    }
    if let Some(order) = settings.orders.iter_mut().find(|order| {
        order.timestamp == created_at && order.symbol == symbol && order.purpose == "stop-loss"
    }) {
        order.status = "cancelled".into();
    }
    request_settings_persist(state)?;
    Ok(true)
}
