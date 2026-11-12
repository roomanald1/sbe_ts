async fn receive_scout_markout(
    State(service): State<ServiceState>,
    Json(markout): Json<ScoutMarkoutRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let instance = {
        let mut state = service.scout.data.lock().await;
        ensure_scout_instance_loaded(&mut state, &service.scout.path, &markout.decision_id)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    };
    let Some(instance) = instance else {
        return Ok(StatusCode::NO_CONTENT);
    };
    let observation = scout_markout_observation(&instance, &markout);
    let observation_valid = observation.is_some();
    if let Some(observation) = observation {
        record_scout_markout_observation(&service, &markout.decision_id, observation).await?;
    }
    if markout.observation_only && !observation_valid {
        return Ok(StatusCode::NO_CONTENT);
    }
    let Some(outcome) = scout_outcome_from_markout(&instance, markout) else {
        return Ok(StatusCode::NO_CONTENT);
    };
    if outcome.exit_scenario.starts_with("horizon-") {
        return receive_scout_horizon_outcome(&service, outcome).await;
    }
    receive_scout_outcome(State(service), Json(outcome)).await
}

fn scout_markout_observation(
    instance: &serde_json::Value,
    markout: &ScoutMarkoutRequest,
) -> Option<serde_json::Value> {
    let timestamp = instance["signal_timestamp"].as_u64()?;
    if instance["symbol"].as_str() != Some(markout.symbol.as_str())
        || instance["decision_id"].as_str() != Some(markout.decision_id.as_str())
        || timestamp.abs_diff(markout.signal_timestamp_ms) > 10_000
        || markout.evaluated_timestamp_ms < markout.signal_timestamp_ms
        || !markout.taker_fee_rate.is_finite()
        || markout.taker_fee_rate < 0.0
        || (markout.observation_only
            && (!matches!(markout.horizon_seconds, 60 | 300 | 900 | 3_600)
                || markout
                    .evaluated_timestamp_ms
                    .saturating_sub(markout.signal_timestamp_ms)
                    < markout.horizon_seconds.saturating_mul(1_000)))
        || [
            markout.long_net_percent,
            markout.short_net_percent,
            markout.reference_entry_price,
            markout.reference_exit_price,
            markout.entry_bid,
            markout.entry_ask,
            markout.exit_bid,
            markout.exit_ask,
            markout.long_gross_percent,
            markout.short_gross_percent,
            markout.path_high,
            markout.path_low,
        ]
        .into_iter()
        .flatten()
        .any(|value| !value.is_finite())
    {
        return None;
    }
    let direction = instance["direction"].as_str()?;
    let direction_net_return = match direction {
        "bullish" => markout.long_net_percent,
        "bearish" => markout.short_net_percent,
        _ => return None,
    };
    direction_net_return?;
    Some(serde_json::json!({
        "kind": if markout.observation_only { "horizon" } else { "strategy_exit" },
        "horizon_seconds": markout.horizon_seconds,
        "evaluated_timestamp_ms": markout.evaluated_timestamp_ms,
        "taker_fee_rate": markout.taker_fee_rate,
        "reference_entry_price": markout.reference_entry_price,
        "reference_exit_price": markout.reference_exit_price,
        "entry_bid": markout.entry_bid,
        "entry_ask": markout.entry_ask,
        "exit_bid": markout.exit_bid,
        "exit_ask": markout.exit_ask,
        "long_gross_percent": markout.long_gross_percent,
        "short_gross_percent": markout.short_gross_percent,
        "long_net_percent": markout.long_net_percent,
        "short_net_percent": markout.short_net_percent,
        "path_high": markout.path_high,
        "path_low": markout.path_low,
        "long_time_to_positive_net_seconds": markout.long_time_to_positive_net_seconds,
        "short_time_to_positive_net_seconds": markout.short_time_to_positive_net_seconds,
    }))
}

async fn record_scout_markout_observation(
    service: &ServiceState,
    decision_id: &str,
    observation: serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    let mut state = service.scout.data.lock().await;
    ensure_scout_instance_loaded(&mut state, &service.scout.path, decision_id)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let Some(instance) = state.scout_instances.get_mut(decision_id) else {
        return Ok(());
    };
    let observations = instance
        .get("scout_markout_observations")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let duplicate = observations.iter().any(|existing| {
        existing["kind"] == observation["kind"]
            && (observation["kind"] == "strategy_exit"
                || existing["horizon_seconds"] == observation["horizon_seconds"])
    });
    if duplicate {
        return Ok(());
    }
    let mut observations = observations;
    observations.push(observation);
    instance["scout_markout_observations"] = serde_json::json!(observations);
    state.scout_lifecycle_updated_at_ms = state.scout_lifecycle_updated_at_ms.max(scout_now_ms());
    persist_scout_state(
        &service.scout.path,
        &state,
        &[&format!("scout_instances@{decision_id}")],
    )?;
    Ok(())
}

fn scout_outcome_from_markout(
    instance: &serde_json::Value,
    markout: ScoutMarkoutRequest,
) -> Option<ScoutOutcomeRequest> {
    if instance["symbol"].as_str() != Some(markout.symbol.as_str())
        || instance["decision_id"].as_str() != Some(markout.decision_id.as_str())
        || instance["signal_timestamp"].as_u64().is_none_or(|timestamp| {
            timestamp.abs_diff(markout.signal_timestamp_ms) > 10_000
        })
        || markout.evaluated_timestamp_ms < markout.signal_timestamp_ms
        || (!markout.observation_only && instance["scout_net_return_percent"].is_number())
        || !markout.taker_fee_rate.is_finite()
        || markout.taker_fee_rate < 0.0
    {
        return None;
    }
    let direction = instance["direction"].as_str().unwrap_or_default();
    let markout_net_return_percent = match direction {
        "bullish" => markout.long_net_percent,
        "bearish" => markout.short_net_percent,
        _ => None,
    };
    let markout_net_return_percent =
        markout_net_return_percent.filter(|value| value.is_finite())?;
    let early_exit_return = (!markout.observation_only
        && instance["scout_exit_scenario"].as_str() == Some("strategy"))
        .then(|| instance["scout_thesis_failure_exit_net_return_percent"].as_f64())
        .flatten()
        .filter(|value| value.is_finite());
    let net_return_percent = early_exit_return.unwrap_or(markout_net_return_percent);
    let notional = instance["scout_notional_usd"].as_f64().unwrap_or_default();
    let learning_context = instance["scout_learning_context"].as_str()?.to_owned();
    if !notional.is_finite() || notional <= 0.0 {
        return None;
    }
    Some(ScoutOutcomeRequest {
        decision_id: markout.decision_id,
        learning_context,
        simulated_net_pnl_usd: notional * net_return_percent / 100.0,
        net_return_percent,
        signal_timestamp_ms: markout.signal_timestamp_ms,
        evaluated_timestamp_ms: markout.evaluated_timestamp_ms,
        taker_fee_rate: markout.taker_fee_rate,
        symbol: markout.symbol,
        strategy: instance["strategy"].as_str().unwrap_or_default().to_owned(),
        strategy_parameters: serde_json::from_value(instance["strategy_parameters"].clone()).unwrap_or_default(),
        exit_scenario: if markout.observation_only {
            format!("horizon-{}", markout.horizon_seconds)
        } else {
            instance["scout_exit_scenario"]
                .as_str()
                .unwrap_or("strategy")
                .to_owned()
        },
        sample_group_id: if markout.observation_only {
            String::new()
        } else {
            instance["scout_sample_group_id"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        },
        scout_parameter_key: instance["scout_parameter_key"].as_str().map(str::to_owned),
        scout_parameter_value: instance["scout_parameter_value"].as_f64(),
        scout_parameter_profile_symbol: instance["scout_parameter_profile_symbol"].as_str().map(str::to_owned),
        scout_comparison_id: instance["scout_comparison_id"].as_str().map(str::to_owned),
        scout_comparison_ids: instance["scout_comparison_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect(),
        scout_comparison_zero_sides: serde_json::from_value(
            instance["scout_comparison_zero_sides"].clone(),
        )
        .unwrap_or_default(),
        scout_comparison_role: instance["scout_comparison_role"].as_str().map(str::to_owned),
        scout_comparison_zero_role: instance["scout_comparison_zero_role"].as_str().map(str::to_owned),
        scout_comparison_parameter_key: instance["scout_comparison_parameter_key"].as_str().map(str::to_owned),
        scout_comparison_parameter_value: instance["scout_comparison_parameter_value"].as_f64(),
        scout_comparison_parameter_profile_symbol: instance["scout_comparison_parameter_profile_symbol"].as_str().map(str::to_owned),
        strategy_parameter_specs: serde_json::from_value(instance["strategy_parameter_specs"].clone()).unwrap_or_default(),
    })
}

async fn receive_scout_horizon_outcome(
    service: &ServiceState,
    outcome: ScoutOutcomeRequest,
) -> Result<StatusCode, (StatusCode, String)> {
    let tested = validate_scout_outcome(&outcome)?;
    let outcome_key = scout_outcome_idempotency_key(&outcome);
    let mut state = service.scout.data.lock().await;
    if state.seen.contains(&outcome_key)
        || scout_record_exists(&service.scout.path, "seen", &outcome_key)?
    {
        return Ok(StatusCode::NO_CONTENT);
    }

    let mut changed_sections = BTreeSet::from(["seen".to_owned()]);
    if outcome.scout_comparison_role.is_some() {
        let (_, paired_ids) = record_paired_scout_outcome(&mut state, &outcome);
        changed_sections.insert("outbox".to_owned());
        changed_sections.extend(paired_ids.into_iter().map(|id| format!("pairs@{id}")));
    } else if outcome.scout_parameter_key.is_none() {
        let event = outcome_event(&outcome, &tested);
        if !outbox_has_learning(&state, &event) {
            state.outbox.push(ScoutOutboxItem::Learning(event));
            changed_sections.insert("outbox".to_owned());
        }
    }
    state.seen.insert(outcome_key);
    while state.seen.len() > 100_000 {
        if let Some(oldest) = state.seen.iter().next().cloned() {
            state.seen.remove(&oldest);
        }
    }
    let changed_sections = changed_sections.iter().map(String::as_str).collect::<Vec<_>>();
    persist_scout_state(&service.scout.path, &state, &changed_sections)?;
    drop(state);
    schedule_scout_outbox_flush(service);
    Ok(StatusCode::NO_CONTENT)
}

async fn receive_scout_outcome(
    State(service): State<ServiceState>,
    Json(outcome): Json<ScoutOutcomeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let tested = validate_scout_outcome(&outcome)?;
    {
        let mut state = service.scout.data.lock().await;
        let outcome_key = scout_outcome_idempotency_key(&outcome);
        if !state.seen.contains(&outcome_key)
            && !scout_record_exists(&service.scout.path, "seen", &outcome_key)?
        {
            let mut changed_records = BTreeSet::from([
                format!("outcomes@{outcome_key}"),
                format!("scout_instances@{}", outcome.decision_id),
                format!("seen@{outcome_key}"),
            ]);
            let event = outcome_event(&outcome, &tested);
            let is_probe = outcome.scout_parameter_key.is_some();
            if !is_probe
                && outcome.scout_comparison_role.as_deref() != Some("baseline")
                && !outbox_has_learning(&state, &event)
            {
                state.outbox.push(ScoutOutboxItem::Learning(event));
            }
            let (paired_updates, paired_ids) = record_paired_scout_outcome(&mut state, &outcome);
            for comparison_id in paired_ids {
                changed_records.insert(format!("pairs@{comparison_id}"));
            }
            insert_scout_outcome(&mut state, outcome_key.clone(), outcome.clone());
            ensure_scout_instance_loaded(
                &mut state,
                &service.scout.path,
                &outcome.decision_id,
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            if let Some(instance) = state.scout_instances.get_mut(&outcome.decision_id) {
                instance["scout_net_return_percent"] = serde_json::json!(outcome.net_return_percent);
                instance["scout_evaluated_timestamp_ms"] = serde_json::json!(outcome.evaluated_timestamp_ms);
                instance["scout_taker_fee_rate"] = serde_json::json!(outcome.taker_fee_rate);
            }
            for (variant_id, delta) in paired_updates {
                changed_records.insert(format!("scout_instances@{variant_id}"));
                ensure_scout_instance_loaded(&mut state, &service.scout.path, &variant_id)
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
                if let Some(instance) = state.scout_instances.get_mut(&variant_id) {
                    instance["paired_delta_percent"] = serde_json::json!(delta);
                }
            }
            trim_scout_outcome_cache(&mut state);
            if !outcome.sample_group_id.is_empty() {
                changed_records.insert(format!("scout_sample_active_scenarios@{}", outcome.sample_group_id));
                changed_records.insert(format!("scout_plan_profiles@{}", outcome.sample_group_id));
                changed_records.insert(format!("scout_active_audits@{}", outcome.sample_group_id));
                let finished = state
                    .scout_sample_active_scenarios
                    .get_mut(&outcome.sample_group_id)
                    .is_some_and(|scenarios| {
                        let token = format!("{}|{}", outcome.decision_id, outcome.exit_scenario);
                        if !scenarios.remove(&token) {
                            scenarios.remove(&outcome.exit_scenario);
                        }
                        scenarios.is_empty()
                    });
                if finished {
                    state
                        .scout_sample_active_scenarios
                        .remove(&outcome.sample_group_id);
                    state.scout_plan_profiles.remove(&outcome.sample_group_id);
                    state.scout_active_audits.remove(&outcome.sample_group_id);
                }
            }
            state.scout_lifecycle_updated_at_ms =
                state.scout_lifecycle_updated_at_ms.max(scout_now_ms());
            state.seen.insert(outcome_key.clone());
            while state.seen.len() > 100_000 {
                if let Some(oldest) = state.seen.iter().next().cloned() {
                    state.seen.remove(&oldest);
                    changed_records.insert(format!("seen@{oldest}"));
                }
            }
            let mut changed_sections = vec!["outbox".to_owned()];
            changed_sections.extend(changed_records);
            let changed_sections = changed_sections.iter().map(String::as_str).collect::<Vec<_>>();
            persist_scout_state(&service.scout.path, &state, &changed_sections)?;
        }
    }
    schedule_scout_outbox_flush(&service);
    Ok(StatusCode::NO_CONTENT)
}

fn outcome_comparison_ids(outcome: &ScoutOutcomeRequest) -> Vec<String> {
    if outcome.scout_comparison_role.as_deref() == Some("baseline")
        && !outcome.scout_comparison_ids.is_empty()
    {
        outcome.scout_comparison_ids.clone()
    } else {
        outcome.scout_comparison_id.iter().cloned().collect()
    }
}

async fn get_scout_lifecycle_state(
    State(service): State<ServiceState>,
) -> Json<ScoutLifecycleSnapshot> {
    let state = service.scout.data.lock().await;
    Json(ScoutLifecycleSnapshot {
        updated_at_ms: state.scout_lifecycle_updated_at_ms,
        scout_sample_at_ms: state.scout_sample_at_ms.clone(),
        scout_sample_active_scenarios: state.scout_sample_active_scenarios.clone(),
        scout_variant_rotation_cursor: state.scout_variant_rotation_cursor,
    })
}

async fn set_scout_lifecycle_state(
    State(service): State<ServiceState>,
    Json(snapshot): Json<ScoutLifecycleSnapshot>,
) -> Result<StatusCode, (StatusCode, String)> {
    if snapshot.scout_sample_at_ms.len() > 100_000
        || snapshot.scout_sample_active_scenarios.len() > 10_000
        || snapshot.scout_variant_rotation_cursor > 1_000_000_000
        || snapshot
            .scout_sample_at_ms
            .keys()
            .any(|key| key.is_empty() || key.len() > 512)
        || snapshot
            .scout_sample_active_scenarios
            .iter()
            .any(|(group, scenarios)| {
                group.is_empty()
                    || group.len() > 512
                    || scenarios.len() > 10_000
                    || scenarios
                        .iter()
                        .any(|scenario| scenario.is_empty() || scenario.len() > 512)
            })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid Scout lifecycle state".into(),
        ));
    }
    let mut state = service.scout.data.lock().await;
    if snapshot.updated_at_ms < state.scout_lifecycle_updated_at_ms {
        return Ok(StatusCode::NO_CONTENT);
    }
    state.scout_sample_at_ms = snapshot.scout_sample_at_ms;
    state.scout_sample_active_scenarios = snapshot.scout_sample_active_scenarios;
    state.scout_variant_rotation_cursor = snapshot.scout_variant_rotation_cursor;
    state.scout_lifecycle_updated_at_ms = snapshot.updated_at_ms;
    persist_scout_state(&service.scout.path, &state, &["scout_sample_at_ms", "scout_sample_active_scenarios"])?;
    Ok(StatusCode::NO_CONTENT)
}

