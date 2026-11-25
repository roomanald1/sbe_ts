//! Paired, fee-aware candidate/baseline observations for Scout replay.

use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
pub(crate) fn build_paired_return_stream(outcomes: &Value, searches: &Value) -> Value {
    let unavailable_intervals = json!({
        "schema_version": 1,
        "coverage": "not_requested",
        "truncated": false,
        "intervals": [],
    });
    build_paired_return_stream_with_portfolio_intervals(outcomes, searches, &unavailable_intervals)
}

pub(crate) fn build_paired_return_stream_with_portfolio_intervals(
    outcomes: &Value,
    searches: &Value,
    portfolio_intervals_response: &Value,
) -> Value {
    let outcome_rows = outcomes["outcomes"].as_array().cloned().unwrap_or_default();
    let baseline_rows = outcome_rows
        .iter()
        .filter(|row| row["scout_comparison_role"].as_str() == Some("baseline"))
        .collect::<Vec<_>>();
    let variant_rows = outcome_rows
        .iter()
        .filter(|row| row["scout_comparison_role"].as_str() == Some("variant"))
        .collect::<Vec<_>>();
    let mut paired_variants = BTreeSet::new();
    let mut ambiguous_variant_matches = 0usize;
    let mut pairs = Vec::new();
    for variant in &variant_rows {
        let Some(comparison_id) = variant["scout_comparison_id"].as_str() else {
            continue;
        };
        let matches = baseline_rows
            .iter()
            .filter(|baseline| {
                let baseline_has_comparison = baseline["scout_comparison_id"].as_str()
                    == Some(comparison_id)
                    || baseline["scout_comparison_ids"]
                        .as_array()
                        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(comparison_id)));
                let has_exact_profile_identity = baseline["profile_id"].as_str().is_some()
                    && variant["profile_id"].as_str().is_some();
                let same_opportunity = has_exact_profile_identity
                    && baseline["symbol"] == variant["symbol"]
                    && baseline["strategy"] == variant["strategy"]
                    && baseline["market_regime"] == variant["market_regime"]
                    && baseline["learning_context"] == variant["learning_context"]
                    && baseline["exit_scenario"] == variant["exit_scenario"]
                    && baseline["signal_timestamp_ms"]
                        .as_u64()
                        .zip(variant["signal_timestamp_ms"].as_u64())
                        .is_some_and(|(left, right)| left.abs_diff(right) <= 10_000);
                baseline_has_comparison && same_opportunity
            })
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            if matches.len() > 1 {
                ambiguous_variant_matches += 1;
            }
            continue;
        }
        let baseline = matches[0];
        let baseline_id = baseline["outcome_id"].as_str().unwrap_or_default();
        let variant_id = variant["outcome_id"].as_str().unwrap_or_default();
        let baseline_return = baseline["net_return_percent"].as_f64();
        let candidate_return = variant["net_return_percent"].as_f64();
        let baseline_evaluated_at_ms = baseline["evaluated_timestamp_ms"].as_u64();
        let candidate_evaluated_at_ms = variant["evaluated_timestamp_ms"].as_u64();
        let label_window_end_timestamp_ms = baseline_evaluated_at_ms
            .zip(candidate_evaluated_at_ms)
            .map(|(baseline_end, candidate_end)| baseline_end.max(candidate_end));
        let signal_timestamp_ms = variant["signal_timestamp_ms"].as_u64();
        let sample_group_id = variant["sample_group_id"].as_str().unwrap_or_default();
        let candidate_search_found = selected_search_contains_group(searches, sample_group_id);
        let baseline_cost_complete =
            baseline["cost_breakdown"]["includesAllExecutionCosts"].as_bool() == Some(true);
        let candidate_cost_complete =
            variant["cost_breakdown"]["includesAllExecutionCosts"].as_bool() == Some(true);
        let both_cost_breakdowns_present =
            baseline["cost_breakdown"].is_object() && variant["cost_breakdown"].is_object();
        let baseline_path = executable_path_comparison(&baseline["cost_breakdown"]);
        let candidate_path = executable_path_comparison(&variant["cost_breakdown"]);
        let paired_path_eligible =
            paired_executable_path_comparison_eligible(&baseline_path, &candidate_path);
        pairs.push(json!({
                "pair_id": format!("{comparison_id}|{}|{baseline_id}|{variant_id}", variant["exit_scenario"].as_str().unwrap_or_default()),
                "comparison_id": comparison_id,
                "evidence_group_id": if sample_group_id.is_empty() { comparison_id } else { sample_group_id },
                "sample_group_id": if sample_group_id.is_empty() { Value::Null } else { json!(sample_group_id) },
                "signal_timestamp_ms": signal_timestamp_ms,
                "evaluated_timestamp_ms": variant["evaluated_timestamp_ms"],
                "label_window_start_timestamp_ms": variant["signal_timestamp_ms"],
                "label_window_end_timestamp_ms": label_window_end_timestamp_ms,
                "label_window_status": if label_window_end_timestamp_ms.is_some() { "observed_for_both_arms" } else { "missing_for_one_or_both_arms" },
                "utc_day": signal_timestamp_ms.map(|timestamp| timestamp / 86_400_000),
                "symbol": variant["symbol"],
                "strategy": variant["strategy"],
                "market_regime": variant["market_regime"],
                "exit_scenario": variant["exit_scenario"],
                "candidate_search_found": candidate_search_found,
                "baseline": {
                    "outcome_id": baseline_id,
                    "profile_id": baseline["profile_id"],
                    "net_return_percent": baseline_return,
                    "notional_usd": baseline["notional_usd"],
                    "portfolio_weight": baseline["portfolio_weight"],
                    "cost_breakdown": baseline["cost_breakdown"],
                },
                "candidate": {
                    "outcome_id": variant_id,
                    "profile_id": variant["profile_id"],
                    "net_return_percent": candidate_return,
                    "notional_usd": variant["notional_usd"],
                    "portfolio_weight": variant["portfolio_weight"],
                    "cost_breakdown": variant["cost_breakdown"],
                },
                "executablePathEvidence": {
                    "baseline": baseline_path,
                    "candidate": candidate_path,
                    "comparisonEligible": paired_path_eligible,
                },
                "net_return_delta_percent": candidate_return.zip(baseline_return).map(|(candidate, baseline)| candidate - baseline),
                "cost_status": if baseline_cost_complete && candidate_cost_complete { "complete_for_both_arms" } else if both_cost_breakdowns_present { "partial_for_both_arms" } else { "one_or_both_arms_missing_cost_breakdown" },
            }));
        paired_variants.insert(variant_id.to_owned());
    }
    pairs.sort_by(|left, right| {
        left["signal_timestamp_ms"]
            .as_u64()
            .cmp(&right["signal_timestamp_ms"].as_u64())
            .then_with(|| {
                left["evidence_group_id"]
                    .as_str()
                    .cmp(&right["evidence_group_id"].as_str())
            })
            .then_with(|| left["pair_id"].as_str().cmp(&right["pair_id"].as_str()))
    });
    let cost_complete_pairs = pairs
        .iter()
        .filter(|pair| pair["cost_status"] == "complete_for_both_arms")
        .count();
    let period_rows = build_observed_period_rows(&pairs);
    let portfolio_interval_rows = portfolio_intervals_response["intervals"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let (flat_cash_periods, portfolio_interval_annotations) =
        build_pm_portfolio_cash_periods(portfolio_intervals_response, &portfolio_interval_rows);
    let cash_period_status = if !flat_cash_periods.is_empty() {
        "pm_internal_complete_flat_intervals_only"
    } else if portfolio_intervals_response["coverage"] == "not_requested" {
        "portfolio_interval_endpoint_not_requested"
    } else {
        "no_pm_internal_complete_flat_intervals"
    };
    let outcomes_history_complete = source_history_complete(outcomes);
    let searches_history_complete = source_history_complete(searches);
    let pairs_missing_search_selection = pairs
        .iter()
        .filter(|pair| pair["candidate_search_found"] != true)
        .count();
    let pairs_missing_complete_costs = pairs.len().saturating_sub(cost_complete_pairs);
    let pairs_missing_utc_day = pairs
        .iter()
        .filter(|pair| pair["utc_day"].as_u64().is_none())
        .count();
    let pairs_missing_arm_returns = pairs
        .iter()
        .filter(|pair| {
            pair["baseline"]["net_return_percent"].as_f64().is_none()
                || pair["candidate"]["net_return_percent"].as_f64().is_none()
        })
        .count();
    let pairs_missing_allocation_metadata = pairs
        .iter()
        .filter(|pair| {
            pair["baseline"]["notional_usd"].as_f64().is_none()
                || pair["baseline"]["portfolio_weight"].as_f64().is_none()
                || pair["candidate"]["notional_usd"].as_f64().is_none()
                || pair["candidate"]["portfolio_weight"].as_f64().is_none()
        })
        .count();
    let pairs_with_complete_executable_path = pairs
        .iter()
        .filter(|pair| pair["executablePathEvidence"]["comparisonEligible"] == true)
        .count();
    let pairs_with_unknown_executable_path = pairs
        .iter()
        .filter(|pair| {
            pair["executablePathEvidence"]["baseline"]["evidence"]["status"] == "unknown"
                || pair["executablePathEvidence"]["candidate"]["evidence"]["status"] == "unknown"
        })
        .count();
    let config = crate::ScoutWalkForwardReplayConfig::default();
    let dataset_hash = replay_dataset_hash(outcomes, searches, &config.config_hash());
    let run_id = format!("scout-replay-v{}-{dataset_hash}", config.config_version);
    let rolling_origin = build_rolling_origin_folds(&pairs, &config);
    let out_of_sample_inference = crate::walk_forward_statistics::summarize_oos_candidate(
        &[],
        "",
        false,
        false,
        config.bootstrap_replicates,
        config.expected_block_length_days,
    );
    let spa_inference = crate::walk_forward_statistics::stationary_bootstrap_spa(
        &[],
        false,
        false,
        config.bootstrap_replicates,
        config.expected_block_length_days,
        config.spa_minimum_day_clusters,
    );
    json!({
        "runId": run_id,
        "datasetHash": dataset_hash,
        "datasetVersion": 1,
        "configVersion": config.config_version,
        "configHash": config.config_hash(),
        "status": "incomplete",
        "selectionAvailable": false,
        "selectionUnavailableReason": "complete time-aligned portfolio streams and observed flat periods are required before candidate selection",
        "coverage": {
            "outcomesRead": outcomes["coverage"]["returnedRecords"],
            "searchesRead": searches["coverage"]["returnedRecords"],
            "outcomesHistoryStatus": if outcomes_history_complete { "complete" } else { "incomplete_or_unreported" },
            "searchesHistoryStatus": if searches_history_complete { "complete" } else { "incomplete_or_unreported" },
            "sourceHistoryComplete": outcomes_history_complete && searches_history_complete,
            "matchedPairs": pairs.len(),
            "unpairedVariantOutcomes": variant_rows.len().saturating_sub(paired_variants.len()),
            "ambiguousVariantMatches": ambiguous_variant_matches,
            "pairsWithCostBreakdowns": cost_complete_pairs,
            "pairsMissingCompleteCostBreakdowns": pairs_missing_complete_costs,
            "pairsMissingSelectedSearchEvidence": pairs_missing_search_selection,
            "pairsMissingUtcDay": pairs_missing_utc_day,
            "pairsMissingArmReturns": pairs_missing_arm_returns,
            "pairsMissingAllocationMetadata": pairs_missing_allocation_metadata,
            "pairsWithCompleteExecutablePathEvidence": pairs_with_complete_executable_path,
            "pairsWithUnknownExecutablePathEvidence": pairs_with_unknown_executable_path,
            "pairsWithPartialCostBreakdowns": pairs
                .iter()
                .filter(|pair| pair["cost_status"] == "partial_for_both_arms")
                .count(),
            "cashPeriodStatus": cash_period_status,
            "cashPeriodsEmitted": flat_cash_periods.len(),
            "portfolioEvidenceIntervalsRead": portfolio_interval_rows.len(),
            "portfolioEvidenceIntervalCoverage": portfolio_intervals_response["coverage"].as_str().unwrap_or("unknown"),
            "portfolioEvidenceIntervalsEndpointTruncated": portfolio_intervals_response["truncated"].as_bool().unwrap_or(true),
            "portfolioEvidenceIntervalsInternallyComplete": portfolio_interval_rows.iter().filter(|interval| interval["complete"] == true).count(),
            "portfolioEvidenceIntervalsWithGaps": portfolio_interval_annotations.iter().filter(|row| row["stream_gap_reason"].is_string()).count(),
            "independentExecutionLedgerReconciled": false,
            "quantitativeEligibility": "blocked_pending_independent_reconciliation",
            "utcPeriodsObserved": period_rows.len(),
            "utcPeriodStatus": if outcomes_history_complete && searches_history_complete && pairs_missing_utc_day == 0 { "observed_pairs_only_missing_periods_unresolved" } else { "incomplete_or_unreported_source_period_coverage" },
            "noPositionEvidenceRecords": 0,
        },
        "dependenceAwareInference": {
            "status": "unavailable_until_complete_portfolio_timeline",
            "bootstrapMethod": config.bootstrap_method,
            "bootstrapReplicates": config.bootstrap_replicates,
            "expectedBlockLengthDays": config.expected_block_length_days,
            "spaMinimumDayClusters": config.spa_minimum_day_clusters,
            "selectedCandidate": out_of_sample_inference,
            "searchedFamilySpa": spa_inference,
        },
        "rollingOrigin": rolling_origin,
        "alignedUtcPeriodRows": period_rows,
        "pmPortfolioEvidenceIntervals": {
            "schemaVersion": portfolio_intervals_response["schema_version"],
            "coverage": portfolio_intervals_response["coverage"],
            "truncated": portfolio_intervals_response["truncated"],
            "nextAfterMs": portfolio_intervals_response["next_after_ms"],
            "quantitativeEligibility": "blocked_pending_independent_reconciliation",
            "independentExecutionLedgerReconciled": false,
            "intervals": portfolio_interval_annotations,
        },
        "pmPortfolioCashPeriods": flat_cash_periods,
        "pairedObservations": pairs,
    })
}

