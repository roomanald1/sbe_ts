//! Bounded in-memory retention for attributed Learning evidence.
//!
//! Profile identity and aggregate counters are retained when old per-event
//! detail is compacted. Detailed outcomes remain useful for paired comparisons
//! and recent profile decisions, so each registered variant keeps its newest
//! records within the existing seven-day window, up to a fixed count.

use super::{LearningState, RegisteredProfile, variant_key};
use axum::http::StatusCode;

pub(super) const MAX_OUTCOMES_PER_VARIANT: usize = 512;
pub(super) const MAX_OUTCOMES_IN_MEMORY: usize = 250_000;
pub(super) const MAX_OUTCOME_DETAIL_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_VARIANTS_IN_MEMORY: usize = 20_000;
pub(super) const MAX_ACTIVE_PROFILES: usize = 4_096;
pub(super) const MAX_CONTEXT_STATS: usize = 10_000;
pub(super) const MAX_EXECUTION_STATS: usize = 10_000;

pub(super) fn compact_variant_outcomes(variant: &mut RegisteredProfile, now_ms: u64) -> bool {
    let initial_len = variant.outcomes.len();
    let cutoff = now_ms.saturating_sub(7 * 24 * 60 * 60 * 1_000);
    variant
        .outcomes
        .retain(|outcome| outcome.observed_at_ms >= cutoff);
    if variant.outcomes.len() > MAX_OUTCOMES_PER_VARIANT {
        variant
            .outcomes
            .sort_by_key(|outcome| outcome.observed_at_ms);
        let remove_count = variant.outcomes.len() - MAX_OUTCOMES_PER_VARIANT;
        variant.outcomes.drain(..remove_count);
    }
    variant.outcomes.len() != initial_len
}

pub(super) fn compact_variants(state: &mut LearningState) -> bool {
    let active_ids = state
        .active_profiles
        .values()
        .map(variant_key)
        .collect::<std::collections::BTreeSet<_>>();
    let active_count = state
        .variants
        .keys()
        .filter(|id| active_ids.contains(*id))
        .count();
    let inactive_budget = MAX_VARIANTS_IN_MEMORY.saturating_sub(active_count);
    if state.variants.len() <= MAX_VARIANTS_IN_MEMORY {
        let compacted = compact_outcome_working_set(&mut state.variants);
        state.outcome_history_truncated |= compacted;
        return compacted;
    }
    let mut inactive = state
        .variants
        .iter()
        .filter(|(id, _)| !active_ids.contains(*id))
        .map(|(id, variant)| {
            let newest = variant
                .outcomes
                .iter()
                .map(|outcome| outcome.observed_at_ms)
                .max()
                .unwrap_or(0);
            (newest, id.clone())
        })
        .collect::<Vec<_>>();
    inactive.sort_by(|left, right| right.cmp(left));
    let keep = inactive
        .into_iter()
        .take(inactive_budget)
        .map(|(_, id)| id)
        .collect::<std::collections::BTreeSet<_>>();
    let variants_compacted = state.variants.len() > MAX_VARIANTS_IN_MEMORY;
    state
        .variants
        .retain(|id, _| active_ids.contains(id) || keep.contains(id));
    let outcomes_compacted = compact_outcome_working_set(&mut state.variants);
    state.outcome_history_truncated |= variants_compacted || outcomes_compacted;
    variants_compacted || outcomes_compacted
}

