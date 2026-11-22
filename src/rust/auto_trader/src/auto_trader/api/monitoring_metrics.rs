async fn get_monitoring_metrics(State(state): State<AutoTraderState>) -> Json<serde_json::Value> {
    let today_start = utc_day_start_ms();
    let today = utc_day();
    let settings = state.settings.lock().await;
    let snapshot = snapshot_activity_positions(&settings, today_start);
    let learning = calculate_activity_learning_metrics(&settings, today_start);
    let mut pnl_by_strategy = BTreeMap::<String, (f64, usize)>::new();
    for order in &snapshot.orders {
        if order.strategy_realized_pnl_day.as_deref() != Some(today.as_str()) {
            continue;
        }
        let strategy = order.strategy.as_deref().unwrap_or("legacy-unknown");
        let pnl = order.strategy_realized_pnl_today.unwrap_or_default();
        let summary = pnl_by_strategy.entry(strategy.to_owned()).or_default();
        summary.0 += pnl;
        if pnl.abs() > 1e-12 {
            summary.1 += 1;
        }
    }
    let realized_pnl = if settings.pnl_utc_day == today {
        settings.daily_realized_pnl
    } else {
        0.0
    };
    drop(settings);

    let carry_positions = state.dated_carry.lock().await.positions.clone();
    let unrealized_pnl = calculate_auto_unrealized_pnl(&snapshot.auto_positions, &carry_positions)
        .await
        .map(|(total, _)| total);
    let processing_times = state.insight_processing_ms.lock().await.clone();
    let insight_processing_latest_ms = processing_times.last().copied();
    let mut sorted_processing_times = processing_times;
    sorted_processing_times.sort_by(f64::total_cmp);
    let insight_processing_samples = sorted_processing_times.len();
    let cycle_lock_wait_ms = state
        .cycle_lock_wait_ms
        .lock()
        .await
        .iter()
        .filter_map(|(worker, values)| {
            summarize_lock_wait(values).map(|summary| (worker.clone(), summary))
        })
        .collect::<BTreeMap<_, _>>();
    let cycle_lock_hold_ms = state
        .cycle_lock_hold_ms
        .lock()
        .ok()
        .map(|samples| {
            samples
                .iter()
                .filter_map(|(worker, values)| {
                    summarize_lock_wait(values).map(|summary| (worker.clone(), summary))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let cycle_lock_scope_ms = state
        .cycle_lock_scope_ms
        .lock()
        .ok()
        .map(|samples| {
            samples
                .iter()
                .filter_map(|(scope, values)| {
                    summarize_lock_wait(values).map(|summary| (scope.clone(), summary))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let settings_persistence_ms = state
        .settings_persist_timings_ms
        .lock()
        .ok()
        .map(|samples| {
            samples
                .iter()
                .filter_map(|(phase, values)| {
                    summarize_lock_wait(values).map(|summary| (phase.clone(), summary))
                })
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    let pnl_breakdown = pnl_by_strategy
        .into_iter()
        .map(|(strategy, (realized_pnl, outcomes))| {
            serde_json::json!({
                "strategy": strategy,
                "symbol": "UNKNOWN-USDT",
                "realized_pnl": realized_pnl,
                "outcomes": outcomes,
            })
        })
        .collect::<Vec<_>>();

    Json(serde_json::json!({
        "orders": snapshot.orders,
        "pnl_breakdown": pnl_breakdown,
        "realized_pnl": realized_pnl,
        "unrealized_pnl": unrealized_pnl,
        "closed_trade_wins": learning.closed_trade_wins,
        "closed_trade_losses": learning.closed_trade_losses,
        "closed_trade_flat": learning.closed_trade_flat,
        "learning_samples": learning.learning_samples,
        "learning_win_rate": learning.learning_win_rate,
        "insight_processing_latest_ms": insight_processing_latest_ms,
        "insight_processing_samples": insight_processing_samples,
        "cycle_lock_wait_ms": cycle_lock_wait_ms,
        "cycle_lock_hold_ms": cycle_lock_hold_ms,
        "cycle_lock_scope_ms": cycle_lock_scope_ms,
        "settings_persistence_ms": settings_persistence_ms,
    }))
}