fn executable_path_comparison(cost_breakdown: &Value) -> Value {
    let evidence = cost_breakdown
        .get("scoutPathEvidence")
        .cloned()
        .unwrap_or_else(|| {
            json!({
                "schema_version": 1,
                "status": "unknown",
                "reason": "legacy_markout_missing_path_evidence",
                "maximum_favorable_excursion_percent": null,
                "maximum_adverse_excursion_percent": null,
            })
        });
    let complete = evidence["schema_version"] == 1
        && evidence["status"] == "complete"
        && evidence["complete"] == true;
    json!({
        "comparisonEligible": complete,
        "evidence": evidence,
        "maximumFavorableExcursionPercent": if complete { evidence["maximum_favorable_excursion_percent"].as_f64() } else { None },
        "maximumAdverseExcursionPercent": if complete { evidence["maximum_adverse_excursion_percent"].as_f64() } else { None },
    })
}

fn build_pm_portfolio_cash_periods(
    response: &Value,
    intervals: &[Value],
) -> (Vec<Value>, Vec<Value>) {
    let mut cash_periods = Vec::new();
    let mut annotated_intervals = Vec::with_capacity(intervals.len());
    let mut previous: Option<&Value> = None;
    for interval in intervals {
        let stream_gap_reason = previous.and_then(|prior| {
            let start = interval["interval_start_utc_ms"].as_u64()?;
            let end = interval["interval_end_utc_ms"].as_u64()?;
            let prior_end = prior["interval_end_utc_ms"].as_u64()?;
            if interval["ledger_epoch"] != prior["ledger_epoch"] {
                Some("ledger_epoch_changed_between_returned_intervals")
            } else if interval["portfolio_id"] != prior["portfolio_id"] {
                Some("portfolio_scope_changed_between_returned_intervals")
            } else if interval["execution_mode"] != prior["execution_mode"] {
                Some("execution_mode_changed_between_returned_intervals")
            } else if interval["completeness_scope"] != prior["completeness_scope"] {
                Some("completeness_scope_changed_between_returned_intervals")
            } else if start > prior_end {
                Some("interval_start_after_previous_end")
            } else if start < prior_end {
                Some("interval_overlaps_previous_interval")
            } else if interval["fill_cursor_start"].as_u64() != prior["fill_cursor_end"].as_u64() {
                Some("fill_cursor_discontinuity")
            } else if interval["topup_cursor_start"].as_u64() != prior["topup_cursor_end"].as_u64()
            {
                Some("topup_cursor_discontinuity")
            } else if end <= start {
                Some("non_positive_interval_duration")
            } else {
                None
            }
        });
        let eligibility_reason =
            pm_flat_cash_interval_ineligibility(interval).or(stream_gap_reason);
        let cash_period_emitted = eligibility_reason.is_none();
        let next_after_ms = previous.and_then(|prior| prior["interval_end_utc_ms"].as_u64());
        annotated_intervals.push(json!({
            "evidence_id": interval["evidence_id"],
            "interval_start_utc_ms": interval["interval_start_utc_ms"],
            "interval_end_utc_ms": interval["interval_end_utc_ms"],
            "stream_gap_reason": stream_gap_reason,
            "cash_period_emitted": cash_period_emitted,
            "cash_period_eligibility_reason": eligibility_reason,
            "next_after_ms": next_after_ms,
            "raw_interval": interval,
        }));
        if cash_period_emitted {
            let start = interval["interval_start_utc_ms"].as_u64().unwrap();
            let end = interval["interval_end_utc_ms"].as_u64().unwrap();
            let mut cash_period = json!({
                "source": "position_management_portfolio_evidence_intervals_v1",
                "evidence_id": interval["evidence_id"],
                "portfolio_id": interval["portfolio_id"],
                "execution_mode": interval["execution_mode"],
                "interval_start_utc_ms": start,
                "interval_end_utc_ms": end,
                "interval_semantics": "[interval_start_utc_ms, interval_end_utc_ms)",
                "period_kind": "pm_internal_complete_flat_cash_interval",
                "ledger_epoch": interval["ledger_epoch"],
                "fill_cursor_start": interval["fill_cursor_start"],
                "fill_cursor_end": interval["fill_cursor_end"],
                "trade_row_cursor_end": interval["trade_row_cursor_end"],
                "topup_cursor_start": interval["topup_cursor_start"],
                "topup_cursor_end": interval["topup_cursor_end"],
                "baseline_evidence_id": interval["baseline_evidence_id"],
                "baseline_source": interval["baseline_source"],
                "opening_cash_amount": interval["opening_cash_amount"],
                "opening_cash_currency": interval["opening_cash_currency"],
                "known_symbols": interval["known_symbols"],
                "explicit_flat_symbols": interval["explicit_flat_symbols"],
                "known_fee_fills": interval["known_fee_fills"],
                "unknown_fee_fills": interval["unknown_fee_fills"],
                "unknown_cashflow_fills": interval["unknown_cashflow_fills"],
                "completeness_reasons": interval["completeness_reasons"],
                "provenance": interval["provenance"],
                "completeness_scope": interval["completeness_scope"],
                "source_interval_complete": true,
                "independent_execution_ledger_reconciled": false,
                "quantitative_eligibility": "blocked_pending_independent_reconciliation",
                "return_percent": null,
                "return_value_status": "not_computed_by_learning_pm_source_values_unreconciled",
            });
            let source_financial_evidence = json!({
                "opening_cash_balances_by_currency": interval["opening_cash_balances_by_currency"],
                "opening_cash_value_usd_by_currency": interval["opening_cash_value_usd_by_currency"],
                "opening_cash_total_usd": interval["opening_cash_total_usd"],
                "opening_fx_rates_usd_by_currency": interval["opening_fx_rates_usd_by_currency"],
                "opening_fx_rates_updated_at_by_currency": interval["opening_fx_rates_updated_at_by_currency"],
                "symbol_coverage_scope": interval["symbol_coverage_scope"],
                "all_symbols_covered": interval["all_symbols_covered"],
                "cash_balances_by_currency": interval["cash_balances_by_currency"],
                "cash_movements_by_currency": interval["cash_movements_by_currency"],
                "cashflow_since_origin_by_currency": interval["cashflow_since_origin_by_currency"],
                "external_contributions_by_currency": interval["external_contributions_by_currency"],
                "external_contribution_events": interval["external_contribution_events"],
                "closing_cash_value_usd_by_currency": interval["closing_cash_value_usd_by_currency"],
                "closing_fx_rates_usd_by_currency": interval["closing_fx_rates_usd_by_currency"],
                "closing_fx_rates_updated_at_by_currency": interval["closing_fx_rates_updated_at_by_currency"],
                "closing_cash_total_usd": interval["closing_cash_total_usd"],
                "portfolio_return_complete": interval["portfolio_return_complete"],
                "portfolio_return_completeness_reasons": interval["portfolio_return_completeness_reasons"],
                "portfolio_return_scope": interval["portfolio_return_scope"],
                "pm_reported_flat_cash_period_net_pnl_usd": interval["flat_cash_period_net_pnl_usd"],
                "pm_reported_flat_cash_period_return_pct": interval["flat_cash_period_return_pct"],
            });
            cash_period
                .as_object_mut()
                .unwrap()
                .extend(source_financial_evidence.as_object().unwrap().clone());
            cash_period["raw_interval"] = interval.clone();
            cash_periods.push(cash_period);
        }
        previous = Some(interval);
    }

    // The endpoint envelope gates consumption as a source. Individual PM
    // intervals remain visible even if pagination or the global history is
    // incomplete; no missing interval is filled or inferred here.
    let source_complete = response["schema_version"] == 1
        && response["coverage"] == "pm_internal_complete"
        && response["truncated"] == false
        && response["intervals"].as_array().is_some()
        && intervals
            .iter()
            .all(|interval| interval["complete"] == true)
        && response["next_cursor"].is_number();
    if !source_complete {
        cash_periods.clear();
        for annotation in &mut annotated_intervals {
            annotation["cash_period_emitted"] = json!(false);
            annotation["cash_period_eligibility_reason"] =
                json!("portfolio_interval_source_not_complete");
        }
    }
    if response["schema_version"] != 1
        || response["intervals"].as_array().is_none()
        || response["truncated"].as_bool().is_none()
    {
        cash_periods.clear();
        for annotation in &mut annotated_intervals {
            annotation["cash_period_emitted"] = json!(false);
            annotation["cash_period_eligibility_reason"] =
                json!("invalid_portfolio_interval_response_envelope");
        }
    }
    (cash_periods, annotated_intervals)
}

