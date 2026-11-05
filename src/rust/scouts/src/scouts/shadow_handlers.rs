async fn receive_shadow_outcome(
    State(service): State<ServiceState>,
    Json(outcome): Json<ShadowOutcomeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let tested = validate_shadow_outcome(&outcome)?;
    {
        let mut state = service.shadow.data.lock().await;
        let outcome_key = shadow_outcome_idempotency_key(&outcome);
        if !state.seen.contains(&outcome_key) {
            let event = outcome_event(&outcome, &tested);
            let is_probe = outcome.scout_parameter_key.is_some();
            if !is_probe && !outbox_has_learning(&state, &event) {
                state.outbox.push(ShadowOutboxItem::Learning(event));
            }
            let mut paired_delta = None;
            if let (Some(comparison_id), Some(role)) = (
                outcome.scout_comparison_id.as_deref(),
                outcome.scout_comparison_role.as_deref(),
            ) {
                let pair = state.pairs.entry(comparison_id.to_owned()).or_default();
                pair.updated_at_ms = outcome.evaluated_timestamp_ms;
                match role {
                    "baseline" => pair.baseline = Some(outcome.clone()),
                    "variant" => pair.variant = Some(outcome.clone()),
                    _ => unreachable!(),
                }
                if let Some(zero_role) = outcome.scout_comparison_zero_role.as_deref() {
                    let mut zero = outcome.clone();
                    zero.scout_comparison_role = Some(zero_role.to_owned());
                    zero.scout_comparison_zero_role = None;
                    zero.net_return_percent = 0.0;
                    if zero_role == "variant" && zero.scout_parameter_key.is_none() {
                        zero.scout_parameter_key = zero.scout_comparison_parameter_key.clone();
                        zero.scout_parameter_value = zero.scout_comparison_parameter_value;
                        zero.scout_parameter_profile_symbol = zero
                            .scout_comparison_parameter_profile_symbol
                            .clone()
                            .or_else(|| zero.scout_parameter_profile_symbol.clone());
                    }
                    if zero_role == "baseline" {
                        pair.baseline = Some(zero);
                    } else {
                        pair.variant = Some(zero);
                    }
                }
                if let Some(event) = paired_event_with_legacy_side(comparison_id, pair, &outcome) {
                    if let FeedbackEvent::ScoutOutcome {
                        net_return_percent, ..
                    } = &event
                    {
                        paired_delta = Some(*net_return_percent);
                    }
                    if !outbox_has_learning(&state, &event) {
                        state.outbox.push(ShadowOutboxItem::Learning(event));
                    }
                    state.pairs.remove(comparison_id);
                }
            }
            state.pairs.retain(|_, pair| {
                outcome
                    .evaluated_timestamp_ms
                    .saturating_sub(pair.updated_at_ms)
                    <= 7 * 86_400_000
            });
            while state.pairs.len() > 20_000 {
                let Some(oldest) = state
                    .pairs
                    .iter()
                    .min_by_key(|(_, pair)| pair.updated_at_ms)
                    .map(|(key, _)| key.clone())
                else {
                    break;
                };
                state.pairs.remove(&oldest);
            }
            state.outcomes.insert(outcome_key.clone(), outcome.clone());
            while state.outcomes.len() > 100_000 {
                if let Some(oldest) = state.outcomes.keys().next().cloned() {
                    state.outcomes.remove(&oldest);
                }
            }
            if !outcome.sample_group_id.is_empty() {
                let finished = state
                    .shadow_sample_active_scenarios
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
                        .shadow_sample_active_scenarios
                        .remove(&outcome.sample_group_id);
                    state.shadow_plan_profiles.remove(&outcome.sample_group_id);
                    state.shadow_active_audits.remove(&outcome.sample_group_id);
                }
            }
            state.shadow_lifecycle_updated_at_ms =
                state.shadow_lifecycle_updated_at_ms.max(shadow_now_ms());
            state
                .outbox
                .push(ShadowOutboxItem::Audit(shadow_outcome_audit(
                    &outcome,
                    paired_delta,
                )));
            state.seen.insert(outcome_key);
            while state.seen.len() > 100_000 {
                if let Some(oldest) = state.seen.iter().next().cloned() {
                    state.seen.remove(&oldest);
                }
            }
            persist_shadow_state(&service.shadow.path, &state)?;
        }
    }
    flush_shadow_outbox(&service.shadow, &service.client, &service.learning_url).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_shadow_lifecycle_state(
    State(service): State<ServiceState>,
) -> Json<ShadowLifecycleSnapshot> {
    let state = service.shadow.data.lock().await;
    Json(ShadowLifecycleSnapshot {
        updated_at_ms: state.shadow_lifecycle_updated_at_ms,
        shadow_sample_at_ms: state.shadow_sample_at_ms.clone(),
        shadow_sample_active_scenarios: state.shadow_sample_active_scenarios.clone(),
        scout_variant_rotation_cursor: state.scout_variant_rotation_cursor,
    })
}

