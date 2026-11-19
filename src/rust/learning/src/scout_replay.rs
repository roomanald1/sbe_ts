//! Paired, fee-aware candidate/baseline observations for Scout replay.

use serde_json::{Value, json};
use std::collections::BTreeSet;

pub(crate) fn build_paired_return_stream(outcomes: &Value, searches: &Value) -> Value {
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
    let mut pairs = Vec::new();
    for variant in &variant_rows {
        let Some(comparison_id) = variant["scout_comparison_id"].as_str() else {
            continue;
        };
        for baseline in &baseline_rows {
            let baseline_has_comparison = baseline["scout_comparison_id"].as_str()
                == Some(comparison_id)
                || baseline["scout_comparison_ids"]
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(comparison_id)));
            let same_opportunity = baseline["symbol"] == variant["symbol"]
                && baseline["strategy"] == variant["strategy"]
                && baseline["market_regime"] == variant["market_regime"]
                && baseline["learning_context"] == variant["learning_context"]
                && baseline["exit_scenario"] == variant["exit_scenario"]
                && baseline["signal_timestamp_ms"]
                    .as_u64()
                    .zip(variant["signal_timestamp_ms"].as_u64())
                    .is_some_and(|(left, right)| left.abs_diff(right) <= 10_000);
            if !baseline_has_comparison || !same_opportunity {
                continue;
            }
            let baseline_id = baseline["outcome_id"].as_str().unwrap_or_default();
            let variant_id = variant["outcome_id"].as_str().unwrap_or_default();
            let baseline_return = baseline["net_return_percent"].as_f64();
            let candidate_return = variant["net_return_percent"].as_f64();
            let signal_timestamp_ms = variant["signal_timestamp_ms"].as_u64();
            let sample_group_id = variant["sample_group_id"].as_str().unwrap_or_default();
            let candidate_search_found = selected_search_contains_group(searches, sample_group_id);
            let baseline_cost_complete = baseline["cost_breakdown"].is_object();
            let candidate_cost_complete = variant["cost_breakdown"].is_object();
            pairs.push(json!({
                "pair_id": format!("{comparison_id}|{}|{baseline_id}|{variant_id}", variant["exit_scenario"].as_str().unwrap_or_default()),
                "comparison_id": comparison_id,
                "evidence_group_id": if sample_group_id.is_empty() { comparison_id } else { sample_group_id },
                "sample_group_id": if sample_group_id.is_empty() { Value::Null } else { json!(sample_group_id) },
                "signal_timestamp_ms": signal_timestamp_ms,
                "evaluated_timestamp_ms": variant["evaluated_timestamp_ms"],
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
                "net_return_delta_percent": candidate_return.zip(baseline_return).map(|(candidate, baseline)| candidate - baseline),
                "cost_status": if baseline_cost_complete && candidate_cost_complete { "both_arms_have_markout_costs" } else { "one_or_both_arms_missing_cost_breakdown" },
            }));
            paired_variants.insert(variant_id.to_owned());
        }
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
        .filter(|pair| pair["cost_status"] == "both_arms_have_markout_costs")
        .count();
    let config = crate::ScoutWalkForwardReplayConfig::default();
    let rolling_origin = build_rolling_origin_folds(&pairs, &config);
    json!({
        "datasetVersion": 1,
        "configVersion": config.config_version,
        "configHash": config.config_hash(),
        "status": "incomplete",
        "selectionAvailable": false,
        "selectionUnavailableReason": "complete time-aligned portfolio streams and observed flat periods are required before candidate selection",
        "coverage": {
            "outcomesRead": outcomes["coverage"]["returnedRecords"],
            "searchesRead": searches["coverage"]["returnedRecords"],
            "matchedPairs": pairs.len(),
            "unpairedVariantOutcomes": variant_rows.len().saturating_sub(paired_variants.len()),
            "pairsWithCostBreakdowns": cost_complete_pairs,
            "cashPeriodStatus": "not_inferred_from_outcome_or_search_rows",
            "cashPeriodsEmitted": 0,
        },
        "rollingOrigin": rolling_origin,
        "pairedObservations": pairs,
    })
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
            })
            .collect::<Vec<_>>();
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
        assert_eq!(stream["coverage"]["cashPeriodsEmitted"], 0);
        assert_eq!(stream["status"], "incomplete");
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