pub(super) fn compact_outcome_working_set(
    variants: &mut std::collections::BTreeMap<String, RegisteredProfile>,
) -> bool {
    let total = variants
        .values()
        .map(|variant| variant.outcomes.len())
        .sum::<usize>();
    let total_bytes = variants
        .values()
        .flat_map(|variant| variant.outcomes.iter())
        .map(approximate_outcome_bytes)
        .sum::<usize>();
    if total <= MAX_OUTCOMES_IN_MEMORY && total_bytes <= MAX_OUTCOME_DETAIL_BYTES {
        return false;
    }
    // Keep the newest globally bounded set without sorting or duplicating the
    // full outcome payloads. The heap contains only 250k compact references.
    let mut newest = std::collections::BinaryHeap::with_capacity(MAX_OUTCOMES_IN_MEMORY);
    let mut kept_bytes = 0usize;
    let mut discarded_through: Option<(u64, &String, usize)> = None;
    for (id, variant) in variants.iter() {
        for (index, outcome) in variant.outcomes.iter().enumerate() {
            let bytes = approximate_outcome_bytes(outcome);
            let key = (outcome.observed_at_ms, id, index);
            if discarded_through.is_some_and(|floor| key <= floor) {
                continue;
            }
            let item = std::cmp::Reverse((key.0, key.1, key.2, bytes));
            if newest.len() < MAX_OUTCOMES_IN_MEMORY {
                kept_bytes = kept_bytes.saturating_add(bytes);
                newest.push(item);
            } else if newest.peek().is_some_and(|oldest| item > *oldest) {
                if let Some(std::cmp::Reverse((timestamp, removed_id, index, removed_bytes))) =
                    newest.pop()
                {
                    let removed = (timestamp, removed_id, index);
                    if discarded_through.is_none_or(|floor| removed > floor) {
                        discarded_through = Some(removed);
                    }
                    kept_bytes = kept_bytes.saturating_sub(removed_bytes);
                }
                kept_bytes = kept_bytes.saturating_add(bytes);
                newest.push(item);
            }
            while kept_bytes > MAX_OUTCOME_DETAIL_BYTES {
                let Some(std::cmp::Reverse((timestamp, removed_id, index, removed_bytes))) =
                    newest.pop()
                else {
                    break;
                };
                let removed = (timestamp, removed_id, index);
                if discarded_through.is_none_or(|floor| removed > floor) {
                    discarded_through = Some(removed);
                }
                kept_bytes = kept_bytes.saturating_sub(removed_bytes);
            }
        }
    }
    let mut kept = std::collections::BTreeMap::<String, std::collections::BTreeSet<usize>>::new();
    for std::cmp::Reverse((_, id, index, _)) in newest {
        kept.entry(id.clone()).or_default().insert(index);
    }
    for (id, variant) in variants {
        let indices = kept.remove(id).unwrap_or_default();
        let mut index = 0;
        variant.outcomes.retain(|_| {
            let keep = indices.contains(&index);
            index += 1;
            keep
        });
    }
    true
}

fn approximate_outcome_bytes(outcome: &super::LearningOutcome) -> usize {
    std::mem::size_of::<super::LearningOutcome>()
        + outcome
            .evidence_group_id
            .as_ref()
            .map_or(0, |value| value.len() + 24)
        + outcome
            .learning_context
            .as_ref()
            .map_or(0, |value| value.len() + 24)
        + outcome
            .tested_parameter_keys
            .iter()
            .map(|key| key.len() + 32)
            .sum::<usize>()
        + outcome
            .baseline_parameters
            .keys()
            .map(|key| key.len() + 64)
            .sum::<usize>()
}

pub(super) fn ensure_variant_room(
    state: &mut LearningState,
    incoming_id: &str,
) -> Result<(), (StatusCode, String)> {
    if state.variants.contains_key(incoming_id) || state.variants.len() < MAX_VARIANTS_IN_MEMORY {
        return Ok(());
    }
    let active_ids = state
        .active_profiles
        .values()
        .map(variant_key)
        .collect::<std::collections::BTreeSet<_>>();
    let victim = state
        .variants
        .iter()
        .filter(|(id, _)| !active_ids.contains(*id))
        .min_by_key(|(id, variant)| {
            (
                variant
                    .outcomes
                    .iter()
                    .map(|outcome| outcome.observed_at_ms)
                    .max()
                    .unwrap_or(0),
                (*id).clone(),
            )
        })
        .map(|(id, _)| id.clone());
    if let Some(victim) = victim {
        state.variants.remove(&victim);
        state.outcome_history_truncated = true;
        Ok(())
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Learning variant working set is full of active profiles".into(),
        ))
    }
}

pub(super) fn ensure_active_profile_room(
    state: &LearningState,
    profile_key: &str,
) -> Result<(), (StatusCode, String)> {
    if state.active_profiles.contains_key(profile_key)
        || state.active_profiles.len() < MAX_ACTIVE_PROFILES
    {
        Ok(())
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Learning active-profile capacity is full".into(),
        ))
    }
}

pub(super) fn ensure_context_room(
    state: &LearningState,
    context_key: &str,
) -> Result<(), (StatusCode, String)> {
    if state.context_learning.contains_key(context_key)
        || state.context_learning.len() < MAX_CONTEXT_STATS
    {
        Ok(())
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Learning context-stat capacity is full".into(),
        ))
    }
}