async fn set_shadow_lifecycle_state(
    State(service): State<ServiceState>,
    Json(snapshot): Json<ShadowLifecycleSnapshot>,
) -> Result<StatusCode, (StatusCode, String)> {
    if snapshot.shadow_sample_at_ms.len() > 100_000
        || snapshot.shadow_sample_active_scenarios.len() > 10_000
        || snapshot.scout_variant_rotation_cursor > 1_000_000_000
        || snapshot
            .shadow_sample_at_ms
            .keys()
            .any(|key| key.is_empty() || key.len() > 512)
        || snapshot
            .shadow_sample_active_scenarios
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
    let mut state = service.shadow.data.lock().await;
    if snapshot.updated_at_ms < state.shadow_lifecycle_updated_at_ms {
        return Ok(StatusCode::NO_CONTENT);
    }
    state.shadow_sample_at_ms = snapshot.shadow_sample_at_ms;
    state.shadow_sample_active_scenarios = snapshot.shadow_sample_active_scenarios;
    state.scout_variant_rotation_cursor = snapshot.scout_variant_rotation_cursor;
    state.shadow_lifecycle_updated_at_ms = snapshot.updated_at_ms;
    persist_shadow_state(&service.shadow.path, &state)?;
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

fn apply_shadow_lifecycle_observation(
    state: &mut PersistedState,
    request: &ShadowLifecycleObserveRequest,
) -> ShadowLifecycleObserveResponse {
    if let Some(response) = state.shadow_observation_results.get(&request.request_id) {
        return response.clone();
    }
    let mut finalized_count = 0usize;
    let ids = state
        .shadow_pending_samples
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for id in ids {
        let Some(ShadowSamplePayload::DatedCarryProbe(probe)) =
            state.shadow_pending_samples.get(&id).cloned()
        else {
            continue;
        };
        let age = request.now_ms.saturating_sub(probe.opened_at_ms);
        if age > 3 * 86_400_000 {
            state.shadow_pending_samples.remove(&id);
            state.shadow_sample_active_scenarios.remove(&id);
            state.shadow_plan_profiles.remove(&id);
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
            specs,
            net_return_percent,
        };
        if !outbox_has_learning(state, &event) {
            state.outbox.push(ShadowOutboxItem::Learning(event));
        }
        state.shadow_pending_samples.remove(&id);
        state.shadow_sample_active_scenarios.remove(&id);
        state.shadow_plan_profiles.remove(&id);
        finalized_count += 1;
    }
    let response = ShadowLifecycleObserveResponse { finalized_count };
    state
        .shadow_observation_results
        .insert(request.request_id.clone(), response.clone());
    while state.shadow_observation_results.len() > 10_000 {
        if let Some(oldest) = state.shadow_observation_results.keys().next().cloned() {
            state.shadow_observation_results.remove(&oldest);
        }
    }
    state.shadow_lifecycle_updated_at_ms = state.shadow_lifecycle_updated_at_ms.max(request.now_ms);
    response
}

async fn observe_shadow_lifecycle(
    State(service): State<ServiceState>,
    Json(request): Json<ShadowLifecycleObserveRequest>,
) -> Result<Json<ShadowLifecycleObserveResponse>, (StatusCode, String)> {
    if request.request_id.is_empty()
        || request.request_id.len() > 240
        || request.candidates.len() > 10_000
        || request.now_ms > shadow_now_ms().saturating_add(5_000)
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
    let mut state = service.shadow.data.lock().await;
    if let Some(response) = state.shadow_observation_results.get(&request.request_id) {
        let response = response.clone();
        drop(state);
        flush_shadow_outbox(&service.shadow, &service.client, &service.learning_url).await?;
        return Ok(Json(response));
    }
    let response = apply_shadow_lifecycle_observation(&mut state, &request);
    persist_shadow_state(&service.shadow.path, &state)?;
    drop(state);
    flush_shadow_outbox(&service.shadow, &service.client, &service.learning_url).await?;
    Ok(Json(response))
}

async fn create_shadow_plan(
    State(service): State<ServiceState>,
    Json(request): Json<ShadowPlanRequest>,
) -> Result<Json<ShadowPlanResponse>, (StatusCode, String)> {
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
        || request.now_ms > shadow_now_ms().saturating_add(5_000)
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
                            .get("shadow_exit_scenario")
                            .and_then(serde_json::Value::as_str)
                            .is_none_or(|scenario| {
                                !matches!(scenario, "strategy" | "tight" | "wide")
                            })
                })
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid Scout shadow-plan request".into(),
        ));
    }
    let mut state = service.shadow.data.lock().await;
    for (key, timestamp) in &request.legacy_cooldowns {
        state
            .shadow_sample_at_ms
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
    if let Some(response) = state.shadow_plan_results.get(&request.request_id) {
        let response = response.clone();
        if !request.legacy_cooldowns.is_empty() || !request.legacy_comparisons.is_empty() {
            persist_shadow_state(&service.shadow.path, &state)?;
        }
        drop(state);
        flush_shadow_outbox(&service.shadow, &service.client, &service.learning_url).await?;
        return Ok(Json(response));
    }
    let mut active_batch_keys = state
        .shadow_sample_active_scenarios
        .keys()
        .map(|key| key.split("|probe:").next().unwrap_or(key))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let cooldowns_at_request_start = state.shadow_sample_at_ms.clone();
    let start = if request.candidates.is_empty() {
        0
    } else {
        state.scout_variant_rotation_cursor % request.candidates.len()
    };
    if !request.candidates.is_empty() {
        state.scout_variant_rotation_cursor = (start + 1) % request.candidates.len();
    }
    let expanded_candidates = expand_daily_shadow_candidates(&request.candidates);
    let mut response = ShadowPlanResponse::default();
    for offset in 0..expanded_candidates.len() {
        let mut candidate =
            expanded_candidates[(start + offset) % expanded_candidates.len()].clone();
        if candidate.generate_group_id {
            let Some((group_id, cooldown_key)) = generated_shadow_group_and_cooldown(&candidate)
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
                .shadow_sample_active_scenarios
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
                ShadowSamplePayload::DatedCarryProbe(probe) => {
                    probe.sample_id != candidate.sample_group_id
                        || probe.opened_at_ms > request.now_ms.saturating_add(5_000)
                        || probe.strategy.is_empty()
                        || probe.profile_symbol.is_empty()
                        || probe
                            .parameters
                            .values()
                            .chain(probe.tested_parameters.values())
                            .any(|v| !v.is_finite())
                        || !probe.parameter_value.is_finite()
                        || if probe.parameter_key.starts_with("combo:") {
                            probe.parameter_value != 0.0
                                || parse_combination(&probe.parameter_key, &probe.parameter_specs)
                                    .as_ref()
                                    != Some(&probe.tested_parameters)
                        } else {
                            probe.tested_parameters.len() != 1
                                || probe.tested_parameters.get(&probe.parameter_key)
                                    != Some(&probe.parameter_value)
                        }
                        || probe.tested_parameters.iter().any(|(key, value)| {
                            let Some(spec) =
                                probe.parameter_specs.iter().find(|spec| spec.key == *key)
                            else {
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
                }
            })
        {
            continue;
        }
        if candidate
            .sample_payload
            .as_ref()
            .is_some_and(|payload| match payload {
                ShadowSamplePayload::DatedCarryProbe(probe) => {
                    state.shadow_pending_samples.contains_key(&probe.sample_id)
                }
            })
        {
            continue;
        }
        if !shadow_comparison_changes_exit(&candidate) {
            continue;
        }
        if candidate.parameter_variant && !candidate.profile_key.is_empty() {
            let active_variants = state
                .shadow_plan_profiles
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
            .flat_map(|template| build_shadow_audits(template, &candidate.sample_group_id))
            .collect::<Vec<_>>();
        let plans = candidate
            .audit_plans
            .iter()
            .cloned()
            .chain(generated)
            .collect::<Vec<_>>();
        for source in &plans {
            let mut audit = source.clone();
            audit["shadow_sample_group_id"] = serde_json::json!(candidate.sample_group_id);
            audit["scout"] = serde_json::json!(true);
            let decision_id = audit["decision_id"].as_str().unwrap_or_default();
            let scenario = audit["shadow_exit_scenario"].as_str().unwrap_or_default();
            if let Some(registration) = scout_shadow_registration(&audit) {
                tokens.insert(format!("{decision_id}|{scenario}"));
                active_audits.push(audit.clone());
                state.outbox.push(ShadowOutboxItem::Audit(audit.clone()));
                state
                    .outbox
                    .push(ShadowOutboxItem::Registration(registration));
            }
            response.audits.push(audit);
        }
        if let Some(ShadowSamplePayload::DatedCarryProbe(probe)) = &candidate.sample_payload {
            state.shadow_pending_samples.insert(
                probe.sample_id.clone(),
                ShadowSamplePayload::DatedCarryProbe(probe.clone()),
            );
            tokens.insert(format!("{}|carry", probe.sample_id));
        }
        state
            .shadow_sample_at_ms
            .insert(candidate.cooldown_key.clone(), request.now_ms);
        if !tokens.is_empty() && candidate.parameter_variant && !candidate.profile_key.is_empty() {
            state.shadow_plan_profiles.insert(
                candidate.sample_group_id.clone(),
                candidate.profile_key.clone(),
            );
        }
        if !active_audits.is_empty() {
            state
                .shadow_active_audits
                .insert(candidate.sample_group_id.clone(), active_audits);
        }
        if !tokens.is_empty() {
            state
                .shadow_sample_active_scenarios
                .insert(candidate.sample_group_id.clone(), tokens);
        }
        response
            .selected_groups
            .push(candidate.sample_group_id.clone());
        active_batch_keys.insert(batch_key);
    }
    state.shadow_lifecycle_updated_at_ms = state.shadow_lifecycle_updated_at_ms.max(request.now_ms);
    state
        .shadow_plan_results
        .insert(request.request_id.clone(), response.clone());
    while state.shadow_plan_results.len() > 10_000 {
        if let Some(oldest) = state.shadow_plan_results.keys().next().cloned() {
            state.shadow_plan_results.remove(&oldest);
        }
    }
    persist_shadow_state(&service.shadow.path, &state)?;
    drop(state);
    flush_shadow_outbox(&service.shadow, &service.client, &service.learning_url).await?;
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
    service
        .client
        .post(format!(
            "{}/feedback",
            service.learning_url.trim_end_matches('/')
        ))
        .json(event)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map(|_| ())
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("learning service rejected Scout feedback: {error}"),
            )
        })
}