fn dated_carry_markout(
    entry: &strategies::DatedCarryEntryCandidate,
    exit: &strategies::DatedCarryEntryCandidate,
) -> Option<f64> {
    const SLIPPAGE: f64 = 0.0005;
    let quantity = entry.quantity;
    let entry_spot = entry.spot_ask * (1.0 + SLIPPAGE);
    let entry_future = entry.future_bid * (1.0 - SLIPPAGE);
    let exit_spot = exit.spot_bid * (1.0 - SLIPPAGE);
    let exit_future = exit.future_ask * (1.0 + SLIPPAGE);
    let entry_fees = quantity * (0.001 * entry_spot + 0.0005 * entry_future);
    let exit_fees = quantity * (0.001 * exit_spot + 0.0005 * exit_future);
    let net =
        quantity * (exit_spot - entry_spot + entry_future - exit_future) - entry_fees - exit_fees;
    let notional = quantity * entry_spot;
    (quantity.is_finite()
        && quantity > 0.0
        && [entry_spot, entry_future, exit_spot, exit_future, notional]
            .iter()
            .all(|v| v.is_finite() && *v > 0.0)
        && net.is_finite())
    .then_some(net / notional * 100.0)
}

#[cfg(test)]
fn apply_scout_lifecycle_observation(
    state: &mut PersistedState,
    request: &ScoutLifecycleObserveRequest,
) -> ScoutLifecycleObserveResponse {
    apply_scout_lifecycle_observation_with_changes(state, request).0
}

fn apply_scout_lifecycle_observation_with_changes(
    state: &mut PersistedState,
    request: &ScoutLifecycleObserveRequest,
) -> (ScoutLifecycleObserveResponse, BTreeSet<String>) {
    if let Some(response) = state.scout_observation_results.get(&request.request_id) {
        return (response.clone(), BTreeSet::new());
    }
    let mut changed_records = BTreeSet::new();
    let mut finalized_count = 0usize;
    let ids = state
        .scout_pending_samples
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for id in ids {
        let Some(ScoutSamplePayload::DatedCarryProbe(probe)) =
            state.scout_pending_samples.get(&id).cloned()
        else {
            continue;
        };
        let age = request.now_ms.saturating_sub(probe.opened_at_ms);
        if age > 3 * 86_400_000 {
            if state.scout_pending_samples.remove(&id).is_some() {
                changed_records.insert(format!("scout_pending_samples@{id}"));
            }
            if state.scout_sample_active_scenarios.remove(&id).is_some() {
                changed_records.insert(format!("scout_sample_active_scenarios@{id}"));
            }
            if state.scout_plan_profiles.remove(&id).is_some() {
                changed_records.insert(format!("scout_plan_profiles@{id}"));
            }
            continue;
        }
        if age < 86_400_000 {
            continue;
        }
        let net_return_percent = if probe.would_enter {
            let Some(exit) = request.candidates.iter().find(|candidate| {
                candidate.spot_symbol == probe.candidate.spot_symbol
                    && candidate.future_symbol == probe.candidate.future_symbol
            }) else {
                continue;
            };
            let Some(markout) = dated_carry_markout(&probe.candidate, exit) else {
                continue;
            };
            markout
        } else {
            0.0
        };
        let specs = probe
            .parameter_specs
            .iter()
            .filter(|spec| probe.tested_parameters.contains_key(&spec.key))
            .cloned()
            .collect::<Vec<_>>();
        let paired = probe.comparison_role.is_some();
        if !paired {
            let event = FeedbackEvent::ScoutOutcome {
                event_id: format!(
                    "dated-carry-scout:{}:{}:{}",
                    probe.candidate.future_symbol, probe.opened_at_ms, probe.parameter_key
                ),
                observed_at_ms: request.now_ms,
                learning_context: None,
                symbol: probe.profile_symbol.clone(),
                strategy: probe.strategy.clone(),
                profile_id: strategies::profile_id(
                    &probe.profile_symbol,
                    &probe.strategy,
                    &probe.parameters,
                ),
                parameters: probe.parameters.clone(),
                baseline_parameters: BTreeMap::new(),
                comparison: strategies::ScoutComparisonMetadata::Independent,
                specs: specs.clone(),
                net_return_percent,
            };
            if !outbox_has_learning(state, &event) {
                state.outbox.push(ScoutOutboxItem::Learning(event));
            }
        }
        if probe.would_enter || paired {
            let mut tested_profile = probe.parameters.clone();
            tested_profile.extend(probe.tested_parameters.clone());
            let notional = probe.candidate.quantity * probe.candidate.spot_ask;
            let outcome = ScoutOutcomeRequest {
                decision_id: probe.sample_id.clone(),
                learning_context: format!("scout:dated-carry:{}", probe.profile_symbol),
                simulated_net_pnl_usd: notional * net_return_percent / 100.0,
                net_return_percent,
                signal_timestamp_ms: probe.opened_at_ms,
                evaluated_timestamp_ms: request.now_ms,
                taker_fee_rate: 0.0,
                symbol: probe.profile_symbol.clone(),
                strategy: strategies::Strategy::DatedCarry.as_str().to_owned(),
                strategy_parameters: tested_profile,
                exit_scenario: "strategy".into(),
                sample_group_id: probe.sample_id.clone(),
                scout_parameter_key: (probe.comparison_role.as_deref() == Some("variant"))
                    .then(|| probe.parameter_key.clone()),
                scout_parameter_value: (probe.comparison_role.as_deref() == Some("variant"))
                    .then_some(probe.parameter_value),
                scout_parameter_profile_symbol: (probe.comparison_role.as_deref() == Some("variant"))
                    .then(|| probe.profile_symbol.clone()),
                scout_comparison_id: probe.comparison_id.clone(),
                scout_comparison_ids: probe.comparison_ids.clone(),
                scout_comparison_zero_sides: Vec::new(),
                scout_comparison_role: probe.comparison_role.clone(),
                scout_comparison_zero_role: None,
                scout_comparison_parameter_key: (probe.comparison_role.as_deref() == Some("variant"))
                    .then(|| probe.parameter_key.clone()),
                scout_comparison_parameter_value: (probe.comparison_role.as_deref() == Some("variant"))
                    .then_some(probe.parameter_value),
                scout_comparison_parameter_profile_symbol: (probe.comparison_role.as_deref() == Some("variant"))
                    .then(|| probe.profile_symbol.clone()),
                strategy_parameter_specs: probe.parameter_specs.clone(),
            };
            let key = scout_outcome_idempotency_key(&outcome);
            insert_scout_outcome(state, key.clone(), outcome.clone());
            changed_records.insert(format!("outcomes@{key}"));
            if paired {
                let (paired_updates, paired_ids) = record_paired_scout_outcome(state, &outcome);
                changed_records.extend(
                    paired_ids.into_iter().map(|id| format!("pairs@{id}")),
                );
                changed_records.extend(
                    paired_updates
                        .into_iter()
                        .map(|(id, _)| format!("scout_instances@{id}")),
                );
            }
            trim_scout_outcome_cache(state);
        }
        if state.scout_pending_samples.remove(&id).is_some() {
            changed_records.insert(format!("scout_pending_samples@{id}"));
        }
        if state.scout_sample_active_scenarios.remove(&id).is_some() {
            changed_records.insert(format!("scout_sample_active_scenarios@{id}"));
        }
        if state.scout_plan_profiles.remove(&id).is_some() {
            changed_records.insert(format!("scout_plan_profiles@{id}"));
        }
        finalized_count += 1;
    }
    let response = ScoutLifecycleObserveResponse { finalized_count };
    state
        .scout_observation_results
        .insert(request.request_id.clone(), response.clone());
    changed_records.insert(format!(
        "scout_observation_results@{}",
        request.request_id
    ));
    while state.scout_observation_results.len() > 10_000 {
        if let Some(oldest) = state.scout_observation_results.keys().next().cloned() {
            state.scout_observation_results.remove(&oldest);
            changed_records.insert(format!("scout_observation_results@{oldest}"));
        }
    }
    trim_scout_instance_cache(state);
    state.scout_lifecycle_updated_at_ms = state.scout_lifecycle_updated_at_ms.max(request.now_ms);
    (response, changed_records)
}

// Active audits are duplicated in SQLite-backed `scout_instances` for report
// and markout use. Keep the replay-state copy bounded independently.
const MAX_ACTIVE_SCOUT_AUDIT_GROUPS: usize = 128;
const MAX_ACTIVE_SCOUT_AUDITS: usize = 4_096;
fn trim_active_scout_audits(state: &mut PersistedState) -> Vec<String> {
    let mut active_audit_count = state
        .scout_active_audits
        .values()
        .map(Vec::len)
        .sum::<usize>();
    let mut evicted = Vec::new();
    while state.scout_active_audits.len() > MAX_ACTIVE_SCOUT_AUDIT_GROUPS
        || active_audit_count > MAX_ACTIVE_SCOUT_AUDITS
    {
        let oldest = state
            .scout_active_audits
            .iter()
            .min_by_key(|(_, audits)| {
                audits
                    .iter()
                    .filter_map(|audit| audit["timestamp"].as_u64())
                    .min()
                    .unwrap_or_default()
            })
            .map(|(group, _)| group.clone());
        let Some(oldest) = oldest else { break };
        if let Some(audits) = state.scout_active_audits.remove(&oldest) {
            active_audit_count = active_audit_count.saturating_sub(audits.len());
        }
        evicted.push(oldest);
    }
    evicted
}

async fn observe_scout_lifecycle(
    State(service): State<ServiceState>,
    Json(request): Json<ScoutLifecycleObserveRequest>,
) -> Result<Json<ScoutLifecycleObserveResponse>, (StatusCode, String)> {
    if request.request_id.is_empty()
        || request.request_id.len() > 240
        || request.candidates.len() > 10_000
        || request.now_ms > scout_now_ms().saturating_add(5_000)
        || request.candidates.iter().any(|candidate| {
            candidate.spot_symbol.is_empty()
                || candidate.future_symbol.is_empty()
                || ![
                    candidate.quantity,
                    candidate.spot_ask,
                    candidate.future_bid,
                    candidate.spot_bid,
                    candidate.future_ask,
                ]
                .iter()
                .all(|v| v.is_finite())
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid Scout lifecycle observation".into(),
        ));
    }
    let mut state = service.scout.data.lock().await;
    if let Some(response) = state.scout_observation_results.get(&request.request_id) {
        let response = response.clone();
        drop(state);
        schedule_scout_outbox_flush(&service);
        return Ok(Json(response));
    }
    let (response, changed_records) =
        apply_scout_lifecycle_observation_with_changes(&mut state, &request);
    if !changed_records.is_empty() {
        let mut changed_sections = vec!["outbox".to_owned()];
        changed_sections.extend(changed_records);
        let changed_sections = changed_sections.iter().map(String::as_str).collect::<Vec<_>>();
        persist_scout_state(&service.scout.path, &state, &changed_sections)?;
    }
    drop(state);
    schedule_scout_outbox_flush(&service);
    Ok(Json(response))
}

