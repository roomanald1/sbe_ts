async fn get_account(
    State(store): State<Arc<PositionStore>>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let ledger = store.ledger.lock().await.clone();
    Ok(Json(account_snapshot(&store, &ledger).await?))
}
async fn set_account_leverage(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<LeverageRequest>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    if !request.max_leverage.is_finite() || !(1.0..=10.0).contains(&request.max_leverage) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Maximum leverage must be between 1x and 10x".into(),
        ));
    }
    let mut ledger = store.ledger.lock().await;
    ledger.max_leverage = Some(request.max_leverage);
    store.persist(&ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist leverage setting: {error}"),
        )
    })?;
    Ok(Json(account_snapshot(&store, &ledger).await?))
}
async fn top_up_account(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<TopupRequest>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    if !request.amount.is_finite() || request.amount <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Top-up amount must be positive".into(),
        ));
    }
    let mut ledger = store.ledger.lock().await;
    ledger.topups.push(AccountTopup {
        amount: request.amount,
        timestamp: now_ms().map_err(internal_error)?,
    });
    store.persist(&ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist account top-up: {error}"),
        )
    })?;
    Ok(Json(account_snapshot(&store, &ledger).await?))
}