pub(super) fn ensure_execution_room(
    state: &LearningState,
    execution_key: &str,
) -> Result<(), (StatusCode, String)> {
    if state.execution_learning.contains_key(execution_key)
        || state.execution_learning.len() < MAX_EXECUTION_STATS
    {
        Ok(())
    } else {
        Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "Learning execution-stat capacity is full".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LearningOutcome, LearningOutcomeSource};
    use std::collections::BTreeMap;
    use strategy::StrategyProfile;

    fn test_profile() -> StrategyProfile {
        StrategyProfile {
            profile_id: "profile-1".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-example".into(),
            parameters: Default::default(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        }
    }

    #[test]
    fn compaction_keeps_newest_attributed_outcomes_and_identity() {
        let mut variant = RegisteredProfile {
            profile: test_profile(),
            specs: Vec::new(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: (0..MAX_OUTCOMES_PER_VARIANT + 3)
                .map(|index| LearningOutcome {
                    observed_at_ms: index as u64 + 1,
                    net_return_percent: 0.25,
                    evidence_group_id: Some(format!("group-{index}")),
                    learning_context: None,
                    source: LearningOutcomeSource::Trade,
                    realized_pnl_usd: Some(1.0),
                    paired_comparison: false,
                    tested_parameter_keys: Vec::new(),
                    baseline_parameters: Default::default(),
                })
                .collect(),
        };

        compact_variant_outcomes(&mut variant, 10_000_000);

        assert_eq!(variant.outcomes.len(), MAX_OUTCOMES_PER_VARIANT);
        assert_eq!(variant.outcomes[0].observed_at_ms, 4);
        assert_eq!(variant.outcomes.last().unwrap().observed_at_ms, 515);
        assert_eq!(
            variant.outcomes[0].evidence_group_id.as_deref(),
            Some("group-3")
        );
        assert_eq!(variant.profile.profile_id, test_profile().profile_id);
    }

    #[test]
    fn compaction_drops_expired_events_before_applying_count_cap() {
        let mut variant = RegisteredProfile {
            profile: test_profile(),
            specs: Vec::new(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: vec![outcome(1), outcome(10_000)],
        };

        compact_variant_outcomes(&mut variant, 7 * 24 * 60 * 60 * 1_000 + 10_000);

        assert_eq!(variant.outcomes.len(), 1);
        assert_eq!(variant.outcomes[0].observed_at_ms, 10_000);
    }

    #[test]
    fn global_variant_compaction_preserves_active_identity_and_newest_inactive_variants() {
        let mut state = super::super::LearningState::default();
        let active = test_profile();
        state.active_profiles.insert(
            super::super::key(&active.symbol, &active.strategy),
            active.clone(),
        );
        for index in 0..=MAX_VARIANTS_IN_MEMORY {
            let mut profile = test_profile();
            profile.profile_id = format!("candidate-{index}");
            profile.parameters.insert("variant".into(), index as f64);
            let id = if index == 0 {
                super::super::variant_key(&active)
            } else {
                super::super::variant_key(&profile)
            };
            let mut variant = RegisteredProfile {
                profile,
                specs: Vec::new(),
                samples: 0,
                positive: 0,
                realized_pnl: 0.0,
                outcomes: vec![outcome(index as u64)],
            };
            if index == 0 {
                variant.profile = active.clone();
            }
            state.variants.insert(id, variant);
        }

        compact_variants(&mut state);

        assert_eq!(state.variants.len(), MAX_VARIANTS_IN_MEMORY);
        assert!(
            state
                .variants
                .contains_key(&super::super::variant_key(&active))
        );
        assert!(!state.variants.contains_key(&super::super::variant_key(&{
            let mut old = test_profile();
            old.profile_id = "candidate-1".into();
            old.parameters.insert("variant".into(), 1.0);
            old
        })));
        let mut newest = test_profile();
        newest.profile_id = format!("candidate-{MAX_VARIANTS_IN_MEMORY}");
        newest
            .parameters
            .insert("variant".into(), MAX_VARIANTS_IN_MEMORY as f64);
        assert!(
            state
                .variants
                .contains_key(&super::super::variant_key(&newest))
        );
    }

    #[test]
    fn legacy_database_compaction_is_persistable_without_dropping_variant_identity() {
        let path = std::env::temp_dir().join(format!(
            "learning-retention-test-{}-{}.sqlite",
            std::process::id(),
            super::super::now_ms()
        ));
        let mut state = super::super::LearningState::default();
        let profile = test_profile();
        let variant_key = super::super::variant_key(&profile);
        let base_timestamp = super::super::now_ms().saturating_sub(1_000);
        state.variants.insert(
            variant_key.clone(),
            RegisteredProfile {
                profile: profile.clone(),
                specs: Vec::new(),
                samples: 0,
                positive: 0,
                realized_pnl: 0.0,
                outcomes: (0..MAX_OUTCOMES_PER_VARIANT + 3)
                    .map(|index| outcome(base_timestamp + index as u64))
                    .collect(),
            },
        );
        let connection = super::super::open_state_db(&path).unwrap();
        super::super::persist_normalized(&connection, &state).unwrap();
        drop(connection);

        let compacted = super::super::load_state(&path).unwrap();
        assert_eq!(
            compacted.variants[&variant_key].outcomes.len(),
            MAX_OUTCOMES_PER_VARIANT
        );
        assert_eq!(
            compacted.variants[&variant_key].profile.profile_id,
            profile.profile_id
        );
        super::super::feedback::persist(&path, &compacted).unwrap();
        let persisted_payload: String = super::super::open_state_db(&path)
            .unwrap()
            .query_row(
                "SELECT payload FROM learning_variants WHERE id = ?1",
                [&variant_key],
                |row| row.get(0),
            )
            .unwrap();
        let persisted_variant: RegisteredProfile =
            serde_json::from_str(&persisted_payload).unwrap();
        assert_eq!(persisted_variant.outcomes.len(), MAX_OUTCOMES_PER_VARIANT);
        assert_eq!(persisted_variant.profile.profile_id, profile.profile_id);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn variant_history_rows_remain_in_sqlite_after_leaving_the_memory_working_set() {
        let path = std::env::temp_dir().join(format!(
            "learning-variant-archive-test-{}-{}.sqlite",
            std::process::id(),
            super::super::now_ms()
        ));
        let connection = super::super::open_state_db(&path).unwrap();
        let profile = test_profile();
        let id = super::super::variant_key(&profile);
        let archived = RegisteredProfile {
            profile,
            specs: Vec::new(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: vec![outcome(super::super::now_ms())],
        };
        let initial = BTreeMap::from([(id.clone(), archived)]);
        super::super::persist_normalized(
            &connection,
            &super::super::LearningState {
                variants: initial,
                ..Default::default()
            },
        )
        .unwrap();
        super::super::persist_normalized(&connection, &super::super::LearningState::default())
            .unwrap();
        let rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM learning_variants WHERE id = ?1",
                [&id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(rows, 1);
        drop(connection);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn active_profile_and_stat_map_limits_fail_admission_without_eviction() {
        let mut state = super::super::LearningState::default();
        for index in 0..MAX_ACTIVE_PROFILES {
            state
                .active_profiles
                .insert(format!("symbol-{index}:strategy"), test_profile());
        }
        assert!(ensure_active_profile_room(&state, "new-symbol:strategy").is_err());
        assert_eq!(state.active_profiles.len(), MAX_ACTIVE_PROFILES);

        for index in 0..MAX_CONTEXT_STATS {
            state.context_learning.insert(
                format!("context-{index}"),
                strategy::ContextLearningStat {
                    samples: 0,
                    positive: 0,
                    realized_pnl: 0.0,
                    updated_at_ms: index as u64,
                },
            );
        }
        assert!(ensure_context_room(&state, "new-context").is_err());
        assert_eq!(state.context_learning.len(), MAX_CONTEXT_STATS);

        for index in 0..MAX_EXECUTION_STATS {
            state.execution_learning.insert(
                format!("execution-{index}"),
                strategy::ExecutionLearningStat {
                    samples: 0,
                    filled_samples: 0,
                    fill_rate_total: 0.0,
                    cost_notional: 0.0,
                    all_in_cost_notional_bps: 0.0,
                    updated_at_ms: index as u64,
                },
            );
        }
        assert!(ensure_execution_room(&state, "new-execution").is_err());
        assert_eq!(state.execution_learning.len(), MAX_EXECUTION_STATS);
    }

    fn outcome(observed_at_ms: u64) -> LearningOutcome {
        LearningOutcome {
            observed_at_ms,
            net_return_percent: 0.0,
            evidence_group_id: None,
            learning_context: None,
            source: LearningOutcomeSource::Trade,
            realized_pnl_usd: Some(0.0),
            paired_comparison: false,
            tested_parameter_keys: Vec::new(),
            baseline_parameters: Default::default(),
        }
    }
}
