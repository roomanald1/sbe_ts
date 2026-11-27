fn diagnostic_strategy_blocker(diagnostic: &StrategyDiagnostic) -> Option<StrategyBlocker> {
    let (symbol, reason) = if diagnostic.btc_gate_required
        && diagnostic.btc_gate_passed == Some(false)
    {
        let window = diagnostic
            .btc_window_days
            .map(|days| format!("SMA{days}"))
            .unwrap_or_else(|| "trend average".to_owned());
        (
            "BTC-USDT".to_owned(),
            match (diagnostic.btc_close, diagnostic.btc_sma) {
                (Some(close), Some(sma)) => format!(
                    "BTC trend gate not passed: close {close:.2} vs {window} {sma:.2}"
                ),
                _ => format!("BTC trend gate not passed ({window})"),
            },
        )
    } else {
        let failed = diagnostic.symbols.iter().find_map(|item| {
            if !item.evaluated || item.signal_active {
                return None;
            }
            if diagnostic.strategy == "relative-strength-90-sma200"
                && item.secondary_passed == Some(false)
            {
                let rule = item.secondary_rule.as_deref().unwrap_or("Own SMA200 gate");
                let threshold = item
                    .secondary_level
                    .map(|value| format!("{value:.8}"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                let distance = item
                    .secondary_distance_percent
                    .map(|value| format!("{value:.2}%"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                return Some((
                    item.symbol.clone(),
                    format!(
                        "{rule} not met: close {:.8}, threshold {threshold}; distance {distance}",
                        item.close
                    ),
                ));
            }
            if !item.eligible {
                return Some((
                    item.symbol.clone(),
                    "not eligible for this strategy".to_owned(),
                ));
            }
            if !item.primary_passed {
                let unit = item.primary_metric_unit.as_deref().unwrap_or("");
                let observed = item
                    .primary_metric_value
                    .map(|value| format!("{value:.2} {unit}"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                let threshold = item
                    .primary_level
                    .map(|value| format!("{value:.2} {unit}"))
                    .unwrap_or_else(|| "unavailable".to_owned());
                let distance = item
                    .primary_distance_percent
                    .map(|value| {
                        format!(
                            "{value:.2} {}",
                            item.primary_distance_unit.as_deref().unwrap_or("%")
                        )
                    })
                    .unwrap_or_else(|| "unavailable".to_owned());
                return Some((
                    item.symbol.clone(),
                    format!(
                        "{} not met: observed {}, threshold {}; distance {}",
                        item.primary_rule,
                        observed,
                        threshold,
                        distance,
                    ),
                ));
            }
            if item.secondary_passed == Some(false) {
                return Some((
                    item.symbol.clone(),
                    format!(
                        "{} not met (distance {:.2}%)",
                        item.secondary_rule.as_deref().unwrap_or("secondary rule"),
                        item.secondary_distance_percent.unwrap_or_default()
                    ),
                ));
            }
            None
        })?;
        failed
    };
    Some(StrategyBlocker {
        strategy: diagnostic.strategy.clone(),
        symbol,
        reason,
        timestamp_ms: diagnostic.as_of_ms,
    })
}

#[cfg(test)]
mod strategy_blocker_tests {
    use super::*;
    use strategy::daily::StrategySymbolDiagnostic;

    #[test]
    fn reports_the_failed_strategy_rule_and_observed_metric() {
        let diagnostic = StrategyDiagnostic {
            strategy: "trend-50-200".to_owned(),
            as_of_ms: 123,
            btc_close: None,
            btc_sma: None,
            btc_distance_percent: None,
            btc_gate_required: false,
            btc_gate_passed: None,
            btc_window_days: None,
            btc_source_close_time_ms: None,
            btc_source_timeframe: None,
            symbols: vec![StrategySymbolDiagnostic {
                symbol: "ETH-USDT".to_owned(),
                close: 90.0,
                eligible: true,
                primary_rule: "price above SMA50".to_owned(),
                primary_level: Some(100.0),
                primary_distance_percent: Some(-10.0),
                primary_distance_unit: Some("%".to_owned()),
                primary_metric_value: Some(90.0),
                primary_metric_unit: Some("price".to_owned()),
                primary_passed: false,
                secondary_rule: None,
                secondary_level: None,
                secondary_distance_percent: None,
                secondary_passed: None,
                evaluated: true,
                signal_active: false,
                leader_gap_percent: None,
                relative_strength_gap_pp: None,
                rsi_exit_level: None,
                maximum_hold_days: None,
                trend_return_windows: None,
            }],
        };

        let blocker = diagnostic_strategy_blocker(&diagnostic).unwrap();
        assert_eq!(blocker.strategy, "trend-50-200");
        assert_eq!(blocker.symbol, "ETH-USDT");
        assert!(blocker.reason.contains("observed 90.00 price"));
        assert!(blocker.reason.contains("threshold 100.00 price"));
        assert_eq!(blocker.timestamp_ms, 123);
    }
}

fn capital_allocation_status(
    state: &AutoTraderState,
    settings: &Settings,
    now_ms: u64,
) -> CapitalAllocationStatus {
    const SNAPSHOT_MAX_AGE_MS: u64 = 60_000;
    let snapshot = state
        .capital_allocation_snapshot
        .try_lock()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_default();
    let candidates = if now_ms.saturating_sub(snapshot.as_of_ms) <= SNAPSHOT_MAX_AGE_MS
        && (snapshot.pool_limit - settings.daily_notional_limit).abs() <= 1e-9
    {
        snapshot.candidates
    } else {
        BTreeMap::new()
    };
    let mut rows = Vec::with_capacity(candidates.len() + settings.strategies.len());
    for ((strategy, symbol, horizon), candidate) in &candidates {
        if !settings
            .strategies
            .iter()
            .any(|configured| configured.as_str() == strategy)
        {
            continue;
        }
        let used = candidate_used_notional(settings, strategy, symbol);
        let reserved = candidate_reserved_notional(settings, strategy, symbol);
        rows.push(CapitalAllocationRow {
            strategy: strategy.clone(),
            symbol: symbol.clone(),
            horizon: horizon.clone(),
            effective_allocation: candidate.effective_allocation,
            reserved,
            used,
            utilization: (candidate.effective_allocation > 0.0)
                .then_some((used + reserved) / candidate.effective_allocation),
            evidence_eligible: candidate.evidence_eligible,
            state: candidate.state.clone(),
            reason: candidate.reason.clone(),
        });
    }
    for strategy in &settings.strategies {
        let name = strategy.as_str();
        if candidates.keys().any(|(candidate, _, _)| candidate == name) {
            continue;
        }
        if strategy.is_dated_carry() && settings.dated_carry_reserved_usd > 0.0 {
            let committed = settings.dated_carry_reserved_usd;
            rows.push(CapitalAllocationRow {
                strategy: name.to_owned(),
                symbol: "*".to_owned(),
                horizon: "daily".to_owned(),
                effective_allocation: committed,
                reserved: 0.0,
                used: committed,
                utilization: Some(1.0),
                evidence_eligible: None,
                state: "committed".to_owned(),
                reason: "Open dated-carry spot and future exposure is committed from the shared pool; new sizing remains bounded by the separate carry-book spot cap".to_owned(),
            });
            continue;
        }
        rows.push(CapitalAllocationRow {
            strategy: name.to_owned(),
            symbol: "*".to_owned(),
            horizon: if strategy.is_live_insight() {
                "intraday"
            } else {
                "daily"
            }
            .to_owned(),
            effective_allocation: 0.0,
            reserved: 0.0,
            used: 0.0,
            utilization: None,
            evidence_eligible: None,
            state: "unavailable".to_owned(),
            reason: "No fresh eligible directional candidate; capital remains in the shared pool"
                .to_owned(),
        });
    }
    if settings.dated_carry_reserved_usd > 0.0
        && !rows
            .iter()
            .any(|row| row.strategy == Strategy::DatedCarry.as_str())
    {
        let committed = settings.dated_carry_reserved_usd;
        rows.push(CapitalAllocationRow {
            strategy: Strategy::DatedCarry.as_str().to_owned(),
            symbol: "*".to_owned(),
            horizon: "daily".to_owned(),
            effective_allocation: committed,
            reserved: 0.0,
            used: committed,
            utilization: Some(1.0),
            evidence_eligible: None,
            state: "committed".to_owned(),
            reason: "Open dated-carry spot and future exposure is committed from the shared pool; new sizing remains bounded by the separate carry-book spot cap".to_owned(),
        });
    }
    let allocated = allocated_notional(settings);
    let reserved = settings
        .orders
        .iter()
        .filter(|order| {
            order.purpose == "signal"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > order.filled_quantity + 1e-12
        })
        .map(|order| {
            let remaining_fraction = ((order.quantity - order.filled_quantity).max(0.0)
                / order.quantity)
                .clamp(0.0, 1.0);
            let reference = if order.reserved_notional > 0.0 {
                order.reserved_notional
            } else {
                order.average_price.unwrap_or(0.0)
                    * order.quantity
                    * quote_usd_rate(settings, &order.symbol)
            };
            reference
                * remaining_fraction
                * scheduled_stop_headroom_multiplier(&order.order_type, order.strategy.as_deref())
        })
        .sum::<f64>();
    CapitalAllocationStatus {
        version: "v1".to_owned(),
        model: "shared_pool".to_owned(),
        pool_limit: settings.daily_notional_limit,
        available: (settings.daily_notional_limit - allocated).max(0.0),
        reserved,
        used: (allocated - reserved).max(0.0),
        rows,
    }
}

#[cfg(test)]
mod capital_allocation_contract_tests {
    use super::*;

    #[test]
    fn status_contract_uses_camel_case_and_reports_committed_carry_exposure() {
        let status = CapitalAllocationStatus {
            version: "v1".to_owned(),
            model: "shared_pool".to_owned(),
            pool_limit: 1_000.0,
            available: 300.0,
            reserved: 100.0,
            used: 600.0,
            rows: vec![CapitalAllocationRow {
                strategy: "dated-futures-carry".to_owned(),
                symbol: "*".to_owned(),
                horizon: "daily".to_owned(),
                effective_allocation: 600.0,
                reserved: 0.0,
                used: 600.0,
                utilization: Some(1.0),
                evidence_eligible: None,
                state: "committed".to_owned(),
                reason: "Open carry exposure is committed from the shared pool".to_owned(),
            }],
        };
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["version"], "v1");
        assert_eq!(value["model"], "shared_pool");
        assert_eq!(value["poolLimit"], 1_000.0);
        assert_eq!(value["available"], 300.0);
        assert_eq!(value["rows"][0]["effectiveAllocation"], 600.0);
        assert_eq!(value["rows"][0]["evidenceEligible"], serde_json::Value::Null);
        assert_eq!(value["rows"][0]["state"], "committed");
    }
}

fn status(state: &AutoTraderState, settings: &Settings) -> StatusResponse {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or_default();
    if let Ok(mut summary) = state.gap_summary.lock() {
        let day = utc_day();
        if summary.utc_day != day {
            let entry_diagnostic_history = std::mem::take(&mut summary.entry_diagnostic_history);
            *summary = GapSummary {
                utc_day: day,
                entry_diagnostic_history,
                ..GapSummary::default()
            };
        }
    }
    let mut latest_strategy_blockers = state
        .latest_strategy_blockers
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    let diagnostics = {
        state
            .strategy_diagnostic
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone()
    };
    for diagnostic in diagnostics.iter() {
        let existing = latest_strategy_blockers.get(&diagnostic.strategy);
        if existing.is_some_and(|blocker| blocker.timestamp_ms > diagnostic.as_of_ms) {
            continue;
        }
        if let Some(blocker) = diagnostic_strategy_blocker(diagnostic) {
            latest_strategy_blockers.insert(diagnostic.strategy.clone(), blocker);
        } else {
            latest_strategy_blockers.remove(&diagnostic.strategy);
            }
    }
    if state.enabled.load(Ordering::Relaxed) {
        for strategy in &settings.strategies {
            let strategy_name = strategy.as_str();
            latest_strategy_blockers
                .entry(strategy_name.to_owned())
                .or_insert_with(|| StrategyBlocker {
                    strategy: strategy_name.to_owned(),
                    symbol: "MARKET".to_owned(),
                    reason: "No entry candidate was reported by the latest strategy evaluation"
                        .to_owned(),
                    timestamp_ms: now_ms,
                });
        }
    }
    StatusResponse {
        enabled: state.enabled.load(Ordering::Relaxed),
        audit_healthy: state.audit_health.load(Ordering::Relaxed) == 3
            && !state.audit_failure_latched.load(Ordering::Relaxed),
        audit_failure_latched: state.audit_failure_latched.load(Ordering::Relaxed),
        paper_learning_exploration_until_ms: settings.paper_learning_exploration_until_ms,
        decision_audit_queue_depth: state.audit_tx.depth(),
        execution_audit_queue_depth: state.execution_audit_tx.depth(),
        decision_audit_queue_estimated_bytes: state.audit_tx.estimated_bytes(),
        execution_audit_queue_estimated_bytes: state.execution_audit_tx.estimated_bytes(),
        audit_coalesced_decisions: state.audit_tx.coalesced.load(Ordering::Relaxed),
        audit_dropped_events: state.audit_tx.dropped.load(Ordering::Relaxed)
            + state.execution_audit_tx.dropped.load(Ordering::Relaxed),
        strategy: settings.strategies.first().copied().unwrap_or_default().as_str().to_owned(),
        strategies: settings.strategies.iter().map(|strategy| strategy.as_str().to_owned()).collect(),
        signal_strategies_enabled: settings.signal_strategies_enabled,
        max_order_notional: settings.max_order_notional,
        daily_notional_limit: settings.daily_notional_limit,
        daily_realized_loss_limit: settings.daily_realized_loss_limit,
        exploration_percent: settings.exploration_percent,
        risk_appetite: settings.risk_appetite,
        max_open_positions: settings.max_open_positions,
        max_intraday_open_positions: settings.max_intraday_open_positions,
        max_daily_open_positions: settings.max_daily_open_positions,
        capital_allocation: capital_allocation_status(state, settings, now_ms),
        max_entries_per_auto_position: settings.max_entries_per_auto_position,
        stop_loss_percent: settings.stop_loss_percent,
        take_profit_percent: settings.take_profit_percent,
        vwap_duration_minutes: settings.vwap_duration_minutes,
        vwap_participation_percent: settings.vwap_participation_percent,
        limit_offset_bps: settings.limit_offset_bps,
        trigger_offset_bps: settings.trigger_offset_bps,
        trailing_distance_percent: settings.trailing_distance_percent,
        closed_trade_window: settings.closed_trade_window,
        minimum_scout_parameter_samples: settings.minimum_live_scout_parameter_samples,
        used_notional_today: settings.used_notional_today,
        available_notional: (settings.daily_notional_limit - settings.used_notional_today).max(0.0),
        last_decision: settings.last_decision.clone(),
        last_decision_at: settings.last_decision_at,
        decisions_since_trade: state
            .gap_summary
            .lock()
            .map(|summary| summary.decisions)
            .unwrap_or_default(),
        gap_reasons: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .reasons
                    .iter()
                    .map(|(reason, count)| GapReason {
                        reason: reason.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .gate_hits
                    .iter()
                    .map(|(gate, count)| GateHit {
                        gate: gate.clone(),
                        count: *count,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        stale_signal_candidates_total: state
            .stale_signal_candidates_total
            .load(Ordering::Relaxed),
        stale_signal_cycles_total: state
            .stale_signal_cycles_total
            .load(Ordering::Relaxed),
        symbol_gate_hits: state
            .gap_summary
            .lock()
            .map(|summary| {
                summary
                    .symbol_gate_hits
                    .iter()
                    .map(|((symbol, direction, gate), count)| SymbolGateHit {
                        symbol: symbol.clone(),
                        direction: direction.clone(),
                        gate: gate.clone(),
                        count: count.count,
                        last_detail: count.last_detail.clone(),
                        last_seen_ms: count.last_seen_ms,
                        distance_to_pass: count.distance_to_pass,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        latest_signal_diagnostic: state
            .gap_summary
            .lock()
            .map(|summary| summary.latest_detail.clone())
            .unwrap_or_default(),
        latest_entry_diagnostic: state
            .gap_summary
            .lock()
            .ok()
            .and_then(|summary| summary.latest_entry_diagnostic.clone()),
        entry_diagnostic_history: state
            .gap_summary
            .lock()
            .map(|summary| summary.entry_diagnostic_history.clone())
            .unwrap_or_default(),
        latest_symbol_entry_results: {
            let reports = state
                .latest_worker_decisions
                .try_lock()
                .ok()
                .map(|decisions| {
                    decisions
                        .values()
                        .filter(|report| settings.strategies.contains(&report.strategy))
                        .map(|report| {
                            (
                                (
                                    report.strategy.as_str().to_owned(),
                                    report.symbol.clone(),
                                    report.profile_id.clone(),
                                ),
                                SymbolEntryResult {
                                    strategy: report.strategy.as_str().to_owned(),
                                    symbol: report.symbol.clone(),
                                    profile_id: Some(report.profile_id.clone()),
                                    event_id: Some(report.event_id.clone()),
                                    decision: Some(report.decision),
                                    reason: report.reason.clone().unwrap_or_default(),
                                    timestamp_ms: Some(report.event_time_ms),
                                },
                            )
                        })
                        .collect::<BTreeMap<_, _>>()
                })
                .unwrap_or_default();
            let mut results = reports;
            let diagnostics = state
                .latest_worker_diagnostics
                .try_lock()
                .ok()
                .map(|diagnostics| diagnostics.values().cloned().collect::<Vec<_>>())
                .unwrap_or_default();
            for diagnostic in diagnostics {
                let key = (
                    diagnostic.strategy.as_str().to_owned(),
                    diagnostic.symbol.clone(),
                    diagnostic.profile_id.clone().unwrap_or_default(),
                );
                if now_ms.saturating_sub(diagnostic.event_time_ms) > 10 * 60_000 {
                    continue;
                }
                let newer_decision_exists = results
                    .get(&key)
                    .and_then(|result| result.timestamp_ms)
                    .is_some_and(|timestamp| timestamp >= diagnostic.event_time_ms);
                if !newer_decision_exists {
                    results.insert(
                        key,
                        SymbolEntryResult {
                            strategy: diagnostic.strategy.as_str().to_owned(),
                            symbol: diagnostic.symbol,
                            profile_id: diagnostic.profile_id,
                            event_id: Some(diagnostic.event_id),
                            decision: None,
                            reason: diagnostic.reason,
                            timestamp_ms: Some(diagnostic.event_time_ms),
                        },
                    );
                }
            }
            for kind in Strategy::live_strategies() {
                let enabled = settings.strategies.contains(&kind);
                for symbol in strategy::autotrader_allowed_symbols() {
                    let configured_profiles = settings
                        .active_live_strategy_profiles
                        .get(kind.as_str())
                        .map(|profiles| {
                            profiles
                                .iter()
                                .filter(|profile| profile.profile_symbol == *symbol)
                                .map(|profile| profile.profile_id.clone())
                                .collect::<Vec<_>>()
                        })
                        .filter(|profiles| !profiles.is_empty())
                        .unwrap_or_else(|| {
                            vec![live_profile_for_symbol(&settings, symbol, kind).profile_id]
                        });
                    for profile_id in configured_profiles {
                        let key = (
                            kind.as_str().to_owned(),
                            (*symbol).to_owned(),
                            profile_id.clone(),
                        );
                        results.entry(key).or_insert_with(|| SymbolEntryResult {
                            strategy: kind.as_str().to_owned(),
                            symbol: (*symbol).to_owned(),
                            profile_id: Some(profile_id),
                            event_id: None,
                            decision: None,
                            reason: if enabled {
                                "No worker decision or skip report received for this profile".to_owned()
                            } else {
                                "Strategy disabled".to_owned()
                            },
                            timestamp_ms: None,
                        });
                    }
                }
            }
            results.into_values().collect()
        },
        strategy_diagnostics: state
            .strategy_diagnostic
            .lock()
            .ok()
            .map(|diagnostics| diagnostics.clone())
            .unwrap_or_default(),
        latest_strategy_blockers: latest_strategy_blockers.into_values().collect(),
    }
}