#[derive(Deserialize)]
struct FinalizeRequest {
    sample: ScoutSample,
    closed_at_ms: u64,
    exit_price: f64,
    cost_percent: f64,
}

impl ScoutSample {
    /// Finalize a shadow sample using the observed exit price and round-trip costs.
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
    Json(request): Json<ScoutEntrySelectionRequest>,
) -> Result<Json<ScoutEntrySelectionResponse>, (StatusCode, String)> {
    if request.candidates.len() > 10_000
        || request.walk_forward.len() > 100_000
        || request.scout_max_active_positions > 5_000
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
                && strategy != "live-insights";
            let (minimum_samples, side_minimum_samples, positive_floor) =
                scout_walk_forward_thresholds(signal, strategy, &request.strategy_parameter_specs)?;
            if !live_scout_only && (strength < strength_floor || score.abs() < score_floor) {
                return None;
            }
            let evidence = request.walk_forward.iter().find(|row| {
                row.get("symbol").and_then(serde_json::Value::as_str)
                    == signal.get("symbol").and_then(serde_json::Value::as_str)
                    && row
                        .get("horizon_seconds")
                        .and_then(serde_json::Value::as_u64)
                        .is_some_and(|seconds| (60..=300).contains(&seconds))
            });
            let Some(evidence) = evidence else {
                return Some((index, strength));
            };
            let side_key = if direction == "bullish" {
                "long"
            } else {
                "short"
            };
            let side = evidence.get(format!("all_signal_{side_key}"))?;
            let research = evidence.get(format!("research_logistic_{side_key}"));
            let negative_at = |metrics: &serde_json::Value, minimum| {
                metrics
                    .get("sample_count")
                    .and_then(serde_json::Value::as_u64)
                    .unwrap_or_default() as usize
                    >= minimum
                    && (number(metrics, "average_net_return_percent").is_none_or(|net| net <= 0.0)
                        || number(metrics, "positive_net_rate")
                            .is_none_or(|rate| rate < positive_floor))
            };
            let measured_negative = negative_at(side, minimum_samples)
                || research.is_some_and(|metrics| negative_at(metrics, side_minimum_samples));
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
) -> Option<(usize, usize, f64)> {
    let parameters = signal
        .get("strategy_decision")?
        .get("strategy_parameters")?;
    let live_scout_only = signal
        .get("strategy_decision")
        .and_then(|decision| decision.get("scout_only"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && strategy.starts_with("live-")
        && strategy != "live-insights";
    if !live_scout_only {
        return Some((10, 30, 0.45));
    }
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