fn pm_flat_cash_interval_ineligibility(interval: &Value) -> Option<&'static str> {
    if interval["schema_version"] != 1 {
        return Some("unsupported_interval_schema_version");
    }
    if interval["is_baseline"] == true {
        return Some("baseline_is_not_a_cash_period");
    }
    let Some(start) = interval["interval_start_utc_ms"].as_u64() else {
        return Some("interval_start_missing");
    };
    let Some(end) = interval["interval_end_utc_ms"].as_u64() else {
        return Some("interval_end_missing");
    };
    if end <= start {
        return Some("invalid_half_open_interval_bounds");
    }
    if interval["complete"] != true {
        return Some("pm_internal_interval_incomplete");
    }
    if interval["valuation_status"] != "flat_cash_only"
        || interval["all_symbols_covered"] != true
        || interval["coverage_gap"] == true
        || interval["heartbeat_gap"] == true
        || interval["gap_before_baseline"] == true
        || interval["ledger_epoch_changed"] == true
        || interval["truncated"] == true
    {
        return Some("flat_or_interval_coverage_not_explicit");
    }
    let Some(_epoch) = interval["ledger_epoch"]
        .as_str()
        .filter(|epoch| !epoch.is_empty())
    else {
        return Some("ledger_epoch_missing");
    };
    let fill_cursors = interval["fill_cursor_start"]
        .as_u64()
        .zip(interval["fill_cursor_end"].as_u64());
    let topup_cursors = interval["topup_cursor_start"]
        .as_u64()
        .zip(interval["topup_cursor_end"].as_u64());
    if fill_cursors.is_none_or(|(start, end)| start > end)
        || topup_cursors.is_none_or(|(start, end)| start > end)
    {
        return Some("allocator_or_topup_cursor_missing_or_inverted");
    }
    if interval["baseline_evidence_id"].as_str().is_none()
        || interval["baseline_source"].as_str().is_none()
    {
        return Some("complete_baseline_provenance_missing");
    }
    if interval["unknown_fee_fills"].as_u64() != Some(0)
        || interval["unknown_cashflow_fills"].as_u64() != Some(0)
        || interval["completeness_reasons"]
            .as_array()
            .is_none_or(|reasons| !reasons.is_empty())
    {
        return Some("fee_cashflow_or_internal_completeness_reason_present");
    }
    if !pm_explicitly_flat_symbols(interval) {
        return Some("position_flatness_not_explicit_for_known_symbols");
    }
    for field in [
        "cash_balances_by_currency",
        "cash_movements_by_currency",
        "cashflow_since_origin_by_currency",
    ] {
        let Some(values) = interval[field].as_object() else {
            return Some("currency_cash_evidence_missing");
        };
        if values
            .values()
            .any(|value| value.as_f64().is_none_or(|n| !n.is_finite()))
        {
            return Some("invalid_currency_cash_evidence");
        }
    }
    if interval["provenance"]
        .as_array()
        .is_none_or(|values| values.is_empty())
    {
        return Some("interval_source_provenance_missing");
    }
    None
}

