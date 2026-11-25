async fn apply_protection_order_failure_reasons(
    orders: &mut [TraderOrder],
    fallback_open_symbols: &[String],
) {
    if !orders.iter().any(|order| {
        order.failure_reason.is_none()
            && order.purpose.to_lowercase().contains("stop")
            && order.status == "unfilled"
    }) {
        return;
    }
    let ledger_open_symbols = crate::service_rpc::position_get("/positions?source=auto")
        .await
        .ok()
        .and_then(|body| {
            body["positions"].as_array().map(|positions| {
                positions
                    .iter()
                    .filter(|position| {
                        position["quantity"]
                            .as_f64()
                            .is_some_and(|quantity| quantity.abs() > 1e-12)
                    })
                    .filter_map(|position| position["symbol"].as_str().map(str::to_owned))
                    .collect::<BTreeSet<_>>()
            })
        })
        .unwrap_or_else(|| fallback_open_symbols.iter().cloned().collect());

    for order in orders.iter_mut().filter(|order| {
        order.failure_reason.is_none()
            && order.purpose.to_lowercase().contains("stop")
            && order.status == "unfilled"
    }) {
        order.failure_reason = Some(if ledger_open_symbols.contains(&order.symbol) {
            "Stop triggered, but the position manager reported no fill.".into()
        } else {
            "Stop triggered after the position was closed; no open quantity remained to reduce."
                .into()
        });
    }
}

const ACTIVITY_SNAPSHOT_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

fn start_activity_snapshot_worker(state: &AutoTraderState) {
    let worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(ACTIVITY_SNAPSHOT_REFRESH_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            let started = Instant::now();
            let activity = match build_activity_response(&worker_state).await {
                Ok(activity) => activity,
                Err(error) => {
                    crate::service_log!(
                        "WARN",
                        "AutoTrader background activity snapshot refresh failed: {error}"
                    );
                    continue;
                }
            };
            let serialized = match serde_json::to_vec(&activity) {
                Ok(serialized) => serialized,
                Err(error) => {
                    crate::service_log!("ERROR", "AutoTrader activity snapshot serialization failed: {error}");
                    continue;
                }
            };
            *worker_state.activity_snapshot.lock().await =
                Some(axum::body::Bytes::from(serialized));
            let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
            if elapsed_ms >= 500.0 {
                crate::service_log!(
                    "WARN",
                    "AutoTrader background activity snapshot refresh completed in {:.1} ms",
                    elapsed_ms,
                );
            }
        }
    });
}

async fn get_activity(State(state): State<AutoTraderState>) -> axum::response::Response {
    let cached = state.activity_snapshot.lock().await.clone();
    let mut response = match cached {
        Some(cached) => axum::response::Response::new(axum::body::Body::from(cached)),
        None => {
            let mut response = axum::response::Response::new(axum::body::Body::from(
                "AutoTrader activity snapshot is warming",
            ));
            *response.status_mut() = StatusCode::SERVICE_UNAVAILABLE;
            return response;
        }
    };
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response
}