fn valid_dated_carry_probe(
    probe: &DatedCarryProbe,
    sample_group_id: &str,
    now_ms: u64,
) -> bool {
    if probe.sample_id != sample_group_id
        || probe.opened_at_ms > now_ms.saturating_add(5_000)
        || probe.strategy.is_empty()
        || probe.profile_symbol.is_empty()
        || probe
            .parameters
            .values()
            .chain(probe.tested_parameters.values())
            .any(|value| !value.is_finite())
        || !probe.parameter_value.is_finite()
        || probe.tested_parameters.iter().any(|(key, value)| {
            let Some(spec) = probe.parameter_specs.iter().find(|spec| spec.key == *key) else {
                return true;
            };
            let position = (*value - spec.min) / spec.step;
            *value < spec.min
                || *value > spec.max
                || spec.step <= 0.0
                || (position - position.round()).abs() > 1e-6
        })
        || !strategies::dated_carry_entry_is_eligible(
            &probe.candidate,
            probe.opened_at_ms,
            probe.opened_at_ms,
            1.0,
            0.0,
        )
    {
        return false;
    }
    match probe.comparison_role.as_deref() {
        Some("baseline") => {
            probe.parameter_key.is_empty()
                && probe.parameter_value == 0.0
                && probe.tested_parameters.is_empty()
                && probe.comparison_id.is_none()
                && !probe.comparison_ids.is_empty()
                && probe.comparison_ids.len() <= MAX_PARAMETER_VARIANTS_PER_BATCH
                && probe.comparison_ids.iter().all(|id| !id.is_empty() && id.len() <= 240)
                && probe.comparison_ids.iter().collect::<BTreeSet<_>>().len()
                    == probe.comparison_ids.len()
        }
        Some("variant") => {
            probe.parameter_key.starts_with("combo:")
                .then(|| {
                    probe.parameter_value == 0.0
                        && parse_combination(&probe.parameter_key, &probe.parameter_specs)
                            .as_ref()
                            == Some(&probe.tested_parameters)
                })
                .unwrap_or_else(|| {
                    probe.tested_parameters.len() == 1
                        && probe.tested_parameters.get(&probe.parameter_key)
                            == Some(&probe.parameter_value)
                })
                && probe.comparison_id.as_ref().is_some_and(|id| !id.is_empty() && id.len() <= 240)
                && probe.comparison_ids.is_empty()
        }
        None => {
            probe.comparison_id.is_none()
                && probe.comparison_ids.is_empty()
                && if probe.parameter_key.starts_with("combo:") {
                    probe.parameter_value == 0.0
                        && parse_combination(&probe.parameter_key, &probe.parameter_specs)
                            .as_ref()
                            == Some(&probe.tested_parameters)
                } else {
                    probe.tested_parameters.len() == 1
                        && probe.tested_parameters.get(&probe.parameter_key)
                            == Some(&probe.parameter_value)
                }
        }
        Some(_) => false,
    }
}

async fn create_scout_plan(
    State(service): State<ServiceState>,
    Json(request): Json<ScoutPlanRequest>,
) -> Result<Json<ScoutPlanResponse>, (StatusCode, String)> {
    if request.request_id.is_empty()
        || request.request_id.len() > 240
        || request.candidates.len() > 512
        || request.legacy_cooldowns.len() > 100_000
        || request
            .legacy_cooldowns
            .keys()
            .any(|key| key.is_empty() || key.len() > 512)
        || request.legacy_comparisons.len() > 20_000
        || request.legacy_comparisons.iter().any(|comparison| {
            comparison.comparison_id.is_empty()
                || comparison.comparison_id.len() > 240
                || comparison
                    .baseline_return_percent
                    .is_some_and(|value| !value.is_finite())
                || comparison
                    .variant_return_percent
                    .is_some_and(|value| !value.is_finite())
                || comparison.learning_key.len() > 512
        })
        || request.max_active_batches > 5_000
        || request.max_active_variants_per_profile > 1_000
        || request.now_ms > scout_now_ms().saturating_add(5_000)
        || request.candidates.iter().any(|candidate| {
            (candidate.sample_group_id.is_empty() && !candidate.generate_group_id)
                || candidate.sample_group_id.len() > 512
                || (candidate.cooldown_key.is_empty() && !candidate.generate_group_id)
                || candidate.cooldown_key.len() > 512
                || candidate.profile_key.len() > 512
                || candidate.cooldown_ms > 30 * 86_400_000
                || candidate
                    .audit_plans
                    .len()
                    .saturating_add(candidate.audit_templates.len())
                    > 64
                || (candidate.audit_plans.is_empty()
                    && candidate.audit_templates.is_empty()
                    && candidate.sample_payload.is_none())
                || candidate.audit_templates.iter().any(|template| {
                    !template.notional_usd.is_finite()
                        || template.notional_usd < 0.0
                        || !template.stop_loss_percent.is_finite()
                        || template.stop_loss_percent <= 0.0
                        || !template.take_profit_percent.is_finite()
                        || template.take_profit_percent <= 0.0
                        || !template.level_percent.is_finite()
                        || template.learning_context.len() > 512
                })
                || candidate.daily_policy.as_ref().is_some_and(|policy| {
                    !matches!(
                        policy.evaluator_decision.as_str(),
                        "enter" | "scout" | "reject"
                    ) || policy.parameter_key.is_some() != policy.parameter_value.is_some()
                        || policy
                            .parameter_value
                            .is_some_and(|value| !value.is_finite())
                        || candidate.audit_templates.is_empty()
                })
                || candidate.live_policy.as_ref().is_some_and(|policy| {
                    policy.now_ms > request.now_ms.saturating_add(5_000)
                        || policy.best_bid.is_some_and(|value| !value.is_finite())
                        || policy.best_ask.is_some_and(|value| !value.is_finite())
                })
                || candidate.audit_plans.iter().any(|audit| {
                    audit
                        .get("decision_id")
                        .and_then(serde_json::Value::as_str)
                        .is_none_or(|id| id.is_empty() || id.len() > 240)
                        || audit
                            .get("scout_exit_scenario")
                            .and_then(serde_json::Value::as_str)
                            .is_none_or(|scenario| {
                                !matches!(scenario, "strategy" | "tight" | "wide")
                            })
                })
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid Scout scout-plan request".into(),
        ));
    }
    let mut state = service.scout.data.lock().await;
    for (key, timestamp) in &request.legacy_cooldowns {
        state
            .scout_sample_at_ms
            .entry(key.clone())
            .and_modify(|current| *current = (*current).max(*timestamp))
            .or_insert(*timestamp);
    }
    for comparison in &request.legacy_comparisons {
        let pair = state
            .pairs
            .entry(comparison.comparison_id.clone())
            .or_default();
        pair.updated_at_ms = pair.updated_at_ms.max(comparison.updated_at_ms);
        if comparison.baseline_return_percent.is_some() {
            pair.legacy_baseline_return_percent = comparison.baseline_return_percent;
        }
        if comparison.variant_return_percent.is_some() {
            pair.legacy_variant_return_percent = comparison.variant_return_percent;
        }
        if !comparison.learning_key.is_empty() {
            pair.legacy_learning_key = comparison.learning_key.clone();
        }
    }
    let cached_plan_result = state.scout_plan_results.get(&request.request_id).cloned();
    let persisted_plan_result = if cached_plan_result.is_none() {
        load_scout_record::<ScoutPlanResponse>(
            &service.scout.path,
            "scout_plan_results",
            &request.request_id,
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    } else {
        None
    };
    if let Some(response) = cached_plan_result.or(persisted_plan_result) {
        state
            .scout_plan_results
            .insert(request.request_id.clone(), response.clone());
        while state.scout_plan_results.len() > MAX_CACHED_SCOUT_PLAN_RESULTS {
            if let Some(oldest) = state.scout_plan_results.keys().next().cloned() {
                state.scout_plan_results.remove(&oldest);
            }
        }
        if !request.legacy_cooldowns.is_empty() || !request.legacy_comparisons.is_empty() {
            persist_scout_state(&service.scout.path, &state, &["scout_sample_at_ms", "pairs"])?;
        }
        drop(state);
        schedule_scout_outbox_flush(&service);
        return Ok(Json(response));
    }
    let mut active_batch_keys = state
        .scout_sample_active_scenarios
        .keys()
        .map(|key| key.split("|probe:").next().unwrap_or(key))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let cooldowns_at_request_start = state.scout_sample_at_ms.clone();
    let start = if request.candidates.is_empty() {
        0
    } else {
        state.scout_variant_rotation_cursor % request.candidates.len()
    };
    if !request.candidates.is_empty() {
        state.scout_variant_rotation_cursor = (start + 1) % request.candidates.len();
    }
    let expanded_candidates = expand_daily_scout_candidates(&request.candidates);
    let mut response = ScoutPlanResponse::default();
    let mut changed_instance_ids = BTreeSet::new();
    let mut changed_group_ids = BTreeSet::new();
    let mut changed_cooldown_keys = BTreeSet::new();
    let mut changed_pending_sample_ids = BTreeSet::new();
    for offset in 0..expanded_candidates.len() {
        let mut candidate =
            expanded_candidates[(start + offset) % expanded_candidates.len()].clone();
        if candidate.generate_group_id {
            let Some((group_id, cooldown_key)) = generated_scout_group_and_cooldown(&candidate)
            else {
                continue;
            };
            candidate.sample_group_id = group_id;
            candidate.cooldown_key = cooldown_key;
        }
        if !candidate.eligible
            || (candidate.audit_plans.is_empty()
                && candidate.audit_templates.is_empty()
                && candidate.sample_payload.is_none())
            || state
                .scout_sample_active_scenarios
                .contains_key(&candidate.sample_group_id)
            || cooldowns_at_request_start
                .get(&candidate.cooldown_key)
                .is_some_and(|previous| {
                    request.now_ms.saturating_sub(*previous) < candidate.cooldown_ms
                })
        {
            continue;
        }
        if candidate
            .sample_payload
            .as_ref()
            .is_some_and(|payload| match payload {
                ScoutSamplePayload::DatedCarryProbe(probe) => {
                    !valid_dated_carry_probe(probe, &candidate.sample_group_id, request.now_ms)
                }
            })
        {
            continue;
        }
        if candidate
            .sample_payload
            .as_ref()
            .is_some_and(|payload| match payload {
                ScoutSamplePayload::DatedCarryProbe(probe) => {
                    state.scout_pending_samples.contains_key(&probe.sample_id)
                }
            })
        {
            continue;
        }
        if !candidate.parameter_variant
            && !candidate.cooldown_key.starts_with("daily|")
            && !scout_comparison_changes_exit(&candidate)
        {
            continue;
        }
        if candidate.parameter_variant && !candidate.profile_key.is_empty() {
            let active_variants = state
                .scout_plan_profiles
                .values()
                .filter(|profile| *profile == &candidate.profile_key)
                .count();
            if active_variants >= request.max_active_variants_per_profile {
                continue;
            }
        }
        let batch_key = candidate
            .sample_group_id
            .split("|probe:")
            .next()
            .unwrap_or(&candidate.sample_group_id)
            .to_owned();
        if !active_batch_keys.contains(&batch_key)
            && active_batch_keys.len() >= request.max_active_batches
        {
            break;
        }
        let mut tokens = BTreeSet::new();
        let mut active_audits = Vec::new();
        let generated = candidate
            .audit_templates
            .iter()
            .flat_map(|template| build_scout_audits(template, &candidate.sample_group_id))
            .collect::<Vec<_>>();
        let plans = candidate
            .audit_plans
            .iter()
            .cloned()
            .chain(generated)
            .collect::<Vec<_>>();
        for source in &plans {
            let mut audit = source.clone();
            audit["scout_sample_group_id"] = serde_json::json!(candidate.sample_group_id);
            audit["scout"] = serde_json::json!(true);
            let decision_id = audit["decision_id"].as_str().unwrap_or_default();
            let scenario = audit["scout_exit_scenario"].as_str().unwrap_or_default();
            if !decision_id.is_empty() {
                insert_scout_instance(&mut state, decision_id, audit.clone());
                changed_instance_ids.insert(decision_id.to_owned());
            }
            if let Some(registration) = scout_registration(&audit) {
                tokens.insert(format!("{decision_id}|{scenario}"));
                active_audits.push(compact_scout_instance(&audit));
                state
                    .outbox
                    .push(ScoutOutboxItem::Registration(registration));
            }
            response.audits.push(audit);
        }
        if let Some(ScoutSamplePayload::DatedCarryProbe(probe)) = &candidate.sample_payload {
            state.scout_pending_samples.insert(
                probe.sample_id.clone(),
                ScoutSamplePayload::DatedCarryProbe(probe.clone()),
            );
            changed_pending_sample_ids.insert(probe.sample_id.clone());
            tokens.insert(format!("{}|carry", probe.sample_id));
        }
        state
            .scout_sample_at_ms
            .insert(candidate.cooldown_key.clone(), request.now_ms);
        if !tokens.is_empty() && candidate.parameter_variant && !candidate.profile_key.is_empty() {
            state.scout_plan_profiles.insert(
                candidate.sample_group_id.clone(),
                candidate.profile_key.clone(),
            );
        }
        if !active_audits.is_empty() {
            state
                .scout_active_audits
                .insert(candidate.sample_group_id.clone(), active_audits);
        }
        if !tokens.is_empty() {
            state
                .scout_sample_active_scenarios
                .insert(candidate.sample_group_id.clone(), tokens);
        }
        response
            .selected_groups
            .push(candidate.sample_group_id.clone());
        changed_group_ids.insert(candidate.sample_group_id.clone());
        changed_cooldown_keys.insert(candidate.cooldown_key.clone());
        active_batch_keys.insert(batch_key);
    }
    trim_scout_instance_cache(&mut state);
    state.scout_lifecycle_updated_at_ms = state.scout_lifecycle_updated_at_ms.max(request.now_ms);
    let mut cached_response = response.clone();
    cached_response.audits = cached_response
        .audits
        .iter()
        .map(compact_scout_instance)
        .collect();
    state
        .scout_plan_results
        .insert(request.request_id.clone(), cached_response);
    while state.scout_plan_results.len() > MAX_CACHED_SCOUT_PLAN_RESULTS {
        if let Some(oldest) = state.scout_plan_results.keys().next().cloned() {
            state.scout_plan_results.remove(&oldest);
        }
    }
    let _evicted_audit_cache_groups = trim_active_scout_audits(&mut state);
    let mut changed_sections = vec!["outbox".to_owned()];
    changed_sections.extend(changed_instance_ids.into_iter().map(|key| format!("scout_instances@{key}")));
    changed_sections.extend(changed_group_ids.iter().flat_map(|key| [
        format!("scout_sample_active_scenarios@{key}"),
        format!("scout_plan_profiles@{key}"),
        format!("scout_active_audits@{key}"),
    ]));
    changed_sections.extend(changed_cooldown_keys.into_iter().map(|key| format!("scout_sample_at_ms@{key}")));
    changed_sections.extend(changed_pending_sample_ids.into_iter().map(|key| format!("scout_pending_samples@{key}")));
    changed_sections.push(format!("scout_plan_results@{}", request.request_id));
    let changed_sections = changed_sections.iter().map(String::as_str).collect::<Vec<_>>();
    persist_scout_state(&service.scout.path, &state, &changed_sections)?;
    drop(state);
    schedule_scout_outbox_flush(&service);
    Ok(Json(response))
}

