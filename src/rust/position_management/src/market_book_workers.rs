async fn subscribe_market_book(
    State(store): State<Arc<PositionStore>>,
    Path(symbol): Path<String>,
) -> StatusCode {
    let symbol = symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return StatusCode::BAD_REQUEST;
    }
    store.ensure_subscription(&symbol);
    StatusCode::NO_CONTENT
}

async fn expire_orders_loop(store: Arc<PositionStore>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Ok(now) = now_ms() else {
            continue;
        };
        let mut ledger = store.ledger.lock().await;
        let old_len = ledger.orders.len();
        ledger
            .orders
            .retain(|order| order.expires_at.is_none_or(|expires| expires > now));
        if ledger.orders.len() != old_len {
            if let Err(error) = store.persist(&ledger).await {
                crate::service_log!("ERROR", "Could not persist GTD expiry: {error}");
            }
        }
    }
}

async fn barrier_order_loop(store: Arc<PositionStore>) {
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let barriers = store
            .ledger
            .lock()
            .await
            .barrier_orders
            .iter()
            .filter(|order| order.status == "armed")
            .cloned()
            .collect::<Vec<_>>();
        if barriers.is_empty() {
            continue;
        }
        let books = store.books_snapshot();
        let Ok(now) = now_ms() else {
            continue;
        };
        for barrier in barriers {
            let Some(book) = books.get(&barrier.symbol) else {
                continue;
            };
            if barrier.retry_after.is_some_and(|retry_after| now < retry_after)
                || now.saturating_sub(book.updated_at_ms) > 5_000
            {
                continue;
            }
            let current = match barrier.side {
                TradeSide::Buy => book.asks.first().map(|level| level.price),
                TradeSide::Sell => book.bids.first().map(|level| level.price),
            };
            let Some(current) = current.filter(|price| price.is_finite() && *price > 0.0) else {
                continue;
            };
            let mut effective_barrier = barrier.clone();
            if barrier.trailing_amount.is_some() || barrier.trailing_percent.is_some() {
                let prior = barrier.extreme_price.unwrap_or(current);
                let favorable = if matches!(barrier.side, TradeSide::Sell) {
                    current > prior
                } else {
                    current < prior
                };
                if favorable {
                    let distance = barrier
                        .trailing_amount
                        .unwrap_or(current * barrier.trailing_percent.unwrap_or_default());
                    effective_barrier.extreme_price = Some(current);
                    effective_barrier.trigger_price = if matches!(barrier.side, TradeSide::Sell) {
                        current - distance
                    } else {
                        current + distance
                    };
                    let mut ledger = store.ledger.lock().await;
                    if let Some(order) = ledger
                        .barrier_orders
                        .iter_mut()
                        .find(|order| order.id == barrier.id && order.status == "armed")
                    {
                        order.extreme_price = effective_barrier.extreme_price;
                        order.trigger_price = effective_barrier.trigger_price;
                        if let Err(error) = store.persist(&ledger).await {
                            crate::service_log!(
                                "ERROR",
                                "Could not persist trailing stop {}: {error}",
                                barrier.id
                            );
                        }
                    }
                }
            }
            let triggered = match effective_barrier.trigger_direction {
                BarrierDirection::Above => current >= effective_barrier.trigger_price,
                BarrierDirection::Below => current <= effective_barrier.trigger_price,
            };
            if !triggered {
                continue;
            }
            let claimed = {
                let mut ledger = store.ledger.lock().await;
                let Some(index) = ledger
                    .barrier_orders
                    .iter()
                    .position(|order| order.id == barrier.id && order.status == "armed")
                else {
                    continue;
                };
                let is_auto_stop = barrier.source == "auto"
                    && barrier.parent_order_id.as_deref().is_some_and(|id| id.starts_with("auto-stop-"));
                let superseded_auto_stop = is_auto_stop && ledger.trades.iter()
                    .filter(|trade| trade.symbol == barrier.symbol && trade.source == barrier.source)
                    .max_by_key(|trade| trade.timestamp)
                    .is_some_and(|trade| {
                        trade.timestamp > barrier.created_at
                            && trade.parent_order_id.as_deref() != Some(barrier.id.as_str())
                    });
                if superseded_auto_stop {
                    // A later trade means this barrier belongs to an older
                    // position generation. Reduce-only alone is insufficient:
                    // it could still close a newer same-side position.
                    ledger.barrier_orders[index].status = "cancelled".to_owned();
                    ledger.barrier_orders[index].triggered_at = None;
                    ledger.barrier_orders[index].retry_after = None;
                    if let Err(error) = store.persist(&ledger).await {
                        crate::service_log!(
                            "ERROR",
                            "Could not cancel superseded protective barrier {}: {error}",
                            barrier.id
                        );
                        ledger.barrier_orders[index].status = "armed".to_owned();
                    }
                    continue;
                }
                ledger.barrier_orders[index].status = "triggering".to_owned();
                ledger.barrier_orders[index].triggered_at = Some(now);
                ledger.barrier_orders[index].retry_after = None;
                if let Err(error) = store.persist(&ledger).await {
                    crate::service_log!(
                        "ERROR",
                        "Could not persist triggered barrier {}: {error}",
                        barrier.id
                    );
                    ledger.barrier_orders[index].status = "armed".to_owned();
                    ledger.barrier_orders[index].triggered_at = None;
                    ledger.barrier_orders[index].retry_after = None;
                    continue;
                }
                ledger.barrier_orders[index].clone()
            };
            let result = submit_order(
                State(Arc::clone(&store)),
                Json(NewOrder {
                    symbol: claimed.symbol.clone(),
                    side: claimed.side.clone(),
                    order_type: if claimed.execution_type == "limit" {
                        OrderType::Limit
                    } else {
                        OrderType::Market
                    },
                    quantity: claimed.remaining_quantity,
                    limit_price: claimed.limit_price,
                    trigger_price: None,
                    trigger_direction: None,
                    trailing_amount: None,
                    trailing_percent: None,
                    expires_at: None,
                    time_in_force: None,
                    measure_execution_quality: false,
                    source: claimed.source.clone(),
                    client_order_id: Some(claimed.id.clone()),
                    reduce_only: claimed.source == "auto"
                        && claimed
                            .parent_order_id
                            .as_deref()
                            .is_some_and(|id| id.starts_with("auto-stop-")),
                }),
            )
            .await;
            let mut ledger = store.ledger.lock().await;
            if let Some(order) = ledger
                .barrier_orders
                .iter_mut()
                .find(|order| order.id == claimed.id)
            {
                match result {
                    Ok(response) => {
                        let response = response.0;
                        let is_auto_stop = claimed.source == "auto"
                            && claimed
                                .parent_order_id
                                .as_deref()
                                .is_some_and(|id| id.starts_with("auto-stop-"));
                        order.filled_quantity += response.filled_quantity;
                        order.remaining_quantity =
                            (order.quantity - order.filled_quantity).max(0.0);
                        if response.filled_quantity > EPSILON {
                            order.average_execution_price = response.average_execution_price;
                        }
                        order.fee_paid += response.estimated_fee;
                        order.status = if is_auto_stop
                            && response.remaining_quantity > EPSILON
                            && order.remaining_quantity > EPSILON
                        {
                            order.triggered_at = None;
                            order.retry_after = Some(now.saturating_add(5_000));
                            "armed".to_owned()
                        } else if is_auto_stop && response.filled_quantity <= EPSILON {
                            "cancelled".to_owned()
                        } else if claimed.execution_type == "limit" && response.order.is_some() {
                            "activated".to_owned()
                        } else if order.remaining_quantity <= EPSILON {
                            "filled".to_owned()
                        } else if order.filled_quantity > EPSILON {
                            "partially_filled".to_owned()
                        } else {
                            "unfilled".to_owned()
                        };
                        order.error = None;
                    }
                    Err((_, error)) => {
                        order.status = "rejected".to_owned();
                        order.error = Some(error);
                    }
                }
            }
            if let Err(error) = store.persist(&ledger).await {
                crate::service_log!(
                    "ERROR",
                    "Could not persist barrier execution {}: {error}",
                    claimed.id
                );
            }
        }
    }
}