async fn build_activity_response(
    state: &AutoTraderState,
) -> Result<ActivityResponse, String> {
    let today_start = utc_day_start_ms();
    let ActivityPositionSnapshot {
        mut orders,
        lifecycle_links,
        auto_positions,
        mut strategy_positions,
        open_position_symbols,
        position_exit_policies,
    } = {
        let settings = state.settings.lock().await;
        snapshot_activity_positions(&settings, today_start)
    };
    let carry_positions = state.dated_carry.lock().await.positions.clone();
    strategy_positions.extend(
        carry_positions
            .clone()
            .into_iter()
            .filter(|position| position.status.eq_ignore_ascii_case("open"))
            .map(|position| OpenStrategyPosition {
                symbol: position.future_symbol.clone(),
                strategy: "dated-futures-carry".into(),
                decision_id: Some(format!("dated-carry:{}", position.future_symbol)),
                quantity: position.quantity,
                average_entry_price: position.spot_entry_price,
                entry_fees: -position.entry_fees.abs(),
                entry_timestamp_ms: position.entry_ms,
                entry_order_type: "dated-carry".into(),
                entry_order_types: vec!["dated-carry".into()],
                stop_loss_percent: None,
                take_profit_percent: None,
                horizon_take_profit_percent: None,
                stop_loss_active: false,
                take_profit_active: false,
                carry_details: Some(position),
            }),
    );
    apply_protection_order_failure_reasons(&mut orders, &open_position_symbols).await;
    // Read the fill ledger and realized P&L once per background snapshot. The
    // browser route serves the cached JSON and never opens ledger queries.
    let ledger_trades = crate::service_rpc::position_get("/positions/trades?source=auto")
        .await
        .map_err(|error| format!("fetching Auto position fills: {error}"))?;
    if !ledger_trades["trades"].is_array() {
        return Err("Position Manager fills response has no trades array".into());
    }
    let ledger_pnl = fetch_ledger_realized_pnl()
        .await
        .map_err(|(_, error)| format!("fetching Auto realized P&L: {error}"))?;
    let realized_by_parent = crate::service_rpc::position_get(
        "/positions/realized-pnl?source=auto&since_ms=0",
    )
    .await
    .ok()
    .and_then(|value| {
        serde_json::from_value::<BTreeMap<String, f64>>(
            value["realized_pnl_all_time_by_parent_order"].clone(),
        )
        .ok()
    });
    let (mut realized_pnl, mut realized_pnl_by_quote, mut pnl_breakdown) =
        activity_realized_pnl(&mut orders, Some(&ledger_pnl));
    // Closed-position ledger rows are supplied by the backend activity contract
    // so the client does not have to infer strategy ownership from fill history.
    let realized_by_parent = realized_by_parent.as_ref();
    let closed_ledger_rows = build_closed_ledger_rows(
        &orders,
        &lifecycle_links,
        &strategy_positions,
        &carry_positions,
        &ledger_trades,
        realized_by_parent,
    );
    for position in carry_positions.iter().filter(|position| {
        position.status.starts_with("closed")
            && position.closed_at_ms.is_some_and(|closed_at| closed_at >= today_start)
    }) {
        let Some(net) = position.net_pnl_usdt.filter(|value| value.is_finite()) else {
            continue;
        };
        let symbol = position.spot_symbol.replace("USDT", "-USDT");
        *realized_pnl_by_quote
            .get_or_insert_with(BTreeMap::new)
            .entry("USDT".into())
            .or_default() += net;
        pnl_breakdown.push(PnlBreakdown {
            symbol,
            direction: "LONG".into(),
            purpose: "dated-carry-close".into(),
            strategy: "dated-futures-carry".into(),
            decision_id: Some(format!("dated-carry:{}", position.future_symbol)),
            realized_pnl: net,
            outcomes: 1,
        });
        realized_pnl = None;
    }
    let unrealized_pnl = calculate_auto_unrealized_pnl(&auto_positions, &carry_positions).await;
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
    let cycle_lock_scope_ms = state
        .cycle_lock_scope_ms
        .lock()
        .ok()
        .map(|samples| samples.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(scope, values)| summarize_lock_wait(&values).map(|summary| (scope, summary)))
        .collect();
    let settings_persistence_ms = state
        .settings_persist_timings_ms
        .lock()
        .ok()
        .map(|samples| samples.clone())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(phase, values)| summarize_lock_wait(&values).map(|summary| (phase, summary)))
        .collect();
    let (unrealized_pnl, unrealized_pnl_by_quote) = unrealized_pnl
        .map(|(total, by_quote)| (Some(total), Some(by_quote)))
        .unwrap_or((None, None));
    Ok(ActivityResponse {
        orders,
        open_position_symbols,
        open_positions: auto_positions
            .iter()
            .map(|(symbol, position)| OpenPositionAttribution {
                symbol: symbol.clone(),
                strategy: position.strategy.clone(),
                decision_id: position.entry_decision_id.clone(),
            })
            .collect(),
        strategy_positions,
        position_exit_policies,
        pnl_breakdown,
        closed_ledger_rows,
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
        closed_trade_needs_audit_count: learning.closed_trade_needs_audit_count,
        closed_trade_needs_audit_recent: learning.closed_trade_needs_audit_recent,
        learning_samples: learning.learning_samples,
        learning_win_rate: learning.learning_win_rate,
        preferred_order_type: learning.preferred_order_type,
        parameter_profile_samples: learning.parameter_profile_samples,
        insight_processing_latest_ms,
        insight_processing_average_ms,
        insight_processing_p95_ms,
        insight_processing_samples,
        cycle_lock_wait_ms,
        cycle_lock_hold_ms,
        cycle_lock_scope_ms,
        settings_persistence_ms,
    })
}