async fn forward_outcome(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<StatusCode, (StatusCode, String)> {
    forward_event(&service, &event).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn forward_event(
    service: &ServiceState,
    event: &FeedbackEvent,
) -> Result<(), (StatusCode, String)> {
    let acknowledgement = strategies::request_scout_feedback(
        &service.learning_rsocket_url,
        event,
    )
    .await
    .map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Learning rejected Scout feedback over RSocket: {error}"),
        )
    })?;
    if acknowledgement.status >= 400 {
        return Err((
            StatusCode::BAD_GATEWAY,
            acknowledgement
                .detail
                .unwrap_or_else(|| "Learning rejected Scout feedback".into()),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct FinalizeRequest {
    sample: ScoutSample,
    closed_at_ms: u64,
    exit_price: f64,
    cost_percent: f64,
}

impl ScoutSample {
    /// Finalize a scout sample using the observed exit price and round-trip costs.
    pub fn finalize(
        self,
        closed_at_ms: u64,
        exit_price: f64,
        cost_percent: f64,
    ) -> Option<FeedbackEvent> {
        if self.entry_price <= 0.0
            || exit_price <= 0.0
            || !self.entry_price.is_finite()
            || !exit_price.is_finite()
            || !cost_percent.is_finite()
            || ![-1, 1].contains(&self.side)
            || closed_at_ms < self.opened_at_ms
        {
            return None;
        }
        let net_return_percent =
            ((exit_price / self.entry_price - 1.0) * f64::from(self.side) * 100.0) - cost_percent;
        Some(FeedbackEvent::ScoutOutcome {
            event_id: self.event_id,
            observed_at_ms: closed_at_ms,
            learning_context: None,
            symbol: self.symbol,
            strategy: self.strategy,
            profile_id: self.profile_id,
            parameters: self.parameters,
            baseline_parameters: BTreeMap::new(),
            comparison: strategies::ScoutComparisonMetadata::Independent,
            specs: self.specs,
            net_return_percent,
        })
    }
}

async fn generate_variants(
    Json(request): Json<ParameterVariantsRequest>,
) -> Result<Json<ParameterVariantsResponse>, (StatusCode, String)> {
    validate_variant_request(&request)?;
    Ok(Json(ParameterVariantsResponse {
        variants: build_parameter_variants(&request),
    }))
}

async fn generate_variants_batch(
    Json(request): Json<ParameterVariantsBatchRequest>,
) -> Result<Json<ParameterVariantsBatchResponse>, (StatusCode, String)> {
    if request.requests.len() > 512 {
        return Err((
            StatusCode::BAD_REQUEST,
            "too many parameter variant requests".into(),
        ));
    }
    for item in &request.requests {
        validate_variant_request(item)?;
    }
    Ok(Json(ParameterVariantsBatchResponse {
        responses: request
            .requests
            .iter()
            .map(|item| ParameterVariantsResponse {
                variants: build_parameter_variants(item),
            })
            .collect(),
    }))
}

fn validate_variant_request(
    request: &ParameterVariantsRequest,
) -> Result<(), (StatusCode, String)> {
    if request.parameters.values().any(|value| !value.is_finite())
        || request.specs.iter().any(|spec| {
            !spec.default.is_finite()
                || !spec.min.is_finite()
                || !spec.max.is_finite()
                || !spec.step.is_finite()
                || !spec.learning_direction.is_finite()
                || spec.min >= spec.max
                || spec.step <= 0.0
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid parameter variant request".into(),
        ));
    }
    Ok(())
}

fn build_parameter_variants(request: &ParameterVariantsRequest) -> Option<Vec<ParameterVariant>> {
    all_parameter_variants(&request.parameters, &request.specs, request.day).map(|variants| {
        variants
            .into_iter()
            .filter(|changes| {
                changes.iter().all(|(key, value)| {
                    !request.rejected_values.get(key).is_some_and(|rejected| {
                        rejected.iter().any(|candidate| (candidate - value).abs() <= 1e-8)
                    })
                })
            })
            .filter_map(|changes| {
                let (key, value) = if changes.len() == 1 {
                    let (key, value) = changes.iter().next()?;
                    (key.clone(), *value)
                } else {
                    (combination_key(&changes, &request.parameters)?, 0.0)
                };
                Some(ParameterVariant {
                    key,
                    value,
                    changes,
                })
            })
            .collect()
    })
}

async fn generate_live_candidates(
    Json(request): Json<LiveScoutCandidatesRequest>,
) -> Result<Json<Vec<serde_json::Value>>, (StatusCode, String)> {
    if request.signals.len() > 2_000
        || request.profiles.len() > 100
        || request
            .profiles
            .iter()
            .any(|profile| profile.parameters.values().any(|value| !value.is_finite()))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid live Scout candidate request".into(),
        ));
    }
    let mut candidates = Vec::new();
    for profile in &request.profiles {
        if !matches!(
            profile.strategy.as_str(),
            "live-momentum"
                | "live-trade-flow"
                | "live-order-book"
                | "live-vwap-dislocation"
                | "live-absorption-pressure"
        ) {
            continue;
        }
        for source in &request.signals {
            if source.get("symbol").and_then(serde_json::Value::as_str)
                != Some(profile.symbol.as_str())
            {
                continue;
            }
            if source
                .get("strategy_candidate")
                .and_then(serde_json::Value::as_bool)
                == Some(true)
            {
                continue;
            }
            let Some(configured) = live_scout_feature_signal(source, profile) else {
                continue;
            };
            let components = &configured["components"];
            let directional_values = match profile.strategy.as_str() {
                "live-momentum" => vec![
                    number(components, "momentum_10s_percent"),
                    number(components, "trend_5m_volatility_units"),
                ],
                "live-trade-flow" => vec![number(components, "trade_flow_imbalance")],
                "live-order-book" => vec![number(components, "book_imbalance")],
                "live-vwap-dislocation" => vec![number(components, "mispricing_vs_vwap_bps")],
                "live-absorption-pressure" => {
                    vec![number(components, "pressure_without_price_move").map(|value| -value)]
                }
                _ => Vec::new(),
            };
            let mut directions = directional_values
                .into_iter()
                .flatten()
                .filter(|value| value.is_finite() && *value != 0.0)
                .map(|value| if value > 0.0 { "bullish" } else { "bearish" })
                .collect::<std::collections::BTreeSet<_>>();
            let mut passing = Vec::new();
            for direction in std::mem::take(&mut directions) {
                let side = if direction == "bullish" { 1.0 } else { -1.0 };
                let enters = live_strategy_enters(
                    profile.strategy.as_str(),
                    configured.clone(),
                    direction,
                    profile.parameters.clone(),
                );
                if enters {
                    let mut candidate = configured.clone();
                    // The input signal has no strategy attribution. The downstream plan
                    // receiver needs the profile owner on the serialized candidate.
                    candidate["strategy"] = serde_json::json!(profile.strategy);
                    candidate["direction"] = serde_json::json!(direction);
                    let source_score = number(source, "score").unwrap_or_default().abs();
                    candidate["score"] = serde_json::json!(source_score * side);
                    let strength = number(source, "strength").unwrap_or_default();
                    candidate["strength"] = serde_json::json!(strength);
                    candidate["strategy_decision"] = serde_json::json!({
                        "scout_only": false,
                        "strategy": profile.strategy,
                        "strategy_parameters": profile.parameters,
                        "observed_insight": source.get("components").cloned().unwrap_or_default(),
                        "direction": direction,
                        "strength": strength,
                        "score": source_score * side,
                        "direction_source": "strategy_insight"
                    });
                    passing.push(candidate);
                }
            }
            let distinct_directions = passing
                .iter()
                .filter_map(|candidate| {
                    candidate
                        .get("direction")
                        .and_then(serde_json::Value::as_str)
                })
                .collect::<std::collections::BTreeSet<_>>();
            if distinct_directions.len() == 1 {
                candidates.extend(passing);
            }
        }
    }
    Ok(Json(candidates))
}

async fn get_strategy_walk_forward(
    State(service): State<ServiceState>,
    Json(request): Json<StrategyEvidenceRequest>,
) -> Result<Json<Vec<Option<StrategyWalkForwardEvidence>>>, (StatusCode, String)> {
    if request.candidates.len() > 10_000
        || request.candidates.iter().any(|candidate| {
            candidate.symbol.is_empty()
                || strategies::Strategy::from_name(&candidate.strategy).is_none()
                || candidate.signal_timestamp_ms > scout_now_ms().saturating_add(5_000)
                || candidate.parameters.values().any(|value| !value.is_finite())
        })
    {
        return Err((StatusCode::BAD_REQUEST, "invalid strategy walk-forward request".into()));
    }

    if !service
        .strategy_outcome_profile_index_ready
        .load(std::sync::atomic::Ordering::Acquire)
    {
        let mut state = service.scout.data.lock().await;
        rebuild_strategy_outcome_profile_index(&mut state);
        service
            .strategy_outcome_profile_index_ready
            .store(true, std::sync::atomic::Ordering::Release);
    }
    let has_portfolio_candidates = request
        .candidates
        .iter()
        .any(|candidate| candidate.strategy == "risk-adjusted-trend-portfolio");
    if !has_portfolio_candidates {
        let index = service
            .strategy_outcomes_by_profile
            .read()
            .unwrap_or_else(|error| error.into_inner());
        let results = request
            .candidates
            .iter()
            .map(|candidate| {
                let profile = strategy_evidence_profile_key(candidate);
                strategy_walk_forward_for_indexed_samples(
                    candidate,
                    index.get(&profile).map(Vec::as_slice).unwrap_or_default(),
                )
            })
            .collect();
        return Ok(Json(results));
    }

    let index = service
        .strategy_outcomes_by_profile
        .read()
        .unwrap_or_else(|error| error.into_inner());
    let portfolio_profiles = request.candidates.iter()
        .filter(|candidate| candidate.strategy == "risk-adjusted-trend-portfolio")
        .map(strategy_evidence_profile_key)
        .collect::<BTreeSet<_>>();
    let mut portfolio_outcomes_by_group =
        BTreeMap::<String, Vec<StrategyOutcomeProfileIndexEntry>>::new();
    for profile in &portfolio_profiles {
        let Some(indexed_outcomes) = index.get(profile) else {
            continue;
        };
        for indexed in indexed_outcomes {
            if !indexed.sample_group_id.is_empty() {
                portfolio_outcomes_by_group
                    .entry(indexed.sample_group_id.clone())
                    .or_default()
                    .push(indexed.clone());
            }
        }
    }
    let instance_index = service
        .portfolio_instances_by_group
        .read()
        .unwrap_or_else(|error| error.into_inner());
    let portfolio_instances_by_group = portfolio_outcomes_by_group
        .keys()
        .filter_map(|group| {
            instance_index
                .get(group)
                .map(|instances| (group.clone(), instances.clone()))
        })
        .collect::<BTreeMap<_, _>>();
    let results = request.candidates.iter().map(|candidate| {
        if candidate.strategy == "risk-adjusted-trend-portfolio" {
            portfolio_strategy_walk_forward(
                candidate,
                &portfolio_outcomes_by_group,
                &portfolio_instances_by_group,
            )
        } else {
            let key = strategy_evidence_profile_key(candidate);
            strategy_walk_forward_for_indexed_samples(
                candidate,
                index.get(&key).map(Vec::as_slice).unwrap_or_default(),
            )
        }
    }).collect();
    Ok(Json(results))
}

fn strategy_walk_forward_for_candidate(
    candidate: &StrategyEvidenceCandidate,
    outcomes: &[&ScoutOutcomeRequest],
) -> Option<StrategyWalkForwardEvidence> {
    let mut samples = outcomes
        .iter()
        .copied()
        .filter(|outcome| {
            outcome.evaluated_timestamp_ms < candidate.signal_timestamp_ms
                && outcome.net_return_percent.is_finite()
        })
        .cloned()
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.signal_timestamp_ms);

    // Evaluate only matured, non-overlapping prior samples. This is a
    // forward-only cohort for the same strategy, market, and full profile.
    let mut non_overlapping = Vec::new();
    let mut next_timestamp_ms = 0_u64;
    for sample in samples {
        if sample.signal_timestamp_ms < next_timestamp_ms {
            continue;
        }
        next_timestamp_ms = sample.evaluated_timestamp_ms.max(
            sample.signal_timestamp_ms.saturating_add(60_000),
        );
        non_overlapping.push(sample);
    }
    if non_overlapping.is_empty() {
        return None;
    }
    let sample_count = non_overlapping.len();
    let positive_count = non_overlapping
        .iter()
        .filter(|sample| sample.net_return_percent > 0.0)
        .count();
    let average_net_return_percent = non_overlapping
        .iter()
        .map(|sample| sample.net_return_percent)
        .sum::<f64>()
        / sample_count as f64;
    Some(StrategyWalkForwardEvidence {
        symbol: candidate.symbol.clone(),
        strategy: candidate.strategy.clone(),
        sample_count,
        positive_net_rate: positive_count as f64 / sample_count as f64,
        average_net_return_percent,
        last_sample_timestamp_ms: non_overlapping.last()?.evaluated_timestamp_ms,
    })
}

fn strategy_walk_forward_for_indexed_samples(
    candidate: &StrategyEvidenceCandidate,
    outcomes: &[StrategyOutcomeProfileIndexEntry],
) -> Option<StrategyWalkForwardEvidence> {
    let mut sample_count = 0;
    let mut positive_count = 0;
    let mut net_return_total = 0.0;
    let mut last_sample_timestamp_ms = 0;
    let mut next_timestamp_ms = 0_u64;

    for outcome in outcomes {
        if outcome.signal_timestamp_ms > candidate.signal_timestamp_ms {
            break;
        }
        if outcome.evaluated_timestamp_ms >= candidate.signal_timestamp_ms
            || !outcome.net_return_percent.is_finite()
            || outcome.signal_timestamp_ms < next_timestamp_ms
        {
            continue;
        }
        next_timestamp_ms = outcome.evaluated_timestamp_ms.max(
            outcome.signal_timestamp_ms.saturating_add(60_000),
        );
        sample_count += 1;
        positive_count += usize::from(outcome.net_return_percent > 0.0);
        net_return_total += outcome.net_return_percent;
        last_sample_timestamp_ms = outcome.evaluated_timestamp_ms;
    }

    (sample_count > 0).then(|| StrategyWalkForwardEvidence {
        symbol: candidate.symbol.clone(),
        strategy: candidate.strategy.clone(),
        sample_count,
        positive_net_rate: positive_count as f64 / sample_count as f64,
        average_net_return_percent: net_return_total / sample_count as f64,
        last_sample_timestamp_ms,
    })
}

