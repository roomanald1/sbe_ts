async fn run_vwap(
    store: Arc<PositionStore>,
    request: NewVwapRun,
    symbol: String,
    status: Arc<Mutex<VwapRunStatus>>,
    cancelled: Arc<AtomicBool>,
) {
    let mut curve = [0.0; NUM_VWAP_BINS];
    let mut total_weight = 0.0;
    for (index, weight) in curve.iter_mut().enumerate() {
        let x = (index as f64 / (NUM_VWAP_BINS - 1) as f64) * 2.0 - 1.0;
        *weight = 1.0 + 3.0 * x * x;
        total_weight += *weight;
    }
    for weight in &mut curve {
        *weight /= total_weight;
    }
    let expected = std::array::from_fn(|index| {
        request.quantity * curve[index] / request.max_participation_rate
    });
    let bin_duration_secs = request.duration_minutes as f64 * 60.0 / NUM_VWAP_BINS as f64;
    let mut manager = match execution::vwap::OrderManager::new(
        execution::vwap::VwapConfig {
            parent_qty: request.quantity,
            historical_vwap_curve: curve,
            max_participation_rate: request.max_participation_rate,
        },
        expected,
        (request.quantity * 0.0001).max(1e-8),
        bin_duration_secs,
    ) {
        Ok(manager) => manager,
        Err(error) => {
            {
                let mut state = status.lock().await;
                state.status = "failed".into();
                state.error = Some(error.to_owned());
            }
            persist_vwap_run_status(&store, &status).await;
            return;
        }
    };
    let run_id = status.lock().await.id.clone();
    let book_slot = store.book_slot(&symbol);
    let (sender, mut receiver) = mpsc::channel(64);
    let trade_symbol = symbol.clone();
    let feed_handle = tokio::spawn(async move {
        loop {
            if let Err(error) = async {
                let mut stream =
                    crate::feed_client::stream(format!("trades:{trade_symbol}")).await?;
                while let Some(item) = stream.next().await {
                    sender
                        .send(item)
                        .await
                        .map_err(|_| anyhow::anyhow!("trade consumer stopped"))?;
                }
                Ok::<(), anyhow::Error>(())
            }
            .await
            {
                crate::service_log!(
                    "ERROR",
                    "VWAP trade feed for {trade_symbol} disconnected: {error:#}; retrying"
                );
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
    let started = tokio::time::Instant::now();
    let duration = Duration::from_secs(request.duration_minutes * 60);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if started.elapsed() >= duration {
            let mut state = status.lock().await;
            if state.status == "running" {
                state.status = "completed".into();
            }
            break;
        }
        let Ok(Some(Ok(bytes))) = timeout(Duration::from_secs(1), receiver.recv()).await else {
            continue;
        };
        if bytes.len() < 97
            || u16::from_le_bytes([bytes[2], bytes[3]]) != 3
            || std::str::from_utf8(&bytes[8..40]).ok().map(str::trim) != Some(symbol.as_str())
        {
            continue;
        }
        let trade_price = f64::from_le_bytes(bytes[80..88].try_into().unwrap());
        let trade_qty = f64::from_le_bytes(bytes[88..96].try_into().unwrap());
        let elapsed = started.elapsed().as_secs_f64();
        let bin_duration = duration.as_secs_f64() / NUM_VWAP_BINS as f64;
        let target_bin = (elapsed / bin_duration).floor() as usize;
        if target_bin >= NUM_VWAP_BINS {
            continue;
        }
        let elapsed_in_bin = elapsed - target_bin as f64 * bin_duration;
        while manager.state().current_bin < target_bin.min(NUM_VWAP_BINS) {
            manager.on_bin_transition();
        }
        let action = manager.on_market_trade(trade_qty, trade_price, elapsed_in_bin);
        let child_qty = match action {
            Some(execution::vwap::OrderAction::NewOrder { qty }) => qty,
            Some(execution::vwap::OrderAction::ModifyOrder { new_qty }) => new_qty,
            Some(execution::vwap::OrderAction::CancelOrder) | None => 0.0,
        };
        if child_qty > EPSILON {
            match execute_vwap_child_from_slot(
                &store,
                &symbol,
                &request.side,
                child_qty,
                &book_slot,
                &request.source,
                &run_id,
            )
            .await
            {
                Ok((filled, average_price)) => {
                    manager.on_execution_fill(average_price.unwrap_or(trade_price), filled)
                }
                Err(error) => {
                    let mut state = status.lock().await;
                    state.error = Some(error);
                }
            }
            manager.on_order_complete();
            let scheduler_state = manager.state();
            let benchmark = manager.benchmark();
            let mut state = status.lock().await;
            state.executed_quantity = scheduler_state.executed_qty;
            state.market_vwap = benchmark.market_vwap();
            state.execution_vwap = benchmark.execution_vwap();
            state.slippage = benchmark.slippage();
            if scheduler_state.remaining_qty <= EPSILON {
                state.status = "completed".into();
            }
            let finished = state.status == "completed";
            drop(state);
            persist_vwap_run_status(&store, &status).await;
            if finished {
                break;
            }
        }
    }
    feed_handle.abort();
    persist_vwap_run_status(&store, &status).await;
}

async fn execute_vwap_child(
    store: &PositionStore,
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    source: &str,
    parent_order_id: &str,
) -> Result<(f64, Option<f64>), String> {
    let slot = store
        .books
        .read()
        .expect("book map poisoned")
        .get(symbol)
        .cloned()
        .ok_or_else(|| "Live order book unavailable for VWAP child order".to_string())?;
    execute_vwap_child_from_slot(
        store,
        symbol,
        side,
        quantity,
        &slot,
        source,
        parent_order_id,
    )
    .await
}

async fn execute_vwap_child_from_slot(
    store: &PositionStore,
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
    book_slot: &Arc<ArcSwapOption<BookSnapshot>>,
    source: &str,
    parent_order_id: &str,
) -> Result<(f64, Option<f64>), String> {
    let book = book_slot
        .load_full()
        .ok_or_else(|| "Live order book unavailable for VWAP child order".to_string())?;
    let matched = match_book(side, quantity, None, &book);
    let filled = matched.filled_quantity;
    if filled <= EPSILON {
        return Ok((0.0, None));
    }
    let average = matched.average_execution_price.unwrap_or_default();
    let reference_price = matched.reference_price;
    let fills = matched
        .fills
        .into_iter()
        .map(|fill| (fill.quantity, fill.price))
        .collect::<Vec<_>>();
    let fee_rate = configured_fee_rates().taker.abs();
    let timestamp = now_ms().map_err(|error| error.to_string())?;
    let mut ledger = store.ledger.lock().await;
    let offset = ledger.trades.len();
    for (index, (qty, execution_price)) in fills.into_iter().enumerate() {
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{offset}"),
            symbol: symbol.to_owned(),
            side: side.clone(),
            quantity: qty,
            execution_price,
            reference_price,
            timestamp,
            source: source.to_owned(),
            parent_order_id: Some(parent_order_id.to_owned()),
            fee: -qty * execution_price * fee_rate.abs(),
            fee_currency: symbol
                .split_once('-')
                .map(|(_, quote)| quote.to_owned())
                .unwrap_or_default(),
            fee_rate,
            fee_liquidity: "taker".to_owned(),
            resting_order_id: None,
            decision_timestamp: None,
            decision_price: None,
            decision_mid_price: None,
            slippage: None,
            slippage_bps: None,
        });
    }
    store
        .persist(&ledger)
        .await
        .map_err(|error| format!("Unable to persist VWAP fills: {error}"))?;
    Ok((filled, Some(average)))
}

