async fn cancel_order(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut ledger = store.ledger.lock().await;
    let original_orders = ledger.orders.len();
    let original_barriers = ledger.barrier_orders.len();
    ledger.orders.retain(|order| order.id != id);
    if ledger.orders.len() == original_orders {
        if let Some(order) = ledger.barrier_orders.iter().find(|order| order.id == id) {
            if order.status != "armed" {
                return Err((
                    StatusCode::CONFLICT,
                    "Barrier order has already triggered and cannot be cancelled".into(),
                ));
            }
        }
        ledger.barrier_orders.retain(|order| order.id != id);
        if ledger.barrier_orders.len() == original_barriers {
            return Err((StatusCode::NOT_FOUND, "Order not found".into()));
        }
    }
    store.persist(&ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist order cancellation: {error}"),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

