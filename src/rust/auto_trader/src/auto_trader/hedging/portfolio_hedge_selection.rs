async fn maybe_place_portfolio_hedge(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let candidates_request = client
        .get(format!("{base}/positions/hedge-candidates"))
        .send();
    let rates_request = client.get(format!("{base}/positions/usd-rates")).send();
    let (candidates_response, rates_response) =
        tokio::try_join!(candidates_request, rates_request).map_err(upstream_error)?;
    let candidates_response = candidates_response
        .error_for_status()
        .map_err(upstream_error)?;
    let rates_response = rates_response.error_for_status().map_err(upstream_error)?;
    let (candidates, rates_response) = tokio::try_join!(
        candidates_response.json::<serde_json::Value>(),
        rates_response.json::<serde_json::Value>(),
    )
    .map_err(upstream_error)?;
    let rates = &rates_response["rates"];
    let positions = candidates["positions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let (holdings, fallback_marks, unknown_marks) = calculate_risk_holdings(&positions, rates);
    update_risk_coverage_note(state, positions.len(), fallback_marks, unknown_marks).await;
    if holdings.is_empty() || unknown_marks > 0 {
        return Ok(None);
    }
    let pair_correlations: Vec<HedgePairCorrelationData> = serde_json::from_value(
        candidates["pair_correlations"].clone(),
    )
    .unwrap_or_default();
    let portfolio_variance = calculate_portfolio_variance(&holdings, &pair_correlations);
    let portfolio_shock_risk = portfolio_variance.max(0.0).sqrt() * PORTFOLIO_SHOCK_PERCENT;
    let settings_snapshot = state.settings.lock().await.clone();
    let hedge_execution = settings_snapshot
        .learning
        .get("portfolio-hedge:execution")
        .cloned()
        .unwrap_or_default();
    let hedge_win_rate = hedge_execution.positive as f64 / hedge_execution.samples.max(1) as f64;
    // A hedge that reliably gets filled can be attempted at lower risk. Poor
    // fill outcomes raise the trigger so the trader does not repeatedly cross
    // into illiquid markets. Keep the adjustment bounded and require evidence.
    let hedge_risk_scale = if hedge_execution.samples >= MIN_HEDGE_EXECUTION_LEARNING_SAMPLES {
        (1.25 - 0.5 * hedge_win_rate).clamp(0.75, 1.25)
    } else {
        1.0
    };
    let risk_trigger = settings_snapshot.daily_realized_loss_limit * hedge_risk_scale;
    if portfolio_shock_risk <= risk_trigger {
        return Ok(None);
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let best = select_portfolio_hedge_candidate(
        &holdings,
        &positions,
        rates,
        &settings_snapshot,
        now_ms,
        portfolio_shock_risk,
    );
    let Some(candidate) = best else {
        return Ok(Some(skipped(
            "Portfolio risk is elevated, but no liquid hedge has enough reliable correlation data; new exposure is blocked",
        )));
    };
    execute_portfolio_hedge(client, base, state, candidate, now_ms, portfolio_shock_risk).await
}