fn strategy_evidence_profile_key(candidate: &StrategyEvidenceCandidate) -> String {
    strategy_evidence_profile_key_from_parts(
        &candidate.strategy,
        if candidate.strategy == "risk-adjusted-trend-portfolio" {
            ""
        } else {
            &candidate.symbol
        },
        &candidate.parameters,
    )
}

fn strategy_evidence_profile_key_from_parts(
    strategy: &str,
    symbol: &str,
    parameters: &BTreeMap<String, f64>,
) -> String {
    let parameters = parameters_without_walk_forward_policy(parameters);
    serde_json::to_string(&(strategy, symbol, parameters)).unwrap_or_default()
}

fn strategy_outcome_profile_key(outcome: &ScoutOutcomeRequest) -> String {
    strategy_evidence_profile_key_from_parts(
        &outcome.strategy,
        if outcome.strategy == "risk-adjusted-trend-portfolio" {
            ""
        } else {
            &outcome.symbol
        },
        &outcome.strategy_parameters,
    )
}

#[derive(Clone, Debug)]
struct StrategyOutcomeProfileIndexEntry {
    outcome_key: String,
    decision_id: String,
    sample_group_id: String,
    signal_timestamp_ms: u64,
    evaluated_timestamp_ms: u64,
    net_return_percent: f64,
}

#[derive(Clone, Debug)]
struct PortfolioInstanceEvidence {
    decision_id: String,
    timestamp_ms: u64,
    strategy_parameters: BTreeMap<String, f64>,
    strategy_order_size_multiplier: Option<f64>,
}

const MAX_CACHED_PORTFOLIO_INSTANCES: usize = 4_096;

fn rebuild_strategy_outcome_profile_index(state: &mut PersistedState) {
    let mut rebuilt = BTreeMap::<String, Vec<StrategyOutcomeProfileIndexEntry>>::new();
    for (outcome_key, outcome) in &state.outcomes {
        if outcome.exit_scenario == "strategy" {
            rebuilt
                .entry(strategy_outcome_profile_key(outcome))
                .or_default()
                .push(StrategyOutcomeProfileIndexEntry {
                    outcome_key: outcome_key.clone(),
                    decision_id: outcome.decision_id.clone(),
                    sample_group_id: outcome.sample_group_id.clone(),
                    signal_timestamp_ms: outcome.signal_timestamp_ms,
                    evaluated_timestamp_ms: outcome.evaluated_timestamp_ms,
                    net_return_percent: outcome.net_return_percent,
                });
        }
    }
    for outcomes in rebuilt.values_mut() {
        outcomes.sort_by(|left, right| {
            left.signal_timestamp_ms
                .cmp(&right.signal_timestamp_ms)
                .then_with(|| left.outcome_key.cmp(&right.outcome_key))
        });
    }
    *state
        .strategy_outcomes_by_profile
        .write()
        .unwrap_or_else(|error| error.into_inner()) = rebuilt;
}

fn portfolio_instance_entry(
    decision_id: &str,
    instance: &serde_json::Value,
) -> Option<(String, PortfolioInstanceEvidence)> {
    if instance.get("strategy").and_then(serde_json::Value::as_str)
        != Some("risk-adjusted-trend-portfolio")
        || instance.get("scout_exit_scenario").and_then(serde_json::Value::as_str)
            != Some("strategy")
        || instance.get("scout_parameter_key").is_some()
    {
        return None;
    }
    let group = instance
        .get("scout_sample_group_id")
        .and_then(serde_json::Value::as_str)?;
    let strategy_parameters = instance
        .get("strategy_parameters")
        .cloned()
        .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok())
        .unwrap_or_default();
    let strategy_order_size_multiplier = instance
        .get("strategy_decision")
        .and_then(|decision| decision.get("strategy_order_size_multiplier"))
        .and_then(serde_json::Value::as_f64)
        .filter(|weight| weight.is_finite() && *weight > 0.0);
    Some((
        group.to_owned(),
        PortfolioInstanceEvidence {
            decision_id: decision_id.to_owned(),
            timestamp_ms: instance["timestamp"].as_u64().unwrap_or_default(),
            strategy_parameters,
            strategy_order_size_multiplier,
        },
    ))
}

#[cfg(test)]
fn rebuild_portfolio_instance_index(state: &mut PersistedState) {
    let mut rebuilt = BTreeMap::<String, Vec<PortfolioInstanceEvidence>>::new();
    for (decision_id, instance) in &state.scout_instances {
        if let Some((group, evidence)) = portfolio_instance_entry(decision_id, instance) {
            rebuilt.entry(group).or_default().push(evidence);
        }
    }
    for instances in rebuilt.values_mut() {
        instances.sort_by(|left, right| {
            left.timestamp_ms.cmp(&right.timestamp_ms)
                .then_with(|| left.decision_id.cmp(&right.decision_id))
        });
    }
    *state
        .portfolio_instances_by_group
        .write()
        .unwrap_or_else(|error| error.into_inner()) = rebuilt;
}

fn remove_portfolio_instance_index_entry_by_id(state: &mut PersistedState, decision_id: &str) {
    let mut index = state
        .portfolio_instances_by_group
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let empty_groups = index
        .iter_mut()
        .filter_map(|(group, instances)| {
        instances.retain(|entry| entry.decision_id != decision_id);
            instances.is_empty().then(|| group.clone())
        })
        .collect::<Vec<_>>();
    for group in empty_groups {
        index.remove(&group);
    }
}

fn add_portfolio_instance_index_entry(
    state: &mut PersistedState,
    decision_id: &str,
    instance: &serde_json::Value,
) {
    let Some((group, evidence)) = portfolio_instance_entry(decision_id, instance) else {
        return;
    };
    let mut index = state
        .portfolio_instances_by_group
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let instances = index.entry(group).or_default();
    let position = instances
        .binary_search_by(|existing| {
            existing.timestamp_ms.cmp(&evidence.timestamp_ms)
                .then_with(|| existing.decision_id.cmp(&evidence.decision_id))
        })
        .unwrap_or_else(|position| position);
    instances.insert(position, evidence);
    while index.values().map(Vec::len).sum::<usize>() > MAX_CACHED_PORTFOLIO_INSTANCES {
        let oldest = index.iter().flat_map(|(group, instances)| {
            instances.iter().enumerate().map(move |(position, instance)| {
                (instance.timestamp_ms, instance.decision_id.clone(), group.clone(), position)
            })
        }).min();
        let Some((_, _, group, position)) = oldest else { break };
        if let Some(instances) = index.get_mut(&group) {
            instances.remove(position);
            if instances.is_empty() { index.remove(&group); }
        }
    }
}

fn insert_scout_instance(
    state: &mut PersistedState,
    decision_id: &str,
    instance: serde_json::Value,
) {
    cache_scout_instance(state, decision_id, instance);
}

fn compact_scout_instance(instance: &serde_json::Value) -> serde_json::Value {
    const FIELDS: &[&str] = &[
        "decision_id", "symbol", "direction", "timestamp", "signal_timestamp",
        "strategy", "scout_entry_price", "scout_entry_bid", "scout_entry_ask",
        "scout_notional_usd", "scout_learning_context", "scout_sample_group_id",
        "scout_exit_scenario", "scout_parameter_key", "scout_parameter_value",
        "scout_parameter_profile_symbol", "scout_parameter_entry_eligible",
        "scout_comparison_id", "scout_comparison_ids", "scout_comparison_zero_sides",
        "scout_comparison_role", "scout_comparison_zero_role",
        "scout_comparison_parameter_key", "scout_comparison_parameter_value",
        "scout_comparison_parameter_profile_symbol", "strategy_parameters",
        "strategy_parameter_specs", "stop_loss_percent", "take_profit_percent",
        "scout_level_percent", "scout_net_return_percent",
        "scout_evaluated_timestamp_ms", "scout_taker_fee_rate", "paired_delta_percent",
        "scout_markout_observations",
        "scout_thesis_failure_confirmation_count", "scout_thesis_failure_last_signal_timestamp_ms",
        "scout_thesis_failure_exit_net_return_percent", "scout_thesis_failure_exit_timestamp_ms",
        "scout_thesis_failure_stop_timestamp_ms",
        "strength",
    ];
    let mut compact = serde_json::Map::new();
    for key in FIELDS {
        if let Some(value) = instance.get(*key) {
            compact.insert((*key).to_owned(), value.clone());
        }
    }
    let mut decision = serde_json::Map::new();
    if let Some(source) = instance.get("strategy_decision") {
        for key in ["observed_close", "strategy_order_size_multiplier", "strategy_parameters"] {
            if let Some(value) = source.get(key) {
                decision.insert(key.to_owned(), value.clone());
            }
        }
    }
    if !decision.is_empty() {
        compact.insert("strategy_decision".to_owned(), decision.into());
    }
    compact.into()
}

#[cfg(test)]
fn compact_persisted_scout_payloads(
    state: &mut PersistedState,
) -> (Vec<String>, Vec<String>, Vec<String>) {
    let mut changed_instances = Vec::new();
    for (decision_id, instance) in &mut state.scout_instances {
        let compact = compact_scout_instance(instance);
        if *instance != compact {
            *instance = compact;
            changed_instances.push(decision_id.clone());
        }
    }
    let mut changed_audit_groups = Vec::new();
    for (group_id, audits) in &mut state.scout_active_audits {
        let mut changed = false;
        for audit in audits {
            let compact = compact_scout_instance(audit);
            if *audit != compact {
                *audit = compact;
                changed = true;
            }
        }
        if changed {
            changed_audit_groups.push(group_id.clone());
        }
    }
    let mut changed_plan_results = Vec::new();
    for (request_id, response) in &mut state.scout_plan_results {
        let mut changed = false;
        for audit in &mut response.audits {
            let compact = compact_scout_instance(audit);
            if *audit != compact {
                *audit = compact;
                changed = true;
            }
        }
        if changed {
            changed_plan_results.push(request_id.clone());
        }
    }
    (changed_instances, changed_audit_groups, changed_plan_results)
}

#[cfg(test)]
fn remove_scout_instance(state: &mut PersistedState, decision_id: &str) {
    state.scout_instances.remove(decision_id);
    remove_portfolio_instance_index_entry_by_id(state, decision_id);
}

fn remove_outcome_from_profile_index(
    state: &mut PersistedState,
    outcome_key: &str,
    outcome: &ScoutOutcomeRequest,
) {
    let profile = strategy_outcome_profile_key(outcome);
    let mut profiles = state
        .strategy_outcomes_by_profile
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let remove_profile = if let Some(indexed) = profiles.get_mut(&profile) {
        if let Ok(index) = indexed.binary_search_by(|entry| {
            entry
                .signal_timestamp_ms
                .cmp(&outcome.signal_timestamp_ms)
                .then_with(|| entry.outcome_key.as_str().cmp(outcome_key))
        }) {
            indexed.remove(index);
        }
        indexed.is_empty()
    } else {
        false
    };
    if remove_profile {
        profiles.remove(&profile);
    }
}

fn add_outcome_to_profile_index(
    state: &mut PersistedState,
    outcome_key: &str,
    outcome: &ScoutOutcomeRequest,
) {
    let profile = strategy_outcome_profile_key(outcome);
    let mut profiles = state
        .strategy_outcomes_by_profile
        .write()
        .unwrap_or_else(|error| error.into_inner());
    let indexed = profiles
        .entry(profile)
        .or_default();
    let index = indexed
        .binary_search_by(|entry| {
            entry
                .signal_timestamp_ms
                .cmp(&outcome.signal_timestamp_ms)
                .then_with(|| entry.outcome_key.as_str().cmp(outcome_key))
        })
        .unwrap_or_else(|index| index);
    indexed.insert(
        index,
        StrategyOutcomeProfileIndexEntry {
            outcome_key: outcome_key.to_owned(),
            decision_id: outcome.decision_id.clone(),
            sample_group_id: outcome.sample_group_id.clone(),
            signal_timestamp_ms: outcome.signal_timestamp_ms,
            evaluated_timestamp_ms: outcome.evaluated_timestamp_ms,
            net_return_percent: outcome.net_return_percent,
        },
    );
}

fn insert_scout_outcome(
    state: &mut PersistedState,
    outcome_key: String,
    outcome: ScoutOutcomeRequest,
) {
    if let Some(previous) = state.outcomes.remove(&outcome_key) {
        if previous.exit_scenario == "strategy" {
            remove_outcome_from_profile_index(state, &outcome_key, &previous);
        }
    }
    if outcome.exit_scenario == "strategy" {
        add_outcome_to_profile_index(state, &outcome_key, &outcome);
    }
    state.outcomes.insert(outcome_key, outcome);
}

fn remove_scout_outcome(state: &mut PersistedState, outcome_key: &str) {
    let Some(outcome) = state.outcomes.remove(outcome_key) else {
        return;
    };
    if outcome.exit_scenario == "strategy" {
        remove_outcome_from_profile_index(state, outcome_key, &outcome);
    }
}

fn trim_scout_outcome_cache(state: &mut PersistedState) {
    while state.outcomes.len() > MAX_CACHED_SCOUT_OUTCOMES {
        let oldest = state
            .outcomes
            .iter()
            .min_by_key(|(key, outcome)| (outcome.evaluated_timestamp_ms, (*key).clone()))
            .map(|(key, _)| key.clone());
        let Some(oldest) = oldest else { break };
        // SQLite is the durable source. Evict only the runtime copy and its
        // derived profile-index entry; never issue a DELETE for history.
        remove_scout_outcome(state, &oldest);
    }
}

fn parameters_without_walk_forward_policy(
    parameters: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    parameters
        .iter()
        .filter(|(key, _)| {
            !matches!(
                key.as_str(),
                "scout_minimum_walk_forward_samples"
                    | "scout_minimum_positive_net_rate"
                    // Cooldown is evaluated as a paired follow-on-entry
                    // policy. It must not reset the evidence cohort for an
                    // otherwise identical entry profile.
                    | "post_stop_cooldown_minutes"
            )
        })
        .map(|(key, value)| (key.clone(), *value))
        .collect()
}

fn same_strategy_parameters(
    observed: &BTreeMap<String, f64>,
    requested: &BTreeMap<String, f64>,
) -> bool {
    parameters_without_walk_forward_policy(observed)
        == parameters_without_walk_forward_policy(requested)
}

