async fn cancel_vwap(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let runs = store.vwap_runs.lock().await;
    let Some(entry) = runs.get(&id) else {
        return Err((StatusCode::NOT_FOUND, "VWAP execution not found".into()));
    };
    let status_arc = Arc::clone(&entry.status);
    entry.cancelled.store(true, Ordering::Relaxed);
    let mut status = status_arc.lock().await;
    if status.status == "running" {
        status.status = "cancelled".into();
    }
    drop(status);
    drop(runs);
    persist_vwap_run_status(&store, &status_arc).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn run_twap(
    store: Arc<PositionStore>,
    request: NewVwapRun,
    symbol: String,
    status: Arc<Mutex<VwapRunStatus>>,
    cancelled: Arc<AtomicBool>,
) {
    const SLICES: usize = 78;
    let interval = Duration::from_secs_f64(request.duration_minutes as f64 * 60.0 / SLICES as f64);
    let run_id = status.lock().await.id.clone();
    let mut executed = 0.0;
    let mut execution_notional = 0.0;
    let mut market_notional = 0.0;
    let mut market_samples = 0usize;
    for slice in 0..SLICES {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if slice > 0 {
            tokio::time::sleep(interval).await;
        }
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        let remaining = (request.quantity - executed).max(0.0);
        if remaining <= EPSILON {
            break;
        }
        let target_cumulative = if slice + 1 == SLICES {
            request.quantity
        } else {
            request.quantity * (slice + 1) as f64 / SLICES as f64
        };
        let qty = (target_cumulative - executed).max(0.0).min(remaining);
        let book = store.book(&symbol);
        if let Some(book) = book {
            if let (Some(bid), Some(ask)) = (book.bids.first(), book.asks.first()) {
                market_notional += (bid.price + ask.price) * 0.5;
                market_samples += 1;
            }
        }
        match execute_vwap_child(
            &store,
            &symbol,
            &request.side,
            qty,
            &request.source,
            &run_id,
        )
        .await
        {
            Ok((filled, average)) => {
                executed += filled;
                if let Some(price) = average {
                    execution_notional += filled * price;
                }
                let mut state = status.lock().await;
                state.executed_quantity = executed;
                state.execution_vwap = if executed > 0.0 {
                    execution_notional / executed
                } else {
                    0.0
                };
                state.market_vwap = if market_samples > 0 {
                    market_notional / market_samples as f64
                } else {
                    0.0
                };
                state.slippage = match request.side {
                    TradeSide::Buy => state.execution_vwap - state.market_vwap,
                    TradeSide::Sell => state.market_vwap - state.execution_vwap,
                };
                if executed + EPSILON >= request.quantity {
                    state.status = "completed".to_owned();
                }
                drop(state);
                persist_vwap_run_status(&store, &status).await;
                if executed + EPSILON >= request.quantity {
                    break;
                }
            }
            Err(error) => {
                status.lock().await.error = Some(error);
                persist_vwap_run_status(&store, &status).await;
            }
        }
    }
    {
        let mut state = status.lock().await;
        if state.status == "running" {
            state.status = if cancelled.load(Ordering::Relaxed) {
                "cancelled"
            } else {
                "completed"
            }
            .to_owned();
        }
    }
    persist_vwap_run_status(&store, &status).await;
}