fn pm_explicitly_flat_symbols(interval: &Value) -> bool {
    let Some(known_symbols) = interval["known_symbols"].as_array() else {
        return false;
    };
    let Some(flat_symbols) = interval["explicit_flat_symbols"].as_array() else {
        return false;
    };
    let Some(positions) = interval["positions"].as_array() else {
        return false;
    };
    let positions_flat = positions.iter().all(|position| {
        if position["status"].as_str().is_some() {
            position["status"]
                .as_str()
                .is_some_and(|status| status.starts_with("closed"))
                && position["closed_at_ms"].as_u64().is_some()
        } else {
            position["quantity"]
                .as_f64()
                .is_some_and(|quantity| quantity.abs() <= 1e-12)
        }
    });
    positions_flat
        && known_symbols.iter().all(|symbol| {
            let Some(symbol) = symbol.as_str() else {
                return false;
            };
            flat_symbols.iter().any(|flat| {
                flat.as_str()
                    .is_some_and(|flat| flat == symbol || flat == format!("carry:{symbol}"))
            })
        })
}

fn paired_executable_path_comparison_eligible(baseline: &Value, candidate: &Value) -> bool {
    baseline["comparisonEligible"] == true
        && candidate["comparisonEligible"] == true
        && baseline["evidence"]["observation_only"] == candidate["evidence"]["observation_only"]
}

/// Build observed-only UTC-period containers. Rows retain the source outcome
/// IDs and fee metadata at opportunity-group granularity; this intentionally
/// does not sum, weight, rank, or estimate candidate returns.
fn build_observed_period_rows(pairs: &[Value]) -> Vec<Value> {
    let mut periods = BTreeMap::<u64, BTreeMap<String, Vec<&Value>>>::new();
    for pair in pairs {
        let Some(day) = pair["utc_day"].as_u64() else {
            continue;
        };
        let group = pair["evidence_group_id"]
            .as_str()
            .filter(|group| !group.is_empty())
            .unwrap_or("missing-opportunity-group");
        periods
            .entry(day)
            .or_default()
            .entry(group.to_owned())
            .or_default()
            .push(pair);
    }

    periods
        .into_iter()
        .map(|(day, groups)| {
            let opportunity_groups = groups
                .into_iter()
                .map(|(group_id, observations)| {
                    let pair_ids = observations
                        .iter()
                        .map(|pair| pair["pair_id"].clone())
                        .collect::<Vec<_>>();
                    let mut seen_baseline_legs = BTreeSet::new();
                    let baseline_legs = observations
                        .iter()
                        .filter_map(|pair| {
                            let leg = &pair["baseline"];
                            let key = leg["outcome_id"]
                                .as_str()
                                .filter(|id| !id.is_empty())
                                .or_else(|| pair["pair_id"].as_str())?;
                            seen_baseline_legs
                                .insert(key.to_owned())
                                .then(|| leg.clone())
                        })
                        .collect::<Vec<_>>();
                    let mut seen_candidate_legs = BTreeSet::new();
                    let candidate_legs = observations
                        .iter()
                        .filter_map(|pair| {
                            let leg = &pair["candidate"];
                            let key = leg["outcome_id"]
                                .as_str()
                                .filter(|id| !id.is_empty())
                                .or_else(|| pair["pair_id"].as_str())?;
                            seen_candidate_legs
                                .insert(key.to_owned())
                                .then(|| leg.clone())
                        })
                        .collect::<Vec<_>>();
                    let all_costs_complete = observations
                        .iter()
                        .all(|pair| pair["cost_status"] == "complete_for_both_arms");
                    json!({
                        "evidenceGroupId": group_id,
                        "pairedObservationCount": observations.len(),
                        "pairIds": pair_ids,
                        "baselineLegCount": baseline_legs.len(),
                        "baselineLegs": baseline_legs,
                        "candidateLegCount": candidate_legs.len(),
                        "candidateLegs": candidate_legs,
                        "sharedLegsDeduplicatedByOutcomeId": true,
                        "costStatus": if all_costs_complete { "complete_for_all_observed_legs" } else { "incomplete_for_one_or_more_observed_legs" },
                        "aggregationStatus": "grouped_and_deduplicated_shared_legs_raw_values_preserved_no_return_aggregation",
                    })
                })
                .collect::<Vec<_>>();
            let observation_count = opportunity_groups
                .iter()
                .map(|group| group["pairedObservationCount"].as_u64().unwrap_or_default())
                .sum::<u64>();
            let groups_with_complete_costs = opportunity_groups
                .iter()
                .filter(|group| group["costStatus"] == "complete_for_all_observed_legs")
                .count();
            json!({
                "utcDay": day,
                "periodStartTimestampMs": day.saturating_mul(86_400_000),
                "periodEndTimestampMs": day.saturating_add(1).saturating_mul(86_400_000),
                "status": "observed_paired_outcomes_only",
                "cashStatus": "not_emitted_without_explicit_no_position_evidence",
                "pairedObservationCount": observation_count,
                "opportunityGroupCount": opportunity_groups.len(),
                "groupsWithCompleteCosts": groups_with_complete_costs,
                "groupsWithIncompleteCosts": opportunity_groups.len().saturating_sub(groups_with_complete_costs),
                "opportunityGroups": opportunity_groups,
            })
        })
        .collect()
}

