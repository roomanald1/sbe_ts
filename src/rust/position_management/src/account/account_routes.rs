async fn get_account(
    State(store): State<Arc<PositionStore>>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let rates = get_usd_rates().await?.0.rates;
    let ledger = store.ledger.lock().await;
    Ok(Json(account_snapshot_with_rates(&store, &ledger, &rates)?))
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
    let rates = get_usd_rates().await?.0.rates;
    let mut ledger = store.ledger.lock().await;
    ledger.max_leverage = Some(request.max_leverage);
    store.persist(&ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist leverage setting: {error}"),
        )
    })?;
    Ok(Json(account_snapshot_with_rates(&store, &ledger, &rates)?))
}
async fn top_up_account(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<TopupRequest>,
) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    if !request.amount.is_finite()
        || request.amount <= 0.0
        || request.contribution_id.trim().is_empty()
        || request.contribution_id.len() > 256
        || request.contribution_id.chars().any(char::is_control)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Top-up amount and stable contribution ID are required".into(),
        ));
    }
    let rates = get_usd_rates().await?.0.rates;
    {
        let ledger = store.ledger.lock().await;
        if let Some(existing) = ledger
            .topups
            .iter()
            .find(|topup| topup.contribution_id.as_deref() == Some(&request.contribution_id))
        {
            if existing.amount != request.amount {
                return Err((
                    StatusCode::CONFLICT,
                    "Contribution ID was already used with a different amount".into(),
                ));
            }
            return Ok(Json(account_snapshot_with_rates(&store, &ledger, &rates)?));
        }
    }

    let server_now = now_ms().map_err(internal_error)?;
    let source_event = paper_exchange_client::paper_exchange_post(
        "/internal/financial/evidence",
        serde_json::json!({
            "schema_version": "paper_exchange_financial_evidence:v1",
            "event_id": format!("operator-contribution:{}", request.contribution_id),
            "source_system": "paper_operator",
            "source_epoch": "paper-operator-contributions:v1",
            "source_cursor": server_now,
            "source_record_id": request.contribution_id,
            "source_timestamp_utc_ms": server_now,
            "event_type": "cash_movement",
            "movement_id": request.contribution_id,
            "portfolio_id": "paper-default",
            "movement_kind": "contribution",
            "currency": "USD",
            "amount": request.amount,
        }),
    )
    .await
    .map_err(|error| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Unable to durably journal paper contribution before applying it: {error}"),
        )
    })?;
    let financial_evidence_epoch = source_event
        .get("source_epoch")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Paper Exchange did not return financial evidence epoch".into(),
        ))?;
    let financial_evidence_cursor = source_event
        .get("source_cursor")
        .and_then(serde_json::Value::as_u64)
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Paper Exchange did not return financial evidence cursor".into(),
        ))?;
    let timestamp = source_event
        .get("source_timestamp_utc_ms")
        .and_then(serde_json::Value::as_u64)
        .filter(|timestamp| *timestamp > 0 && *timestamp <= server_now)
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Paper Exchange did not return a valid authoritative contribution timestamp".into(),
        ))?;

    let mut ledger = store.ledger.lock().await;
    if let Some(existing) = ledger
        .topups
        .iter()
        .find(|topup| topup.contribution_id.as_deref() == Some(&request.contribution_id))
    {
        if existing.amount != request.amount {
            return Err((
                StatusCode::CONFLICT,
                "Contribution ID was already used with a different amount".into(),
            ));
        }
        return Ok(Json(account_snapshot_with_rates(&store, &ledger, &rates)?));
    }
    ledger.topups.push(AccountTopup {
        amount: request.amount,
        timestamp,
        contribution_id: Some(request.contribution_id),
        financial_evidence_epoch: Some(financial_evidence_epoch),
        financial_evidence_cursor: Some(financial_evidence_cursor),
    });
    if let Err(error) = store.persist(&ledger).await {
        ledger.topups.pop();
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist account top-up after durable evidence: {error}"),
        ));
    }
    Ok(Json(account_snapshot_with_rates(&store, &ledger, &rates)?))
}