fn portfolio_strategy_walk_forward(
    candidate: &StrategyEvidenceCandidate,
    outcomes_by_group: &BTreeMap<String, Vec<StrategyOutcomeProfileIndexEntry>>,
    instances_by_group: &BTreeMap<String, Vec<PortfolioInstanceEvidence>>,
) -> Option<StrategyWalkForwardEvidence> {
    let mut samples = Vec::<(u64, u64, f64)>::new();
    for (group, expected) in instances_by_group {
        if expected.len() < 2 {
            continue;
        }
        let expected_ids = expected
            .iter()
            .map(|instance| instance.decision_id.as_str())
            .collect::<BTreeSet<_>>();
        let mut weights = BTreeMap::<String, f64>::new();
        let mut valid_group = true;
        for instance in expected {
            if !same_strategy_parameters(
                &instance.strategy_parameters,
                &candidate.parameters,
            ) || instance.strategy_order_size_multiplier.is_none()
            {
                valid_group = false;
                break;
            }
            weights.insert(
                instance.decision_id.clone(),
                instance.strategy_order_size_multiplier.unwrap(),
            );
        }
        if !valid_group {
            continue;
        }
        let matching = outcomes_by_group
            .get(group)
            .into_iter()
            .flatten()
            .filter(|outcome| {
                expected_ids.contains(outcome.decision_id.as_str())
                    && outcome.evaluated_timestamp_ms < candidate.signal_timestamp_ms
                    && outcome.net_return_percent.is_finite()
            })
            .collect::<Vec<_>>();
        if matching.len() != expected.len() {
            continue;
        }
        let mut portfolio_return = 0.0;
        let mut signal_timestamp_ms = u64::MAX;
        let mut evaluated_timestamp_ms = 0;
        for outcome in matching {
            let weight = weights.get(&outcome.decision_id).copied()?;
            portfolio_return += weight * outcome.net_return_percent;
            signal_timestamp_ms = signal_timestamp_ms.min(outcome.signal_timestamp_ms);
            evaluated_timestamp_ms = evaluated_timestamp_ms.max(outcome.evaluated_timestamp_ms);
        }
        if signal_timestamp_ms != u64::MAX {
            samples.push((signal_timestamp_ms, evaluated_timestamp_ms, portfolio_return));
        }
    }
    samples.sort_by_key(|sample| sample.0);
    let mut non_overlapping = Vec::new();
    let mut next_timestamp_ms = 0_u64;
    for sample in samples {
        if sample.0 < next_timestamp_ms {
            continue;
        }
        next_timestamp_ms = sample.1;
        non_overlapping.push(sample);
    }
    if non_overlapping.is_empty() {
        return None;
    }
    let sample_count = non_overlapping.len();
    let positive_count = non_overlapping.iter().filter(|sample| sample.2 > 0.0).count();
    let average_net_return_percent = non_overlapping.iter()
        .map(|sample| sample.2)
        .sum::<f64>() / sample_count as f64;
    Some(StrategyWalkForwardEvidence {
        symbol: candidate.symbol.clone(),
        strategy: candidate.strategy.clone(),
        sample_count,
        positive_net_rate: positive_count as f64 / sample_count as f64,
        average_net_return_percent,
        last_sample_timestamp_ms: non_overlapping.last()?.1,
    })
}

#[cfg(test)]
mod strategy_walk_forward_tests {
    use super::*;

