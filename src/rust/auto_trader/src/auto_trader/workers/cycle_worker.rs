async fn acquire_cycle_lock(state: &AutoTraderState, worker: &str) -> CycleLockGuard {
    let wait_started = Instant::now();
    let guard = state.cycle_lock.clone().lock_owned().await;
    record_cycle_lock_wait(state, worker, wait_started.elapsed()).await;
    CycleLockGuard {
        _guard: guard,
        hold_samples: state.cycle_lock_hold_ms.clone(),
        worker: worker.to_owned(),
        acquired_at: Instant::now(),
    }
}

async fn record_cycle_lock_wait(state: &AutoTraderState, worker: &str, elapsed: Duration) {
    let mut samples = state.cycle_lock_wait_ms.lock().await;
    let values = samples.entry(worker.to_owned()).or_default();
    values.push(elapsed.as_secs_f64() * 1000.0);
    if values.len() > 500 {
        values.remove(0);
    }
}

fn summarize_lock_wait(values: &[f64]) -> Option<CycleLockWaitSummary> {
    if values.is_empty() {
        return None;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    Some(CycleLockWaitSummary {
        latest_ms: *values.last()?,
        average_ms: sorted.iter().sum::<f64>() / sorted.len() as f64,
        p95_ms: sorted[(95 * sorted.len()).div_ceil(100).saturating_sub(1)],
        samples: sorted.len(),
    })
}

async fn run_cycle(
    state: &AutoTraderState,
    dashboard: Dashboard,
) -> Result<CycleResult, (StatusCode, String)> {
    let processing_started = Instant::now();
    if let Err(error) = refresh_usd_rates(state).await {
        crate::service_log!(
            "WARN",
            "USD allocation rates refresh failed; using cached rates: {}",
            error.1
        );
    }
    let result = run_cycle_inner(state, dashboard).await;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    record_cycle_gap_summary(state, &result);
    let mut message = cycle_decision_message(&result);
    let mut settings = state.settings.lock().await;
    if !settings.last_risk_note.is_empty() {
        message.push_str(" · ");
        message.push_str(&settings.last_risk_note);
    }
    settings.last_decision = message;
    settings.last_decision_at = now;
    // The last decision is informational. Keep it in memory immediately and
    // let the next durable settings write include it, rather than writing the
    // full state file on every signal cycle.
    drop(settings);
    let mut processing_times = state.insight_processing_ms.lock().await;
    processing_times.push(processing_started.elapsed().as_secs_f64() * 1000.0);
    if processing_times.len() > 500 {
        processing_times.remove(0);
    }
    result
}

fn cycle_decision_message(result: &Result<CycleResult, (StatusCode, String)>) -> String {
    match result {
        Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted" => {
            format!(
                "{}: {} {} {}",
                cycle.status,
                cycle.side.as_deref().unwrap_or(""),
                cycle.symbol.as_deref().unwrap_or(""),
                cycle.order_type.as_deref().unwrap_or("")
            )
        }
        Ok(cycle) => cycle
            .reason
            .clone()
            .unwrap_or_else(|| cycle.status.to_owned()),
        Err((_, error)) => format!("Cycle error: {error}"),
    }
}

fn record_cycle_gap_summary(
    state: &AutoTraderState,
    result: &Result<CycleResult, (StatusCode, String)>,
) {
    let traded = matches!(&result, Ok(cycle) if cycle.status == "order_submitted" || cycle.status == "hedge_submitted");
    if let Ok(mut summary) = state.gap_summary.lock() {
        let day = utc_day();
        if summary.utc_day != day {
            *summary = GapSummary {
                utc_day: day.clone(),
                ..GapSummary::default()
            };
        }
        if traded {
            *summary = GapSummary {
                utc_day: day,
                ..GapSummary::default()
            };
            if let Ok(cycle) = &result {
                if cycle.status == "order_submitted"
                    && cycle.order_type.as_deref() != Some("Ioc exit")
                {
                    if let (Some(symbol), Some(side)) = (&cycle.symbol, &cycle.side) {
                        let direction = match side.as_str() {
                            "buy" => "BULLISH",
                            "sell" => "BEARISH",
                            other => other,
                        };
                        summary.symbol_gate_hits.insert(
                            (symbol.clone(), direction.to_owned(), "Readiness".to_owned()),
                            SymbolGateCount {
                                count: 0,
                                last_detail:
                                    "All entry, risk, and execution gates passed; order submitted."
                                        .to_owned(),
                                distance_to_pass: Some(0.0),
                            },
                        );
                    }
                }
            }
        } else {
            summary
                .symbol_gate_hits
                .retain(|(_, _, gate), _| gate != "Readiness");
            summary.decisions = summary.decisions.saturating_add(1);
            let raw_reason = match &result {
                Ok(cycle) => cycle.reason.as_deref().unwrap_or(cycle.status),
                Err((_, error)) => error.as_str(),
            };
            let reason = gap_reason_category(raw_reason);
            *summary.reasons.entry(reason).or_default() += 1;
            for gate in gate_hits_for_reason(raw_reason) {
                *summary.gate_hits.entry(gate).or_default() += 1;
            }
            for (symbol, direction, gate, detail, distance_to_pass) in
                symbol_gate_hits_for_reason(raw_reason)
            {
                let entry = summary
                    .symbol_gate_hits
                    .entry((symbol, direction, gate))
                    .or_default();
                entry.count = entry.count.saturating_add(1);
                entry.last_detail = detail;
                entry.distance_to_pass = distance_to_pass;
            }
            if let Ok(cycle) = &result {
                if let (Some(symbol), Some(direction)) = (&cycle.symbol, &cycle.side) {
                    for gate in gate_hits_for_reason(raw_reason) {
                        let distance_to_pass = gate_distance_to_pass(&gate, raw_reason);
                        let entry = summary
                            .symbol_gate_hits
                            .entry((symbol.clone(), direction.clone(), gate))
                            .or_default();
                        entry.count = entry.count.saturating_add(1);
                        entry.last_detail = raw_reason.to_owned();
                        entry.distance_to_pass = distance_to_pass;
                    }
                }
            }
            summary.latest_detail = raw_reason.to_owned();
        }
    }

}

