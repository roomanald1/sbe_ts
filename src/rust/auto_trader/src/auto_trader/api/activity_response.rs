async fn apply_protection_order_failure_reasons(
    client: &reqwest::Client,
    position_url: &str,
    orders: &mut [TraderOrder],
    fallback_open_symbols: &[String],
) {
    let ledger_open_symbols = match client
        .get(format!("{position_url}/positions?source=auto"))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
    {
        Ok(response) => match response.json::<serde_json::Value>().await {
            Ok(body) => body["positions"]
                .as_array()
                .map(|positions| {
                    positions
                        .iter()
                        .filter(|position| {
                            position["quantity"]
                                .as_f64()
                                .is_some_and(|quantity| quantity.abs() > 1e-12)
                        })
                        .filter_map(|position| {
                            position["symbol"].as_str().map(str::to_owned)
                        })
                        .collect::<BTreeSet<_>>()
                }),
            Err(_) => None,
        },
        Err(_) => None,
    }
    .unwrap_or_else(|| fallback_open_symbols.iter().cloned().collect());

    for order in orders.iter_mut().filter(|order| {
        order.failure_reason.is_none()
            && order.purpose.to_lowercase().contains("stop")
            && order.status == "unfilled"
    }) {
        order.failure_reason = Some(if ledger_open_symbols.contains(&order.symbol) {
            "Stop triggered, but the position manager reported no fill.".into()
        } else {
            "Stop triggered after the position was closed; no open quantity remained to reduce.".into()
        });
    }
}

async fn get_activity(State(state): State<AutoTraderState>) -> Json<ActivityResponse> {
    let today_start = utc_day_start_ms();
    let ActivityPositionSnapshot {
        mut orders,
        auto_positions,
        open_position_symbols,
        position_exit_policies,
    } = {
        let settings = state.settings.lock().await;
        snapshot_activity_positions(&settings, today_start)
    };
    let position_url =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    apply_protection_order_failure_reasons(
        &state.client,
        &position_url,
        &mut orders,
        &open_position_symbols,
    )
    .await;
    let (realized_pnl, realized_pnl_by_quote, pnl_breakdown) =
        await_activity_realized_pnl(&state.client, &position_url, &mut orders).await;
    let unrealized_pnl = calculate_auto_unrealized_pnl(&auto_positions).await;
    let learning = {
        let settings = state.settings.lock().await;
        calculate_activity_learning_metrics(&settings, today_start)
    };
    let processing_times = state.insight_processing_ms.lock().await.clone();
    let insight_processing_latest_ms = processing_times.last().copied();
    let mut sorted_processing_times = processing_times;
    sorted_processing_times.sort_by(f64::total_cmp);
    let insight_processing_samples = sorted_processing_times.len();
    let insight_processing_average_ms = (insight_processing_samples > 0)
        .then(|| sorted_processing_times.iter().sum::<f64>() / insight_processing_samples as f64);
    let insight_processing_p95_ms = (insight_processing_samples > 0).then(|| {
        sorted_processing_times[(95 * insight_processing_samples)
            .div_ceil(100)
            .saturating_sub(1)]
    });
    let lock_wait_samples = state.cycle_lock_wait_ms.lock().await.clone();
    let cycle_lock_wait_ms = lock_wait_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    let lock_hold_samples = state
        .cycle_lock_hold_ms
        .lock()
        .ok()
        .map(|samples| samples.clone())
        .unwrap_or_default();
    let cycle_lock_hold_ms = lock_hold_samples
        .into_iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(&values).map(|summary| (worker, summary))
        })
        .collect();
    let (unrealized_pnl, unrealized_pnl_by_quote) = unrealized_pnl
        .map(|(total, by_quote)| (Some(total), Some(by_quote)))
        .unwrap_or((None, None));
    Json(ActivityResponse {
        orders,
        open_position_symbols,
        position_exit_policies,
        pnl_breakdown,
        realized_pnl,
        realized_pnl_by_quote,
        unrealized_pnl,
        unrealized_pnl_by_quote,
        exit_learning_samples: learning.exit_learning_samples,
        closed_trade_average_return_percent: learning.closed_trade_average_return_percent,
        closed_trade_wins: learning.closed_trade_wins,
        closed_trade_losses: learning.closed_trade_losses,
        closed_trade_flat: learning.closed_trade_flat,
        closed_trade_realized_usd: learning.closed_trade_realized_usd,
        closed_trade_fees_usd: learning.closed_trade_fees_usd,
        learning_samples: learning.learning_samples,
        learning_win_rate: learning.learning_win_rate,
        preferred_order_type: learning.preferred_order_type,
        parameter_profile_samples: learning.parameter_profile_samples,
        scout_open_positions: learning.scout_open_positions,
        scout_max_active_positions: SCOUT_MAX_ACTIVE_POSITIONS,
        scout_learning_samples: learning.scout_learning_samples,
        scout_learning_win_rate: learning.scout_learning_win_rate,
        scout_learning_realized_pnl: learning.scout_learning_realized_pnl,
        insight_processing_latest_ms,
        insight_processing_average_ms,
        insight_processing_p95_ms,
        insight_processing_samples,
        cycle_lock_wait_ms,
        cycle_lock_hold_ms,
    })
}