fn source_history_complete(source: &Value) -> bool {
    source["coverage"]["complete"].as_bool() == Some(true)
        && !source["coverage"]["hasMore"].as_bool().unwrap_or(false)
        && source["coverage"]["status"].as_str() == Some("complete")
}

fn replay_dataset_hash(outcomes: &Value, searches: &Value, config_hash: &str) -> String {
    let bytes = serde_json::to_vec(&json!({
        "outcomes": outcomes,
        "searches": searches,
        "configHash": config_hash,
    }))
    .expect("Scout replay inputs are serializable");
    let hash = bytes
        .into_iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        });
    format!("fnv1a64:{hash:016x}")
}

/// Assign durable matched observations to chronological training, evaluation,
/// and final-holdout windows. This is an audit assignment only: candidate
/// selection remains disabled until the source proves the complete portfolio
/// timeline, including established flat periods.
fn build_rolling_origin_folds(
    pairs: &[Value],
    config: &crate::ScoutWalkForwardReplayConfig,
) -> Value {
    const DAY_MS: u64 = 86_400_000;
    let mut days = pairs
        .iter()
        .filter_map(|pair| pair["utc_day"].as_u64())
        .collect::<Vec<_>>();
    days.sort_unstable();
    days.dedup();
    let Some(first_day) = days.first().copied() else {
        return json!({
            "status": "incomplete",
            "configVersion": config.config_version,
            "configHash": config.config_hash(),
            "reason": "no_matched_observations",
            "folds": [],
            "finalHoldout": Value::Null,
        });
    };
    let Some(last_day) = days.last().copied() else {
        unreachable!();
    };
    let holdout_start_day = last_day.saturating_sub(config.final_holdout_days.saturating_sub(1));
    let first_origin_day = first_day
        // The evaluated timestamp's UTC day must be strictly before origin;
        // reserve one additional day because an observation signalled on day N
        // can only complete on day N+1 or later.
        .saturating_add(config.minimum_history_day_clusters as u64)
        .saturating_add(1)
        .max(first_day.saturating_add(config.origin_cadence_days));
    let mut folds = Vec::new();
    let mut origin_day = first_origin_day;
    while origin_day.saturating_add(config.evaluation_window_days) <= holdout_start_day {
        let origin_at_ms = origin_day.saturating_mul(DAY_MS);
        let evaluation_end_ms = origin_day
            .saturating_add(config.evaluation_window_days)
            .saturating_mul(DAY_MS);
        let training = pairs
            .iter()
            .filter(|pair| {
                pair["evaluated_timestamp_ms"]
                    .as_u64()
                    .is_some_and(|at| at < origin_at_ms)
                    && pair["label_window_end_timestamp_ms"]
                        .as_u64()
                        .is_some_and(|end| end < origin_at_ms)
            })
            .collect::<Vec<_>>();
        let training_candidates = pairs
            .iter()
            .filter(|pair| {
                pair["evaluated_timestamp_ms"]
                    .as_u64()
                    .is_some_and(|at| at < origin_at_ms)
            })
            .count();
        let training_days = training
            .iter()
            .filter_map(|pair| pair["evaluated_timestamp_ms"].as_u64())
            .map(|timestamp| timestamp / DAY_MS)
            .collect::<BTreeSet<_>>();
        let evaluation = pairs
            .iter()
            .filter(|pair| {
                pair["signal_timestamp_ms"]
                    .as_u64()
                    .is_some_and(|at| at >= origin_at_ms && at < evaluation_end_ms)
            })
            .collect::<Vec<_>>();
        let evaluation_matured_after_window = evaluation
            .iter()
            .filter(|pair| {
                pair["evaluated_timestamp_ms"]
                    .as_u64()
                    .is_some_and(|at| at >= evaluation_end_ms)
            })
            .count();
        folds.push(json!({
            "foldIndex": folds.len(),
            "originAtMs": origin_at_ms,
            "evaluationStartAtMs": origin_at_ms,
            "evaluationEndAtMs": evaluation_end_ms,
            "trainingObservationCount": training.len(),
            "purgedOrUnverifiableTrainingObservationCount": training_candidates.saturating_sub(training.len()),
            "trainingDayClusters": training_days.len(),
            "trainingEffectiveDayClusterCount": training_days.len(),
            "historyMeetsMinimumDayClusters": training_days.len() >= config.minimum_history_day_clusters,
            "historyMeetsMinimumEffectiveSampleSize": (training_days.len() as f64) >= config.minimum_effective_sample_size,
            "strictlyBeforeOrigin": training.iter().all(|pair| pair["evaluated_timestamp_ms"].as_u64().is_some_and(|at| at < origin_at_ms)),
            "evaluationObservationCount": evaluation.len(),
            "evaluationOutcomesMaturedAfterWindowCount": evaluation_matured_after_window,
            "selectedCandidateProfileId": Value::Null,
            "selectionStatus": "unavailable_until_portfolio_timeline_complete",
            "trainingPairIds": training.iter().map(|pair| pair["pair_id"].clone()).collect::<Vec<_>>(),
            "evaluationPairIds": evaluation.iter().map(|pair| pair["pair_id"].clone()).collect::<Vec<_>>(),
        }));
        origin_day = origin_day.saturating_add(config.origin_cadence_days);
    }
    let holdout_start_ms = holdout_start_day.saturating_mul(DAY_MS);
    let holdout_end_ms = last_day.saturating_add(1).saturating_mul(DAY_MS);
    let holdout = pairs
        .iter()
        .filter(|pair| {
            pair["signal_timestamp_ms"]
                .as_u64()
                .is_some_and(|at| at >= holdout_start_ms && at < holdout_end_ms)
        })
        .collect::<Vec<_>>();
    json!({
        "status": "incomplete",
        "configVersion": config.config_version,
        "configHash": config.config_hash(),
        "trainingWindow": "expanding",
        "originCadenceDays": config.origin_cadence_days,
        "evaluationWindowDays": config.evaluation_window_days,
        "finalHoldoutDays": config.final_holdout_days,
        "holdoutStartAtMs": holdout_start_ms,
        "holdoutEndAtMs": holdout_end_ms,
        "finalHoldoutAssignmentCount": holdout.len(),
        "finalHoldoutUsedForSelection": false,
        "cashPeriodStatus": "not_established",
        "selectionAvailable": false,
        "folds": folds,
        "finalHoldoutPairIds": holdout.iter().map(|pair| pair["pair_id"].clone()).collect::<Vec<_>>(),
    })
}

