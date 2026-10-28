async fn persist_vwap_run_status(store: &PositionStore, status: &Arc<Mutex<VwapRunStatus>>) {
    let snapshot = status.lock().await.clone();
    let mut ledger = store.ledger.lock().await;
    if let Some(index) = ledger
        .scheduled_runs
        .iter()
        .position(|run| run.id == snapshot.id)
    {
        ledger.scheduled_runs[index] = snapshot;
        let id = ledger.scheduled_runs[index].id.clone();
        if let Err(error) = store.persist(&ledger).await {
            crate::service_log!(
                "ERROR",
                "Unable to persist scheduled execution {id}: {error:#}"
            );
        }
    }
}

async fn get_vwap_runs(State(store): State<Arc<PositionStore>>) -> Json<VwapRunsResponse> {
    let entries = store.vwap_runs.lock().await;
    let mut runs = Vec::with_capacity(entries.len());
    for entry in entries.values() {
        runs.push(entry.status.lock().await.clone());
    }
    Json(VwapRunsResponse { runs })
}

async fn start_vwap(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<NewVwapRun>,
) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    start_scheduled_run(store, request).await
}
async fn start_twap(
    State(store): State<Arc<PositionStore>>,
    Json(mut request): Json<NewVwapRun>,
) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    request.strategy = ExecutionStrategy::Twap;
    start_scheduled_run(store, request).await
}
async fn start_scheduled_run(
    store: Arc<PositionStore>,
    request: NewVwapRun,
) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !matches!(request.source.as_str(), "manual" | "auto") {
        return Err((
            StatusCode::BAD_REQUEST,
            "Trade source must be manual or auto".into(),
        ));
    }
    if !request.quantity.is_finite() || request.quantity <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Quantity must be finite and greater than zero".into(),
        ));
    }
    if !(5..=390).contains(&request.duration_minutes) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Duration must be between 5 and 390 minutes".into(),
        ));
    }
    if request.strategy == ExecutionStrategy::Vwap
        && (!request.max_participation_rate.is_finite()
            || !(0.01..=1.0).contains(&request.max_participation_rate))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Participation rate must be between 1% and 100%".into(),
        ));
    }
    store.ensure_subscription(&symbol);
    if store.book(&symbol).is_none() {
        let wait = async {
            loop {
                let notified = store.book_changed.notified();
                if store.book(&symbol).is_some() {
                    break;
                }
                notified.await;
            }
        };
        timeout(Duration::from_secs(8), wait).await.map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Waiting for live OKX book timed out".to_owned(),
            )
        })?;
    }
    let side_book = store.book(&symbol).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Live book is required to reserve VWAP buying power".to_owned(),
    ))?;
    let price = match request.side {
        TradeSide::Buy => side_book
            .asks
            .iter()
            .map(|level| level.price)
            .fold(0.0_f64, f64::max),
        TradeSide::Sell => side_book
            .bids
            .first()
            .map(|level| level.price)
            .unwrap_or(0.0),
    };
    if price <= 0.0 {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Live reference price unavailable for VWAP".into(),
        ));
    }
    let mut account_ledger = store.ledger.lock().await;
    ensure_paper_capacity(
        &store,
        &account_ledger,
        &symbol,
        &request.side,
        request.quantity,
        price,
        configured_fee_rates().taker,
    )
    .await?;
    let timestamp = now_ms().map_err(internal_error)?;
    let id = request
        .client_order_id
        .clone()
        .unwrap_or_else(|| format!("vwap-{timestamp}-{}", symbol.to_ascii_lowercase()));
    let status = Arc::new(Mutex::new(VwapRunStatus {
        id: id.clone(),
        symbol: symbol.clone(),
        side: request.side.clone(),
        quantity: request.quantity,
        executed_quantity: 0.0,
        strategy: if request.strategy == ExecutionStrategy::Twap {
            "twap".to_owned()
        } else {
            "vwap".to_owned()
        },
        duration_minutes: request.duration_minutes,
        started_at: timestamp,
        status: "running".into(),
        market_vwap: 0.0,
        execution_vwap: 0.0,
        slippage: 0.0,
        error: None,
    }));
    let cancelled = Arc::new(AtomicBool::new(false));
    account_ledger
        .scheduled_runs
        .push(status.lock().await.clone());
    if account_ledger.scheduled_runs.len() > 500 {
        account_ledger.scheduled_runs.remove(0);
    }
    store.persist(&account_ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist scheduled execution: {error}"),
        )
    })?;
    store.vwap_runs.lock().await.insert(
        id,
        VwapRunEntry {
            status: status.clone(),
            cancelled: cancelled.clone(),
        },
    );
    store.ensure_subscription(&symbol);
    drop(account_ledger);
    let initial_status = status.lock().await.clone();
    if request.strategy == ExecutionStrategy::Twap {
        tokio::spawn(run_twap(
            Arc::clone(&store),
            request,
            symbol,
            Arc::clone(&status),
            cancelled,
        ));
    } else {
        tokio::spawn(run_vwap(
            Arc::clone(&store),
            request,
            symbol,
            Arc::clone(&status),
            cancelled,
        ));
    }
    Ok(Json(initial_status))
}