    fn test_service(path: &std::path::Path, state: PersistedState) -> ServiceState {
        ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            auto_trader_rsocket_url: String::new(),
            strategy_outcomes_by_profile: Arc::clone(&state.strategy_outcomes_by_profile),
            portfolio_instances_by_group: Arc::clone(&state.portfolio_instances_by_group),
            strategy_outcome_profile_index_ready: Arc::new(
                std::sync::atomic::AtomicBool::new(false),
            ),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(state)),
                path: path.to_path_buf(),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            },
        }
    }

    #[test]
    fn active_scout_audit_cache_evicts_payload_but_preserves_lifecycle_state() {
        let mut state = PersistedState::default();
        state.scout_active_audits.insert(
            "old-group".into(),
            vec![serde_json::json!({"timestamp": 1}); MAX_ACTIVE_SCOUT_AUDITS + 1],
        );
        state
            .scout_sample_active_scenarios
            .insert("old-group".into(), BTreeSet::from(["decision|strategy".into()]));
        state
            .scout_plan_profiles
            .insert("old-group".into(), "profile".into());

        assert_eq!(trim_active_scout_audits(&mut state), vec!["old-group"]);
        assert!(state.scout_active_audits.is_empty());
        assert!(state.scout_sample_active_scenarios.contains_key("old-group"));
        assert!(state.scout_plan_profiles.contains_key("old-group"));
    }

    #[test]
    fn portfolio_evidence_index_rebuilds_from_sqlite_without_resident_instances() {
        let path = std::env::temp_dir().join(format!(
            "scout-portfolio-index-restart-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        for (decision_id, weight) in [("portfolio-a", 0.6), ("portfolio-b", 0.4)] {
            persisted.scout_instances.insert(
                decision_id.into(),
                serde_json::json!({
                    "strategy":"risk-adjusted-trend-portfolio",
                    "scout_exit_scenario":"strategy",
                    "scout_sample_group_id":"portfolio-group",
                    "strategy_parameters":{"risk_target":0.1},
                    "strategy_decision":{"strategy_order_size_multiplier":weight}
                }),
            );
        }
        persist_scout_state(&path, &persisted, &["scout_instances"]).unwrap();
        let mut restarted = load_scout_state(&path).unwrap();
        assert!(restarted.scout_instances.is_empty());
        rebuild_portfolio_instance_index_from_db(&path, &mut restarted).unwrap();
        let index = restarted
            .portfolio_instances_by_group
            .read()
            .unwrap();
        let evidence = &index["portfolio-group"];
        assert_eq!(evidence.len(), 2);
        assert_eq!(evidence[0].decision_id, "portfolio-a");
        assert_eq!(evidence[1].decision_id, "portfolio-b");
        drop(index);
        let _ = std::fs::remove_file(path);
    }

    fn outcome(id: &str, profile: BTreeMap<String, f64>, signal_ms: u64, evaluated_ms: u64, net: f64) -> ScoutOutcomeRequest {
        ScoutOutcomeRequest {
            decision_id: id.into(),
            learning_context: "scout:live-momentum:BTC-USDT:bullish:scenario".into(),
            simulated_net_pnl_usd: net,
            net_return_percent: net,
            signal_timestamp_ms: signal_ms,
            evaluated_timestamp_ms: evaluated_ms,
            taker_fee_rate: 0.001,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            strategy_parameters: profile,
            exit_scenario: "strategy".into(),
            sample_group_id: id.into(),
            scout_parameter_key: None,
            scout_parameter_value: None,
            scout_parameter_profile_symbol: None,
            scout_comparison_id: None,
            scout_comparison_ids: Vec::new(),
            scout_comparison_zero_sides: Vec::new(),
            scout_comparison_role: None,
            scout_comparison_zero_role: None,
            scout_comparison_parameter_key: None,
            scout_comparison_parameter_value: None,
            scout_comparison_parameter_profile_symbol: None,
            strategy_parameter_specs: Vec::new(),
        }
    }

    #[tokio::test]
    async fn persisted_outcome_retry_is_idempotent_after_worker_restart() {
        let path = std::env::temp_dir().join(format!(
            "scout-outcome-restart-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let initial = PersistedState::default();
        persist_scout_state(&path, &initial, &["scout_configuration"]).unwrap();
        let first = test_service(&path, load_scout_state(&path).unwrap());
        let now = scout_now_ms();
        let request = outcome(
            "restart-idempotency",
            BTreeMap::new(),
            now.saturating_sub(100),
            now,
            0.25,
        );
        receive_scout_outcome(State(first), Json(request.clone()))
            .await
            .unwrap();

        let restarted_state = load_scout_state(&path).unwrap();
        assert_eq!(restarted_state.outcomes.len(), 1);
        assert!(restarted_state.seen.is_empty());
        assert!(scout_record_exists(
            &path,
            "seen",
            &scout_outcome_idempotency_key(&request)
        )
        .unwrap());
        let restarted = test_service(&path, restarted_state);
        receive_scout_outcome(State(restarted), Json(request))
            .await
            .unwrap();
        assert_eq!(load_scout_state(&path).unwrap().outcomes.len(), 1);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn scout_outcome_hydration_is_bounded_without_deleting_durable_history() {
        let path = std::env::temp_dir().join(format!(
            "scout-outcome-cache-bound-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        for index in 0..(MAX_CACHED_SCOUT_OUTCOMES + 19) {
            let row = outcome(
                &format!("decision-{index:05}"),
                BTreeMap::new(),
                index as u64 + 1,
                index as u64 + 1,
                0.25,
            );
            let key = scout_outcome_idempotency_key(&row);
            persisted.outcomes.insert(key, row);
        }
        persist_scout_state(&path, &persisted, &["outcomes"]).unwrap();

        let hydrated = load_scout_state(&path).unwrap();
        assert_eq!(hydrated.outcomes.len(), MAX_CACHED_SCOUT_OUTCOMES);
        assert!(hydrated.outcomes.values().any(|row| row.decision_id == "decision-04114"));
        assert!(!hydrated.outcomes.values().any(|row| row.decision_id == "decision-00000"));
        assert_eq!(
            rusqlite::Connection::open(&path)
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM scout_records WHERE collection='outcomes'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap() as usize,
            MAX_CACHED_SCOUT_OUTCOMES + 19
        );
        assert_eq!(
            load_scout_outcomes_since(&path, 1).unwrap().len(),
            MAX_CACHED_SCOUT_OUTCOMES + 19,
            "time-range report reads must still see outcomes outside the RAM cache"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn paired_outcome_completes_and_retries_after_worker_restart() {
        let path = std::env::temp_dir().join(format!(
            "scout-paired-outcome-restart-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let initial = PersistedState::default();
        persist_scout_state(&path, &initial, &["scout_configuration"]).unwrap();
        let now = scout_now_ms();
        let mut baseline = outcome(
            "paired-baseline",
            BTreeMap::from([("minimum_momentum".into(), 0.5)]),
            now.saturating_sub(100),
            now,
            0.2,
        );
        baseline.scout_comparison_role = Some("baseline".into());
        baseline.scout_comparison_ids = vec!["restart-comparison".into()];
        baseline.strategy_parameter_specs = vec![strategies::ParameterSpec {
            key: "minimum_momentum".into(),
            label: "Minimum momentum".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "rate".into(),
            learning_direction: 1.0,
        }];
        receive_scout_outcome(
            State(test_service(&path, load_scout_state(&path).unwrap())),
            Json(baseline),
        )
        .await
        .unwrap();

        let mut variant = outcome(
            "paired-variant",
            BTreeMap::from([("minimum_momentum".into(), 0.6)]),
            now.saturating_sub(100),
            now,
            0.5,
        );
        variant.scout_parameter_key = Some("minimum_momentum".into());
        variant.scout_parameter_value = Some(0.6);
        variant.scout_parameter_profile_symbol = Some("BTC-USDT".into());
        variant.scout_comparison_id = Some("restart-comparison".into());
        variant.scout_comparison_role = Some("variant".into());
        variant.strategy_parameter_specs = vec![strategies::ParameterSpec {
            key: "minimum_momentum".into(),
            label: "Minimum momentum".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "rate".into(),
            learning_direction: 1.0,
        }];
        receive_scout_outcome(
            State(test_service(&path, load_scout_state(&path).unwrap())),
            Json(variant.clone()),
        )
        .await
        .unwrap();
        let completed = load_scout_state(&path).unwrap();
        assert_eq!(completed.outcomes.len(), 2);
        assert!(completed.pairs.is_empty());
        let feedback_count = completed.outbox.len();
        assert!(feedback_count <= 1);

        receive_scout_outcome(
            State(test_service(&path, load_scout_state(&path).unwrap())),
            Json(variant),
        )
        .await
        .unwrap();
        let retried = load_scout_state(&path).unwrap();
        assert_eq!(retried.outcomes.len(), 2);
        assert_eq!(retried.outbox.len(), feedback_count);
        let _ = std::fs::remove_file(path);
    }

    fn indexed_outcome(outcome: &ScoutOutcomeRequest) -> StrategyOutcomeProfileIndexEntry {
        StrategyOutcomeProfileIndexEntry {
            outcome_key: scout_outcome_idempotency_key(outcome),
            decision_id: outcome.decision_id.clone(),
            sample_group_id: outcome.sample_group_id.clone(),
            signal_timestamp_ms: outcome.signal_timestamp_ms,
            evaluated_timestamp_ms: outcome.evaluated_timestamp_ms,
            net_return_percent: outcome.net_return_percent,
        }
    }

    fn portfolio_instance(
        decision_id: &str,
        parameters: BTreeMap<String, f64>,
        weight: Option<f64>,
    ) -> PortfolioInstanceEvidence {
        PortfolioInstanceEvidence {
            decision_id: decision_id.into(),
            timestamp_ms: 1,
            strategy_parameters: parameters,
            strategy_order_size_multiplier: weight,
        }
    }

    #[test]
    fn strategy_outcome_profile_index_tracks_additions_and_evictions() {
        let profile = BTreeMap::from([("minimum_momentum".into(), 0.2)]);
        let mut state = PersistedState::default();
        rebuild_strategy_outcome_profile_index(&mut state);

        insert_scout_outcome(
            &mut state,
            "first".into(),
            outcome("first", profile.clone(), 100, 60_100, -0.1),
        );
        insert_scout_outcome(
            &mut state,
            "second".into(),
            outcome("second", profile.clone(), 200, 60_200, 0.2),
        );
        let index_key = strategy_evidence_profile_key_from_parts(
            "live-momentum",
            "BTC-USDT",
            &profile,
        );
        {
            let index = state
                .strategy_outcomes_by_profile
                .read()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(
                index[&index_key]
                    .iter()
                    .map(|entry| entry.outcome_key.as_str())
                    .collect::<Vec<_>>(),
                vec!["first", "second"]
            );
        }

        remove_scout_outcome(&mut state, "first");
        let index = state
            .strategy_outcomes_by_profile
            .read()
            .unwrap_or_else(|error| error.into_inner());
        assert_eq!(
            index[&index_key]
                .iter()
                .map(|entry| entry.outcome_key.as_str())
                .collect::<Vec<_>>(),
            vec!["second"]
        );
    }

    #[test]
    fn portfolio_instance_cache_tracks_plan_insertions_and_evictions() {
        let mut state = PersistedState::default();
        rebuild_portfolio_instance_index(&mut state);
        let instance = serde_json::json!({
            "strategy": "risk-adjusted-trend-portfolio",
            "scout_exit_scenario": "strategy",
            "scout_sample_group_id": "portfolio-group",
            "strategy_parameters": {"trend_portfolio_lookback_days": 90.0},
            "strategy_decision": {"strategy_order_size_multiplier": 0.5}
        });
        insert_scout_instance(&mut state, "decision-btc", instance);
        {
            let index = state
                .portfolio_instances_by_group
                .read()
                .unwrap_or_else(|error| error.into_inner());
            assert_eq!(index["portfolio-group"].len(), 1);
            assert_eq!(index["portfolio-group"][0].decision_id, "decision-btc");
        }

        remove_scout_instance(&mut state, "decision-btc");
        let index = state
            .portfolio_instances_by_group
            .read()
            .unwrap_or_else(|error| error.into_inner());
        assert!(!index.contains_key("portfolio-group"));
    }

    #[tokio::test]
    async fn evidence_uses_only_prior_non_overlapping_samples_from_the_exact_profile() {
        let profile = BTreeMap::from([("minimum_momentum".into(), 0.2)]);
        let mut persisted = PersistedState::default();
        let rows = [
            outcome("prior-loss", profile.clone(), 100, 60_100, -0.1),
            outcome("overlap-win", profile.clone(), 30_000, 90_000, 0.2),
            outcome("other-profile", BTreeMap::from([("minimum_momentum".into(), 0.3)]), 100, 60_100, 1.0),
            outcome("future", profile.clone(), 80_000, 120_000, 1.0),
        ];
        for row in rows {
            persisted.outcomes.insert(scout_outcome_idempotency_key(&row), row);
        }
        let strategy_outcomes_by_profile = Arc::clone(&persisted.strategy_outcomes_by_profile);
        let portfolio_instances_by_group = Arc::clone(&persisted.portfolio_instances_by_group);
        let service = ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            auto_trader_rsocket_url: String::new(),
            strategy_outcomes_by_profile,
            portfolio_instances_by_group,
            strategy_outcome_profile_index_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(persisted)),
                path: PathBuf::from("unused"),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        };
        let response = get_strategy_walk_forward(
            State(service),
            Json(StrategyEvidenceRequest {
                candidates: vec![StrategyEvidenceCandidate {
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    signal_timestamp_ms: 70_000,
                    parameters: BTreeMap::from([
                        ("minimum_momentum".into(), 0.2),
                        ("scout_minimum_walk_forward_samples".into(), 10.0),
                        ("scout_minimum_positive_net_rate".into(), 0.45),
                    ]),
                }],
            }),
        )
        .await
        .unwrap()
        .0;

        let evidence = response[0].as_ref().unwrap();
        assert_eq!(evidence.sample_count, 1);
        assert_eq!(evidence.positive_net_rate, 0.0);
        assert_eq!(evidence.average_net_return_percent, -0.1);
        assert_eq!(evidence.last_sample_timestamp_ms, 60_100);
    }

    #[test]
    fn portfolio_strategy_evidence_combines_complete_weighted_candidate_batches() {
        let profile = BTreeMap::from([("trend_portfolio_lookback_days".into(), 90.0)]);
        let candidate = StrategyEvidenceCandidate {
            symbol: "BTC-USDT".into(),
            strategy: "risk-adjusted-trend-portfolio".into(),
            signal_timestamp_ms: 70_000,
            parameters: profile.clone(),
        };
        let mut btc = outcome("decision-btc", profile.clone(), 100, 60_100, 1.0);
        btc.strategy = candidate.strategy.clone();
        btc.sample_group_id = "portfolio-sample-1".into();
        let mut eth = btc.clone();
        eth.decision_id = "decision-eth".into();
        eth.symbol = "ETH-USDT".into();
        eth.net_return_percent = -0.5;
        let outcomes = vec![btc, eth];
        let instances = vec![
            portfolio_instance("decision-btc", profile.clone(), Some(0.5)),
            portfolio_instance(
                "decision-eth",
                BTreeMap::from([("trend_portfolio_lookback_days".into(), 90.0)]),
                Some(0.25),
            ),
        ];

        let outcomes_by_group = BTreeMap::from([(
            "portfolio-sample-1".into(),
            outcomes.iter().map(indexed_outcome).collect::<Vec<_>>(),
        )]);
        let instances_by_group = BTreeMap::from([(
            "portfolio-sample-1".into(),
            instances.clone(),
        )]);
        let evidence = portfolio_strategy_walk_forward(
            &candidate,
            &outcomes_by_group,
            &instances_by_group,
        )
        .unwrap();

        assert_eq!(evidence.sample_count, 1);
        assert_eq!(evidence.positive_net_rate, 1.0);
        assert!((evidence.average_net_return_percent - 0.375).abs() < 1e-9);

        let mut incomplete_profile = instances.clone();
        incomplete_profile[1].strategy_parameters =
            BTreeMap::from([("trend_portfolio_lookback_days".into(), 91.0)]);
        let incomplete_instances_by_group = BTreeMap::from([(
            "portfolio-sample-1".into(),
            incomplete_profile,
        )]);
        assert!(portfolio_strategy_walk_forward(
            &candidate,
            &outcomes_by_group,
            &incomplete_instances_by_group,
        )
        .is_none());

        let single_asset = vec![portfolio_instance(
            "decision-btc",
            profile,
            Some(0.5),
        )];
        let single_outcomes_by_group = BTreeMap::from([(
            "portfolio-sample-1".into(),
            outcomes[..1].iter().map(indexed_outcome).collect::<Vec<_>>(),
        )]);
        let single_instances_by_group = BTreeMap::from([(
            "portfolio-sample-1".into(),
            single_asset,
        )]);
        assert!(portfolio_strategy_walk_forward(
            &candidate,
            &single_outcomes_by_group,
            &single_instances_by_group,
        )
        .is_none());
    }

    #[tokio::test]
    async fn generic_signal_walk_forward_does_not_veto_a_strategy_candidate() {
        let persisted = PersistedState::default();
        let strategy_outcomes_by_profile = Arc::clone(&persisted.strategy_outcomes_by_profile);
        let portfolio_instances_by_group = Arc::clone(&persisted.portfolio_instances_by_group);
        let service = ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            auto_trader_rsocket_url: String::new(),
            strategy_outcomes_by_profile,
            portfolio_instances_by_group,
            strategy_outcome_profile_index_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(persisted)),
                path: PathBuf::from("unused"),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        };
        let response = select_scout_entry(
            State(service),
            Json(ScoutEntrySelectionRequest {
                candidates: vec![serde_json::json!({
                    "symbol":"BTC-USDT",
                    "strategy":"live-momentum",
                    "direction":"bullish",
                    "timestamp":70_000,
                    "strength":0.8,
                    "score":0.8,
                    "strategy_decision":{"strategy_parameters":{
                        "scout_minimum_walk_forward_samples":10.0,
                        "scout_minimum_positive_net_rate":0.45
                    },"scout_only":false}
                })],
                walk_forward: vec![serde_json::json!({
                    "symbol":"BTC-USDT",
                    "horizon_seconds":60,
                    "all_signal_long":{"sample_count":80,"average_net_return_percent":-1.0,"positive_net_rate":0.0}
                })],
                strategy_parameter_specs: BTreeMap::from([(
                    "live-momentum".into(),
                    strategies::scout_parameter_specs(strategies::Strategy::LiveMomentum),
                )]),
                active_scout_count: 1,
                scout_max_active_positions: 100,
                risk_appetite: 50.0,
            }),
        )
        .await
        .unwrap()
        .0;

        assert_eq!(response.candidate_index, Some(0));
    }
}

fn live_strategy_enters(
    strategy_name: &str,
    mut candidate: serde_json::Value,
    direction: &str,
    parameters: BTreeMap<String, f64>,
) -> bool {
    candidate["strategy"] = serde_json::json!(strategy_name);
    candidate["direction"] = serde_json::json!(direction);
    let Ok(input) = strategies::StrategySignal::from_candidate_json(candidate, parameters) else {
        return false;
    };
    input
        .strategy
        .is_some_and(|strategy| matches!(strategies::evaluate(strategy, &input), Ok(strategies::StrategyDecision::Enter)))
}

async fn select_scout_entry(
    State(service): State<ServiceState>,
    Json(request): Json<ScoutEntrySelectionRequest>,
) -> Result<Json<ScoutEntrySelectionResponse>, (StatusCode, String)> {
    if request.candidates.len() > 10_000
        || request.walk_forward.len() > 100_000
        || request.scout_max_active_positions > 10_000
        || !request.risk_appetite.is_finite()
        || request.candidates.iter().any(|candidate| {
            number(candidate, "strength").is_some_and(|value| !value.is_finite())
                || number(candidate, "score").is_some_and(|value| !value.is_finite())
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid Scout entry selection request".into(),
        ));
    }
    if request.active_scout_count >= request.scout_max_active_positions {
        return Ok(Json(ScoutEntrySelectionResponse {
            candidate_index: None,
        }));
    }
    let density = request
        .active_scout_count
        .min(request.scout_max_active_positions) as f64;
    let risk_scale = 1.5 - request.risk_appetite.clamp(0.0, 100.0) / 100.0;
    let strength_floor = (density * 0.025 * risk_scale).clamp(0.0, 1.0);
    let score_floor = ((0.05 + density * 0.05) * risk_scale).max(0.0);
    let state = service.scout.data.lock().await;
    let prior_outcomes = state.outcomes.values().collect::<Vec<_>>();
    let selected = request
        .candidates
        .iter()
        .enumerate()
        .filter_map(|(index, signal)| {
            let direction = signal
                .get("direction")
                .and_then(serde_json::Value::as_str)?;
            if !matches!(direction, "bullish" | "bearish") {
                return None;
            }
            let strength = number(signal, "strength")?;
            let score = number(signal, "score")?;
            let strategy = signal
                .get("strategy")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let live_scout_only = signal
                .get("strategy_decision")
                .and_then(|decision| decision.get("scout_only"))
                .and_then(serde_json::Value::as_bool)
                == Some(true)
                && strategy.starts_with("live-")
                && strategy != "unattributed";
            let (minimum_samples, positive_floor) =
                scout_walk_forward_thresholds(signal, strategy, &request.strategy_parameter_specs)?;
            if !live_scout_only && (strength < strength_floor || score.abs() < score_floor) {
                return None;
            }
            let evidence = signal
                .get("strategy")
                .and_then(serde_json::Value::as_str)
                .zip(signal.get("symbol").and_then(serde_json::Value::as_str))
                .zip(signal.get("signal_timestamp").or_else(|| signal.get("timestamp"))
                    .and_then(serde_json::Value::as_u64))
                .and_then(|((strategy, symbol), signal_timestamp_ms)| {
                    let parameters = signal.get("strategy_decision")?.get("strategy_parameters")?
                        .clone();
                    let parameters = serde_json::from_value::<BTreeMap<String, f64>>(parameters).ok()?;
                    Some(StrategyEvidenceCandidate {
                        symbol: symbol.to_owned(),
                        strategy: strategy.to_owned(),
                        signal_timestamp_ms,
                        parameters,
                    })
                })
                .and_then(|candidate| {
                    if candidate.strategy == "risk-adjusted-trend-portfolio" {
                        return None;
                    }
                    let matching = prior_outcomes
                        .iter()
                        .copied()
                        .filter(|outcome| {
                            outcome.strategy == candidate.strategy
                                && outcome.symbol == candidate.symbol
                                && outcome.exit_scenario == "strategy"
                                && same_strategy_parameters(
                                    &outcome.strategy_parameters,
                                    &candidate.parameters,
                                )
                        })
                        .collect::<Vec<_>>();
                    strategy_walk_forward_for_candidate(&candidate, &matching)
                });
            let measured_negative = evidence.is_some_and(|evidence| {
                evidence.sample_count >= minimum_samples
                    && (evidence.average_net_return_percent <= 0.0
                        || evidence.positive_net_rate < positive_floor)
            });
            (!measured_negative || request.active_scout_count == 0).then_some((index, strength))
        })
        .max_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(index, _)| index);
    Ok(Json(ScoutEntrySelectionResponse {
        candidate_index: selected,
    }))
}

fn scout_walk_forward_thresholds(
    signal: &serde_json::Value,
    strategy: &str,
    specs_by_strategy: &BTreeMap<String, Vec<ParameterSpec>>,
) -> Option<(usize, f64)> {
    let parameters = signal
        .get("strategy_decision")?
        .get("strategy_parameters")?;
    let specs = specs_by_strategy.get(strategy)?;
    let read_bounded = |key: &str| {
        let spec = specs.iter().find(|spec| spec.key == key)?;
        let value = parameters.get(key)?.as_f64()?;
        let position = (value - spec.min) / spec.step;
        (value.is_finite()
            && value >= spec.min
            && value <= spec.max
            && (position - position.round()).abs() <= 1e-6)
            .then_some(value)
    };
    Some((
        read_bounded("scout_minimum_walk_forward_samples")?.round() as usize,
        read_bounded("scout_minimum_positive_net_rate")?,
    ))
}

fn number(value: &serde_json::Value, key: &str) -> Option<f64> {
    value
        .get(key)
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
}

fn parameter(profile: &LiveScoutProfile, key: &str) -> Option<f64> {
    profile
        .parameters
        .get(key)
        .copied()
        .filter(|value| value.is_finite())
}

fn live_scout_feature_signal(
    source: &serde_json::Value,
    profile: &LiveScoutProfile,
) -> Option<serde_json::Value> {
    let mut configured = source.clone();
    let components = configured.get_mut("components")?.as_object_mut()?;
    let source_components = source.get("components")?;
    let window_value = |key: &str, field: &str| -> Option<&serde_json::Value> {
        let window = parameter(profile, key)? as u64;
        source_components
            .get(field)?
            .as_array()?
            .iter()
            .find(|item| {
                item.get("window_seconds")
                    .and_then(serde_json::Value::as_u64)
                    == Some(window)
            })
    };
    let set_optional = |target: &mut serde_json::Map<String, serde_json::Value>,
                        name: &str,
                        value: Option<f64>| {
        target.insert(
            name.to_owned(),
            value
                .filter(|value| value.is_finite())
                .map_or(serde_json::Value::Null, |value| serde_json::json!(value)),
        );
    };
    match profile.strategy.as_str() {
        "live-momentum" => {
            set_optional(
                components,
                "momentum_10s_percent",
                window_value("momentum_lookback_seconds", "momentum_by_window")
                    .and_then(|item| number(item, "value")),
            );
            set_optional(
                components,
                "trend_5m_volatility_units",
                window_value("trend_lookback_seconds", "trend_volatility_units_by_window")
                    .and_then(|item| number(item, "value")),
            );
        }
        "live-trade-flow" => set_optional(
            components,
            "trade_flow_imbalance",
            window_value("trade_flow_window_seconds", "trade_flow_by_window")
                .and_then(|item| number(item, "value")),
        ),
        "live-order-book" => {
            let depth = parameter(profile, "book_depth_levels")? as u64;
            let imbalance = source_components
                .get("book_imbalance_by_depth")?
                .as_array()?
                .iter()
                .find(|item| {
                    item.get("depth_levels").and_then(serde_json::Value::as_u64) == Some(depth)
                })
                .and_then(|item| number(item, "value"));
            set_optional(components, "book_imbalance", imbalance);
            set_optional(
                components,
                "momentum_10s_percent",
                window_value("momentum_lookback_seconds", "momentum_by_window")
                    .and_then(|item| number(item, "value")),
            );
        }
        "live-vwap-dislocation" => {
            let window = parameter(profile, "vwap_lookback_seconds")? as u64;
            let feature = source_components
                .get("vwap_dislocation_by_window")?
                .as_array()?
                .iter()
                .find(|item| {
                    item.get("window_seconds")
                        .and_then(serde_json::Value::as_u64)
                        == Some(window)
                });
            let vwap = feature
                .filter(|item| {
                    item.get("sample_count")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or_default()
                        >= parameter(profile, "vwap_minimum_trade_count").unwrap_or(f64::INFINITY)
                            as u64
                        && item
                            .get("span_ms")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or_default()
                            >= (parameter(profile, "vwap_minimum_trade_span_seconds")
                                .unwrap_or(f64::INFINITY)
                                * 1_000.0) as u64
                })
                .and_then(|item| number(item, "value"));
            set_optional(components, "mispricing_vs_vwap_bps", vwap);
            set_optional(
                components,
                "momentum_10s_percent",
                window_value("momentum_lookback_seconds", "momentum_by_window")
                    .and_then(|item| number(item, "value")),
            );
        }
        "live-absorption-pressure" => {
            let momentum = window_value("momentum_lookback_seconds", "momentum_by_window")
                .and_then(|item| number(item, "value"))?;
            if momentum.abs() > parameter(profile, "absorption_maximum_price_move_percent")? {
                return None;
            }
            let window = parameter(profile, "absorption_lookback_seconds")? as u64;
            let replenishment_ms =
                (parameter(profile, "absorption_replenishment_seconds")? * 1_000.0).round() as u64;
            let tolerance = parameter(profile, "absorption_touch_tolerance_bps")?;
            let pressure = source_components
                .get("absorption_pressure_by_window")?
                .as_array()?
                .iter()
                .find(|item| {
                    item.get("window_seconds")
                        .and_then(serde_json::Value::as_u64)
                        == Some(window)
                        && item
                            .get("replenishment_window_ms")
                            .and_then(serde_json::Value::as_u64)
                            == Some(replenishment_ms)
                        && item
                            .get("touch_tolerance_bps")
                            .and_then(serde_json::Value::as_f64)
                            .is_some_and(|value| (value - tolerance).abs() < 1e-6)
                })
                .and_then(|item| number(item, "value"));
            let flow_window = parameter(profile, "trade_flow_window_seconds")? as u64;
            let minimum_flow = parameter(profile, "absorption_minimum_flow_imbalance")?;
            let flow = source_components
                .get("trade_flow_by_window")?
                .as_array()?
                .iter()
                .find(|item| {
                    item.get("window_seconds")
                        .and_then(serde_json::Value::as_u64)
                        == Some(flow_window)
                })
                .and_then(|item| number(item, "value"))
                .filter(|flow| flow.abs() >= minimum_flow)
                .map(|flow| -flow);
            set_optional(components, "momentum_10s_percent", Some(momentum));
            set_optional(components, "pressure_without_price_move", pressure.or(flow));
        }
        _ => return None,
    }
    for key in [
        "momentum_by_window",
        "trend_volatility_units_by_window",
        "trade_flow_by_window",
        "book_imbalance_by_depth",
        "vwap_dislocation_by_window",
        "absorption_pressure_by_window",
    ] {
        components.insert(key.to_owned(), serde_json::Value::Array(Vec::new()));
    }
    Some(configured)
}

#[cfg(test)]
mod scout_markout_instance_tests {
    use super::*;

    #[tokio::test]
    async fn markout_observation_loads_and_updates_instance_after_restart() {
        let path = std::env::temp_dir().join(format!(
            "scout-markout-restart-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let mut persisted = PersistedState::default();
        persisted.scout_instances.insert(
            "decision-restart".into(),
            serde_json::json!({
                "decision_id":"decision-restart",
                "symbol":"BTC-USDT",
                "strategy":"live-trade-flow",
                "direction":"bullish",
                "signal_timestamp":100,
                "scout_sample_group_id":"restart-group"
            }),
        );
        persist_scout_state(&path, &persisted, &["scout_instances"]).unwrap();
        let restarted = load_scout_state(&path).unwrap();
        assert!(restarted.scout_instances.is_empty());
        let service = ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            auto_trader_rsocket_url: String::new(),
            strategy_outcomes_by_profile: Arc::clone(&restarted.strategy_outcomes_by_profile),
            portfolio_instances_by_group: Arc::clone(&restarted.portfolio_instances_by_group),
            strategy_outcome_profile_index_ready: Arc::new(
                std::sync::atomic::AtomicBool::new(false),
            ),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(restarted)),
                path: path.clone(),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            },
        };
        record_scout_markout_observation(
            &service,
            "decision-restart",
            serde_json::json!({"kind":"horizon","horizon_seconds":60}),
        )
        .await
        .unwrap();
        let updated = load_scout_instance(&path, "decision-restart")
            .unwrap()
            .unwrap();
        assert_eq!(updated["scout_markout_observations"][0]["kind"], "horizon");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn parameter_variant_generation_skips_confidently_rejected_values() {
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.1,
            max: 0.9,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let request = ParameterVariantsRequest {
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            specs: vec![spec],
            day: 0,
            rejected_values: BTreeMap::from([("threshold".into(), vec![0.6])]),
        };
        let variants = build_parameter_variants(&request).unwrap();
        assert!(variants.iter().all(|variant| {
            variant.changes.get("threshold") != Some(&0.6)
        }));
    }

    #[test]
    fn cooldown_variants_share_their_parent_entry_evidence() {
        let parent = BTreeMap::from([
            ("minimum_momentum_percent".into(), 0.04),
            ("post_stop_cooldown_minutes".into(), 2.0),
        ]);
        let cooldown_variant = BTreeMap::from([
            ("minimum_momentum_percent".into(), 0.04),
            ("post_stop_cooldown_minutes".into(), 4.0),
        ]);
        let entry_variant = BTreeMap::from([
            ("minimum_momentum_percent".into(), 0.06),
            ("post_stop_cooldown_minutes".into(), 4.0),
        ]);

        assert!(same_strategy_parameters(&parent, &cooldown_variant));
        assert!(!same_strategy_parameters(&parent, &entry_variant));
    }

    #[test]
    fn scouts_generate_bounded_early_exit_policy_probes() {
        let specs = [
            ("thesis_failure_loss_activation_percent", 0.20, 0.10, 0.75, 0.05),
            ("thesis_failure_confirmation_samples", 2.0, 1.0, 4.0, 1.0),
            ("thesis_failure_minimum_signal_strength", 0.60, 0.40, 0.90, 0.05),
        ]
        .into_iter()
        .map(|(key, default, min, max, step)| ParameterSpec {
            key: key.into(),
            label: key.into(),
            default,
            min,
            max,
            step,
            unit: "".into(),
            learning_direction: 0.0,
        })
        .collect::<Vec<_>>();
        let parameters = specs
            .iter()
            .map(|spec| (spec.key.clone(), spec.default))
            .collect();

        let variants = single_parameter_variants(&parameters, &specs, 0).unwrap();
        assert_eq!(variants.len(), 6);
        assert!(variants.iter().all(|changes| {
            changes.len() == 1
                && changes.keys().all(|key| key.starts_with("thesis_failure_"))
        }));
    }

    #[test]
    fn markout_uses_the_scout_instance_stored_by_this_service() {
        let instance = serde_json::json!({
            "decision_id":"decision-1", "symbol":"BTC-USDT", "strategy":"live-trade-flow",
            "direction":"bullish", "signal_timestamp":10, "scout_notional_usd":100.0, "scout_learning_context":"scout:live:test",
            "strategy_parameters":{"stop_loss_percent":0.5}, "scout_exit_scenario":"strategy",
            "scout_sample_group_id":"group-1", "strategy_parameter_specs":[]
        });
        let outcome = scout_outcome_from_markout(&instance, ScoutMarkoutRequest {
            decision_id:"decision-1".into(), symbol:"BTC-USDT".into(), signal_timestamp_ms:10,
            evaluated_timestamp_ms:20, taker_fee_rate:0.001, long_net_percent:Some(0.25), short_net_percent:Some(-0.3),
            ..ScoutMarkoutRequest::default()
        }).unwrap();
        assert_eq!(outcome.net_return_percent, 0.25);
        assert_eq!(outcome.simulated_net_pnl_usd, 0.25);
        assert_eq!(outcome.strategy, "live-trade-flow");
        assert!(scout_outcome_from_markout(&instance, ScoutMarkoutRequest {
            decision_id:"other".into(), symbol:"ETH-USDT".into(), signal_timestamp_ms:10,
            evaluated_timestamp_ms:20, taker_fee_rate:0.001, long_net_percent:Some(0.25), short_net_percent:None,
            ..ScoutMarkoutRequest::default()
        }).is_none());
    }

    #[test]
    fn horizon_markout_requires_a_completed_supported_window() {
        let instance = serde_json::json!({
            "decision_id":"decision-horizon",
            "symbol":"BTC-USDT",
            "direction":"bullish",
            "signal_timestamp":10
        });
        let request = || ScoutMarkoutRequest {
            decision_id: "decision-horizon".into(),
            symbol: "BTC-USDT".into(),
            signal_timestamp_ms: 10,
            evaluated_timestamp_ms: 60_010,
            taker_fee_rate: 0.001,
            observation_only: true,
            horizon_seconds: 60,
            long_net_percent: Some(0.2),
            short_net_percent: Some(-0.3),
            reference_entry_price: Some(100.0),
            reference_exit_price: Some(100.3),
            long_gross_percent: Some(0.3),
            short_gross_percent: Some(-0.3),
            ..ScoutMarkoutRequest::default()
        };
        let observation = scout_markout_observation(&instance, &request()).unwrap();
        assert_eq!(observation["kind"], "horizon");
        assert_eq!(observation["horizon_seconds"], 60);
        assert_eq!(observation["long_net_percent"], 0.2);
        let evaluated_at_ms = scout_now_ms();
        let signal_at_ms = evaluated_at_ms.saturating_sub(60_000);
        let completed_instance = serde_json::json!({
            "decision_id":"decision-horizon",
            "symbol":"BTC-USDT",
            "strategy":"live-momentum",
            "direction":"bullish",
            "signal_timestamp":signal_at_ms,
            "scout_notional_usd":100.0,
            "scout_learning_context":"scout:live:momentum",
            "scout_exit_scenario":"strategy",
            "scout_sample_group_id":"group-1",
            "scout_net_return_percent":1.5
        });
        let mut learning_request = request();
        learning_request.signal_timestamp_ms = signal_at_ms;
        learning_request.evaluated_timestamp_ms = evaluated_at_ms;
        let horizon_outcome =
            scout_outcome_from_markout(&completed_instance, learning_request).unwrap();
        assert_eq!(horizon_outcome.net_return_percent, 0.2);
        assert_eq!(horizon_outcome.exit_scenario, "horizon-60");
        assert!(horizon_outcome.sample_group_id.is_empty());
        assert!(validate_scout_outcome(&horizon_outcome).is_ok());
        assert!(scout_markout_observation(
            &instance,
            &ScoutMarkoutRequest {
                evaluated_timestamp_ms: 60_009,
                ..request()
            }
        )
        .is_none());
    }

    #[test]
    fn strategy_markout_uses_the_replayed_early_exit_return_when_triggered() {
        let mut instance = serde_json::json!({
            "decision_id":"decision-early", "symbol":"BTC-USDT", "strategy":"live-momentum",
            "direction":"bullish", "signal_timestamp":10, "scout_notional_usd":100.0,
            "scout_learning_context":"scout:live:momentum", "strategy_parameters":{},
            "scout_exit_scenario":"strategy", "scout_thesis_failure_exit_net_return_percent":-0.35
        });
        let request = || ScoutMarkoutRequest {
            decision_id: "decision-early".into(),
            symbol: "BTC-USDT".into(),
            signal_timestamp_ms: 10,
            evaluated_timestamp_ms: 500,
            taker_fee_rate: 0.001,
            long_net_percent: Some(-1.0),
            short_net_percent: None,
            ..ScoutMarkoutRequest::default()
        };

        let outcome = scout_outcome_from_markout(&instance, request()).unwrap();
        assert_eq!(outcome.net_return_percent, -0.35);
        assert_eq!(outcome.simulated_net_pnl_usd, -0.35);

        instance["scout_exit_scenario"] = serde_json::json!("stop");
        assert_eq!(
            scout_outcome_from_markout(&instance, request())
                .unwrap()
                .net_return_percent,
            -1.0
        );
    }

    #[test]
    fn one_baseline_markout_fans_out_to_each_variant_comparison() {
        let instance = serde_json::json!({
            "decision_id":"baseline-1", "symbol":"BTC-USDT", "strategy":"test-strategy",
            "direction":"bullish", "signal_timestamp":10, "scout_notional_usd":100.0,
            "scout_learning_context":"scout:daily:test", "strategy_parameters":{"threshold":0.5},
            "scout_exit_scenario":"strategy", "scout_comparison_role":"baseline",
            "scout_comparison_ids":["pair-a", "pair-b", "pair-c"]
        });
        let outcome = scout_outcome_from_markout(&instance, ScoutMarkoutRequest {
            decision_id:"baseline-1".into(), symbol:"BTC-USDT".into(), signal_timestamp_ms:10,
            evaluated_timestamp_ms:20, taker_fee_rate:0.001, long_net_percent:Some(0.25), short_net_percent:None,
            ..ScoutMarkoutRequest::default()
        }).unwrap();
        assert_eq!(outcome_comparison_ids(&outcome), ["pair-a", "pair-b", "pair-c"]);
    }
}