fn selected_search_contains_group(searches: &Value, sample_group_id: &str) -> bool {
    !sample_group_id.is_empty()
        && searches["searches"].as_array().is_some_and(|rows| {
            rows.iter().any(|search| {
                search["candidates"].as_array().is_some_and(|candidates| {
                    candidates.iter().any(|candidate| {
                        candidate["stage"] == "expanded"
                            && candidate["selection_result"] == "selected"
                            && candidate["sample_group_id"].as_str() == Some(sample_group_id)
                    })
                })
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pm_interval(
        evidence_id: &str,
        start_ms: u64,
        end_ms: u64,
        fill_start: u64,
        fill_end: u64,
        topup_start: u64,
        topup_end: u64,
        baseline: bool,
    ) -> Value {
        let mut interval = json!({
            "schema_version": 1,
            "evidence_id": evidence_id,
            "portfolio_id": "paper-default",
            "execution_mode": "paper",
            "interval_start_utc_ms": start_ms,
            "interval_end_utc_ms": end_ms,
            "ledger_epoch": "epoch-1",
            "fill_cursor_start": fill_start,
            "fill_cursor_end": fill_end,
            "trade_row_cursor_end": fill_end,
            "topup_cursor_start": topup_start,
            "topup_cursor_end": topup_end,
            "baseline_source": "position_manager_account_fill_topup_ledger_v1",
            "baseline_evidence_id": "baseline-0",
            "is_baseline": baseline,
            "opening_cash_amount": 10_000.0,
            "opening_cash_currency": "USD",
            "opening_cash_balances_by_currency": {"USD": 10_000.0, "USDT": 1.0},
            "opening_cash_value_usd_by_currency": {"USD": 10_000.0, "USDT": 1.0},
            "opening_cash_total_usd": 10_001.0,
            "opening_fx_rates_usd_by_currency": {"USD": 1.0, "USDT": 1.0},
            "opening_fx_rates_updated_at_by_currency": {"USD": start_ms, "USDT": start_ms},
            "known_symbols": ["BTC-USDT"],
            "positions": [{"symbol":"BTC-USDT","quantity":0.0}],
            "explicit_flat_symbols": ["BTC-USDT"],
            "all_symbols_covered": true,
        });
        let coverage = json!({
            "symbol_coverage_scope": "all_position_symbols_in_position_manager_ledger",
            "cashflow_since_origin_by_currency": {"USD": 25.0, "USDT": -0.1},
            "cash_balances_by_currency": {"USD": 10_025.0, "USDT": 0.9},
            "cash_movements_by_currency": {"USD": 25.0, "USDT": -0.1},
            "external_contributions_by_currency": {"USD": 25.0},
            "external_contribution_events": [{"contribution_id":"pm-topup:v1:0","source":"position_manager_account_topup_ledger","source_cursor":0,"timestamp_utc_ms":end_ms,"currency":"USD","amount":25.0,"value_usd":25.0}],
            "closing_cash_value_usd_by_currency": {"USD": 10_025.0, "USDT": 0.9},
            "closing_fx_rates_usd_by_currency": {"USD": 1.0, "USDT": 1.0},
            "closing_fx_rates_updated_at_by_currency": {"USD": end_ms, "USDT": end_ms},
            "closing_cash_total_usd": 10_025.9,
            "portfolio_return_complete": true,
            "portfolio_return_completeness_reasons": [],
            "flat_cash_period_net_pnl_usd": -0.1,
            "flat_cash_period_return_pct": -0.000999900009999,
            "portfolio_return_scope": "pm_internal_flat_cash_only",
            "unknown_cashflow_fills": 0,
            "known_fee_fills": 2,
            "unknown_fee_fills": 0,
            "legacy_fill_rows_without_allocator_identity": 0,
            "provenance": ["position_manager_sqlite_fill_ledger", "position_manager_account_api_usd_cash_baseline"],
            "heartbeat_at_utc_ms": end_ms,
            "heartbeat_source": "position_manager_ledger_persistence_only",
            "heartbeat_gap": false,
            "gap_before_baseline": false,
            "ledger_epoch_changed": false,
            "valuation_status": "flat_cash_only",
            "market_mark_at_utc_ms": null,
            "coverage_gap": false,
            "truncated": false,
            "complete": true,
            "completeness_scope": "position_manager_internal_source",
            "independent_execution_ledger_reconciled": false,
            "quantitative_eligibility": "blocked_pending_independent_reconciliation",
            "completeness_reasons": []
        });
        interval
            .as_object_mut()
            .unwrap()
            .extend(coverage.as_object().unwrap().clone());
        interval
    }

    #[test]
    fn executable_path_replay_exposes_mfe_mae_only_for_complete_v1_coverage() {
        let complete = executable_path_comparison(&json!({
            "scoutPathEvidence": {
                "schema_version": 1,
                "status": "complete",
                "complete": true,
                "maximum_favorable_excursion_percent": 0.8,
                "maximum_adverse_excursion_percent": -0.3,
                "observation_only": true
            }
        }));
        assert_eq!(complete["comparisonEligible"], true);
        assert_eq!(complete["maximumFavorableExcursionPercent"], 0.8);
        assert_eq!(complete["maximumAdverseExcursionPercent"], -0.3);

        let incomplete = executable_path_comparison(&json!({
            "scoutPathEvidence": {
                "schema_version": 1,
                "status": "incomplete",
                "complete": false,
                "maximum_favorable_excursion_percent": 2.0,
                "maximum_adverse_excursion_percent": -1.0
            }
        }));
        assert_eq!(incomplete["comparisonEligible"], false);
        assert_eq!(incomplete["maximumFavorableExcursionPercent"], Value::Null);
        assert_eq!(incomplete["maximumAdverseExcursionPercent"], Value::Null);

        let unknown = executable_path_comparison(&json!({}));
        assert_eq!(unknown["evidence"]["status"], "unknown");
        assert_eq!(unknown["maximumFavorableExcursionPercent"], Value::Null);

        let executable = executable_path_comparison(&json!({
            "scoutPathEvidence": {
                "schema_version": 1,
                "status": "complete",
                "complete": true,
                "observation_only": false
            }
        }));
        assert!(!paired_executable_path_comparison_eligible(
            &complete,
            &executable
        ));
    }

    #[test]
    fn pm_bridge_emits_only_complete_explicitly_flat_half_open_cash_intervals() {
        let outcomes = json!({"coverage":{"status":"incomplete","complete":false,"returnedRecords":0},"outcomes":[]});
        let searches = json!({"coverage":{"status":"incomplete","complete":false,"returnedRecords":0},"searches":[]});
        let intervals = json!({
            "schema_version":1,
            "coverage":"pm_internal_complete",
            "quantitative_eligibility":"blocked_pending_independent_reconciliation",
            "truncated":false,
            "next_cursor":2,
            "intervals":[
                pm_interval("baseline-0", 1_000, 1_000, 0, 0, 0, 0, true),
                pm_interval("interval-1", 1_000, 2_000, 0, 0, 0, 0, false)
            ]
        });

        let stream =
            build_paired_return_stream_with_portfolio_intervals(&outcomes, &searches, &intervals);
        assert_eq!(
            stream["pmPortfolioCashPeriods"].as_array().unwrap().len(),
            1
        );
        let cash_period = &stream["pmPortfolioCashPeriods"][0];
        assert_eq!(cash_period["interval_start_utc_ms"], 1_000);
        assert_eq!(cash_period["interval_end_utc_ms"], 2_000);
        assert_eq!(
            cash_period["interval_semantics"],
            "[interval_start_utc_ms, interval_end_utc_ms)"
        );
        assert_eq!(cash_period["cash_movements_by_currency"]["USD"], 25.0);
        assert_eq!(cash_period["cash_movements_by_currency"]["USDT"], -0.1);
        assert_eq!(
            cash_period["opening_cash_balances_by_currency"]["USDT"],
            1.0
        );
        assert_eq!(
            cash_period["opening_cash_value_usd_by_currency"]["USDT"],
            1.0
        );
        assert_eq!(cash_period["opening_cash_total_usd"], 10_001.0);
        assert_eq!(cash_period["opening_fx_rates_usd_by_currency"]["USDT"], 1.0);
        assert_eq!(
            cash_period["opening_fx_rates_updated_at_by_currency"]["USDT"],
            1_000
        );
        assert_eq!(
            cash_period["external_contributions_by_currency"]["USD"],
            25.0
        );
        assert_eq!(
            cash_period["external_contribution_events"][0]["contribution_id"],
            "pm-topup:v1:0"
        );
        assert_eq!(
            cash_period["closing_cash_value_usd_by_currency"]["USDT"],
            0.9
        );
        assert_eq!(
            cash_period["closing_fx_rates_updated_at_by_currency"]["USDT"],
            2_000
        );
        assert_eq!(cash_period["closing_cash_total_usd"], 10_025.9);
        assert_eq!(cash_period["portfolio_return_complete"], true);
        assert_eq!(
            cash_period["portfolio_return_completeness_reasons"],
            json!([])
        );
        assert_eq!(
            cash_period["portfolio_return_scope"],
            "pm_internal_flat_cash_only"
        );
        assert_eq!(
            cash_period["pm_reported_flat_cash_period_net_pnl_usd"],
            -0.1
        );
        assert_eq!(
            cash_period["pm_reported_flat_cash_period_return_pct"],
            -0.000999900009999
        );
        assert_eq!(cash_period["ledger_epoch"], "epoch-1");
        assert_eq!(cash_period["fill_cursor_start"], 0);
        assert_eq!(cash_period["fill_cursor_end"], 0);
        assert_eq!(cash_period["topup_cursor_start"], 0);
        assert_eq!(cash_period["topup_cursor_end"], 0);
        assert_eq!(cash_period["known_fee_fills"], 2);
        assert_eq!(cash_period["provenance"].as_array().unwrap().len(), 2);
        assert_eq!(cash_period["return_percent"], Value::Null);
        assert_eq!(
            cash_period["return_value_status"],
            "not_computed_by_learning_pm_source_values_unreconciled"
        );
        assert_eq!(
            cash_period["independent_execution_ledger_reconciled"],
            false
        );
        assert_eq!(stream["selectionAvailable"], false);
        assert_eq!(
            stream["dependenceAwareInference"]["status"],
            "unavailable_until_complete_portfolio_timeline"
        );
        assert_eq!(stream["coverage"]["portfolioEvidenceIntervalsRead"], 2);
    }

    #[test]
    fn pm_bridge_preserves_gaps_and_rejects_incomplete_or_nonflat_periods() {
        let complete = pm_interval("period-1", 1_000, 2_000, 0, 1, 0, 1, false);
        let mut gap = pm_interval("period-gap", 3_000, 4_000, 1, 1, 1, 1, false);
        gap["cash_movements_by_currency"] = json!({"USD":99.0});
        let mut incomplete = pm_interval("period-incomplete", 4_000, 5_000, 1, 1, 1, 1, false);
        incomplete["complete"] = json!(false);
        incomplete["coverage_gap"] = json!(true);
        incomplete["completeness_reasons"] =
            json!(["persistence_checkpoint_gap_exceeds_60_seconds"]);
        let mut nonflat = pm_interval("period-nonflat", 5_000, 6_000, 1, 1, 1, 1, false);
        nonflat["valuation_status"] = json!("unavailable_for_open_positions");
        nonflat["all_symbols_covered"] = json!(false);
        nonflat["explicit_flat_symbols"] = json!([]);
        nonflat["positions"] = json!([{"symbol":"BTC-USDT","quantity":1.0}]);
        let response = json!({
            "schema_version":1,"coverage":"pm_internal_complete","truncated":false,"next_cursor":4,
            "intervals":[complete,gap,incomplete,nonflat]
        });
        let (periods, annotations) =
            build_pm_portfolio_cash_periods(&response, response["intervals"].as_array().unwrap());
        assert!(
            periods.is_empty(),
            "an incomplete source history must not emit aligned cash periods"
        );
        assert_eq!(
            annotations[1]["stream_gap_reason"],
            "interval_start_after_previous_end"
        );
        assert_eq!(annotations[1]["cash_period_emitted"], false);
        assert_eq!(annotations[2]["cash_period_emitted"], false);
        assert_eq!(annotations[3]["cash_period_emitted"], false);
        assert_eq!(
            annotations[2]["raw_interval"]["completeness_reasons"][0],
            "persistence_checkpoint_gap_exceeds_60_seconds"
        );
        assert_eq!(
            annotations[1]["raw_interval"]["cash_movements_by_currency"]["USD"],
            99.0
        );
    }

    #[test]
    fn pm_bridge_suppresses_periods_when_source_history_is_incomplete_or_truncated() {
        let interval = pm_interval("period-1", 1_000, 2_000, 0, 0, 0, 0, false);
        for (coverage, truncated) in [("incomplete_or_unknown", false), ("truncated", true)] {
            let response = json!({
                "schema_version":1,"coverage":coverage,"truncated":truncated,
                "next_cursor":1,"intervals":[interval.clone()]
            });
            let (periods, annotations) = build_pm_portfolio_cash_periods(
                &response,
                response["intervals"].as_array().unwrap(),
            );
            assert!(periods.is_empty());
            assert_eq!(annotations[0]["cash_period_emitted"], false);
            assert_eq!(
                annotations[0]["cash_period_eligibility_reason"],
                "portfolio_interval_source_not_complete"
            );
        }
    }

    #[test]
    fn pm_bridge_preserves_return_completeness_reasons_without_computing_returns() {
        let mut interval = pm_interval("period-no-fx", 1_000, 2_000, 0, 1, 0, 1, false);
        interval["portfolio_return_complete"] = json!(false);
        interval["portfolio_return_completeness_reasons"] =
            json!(["closing_timestamp_aligned_fx_valuation_incomplete"]);
        interval["closing_cash_value_usd_by_currency"] = Value::Null;
        interval["closing_cash_total_usd"] = Value::Null;
        interval["flat_cash_period_net_pnl_usd"] = Value::Null;
        interval["flat_cash_period_return_pct"] = Value::Null;
        let response = json!({
            "schema_version":1,
            "coverage":"pm_internal_complete",
            "truncated":false,
            "next_cursor":1,
            "intervals":[interval]
        });

        let (periods, _) =
            build_pm_portfolio_cash_periods(&response, response["intervals"].as_array().unwrap());

        assert_eq!(periods.len(), 1);
        assert_eq!(periods[0]["portfolio_return_complete"], false);
        assert_eq!(
            periods[0]["portfolio_return_completeness_reasons"][0],
            "closing_timestamp_aligned_fx_valuation_incomplete"
        );
        assert_eq!(
            periods[0]["pm_reported_flat_cash_period_net_pnl_usd"],
            Value::Null
        );
        assert_eq!(
            periods[0]["pm_reported_flat_cash_period_return_pct"],
            Value::Null
        );
        assert_eq!(periods[0]["return_percent"], Value::Null);
        assert_eq!(
            periods[0]["return_value_status"],
            "not_computed_by_learning_pm_source_values_unreconciled"
        );
    }

    #[test]
    fn paired_returns_require_comparable_arms_and_never_invent_cash_periods() {
        let outcomes = json!({
            "coverage": {"returnedRecords": 2},
            "outcomes": [
                {
                    "outcome_id":"baseline-1",
                    "scout_comparison_role":"baseline",
                    "scout_comparison_ids":["pair-1"],
                    "symbol":"BTC-USDT",
                    "strategy":"live-momentum",
                    "profile_id":"baseline-profile",
                    "market_regime":"trend",
                    "learning_context":"scout:trend",
                    "exit_scenario":"strategy",
                    "signal_timestamp_ms":1000,
                    "evaluated_timestamp_ms":2000,
                    "sample_group_id":"group-1",
                    "net_return_percent":0.2,
                    "notional_usd":100.0,
                    "portfolio_weight":1.0,
                    "cost_breakdown":{"executionCostPercent":0.1}
                },
                {
                    "outcome_id":"variant-1",
                    "scout_comparison_role":"variant",
                    "scout_comparison_id":"pair-1",
                    "symbol":"BTC-USDT",
                    "strategy":"live-momentum",
                    "profile_id":"variant-profile",
                    "market_regime":"trend",
                    "learning_context":"scout:trend",
                    "exit_scenario":"strategy",
                    "signal_timestamp_ms":1001,
                    "evaluated_timestamp_ms":2100,
                    "sample_group_id":"group-1",
                    "net_return_percent":0.5,
                    "notional_usd":100.0,
                    "portfolio_weight":1.0,
                    "cost_breakdown":{"executionCostPercent":0.1}
                }
            ]
        });
        let searches = json!({
            "coverage":{"returnedRecords":1},
            "searches":[{"candidates":[{
                "stage":"expanded",
                "selection_result":"selected",
                "sample_group_id":"group-1"
            }]}]
        });

        let stream = build_paired_return_stream(&outcomes, &searches);
        let repeated = build_paired_return_stream(&outcomes, &searches);
        assert_eq!(stream["runId"], repeated["runId"]);
        assert_eq!(stream["datasetHash"], repeated["datasetHash"]);
        assert_eq!(stream["pairedObservations"].as_array().unwrap().len(), 1);
        assert!(
            (stream["pairedObservations"][0]["net_return_delta_percent"]
                .as_f64()
                .unwrap()
                - 0.3)
                .abs()
                < 1e-12
        );
        assert_eq!(
            stream["pairedObservations"][0]["candidate_search_found"],
            true
        );
        assert_eq!(
            stream["pairedObservations"][0]["cost_status"],
            "partial_for_both_arms"
        );
        assert_eq!(stream["coverage"]["pairsWithCostBreakdowns"], 0);
        assert_eq!(stream["coverage"]["pairsWithPartialCostBreakdowns"], 1);
        assert_eq!(
            stream["dependenceAwareInference"]["status"],
            "unavailable_until_complete_portfolio_timeline"
        );
        assert_eq!(
            stream["dependenceAwareInference"]["searchedFamilySpa"]["status"],
            "source_history_incomplete"
        );
        assert_eq!(stream["coverage"]["cashPeriodsEmitted"], 0);
        assert_eq!(stream["coverage"]["utcPeriodsObserved"], 1);
        assert_eq!(
            stream["coverage"]["outcomesHistoryStatus"],
            "incomplete_or_unreported"
        );
        assert_eq!(
            stream["alignedUtcPeriodRows"][0]["cashStatus"],
            "not_emitted_without_explicit_no_position_evidence"
        );
        assert_eq!(stream["status"], "incomplete");
        assert_eq!(stream["selectionAvailable"], false);
    }

    #[test]
    fn observed_utc_periods_group_shared_opportunity_legs_without_aggregating_returns() {
        let outcomes = json!({
            "coverage": {"status":"incomplete", "complete":false, "returnedRecords":4},
            "outcomes": [
                {"outcome_id":"b1", "scout_comparison_role":"baseline", "scout_comparison_ids":["c1", "c2"], "symbol":"BTC-USDT", "strategy":"live-momentum", "profile_id":"base", "market_regime":"trend", "learning_context":"ctx", "exit_scenario":"strategy", "signal_timestamp_ms":1000, "evaluated_timestamp_ms":2000, "sample_group_id":"shared", "net_return_percent":1.25, "notional_usd":100.0, "portfolio_weight":0.5, "cost_breakdown":{"includesAllExecutionCosts":true}},
                {"outcome_id":"v1", "scout_comparison_role":"variant", "scout_comparison_id":"c1", "symbol":"BTC-USDT", "strategy":"live-momentum", "profile_id":"candidate-1", "market_regime":"trend", "learning_context":"ctx", "exit_scenario":"strategy", "signal_timestamp_ms":1001, "evaluated_timestamp_ms":2100, "sample_group_id":"shared", "net_return_percent":1.5, "notional_usd":100.0, "portfolio_weight":0.5, "cost_breakdown":{"includesAllExecutionCosts":true}},
                {"outcome_id":"v2", "scout_comparison_role":"variant", "scout_comparison_id":"c2", "symbol":"BTC-USDT", "strategy":"live-momentum", "profile_id":"candidate-2", "market_regime":"trend", "learning_context":"ctx", "exit_scenario":"strategy", "signal_timestamp_ms":1101, "evaluated_timestamp_ms":2300, "sample_group_id":"shared", "net_return_percent":-0.5, "notional_usd":100.0, "portfolio_weight":0.5, "cost_breakdown":{"includesAllExecutionCosts":true}}
            ]
        });
        let searches = json!({
            "coverage": {"status":"incomplete", "complete":false, "returnedRecords":0},
            "searches": []
        });

        let stream = build_paired_return_stream(&outcomes, &searches);
        let day = &stream["alignedUtcPeriodRows"][0];
        assert_eq!(stream["coverage"]["sourceHistoryComplete"], false);
        assert_eq!(stream["coverage"]["cashPeriodsEmitted"], 0);
        assert_eq!(stream["coverage"]["pairsMissingCompleteCostBreakdowns"], 0);
        assert_eq!(day["opportunityGroupCount"], 1);
        assert_eq!(day["pairedObservationCount"], 2);
        assert_eq!(day["opportunityGroups"][0]["pairedObservationCount"], 2);
        assert_eq!(day["opportunityGroups"][0]["baselineLegCount"], 1);
        assert_eq!(
            day["opportunityGroups"][0]["baselineLegs"]
                .as_array()
                .unwrap()
                .len(),
            1,
            "a baseline outcome shared by two candidate comparisons is one leg"
        );
        assert_eq!(day["opportunityGroups"][0]["candidateLegCount"], 2);
        assert_eq!(
            day["opportunityGroups"][0]["candidateLegs"][0]["net_return_percent"],
            1.5
        );
        assert_eq!(
            day["opportunityGroups"][0]["candidateLegs"][1]["net_return_percent"],
            -0.5
        );
        assert_eq!(
            day["opportunityGroups"][0]["aggregationStatus"],
            "grouped_and_deduplicated_shared_legs_raw_values_preserved_no_return_aggregation"
        );
        assert_eq!(stream["selectionAvailable"], false);
    }

    #[test]
    fn rolling_origin_assignments_use_strict_cutoff_and_reserve_holdout() {
        let day_ms = 86_400_000_u64;
        let rows = (0..100_u64)
            .map(|day| {
                json!({
                    "pair_id": format!("pair-{day}"),
                    "utc_day": day,
                    "signal_timestamp_ms": day * day_ms + 1,
                    "evaluated_timestamp_ms": (day + 1) * day_ms,
                    "label_window_end_timestamp_ms": (day + 1) * day_ms,
                })
            })
            .collect::<Vec<_>>();
        let config = crate::ScoutWalkForwardReplayConfig::default();
        let replay = build_rolling_origin_folds(&rows, &config);
        assert_eq!(replay["configHash"], config.config_hash());
        assert_eq!(replay["finalHoldoutUsedForSelection"], false);
        assert_eq!(replay["finalHoldoutAssignmentCount"], 28);
        let folds = replay["folds"].as_array().unwrap();
        assert!(!folds.is_empty());
        assert_eq!(folds[0]["strictlyBeforeOrigin"], true);
        assert_eq!(folds[0]["trainingDayClusters"], 30);
        assert_eq!(folds[0]["historyMeetsMinimumDayClusters"], true);
        assert_eq!(folds[0]["evaluationObservationCount"], 7);
        assert_eq!(folds[0]["evaluationOutcomesMaturedAfterWindowCount"], 1);
        assert_eq!(
            folds[0]["selectionStatus"],
            "unavailable_until_portfolio_timeline_complete"
        );
    }
}
