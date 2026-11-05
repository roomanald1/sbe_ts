#[derive(Clone)]
struct ServiceState {
    client: reqwest::Client,
    learning_url: String,
    seen: Arc<Mutex<BTreeSet<String>>>,
    scout: ScoutState,
}

pub fn router(learning_url: String) -> Router {
    let state_path = std::env::var_os("SCOUTS_STATE_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("scouts_state.json"));
    let persisted = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| {
            let value = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
            serde_json::from_value::<PersistedState>(value.clone())
                .ok()
                .or_else(|| {
                    serde_json::from_value::<BTreeSet<String>>(value)
                        .ok()
                        .map(|seen| PersistedState {
                            seen,
                            ..PersistedState::default()
                        })
                })
        })
        .unwrap_or_default();
    let seen = persisted.seen.clone();
    let client = reqwest::Client::new();
    let insights_url =
        std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let scout = ScoutState {
        data: Arc::new(Mutex::new(persisted)),
        path: state_path,
        flush_lock: Arc::new(Mutex::new(())),
    };
    let replay_scout = scout.clone();
    let replay_client = client.clone();
    let replay_insights_url = insights_url.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let audits = replay_scout
                .data
                .lock()
                .await
                .scout_active_audits
                .values()
                .flat_map(|items| items.iter().filter_map(scout_registration))
                .collect::<Vec<_>>();
            for registration in audits {
                let _ = replay_client
                    .post(format!(
                        "{}/internal/scouts/register",
                        replay_insights_url.trim_end_matches('/')
                    ))
                    .timeout(std::time::Duration::from_secs(2))
                    .json(&registration)
                    .send()
                    .await;
            }
        }
    });
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/configuration", get(get_scout_configuration).post(set_scout_configuration))
        .route("/status", get(get_scout_status))
        .route("/outcomes", get(get_scout_outcomes).post(forward_outcome))
        .route("/samples", post(finalize_sample))
        .route("/scout-outcome", post(receive_scout_outcome))
        .route("/scout-markout", post(receive_scout_markout))
        .route("/scout-plan", post(create_scout_plan))
        .route("/daily-inputs", post(receive_daily_scout_inputs))
        .route("/live-inputs", post(receive_live_scout_inputs))
        .route("/dated-carry-inputs", post(receive_dated_carry_scout_inputs))
        .route(
            "/scout-lifecycle/state",
            get(get_scout_lifecycle_state).post(set_scout_lifecycle_state),
        )
        .route("/scout-lifecycle/observe", post(observe_scout_lifecycle))
        .route("/variants", post(generate_variants))
        .route("/variants/batch", post(generate_variants_batch))
        .route("/live-candidates", post(generate_live_candidates))
        .route("/select-entry", post(select_scout_entry))
        .with_state(ServiceState {
            client,
            learning_url,
            seen: Arc::new(Mutex::new(seen)),
            scout,
        })
}

async fn get_scout_configuration(
    State(service): State<ServiceState>,
) -> Json<ScoutConfiguration> {
    Json(service.scout.data.lock().await.scout_configuration.clone())
}

async fn get_scout_status(State(service): State<ServiceState>) -> Json<ScoutStatus> {
    let state = service.scout.data.lock().await;
    let active_samples = state.scout_sample_active_scenarios.keys()
        .map(|key| key.split("|probe:").next().unwrap_or(key))
        .collect::<BTreeSet<_>>().len();
    let learning_samples = state.outcomes.len();
    let positive_samples = state.outcomes.values()
        .filter(|outcome| outcome.net_return_percent > 0.0).count();
    let realized_pnl_usd = state.outcomes.values()
        .map(|outcome| outcome.simulated_net_pnl_usd).sum();
    Json(ScoutStatus {
        active_samples,
        learning_samples,
        positive_samples,
        win_rate: (learning_samples > 0).then_some(positive_samples as f64 / learning_samples as f64),
        realized_pnl_usd,
    })
}

async fn set_scout_configuration(
    State(service): State<ServiceState>,
    Json(configuration): Json<ScoutConfiguration>,
) -> Result<StatusCode, (StatusCode, String)> {
    if configuration.strategies.len() > 128
        || configuration.max_active_samples > 5_000
        || configuration.max_active_positions > 5_000
        || !configuration.max_sample_notional.is_finite()
        || configuration.max_sample_notional <= 0.0
        || !configuration.stop_loss_percent.is_finite()
        || configuration.stop_loss_percent <= 0.0
        || !configuration.take_profit_percent.is_finite()
        || configuration.take_profit_percent <= 0.0
        || configuration.strategies.iter().collect::<BTreeSet<_>>().len()
            != configuration.strategies.len()
        || configuration.strategies.iter().any(|name| {
            strategies::Strategy::from_name(name).is_none_or(|strategy| {
                strategy.is_legacy_composite()
            })
        })
    {
        return Err((StatusCode::BAD_REQUEST, "invalid Scout configuration".into()));
    }
    let mut state = service.scout.data.lock().await;
    state.scout_configuration = configuration;
    persist_scout_state(&service.scout.path, &state)?;
    Ok(StatusCode::ACCEPTED)
}

async fn receive_daily_scout_inputs(
    State(service): State<ServiceState>,
    Json(request): Json<DailyScoutSignalsRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if request.request_id.is_empty() || request.request_id.len() > 240 || request.signals.len() > 2_000 {
        return Err((StatusCode::BAD_REQUEST, "invalid daily Scout input batch".into()));
    }
    let configuration = service.scout.data.lock().await.scout_configuration.clone();
    let mut candidates = Vec::new();
    for (index, signal) in request.signals.into_iter().enumerate() {
        let Some(strategy_name) = signal.get("strategy").and_then(serde_json::Value::as_str) else { continue };
        if !configuration.strategies.iter().any(|selected| selected == strategy_name) { continue; }
        let Some(strategy) = strategies::Strategy::from_name(strategy_name) else { continue };
        if strategy.is_legacy_composite() || strategy.is_live_insight() || strategy.is_dated_carry() { continue; }
        let parameters = signal.get("strategy_decision")
            .and_then(|decision| decision.get("strategy_parameters"))
            .cloned().and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok())
            .unwrap_or_default();
        let input = strategies::StrategyInput::from_candidate_json(signal.clone(), parameters.clone())
            .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
        let decision = strategy.evaluate(&input).unwrap_or(strategies::StrategyDecision::Reject);
        let decision_name = match decision {
            strategies::StrategyDecision::Enter => "enter",
            strategies::StrategyDecision::Scout => "scout",
            _ => "reject",
        };
        let probe = signal.get("strategy_decision").and_then(|decision| {
            Some((decision.get("scout_parameter_key")?.as_str()?, decision.get("scout_parameter_value")?.as_f64()?))
        });
        let Some(symbol) = signal.get("symbol").and_then(serde_json::Value::as_str) else { continue };
        let Some(direction) = signal.get("direction").and_then(serde_json::Value::as_str) else { continue };
        let timestamp = signal.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or(request.now_ms);
        let decision_id = format!("{}|{}|{}|{}|scout:daily:{index}", timestamp, symbol, direction, strategy_name);
        let mut audit = signal.clone();
        audit["decision_id"] = serde_json::json!(decision_id);
        audit["trade_id"] = serde_json::json!(decision_id);
        audit["timestamp"] = serde_json::json!(request.now_ms);
        audit["signal_timestamp"] = serde_json::json!(timestamp);
        audit["stage"] = serde_json::json!("decision");
        audit["status"] = serde_json::json!("scouted");
        audit["scout"] = serde_json::json!(true);
        audit["strategy_parameters"] = serde_json::to_value(&parameters).unwrap_or_default();
        audit["scout_notional_usd"] = serde_json::json!(configuration.max_sample_notional);
        audit["scout_learning_context"] = serde_json::json!(format!("scout:daily:{strategy_name}:{symbol}:{direction}:trade"));
        audit["market_context"] = serde_json::json!(format!("daily:{strategy_name}:{symbol}:{direction}"));
        if let Some((key, value)) = probe {
            audit["scout_parameter_key"] = serde_json::json!(key);
            audit["scout_parameter_value"] = serde_json::json!(value);
            audit["scout_parameter_profile_symbol"] = serde_json::json!(symbol);
        }
        let stop = parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent);
        let target = parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent);
        audit["stop_loss_percent"] = serde_json::json!(stop);
        audit["take_profit_percent"] = serde_json::json!(target);
        let daily_features_present = input.daily_features.is_some();
        candidates.push(ScoutPlanCandidate {
            generate_group_id: true,
            sample_group_id: String::new(),
            cooldown_key: String::new(),
            profile_key: format!("{strategy_name}|{symbol}"),
            eligible: true,
            parameter_variant: probe.is_some(),
            cooldown_ms: if probe.is_some() { 86_400_000 } else { 300_000 },
            audit_plans: Vec::new(),
            audit_templates: vec![ScoutAuditTemplate {
                base_audit: audit,
                notional_usd: configuration.max_sample_notional,
                learning_context: format!("scout:daily:{strategy_name}:{symbol}:{direction}:scenario"),
                stop_loss_percent: stop,
                take_profit_percent: target,
                level_percent: 0.0,
                strategy_only: false,
                parameter_entry_eligible: probe.map(|_| matches!(decision, strategies::StrategyDecision::Enter | strategies::StrategyDecision::Scout)),
                parameter_specs: strategies::scout_parameter_specs(strategy),
            }],
            sample_payload: None,
            daily_policy: Some(DailyScoutPolicy {
                evaluator_decision: decision_name.into(),
                selected_scout: true,
                strategy_candidate: true,
                daily_features_present,
                independent_non_donchian_strategy: !strategy_name.starts_with("donchian-"),
                parameter_key: probe.map(|(key, _)| key.to_owned()),
                parameter_value: probe.map(|(_, value)| value),
            }),
            live_policy: None,
        });
    }
    if candidates.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let plan = ScoutPlanRequest {
        request_id: request.request_id,
        now_ms: request.now_ms,
        max_active_batches: configuration.max_active_samples,
        max_active_variants_per_profile: 3,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates,
    };
    let _ = create_scout_plan(State(service), Json(plan)).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn receive_live_scout_inputs(
    State(service): State<ServiceState>,
    Json(request): Json<LiveScoutSignalsRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if request.signals.len() > 2_000 {
        return Err((StatusCode::BAD_REQUEST, "too many live Scout signals".into()));
    }
    let configuration = service.scout.data.lock().await.scout_configuration.clone();
    if configuration.strategies.is_empty() || request.signals.is_empty() {
        return Ok(StatusCode::NO_CONTENT);
    }
    let now_ms = scout_now_ms();
    let signals = request.signals.into_iter().filter(|signal| {
        let updated = signal.get("book_updated_at_ms").and_then(serde_json::Value::as_u64);
        let bid = signal.get("best_bid").and_then(serde_json::Value::as_f64);
        let ask = signal.get("best_ask").and_then(serde_json::Value::as_f64);
        let price = signal.get("price").and_then(serde_json::Value::as_f64);
        updated.is_some_and(|updated| now_ms.saturating_sub(updated) <= 5_000)
            && bid.zip(ask).is_some_and(|(bid, ask)| bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid)
            && price.is_some_and(|price| price.is_finite() && price > 0.0)
    }).collect::<Vec<_>>();
    if signals.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let snapshot = service.client.get(format!("{}/decision-snapshot", service.learning_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(2)).send().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?
        .error_for_status().map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?
        .json::<serde_json::Value>().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let mut profiles = Vec::new();
    let symbols = signals.iter().filter_map(|signal| signal.get("symbol").and_then(serde_json::Value::as_str))
        .collect::<BTreeSet<_>>();
    for profile in snapshot.get("profiles").and_then(serde_json::Value::as_array).into_iter().flatten() {
        let (Some(symbol), Some(strategy)) = (
            profile.get("symbol").and_then(serde_json::Value::as_str),
            profile.get("strategy").and_then(serde_json::Value::as_str),
        ) else { continue };
        if !symbols.contains(symbol) || !configuration.strategies.iter().any(|selected| selected == strategy) { continue; }
        let parameters = profile.get("parameters").cloned()
            .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok()).unwrap_or_default();
        profiles.push(LiveScoutProfile { symbol: symbol.to_owned(), strategy: strategy.to_owned(), parameters });
    }
    if profiles.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let generated = generate_live_candidates(Json(LiveScoutCandidatesRequest {
        signals,
        profiles,
    })).await?.0;
    let mut candidates = Vec::new();
    for (index, mut signal) in generated.into_iter().enumerate() {
        let (Some(strategy), Some(symbol), Some(direction)) = (
            signal.get("strategy").and_then(serde_json::Value::as_str).map(str::to_owned),
            signal.get("symbol").and_then(serde_json::Value::as_str).map(str::to_owned),
            signal.get("direction").and_then(serde_json::Value::as_str).map(str::to_owned),
        ) else { continue };
        let parameters = signal.get("strategy_decision").and_then(|decision| decision.get("strategy_parameters"))
            .cloned().and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok()).unwrap_or_default();
        let stop = parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent);
        let target = parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent);
        let decision_id = format!("live-scout:{}:{}:{}:{}", strategy, symbol, direction, now_ms.saturating_add(index as u64));
        signal["decision_id"] = serde_json::json!(decision_id);
        signal["trade_id"] = serde_json::json!(decision_id);
        signal["timestamp"] = serde_json::json!(now_ms);
        signal["stage"] = serde_json::json!("decision");
        signal["status"] = serde_json::json!("scouted");
        signal["scout"] = serde_json::json!(true);
        signal["scout_notional_usd"] = serde_json::json!(configuration.max_sample_notional);
        signal["scout_learning_context"] = serde_json::json!(format!("scout:live:{strategy}:{symbol}:{direction}:trade"));
        signal["scout_entry_price"] = signal.get("price").cloned()
            .or_else(|| signal.get("observed_close").cloned())
            .or_else(|| signal.get("strategy_decision").and_then(|decision| decision.get("observed_close")).cloned())
            .unwrap_or(serde_json::Value::Null);
        let group = format!("live-scout:{strategy}|{symbol}|{direction}");
        signal["scout_sample_group_id"] = serde_json::json!(group);
        signal["scout_cooldown_key"] = serde_json::json!(group);
        signal["scout_exit_scenario"] = serde_json::json!("strategy");
        signal["strategy_parameters"] = serde_json::to_value(&parameters).unwrap_or_default();
        let strategy_kind = strategies::Strategy::from_name(&strategy).unwrap();
        signal["strategy_parameter_specs"] = serde_json::to_value(strategies::scout_parameter_specs(
            strategy_kind,
        )).unwrap_or_default();
        signal["stop_loss_percent"] = serde_json::json!(stop);
        signal["take_profit_percent"] = serde_json::json!(target);
        candidates.push(ScoutPlanCandidate {
            generate_group_id: false,
            sample_group_id: group.clone(),
            cooldown_key: group,
            profile_key: format!("{strategy}|{symbol}"),
            eligible: true,
            parameter_variant: false,
            cooldown_ms: 300_000,
            audit_plans: vec![signal.clone()],
            audit_templates: vec![ScoutAuditTemplate {
                base_audit: signal,
                notional_usd: configuration.max_sample_notional,
                learning_context: format!("scout:live:{strategy}:{symbol}:{direction}:scenario"),
                stop_loss_percent: stop,
                take_profit_percent: target,
                level_percent: 0.0,
                strategy_only: false,
                parameter_entry_eligible: None,
                parameter_specs: strategies::scout_parameter_specs(strategy_kind),
            }],
            sample_payload: None,
            daily_policy: None,
            live_policy: None,
        });
    }
    if candidates.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let request_id = format!("live-inputs:{}", probe_id(&serde_json::to_string(&candidates).unwrap_or_default()));
    let plan = ScoutPlanRequest {
        request_id,
        now_ms,
        max_active_batches: configuration.max_active_samples,
        max_active_variants_per_profile: 3,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates,
    };
    let _ = create_scout_plan(State(service), Json(plan)).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn receive_dated_carry_scout_inputs(
    State(service): State<ServiceState>,
    Json(request): Json<DatedCarryScoutInputsRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    if request.request_id.is_empty() || request.request_id.len() > 240 || request.candidates.len() > 2_000 {
        return Err((StatusCode::BAD_REQUEST, "invalid dated-carry Scout input batch".into()));
    }
    let configuration = service.scout.data.lock().await.scout_configuration.clone();
    if !configuration.strategies.iter().any(|strategy| strategy == "dated-futures-carry") {
        return Ok(StatusCode::NO_CONTENT);
    }
    let candidates = request.candidates.into_iter()
        .filter_map(|candidate| serde_json::from_value::<strategies::DatedCarryEntryCandidate>(candidate).ok())
        .collect::<Vec<_>>();
    if candidates.is_empty() { return Ok(StatusCode::NO_CONTENT); }

    let observation = ScoutLifecycleObserveRequest {
        request_id: format!("dated-carry-mark:{}", request.now_ms),
        now_ms: request.now_ms,
        candidates: candidates.clone(),
    };
    {
        let mut state = service.scout.data.lock().await;
        let _ = apply_scout_lifecycle_observation(&mut state, &observation);
        persist_scout_state(&service.scout.path, &state)?;
    }
    flush_scout_outbox(&service.scout, &service.client, &service.learning_url).await?;

    let snapshot = service.client.get(format!("{}/decision-snapshot", service.learning_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(2)).send().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?
        .error_for_status().map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?
        .json::<serde_json::Value>().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let profiles = snapshot.get("profiles").and_then(serde_json::Value::as_array).cloned().unwrap_or_default();
    let day = request.now_ms / 86_400_000;
    let mut plans = Vec::new();
    for candidate in candidates.iter().filter(|candidate| candidate.eligible && candidate.expiry_ms > request.now_ms.saturating_add(86_400_000)) {
        let profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
        let Some(profile) = profiles.iter().find(|profile| {
            profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol.as_str())
                && profile.get("strategy").and_then(serde_json::Value::as_str) == Some("dated-futures-carry")
        }) else { continue };
        let baseline = profile.get("parameters").cloned()
            .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok()).unwrap_or_default();
        if baseline.is_empty() { continue; }
        let specs = strategies::scout_parameter_specs(strategies::Strategy::DatedCarry);
        let variants = build_parameter_variants(&ParameterVariantsRequest { parameters: baseline.clone(), specs: specs.clone(), day })
            .unwrap_or_default();
        for variant in variants {
            let mut parameters = baseline.clone();
            parameters.extend(variant.changes.iter().map(|(key, value)| (key.clone(), *value)));
            let minimum_days = parameters.get("carry_minimum_days_to_expiry").copied().unwrap_or(f64::NAN);
            let cost_gate = parameters.get("carry_cost_gate_percent").copied().unwrap_or(f64::NAN);
            let would_enter = strategies::dated_carry_entry_is_eligible(candidate, request.now_ms, request.now_ms, minimum_days, cost_gate);
            let sample_id = format!("dated-carry-scout:{}:{}:{}", candidate.future_symbol, request.now_ms, probe_id(&variant.key));
            let tested_specs = specs.iter().filter(|spec| variant.changes.contains_key(&spec.key)).cloned().collect();
            let cooldown_key = format!("{}:{}:{:.8}", profile_symbol, variant.key, variant.value);
            plans.push(ScoutPlanCandidate {
                generate_group_id: false,
                sample_group_id: sample_id.clone(),
                cooldown_key,
                profile_key: format!("{profile_symbol}:dated-futures-carry"),
                eligible: true,
                parameter_variant: true,
                cooldown_ms: 86_400_000,
                audit_plans: Vec::new(),
                audit_templates: Vec::new(),
                sample_payload: Some(ScoutSamplePayload::DatedCarryProbe(DatedCarryProbe {
                    sample_id,
                    parameter_key: variant.key,
                    parameter_value: variant.value,
                    opened_at_ms: request.now_ms,
                    candidate: candidate.clone(),
                    strategy: "dated-futures-carry".into(),
                    profile_symbol: profile_symbol.clone(),
                    parameters,
                    tested_parameters: variant.changes,
                    parameter_specs: tested_specs,
                    would_enter,
                })),
                daily_policy: None,
                live_policy: None,
            });
        }
    }
    if plans.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let plan = ScoutPlanRequest {
        request_id: format!("dated-carry-input:{}", request.request_id),
        now_ms: request.now_ms,
        max_active_batches: configuration.max_active_samples,
        max_active_variants_per_profile: 3,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates: plans,
    };
    let _ = create_scout_plan(State(service), Json(plan)).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn finalize_sample(
    State(service): State<ServiceState>,
    Json(request): Json<FinalizeRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    let event_id = request.sample.event_id.clone();
    if service.seen.lock().await.contains(&event_id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let event = request
        .sample
        .finalize(
            request.closed_at_ms,
            request.exit_price,
            request.cost_percent,
        )
        .ok_or((StatusCode::BAD_REQUEST, "invalid scout sample".into()))?;
    forward_event(&service, &event).await?;
    {
        let mut seen = service.seen.lock().await;
        seen.insert(event_id.clone());
        while seen.len() > 100_000 {
            if let Some(oldest) = seen.iter().next().cloned() {
                seen.remove(&oldest);
            }
        }
        let mut persisted = service.scout.data.lock().await;
        persisted.seen.insert(event_id);
        persisted.seen.extend(seen.iter().cloned());
        persist_scout_state(&service.scout.path, &persisted)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn scout_context_learning_key(context: &str) -> Option<String> {
    let context = context.strip_prefix("scout:")?.split(":scenario:").next()?;
    let context = context.strip_suffix(":trade")?;
    (!context.is_empty()).then(|| format!("scout:{context}:trade"))
}

fn scout_outcome_idempotency_key(outcome: &ScoutOutcomeRequest) -> String {
    format!(
        "{}|{}|{}",
        outcome.decision_id,
        outcome.exit_scenario,
        outcome.scout_comparison_role.as_deref().unwrap_or("single")
    )
}

fn scout_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn parameter_keys(outcome: &ScoutOutcomeRequest) -> Option<BTreeMap<String, f64>> {
    match (
        outcome.scout_parameter_key.as_deref(),
        outcome.scout_parameter_value,
    ) {
        (None, None) => Some(BTreeMap::new()),
        (Some(key), Some(value)) if key.starts_with("combo:") => (value == 0.0
            && value.is_finite())
        .then(|| parse_combination(key, &outcome.strategy_parameter_specs))
        .flatten(),
        (Some(key), Some(value)) => {
            if key == "learning_rate" || !value.is_finite() {
                return None;
            }
            let spec = outcome
                .strategy_parameter_specs
                .iter()
                .find(|spec| spec.key == key)?;
            let position = (value - spec.min) / spec.step;
            (spec.step > 0.0
                && value >= spec.min
                && value <= spec.max
                && (position - position.round()).abs() <= 1e-6)
                .then(|| BTreeMap::from([(key.to_owned(), value)]))
        }
        _ => None,
    }
}

fn validate_scout_outcome(
    outcome: &ScoutOutcomeRequest,
) -> Result<BTreeMap<String, f64>, (StatusCode, String)> {
    if outcome.decision_id.is_empty()
        || outcome.decision_id.len() > 240
        || !outcome.learning_context.starts_with("scout:")
        || !outcome.simulated_net_pnl_usd.is_finite()
        || outcome.symbol.is_empty()
        || outcome.symbol.len() > 32
        || outcome.strategy.is_empty()
        || outcome.strategy.len() > 120
        || outcome
            .strategy_parameters
            .values()
            .any(|value| !value.is_finite())
        || !matches!(
            outcome.exit_scenario.as_str(),
            "strategy" | "tight" | "wide"
        )
        || outcome.scout_comparison_id.is_some() != outcome.scout_comparison_role.is_some()
        || outcome
            .scout_comparison_id
            .as_ref()
            .is_some_and(|id| id.is_empty() || id.len() > 240)
        || outcome
            .scout_comparison_role
            .as_deref()
            .is_some_and(|role| !matches!(role, "baseline" | "variant"))
        || outcome
            .scout_comparison_zero_role
            .as_deref()
            .is_some_and(|role| !matches!(role, "baseline" | "variant"))
        || outcome
            .scout_comparison_zero_role
            .as_deref()
            .is_some_and(|zero_role| Some(zero_role) == outcome.scout_comparison_role.as_deref())
        || outcome.scout_comparison_parameter_key.is_some()
            != outcome.scout_comparison_parameter_value.is_some()
        || (outcome.scout_comparison_role.as_deref() == Some("variant")
            && outcome.scout_parameter_key.is_none())
        || (outcome.scout_comparison_role.as_deref() == Some("baseline")
            && outcome.scout_parameter_key.is_some())
        || outcome.signal_timestamp_ms > outcome.evaluated_timestamp_ms
        || outcome.evaluated_timestamp_ms > scout_now_ms().saturating_add(5_000)
        || scout_now_ms().saturating_sub(outcome.signal_timestamp_ms) > 7 * 86_400_000
        || !outcome.net_return_percent.is_finite()
        || !outcome.taker_fee_rate.is_finite()
        || outcome.taker_fee_rate < 0.0
        || outcome.strategy_parameter_specs.iter().any(|spec| {
            spec.key.is_empty()
                || !spec.default.is_finite()
                || !spec.min.is_finite()
                || !spec.max.is_finite()
                || !spec.step.is_finite()
                || !spec.learning_direction.is_finite()
                || spec.min > spec.max
                || spec.step <= 0.0
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid scout learning outcome".into(),
        ));
    }
    let keys = parameter_keys(outcome).ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid Scout parameter probe".into(),
    ))?;
    if let (Some(key), Some(value)) = (
        &outcome.scout_comparison_parameter_key,
        outcome.scout_comparison_parameter_value,
    ) {
        if key.starts_with("combo:") {
            if !value.is_finite()
                || value != 0.0
                || parse_combination(key, &outcome.strategy_parameter_specs).is_none()
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Invalid Scout comparison combination".into(),
                ));
            }
        } else {
            let Some(spec) = outcome
                .strategy_parameter_specs
                .iter()
                .find(|spec| spec.key == *key)
            else {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Unknown Scout comparison parameter".into(),
                ));
            };
            let position = (value - spec.min) / spec.step;
            if !value.is_finite()
                || value < spec.min
                || value > spec.max
                || spec.step <= 0.0
                || (position - position.round()).abs() > 1e-6
            {
                return Err((
                    StatusCode::BAD_REQUEST,
                    "Scout comparison parameter is outside its bounds or step".into(),
                ));
            }
        }
    }
    if let Some(profile_symbol) = outcome.scout_parameter_profile_symbol.as_deref() {
        if profile_symbol.is_empty() || profile_symbol.len() > 32 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Invalid strategy profile symbol".into(),
            ));
        }
    }
    Ok(keys)
}

fn outcome_event(outcome: &ScoutOutcomeRequest, tested: &BTreeMap<String, f64>) -> FeedbackEvent {
    let profile_symbol = outcome
        .scout_parameter_profile_symbol
        .as_deref()
        .unwrap_or(&outcome.symbol);
    let mut parameters = outcome.strategy_parameters.clone();
    parameters.extend(tested.clone());
    let specs = outcome
        .strategy_parameter_specs
        .iter()
        .filter(|spec| tested.contains_key(&spec.key) && parameters.contains_key(&spec.key))
        .cloned()
        .collect();
    FeedbackEvent::ScoutOutcome {
        event_id: format!("scout:{}:{}", outcome.decision_id, outcome.exit_scenario),
        observed_at_ms: outcome.evaluated_timestamp_ms,
        learning_context: (outcome.exit_scenario == "strategy"
            && outcome.scout_comparison_role.as_deref() != Some("baseline"))
        .then(|| scout_context_learning_key(&outcome.learning_context))
        .flatten(),
        symbol: profile_symbol.to_owned(),
        strategy: outcome.strategy.clone(),
        profile_id: strategies::profile_id(profile_symbol, &outcome.strategy, &parameters),
        parameters,
        specs,
        net_return_percent: outcome.net_return_percent,
    }
}

fn paired_event(
    comparison_id: &str,
    baseline: &ScoutOutcomeRequest,
    variant: &ScoutOutcomeRequest,
) -> Option<FeedbackEvent> {
    let tested = parameter_keys(variant)?;
    let mut parameters = baseline.strategy_parameters.clone();
    parameters.extend(tested.clone());
    let profile_symbol = variant
        .scout_parameter_profile_symbol
        .as_deref()
        .unwrap_or(&variant.symbol);
    let specs = variant
        .strategy_parameter_specs
        .iter()
        .filter(|spec| tested.contains_key(&spec.key) && parameters.contains_key(&spec.key))
        .cloned()
        .collect();
    Some(FeedbackEvent::ScoutOutcome {
        event_id: format!("paired-scout:{comparison_id}"),
        observed_at_ms: baseline
            .evaluated_timestamp_ms
            .max(variant.evaluated_timestamp_ms),
        learning_context: None,
        symbol: profile_symbol.to_owned(),
        strategy: variant.strategy.clone(),
        profile_id: strategies::profile_id(profile_symbol, &variant.strategy, &parameters),
        parameters,
        specs,
        net_return_percent: variant.net_return_percent - baseline.net_return_percent,
    })
}

fn parse_legacy_learning_key(key: &str) -> Option<(String, String, String, f64)> {
    if let Some(rest) = key.strip_prefix("paired_probe_combo:") {
        let (profile, rest) = rest.split_once(':')?;
        let (strategy, parameter_key) = rest.split_once(':')?;
        return Some((
            profile.to_owned(),
            strategy.to_owned(),
            parameter_key.to_owned(),
            0.0,
        ));
    }
    let rest = key.strip_prefix("paired_probe:")?;
    let mut parts = rest.splitn(4, ':');
    let profile = parts.next()?;
    let strategy = parts.next()?;
    let parameter_key = parts.next()?;
    let parameter_value = parts.next()?.parse::<f64>().ok()?;
    parameter_value.is_finite().then(|| {
        (
            profile.to_owned(),
            strategy.to_owned(),
            parameter_key.to_owned(),
            parameter_value,
        )
    })
}

fn paired_event_with_legacy_side(
    comparison_id: &str,
    pair: &ScoutPair,
    latest: &ScoutOutcomeRequest,
) -> Option<FeedbackEvent> {
    let baseline_return = pair
        .baseline
        .as_ref()
        .map(|o| o.net_return_percent)
        .or(pair.legacy_baseline_return_percent)?;
    let variant_return = pair
        .variant
        .as_ref()
        .map(|o| o.net_return_percent)
        .or(pair.legacy_variant_return_percent)?;
    let mut baseline = pair.baseline.clone().unwrap_or_else(|| latest.clone());
    let mut variant = pair.variant.clone().unwrap_or_else(|| latest.clone());
    baseline.net_return_percent = baseline_return;
    variant.net_return_percent = variant_return;
    if pair.variant.is_none() {
        let (profile_symbol, strategy, key, value) =
            parse_legacy_learning_key(&pair.legacy_learning_key)?;
        if strategy != latest.strategy {
            return None;
        }
        variant.scout_parameter_profile_symbol = Some(profile_symbol);
        variant.scout_parameter_key = Some(key);
        variant.scout_parameter_value = Some(value);
        variant.scout_comparison_role = Some("variant".into());
    }
    if pair.baseline.is_none() {
        baseline.scout_parameter_key = None;
        baseline.scout_parameter_value = None;
        baseline.scout_comparison_role = Some("baseline".into());
    }
    paired_event(comparison_id, &baseline, &variant)
}

fn scout_registration(audit: &serde_json::Value) -> Option<serde_json::Value> {
    let decision = audit.get("strategy_decision")?;
    let decision_id = audit.get("decision_id")?.as_str()?;
    let strategy = audit.get("strategy")?.as_str()?;
    let symbol = audit.get("symbol")?.as_str()?;
    let direction = audit.get("direction")?.as_str()?;
    let timestamp = audit.get("timestamp")?.as_u64()?;
    let entry_price = audit.get("scout_entry_price")?.as_f64()?;
    let parameters = decision.get("strategy_parameters");
    let parameter = |key: &str, default: f64| {
        parameters
            .and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(default)
    };
    let strategy_exit_enabled =
        !strategy.starts_with("live-") && strategy != "dated-carry" && strategy != "live-insights";
    Some(serde_json::json!({
        "decision_id": decision_id,
        "symbol": symbol,
        "timestamp": timestamp,
        "direction": direction,
        "entry_price": entry_price,
        "strategy": strategy,
        "exit_scenario": audit.get("scout_exit_scenario").and_then(serde_json::Value::as_str).unwrap_or("strategy"),
        "strategy_exit_enabled": strategy_exit_enabled,
        "stop_loss_percent": audit.get("stop_loss_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.5),
        "take_profit_percent": audit.get("take_profit_percent").and_then(serde_json::Value::as_f64).unwrap_or(0.8),
        "profit_protection_activation_percent": parameter("profit_protection_activation_percent", 0.15),
        "profit_protection_giveback_percent": parameter("profit_protection_giveback_percent", 0.10),
        "profit_protection_minimum_lock_percent": parameter("profit_protection_minimum_lock_percent", 0.05),
        "scout_parameter_key": decision.get("scout_parameter_key"),
        "scout_parameter_value": decision.get("scout_parameter_value"),
        "scout_minimum_hold_days": parameters.and_then(|p| p.get("minimum_hold_days")),
        "scout_maximum_hold_days": parameters.and_then(|p| p.get("maximum_hold_days")),
        "scout_comparison_id": audit.get("scout_comparison_id"),
        "scout_comparison_role": audit.get("scout_comparison_role"),
    }))
}

fn build_scout_audits(template: &ScoutAuditTemplate, group_id: &str) -> Vec<serde_json::Value> {
    let mut base = template.base_audit.clone();
    let strategy = base
        .get("strategy")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let symbol = base
        .get("symbol")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown")
        .to_owned();
    let decision = base.get("strategy_decision").cloned().unwrap_or_default();
    let parameters = decision
        .get("strategy_parameters")
        .cloned()
        .unwrap_or_else(|| serde_json::json!({}));
    let parameter_map =
        serde_json::from_value::<BTreeMap<String, f64>>(parameters.clone()).unwrap_or_default();
    base["signal_timestamp"] = serde_json::json!(
        base.get("timestamp")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default()
    );
    base["scout"] = serde_json::json!(true);
    base["scout_notional_usd"] = serde_json::json!(template.notional_usd);
    base["strategy_parameters"] = serde_json::json!(parameter_map);
    base["scout_sample_group_id"] = serde_json::json!(group_id);
    base["strategy_parameter_specs"] =
        serde_json::to_value(&template.parameter_specs).unwrap_or_else(|_| serde_json::json!([]));
    base["learning_profile_id"] = serde_json::json!(strategies::profile_id(
        &symbol,
        &strategy,
        &parameter_map
    ));
    if strategy.starts_with("donchian-") {
        base["scout_level_percent"] = serde_json::json!(template.level_percent);
    }
    let parameter_key = decision
        .get("scout_parameter_key")
        .and_then(serde_json::Value::as_str);
    if let Some(key) = parameter_key {
        base["scout_parameter_key"] = serde_json::json!(key);
        base["scout_parameter_value"] = decision
            .get("scout_parameter_value")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        base["scout_parameter_profile_symbol"] = decision
            .get("scout_parameter_profile_symbol")
            .cloned()
            .unwrap_or_else(|| serde_json::json!(symbol));
        if let Some(eligible) = template.parameter_entry_eligible {
            base["scout_parameter_entry_eligible"] = serde_json::json!(eligible);
        }
    }
    for field in [
        "scout_comparison_id",
        "scout_comparison_role",
        "scout_comparison_zero_role",
        "scout_comparison_parameter_key",
        "scout_comparison_parameter_value",
        "scout_comparison_parameter_profile_symbol",
    ] {
        if let Some(value) = decision
            .get(field)
            .or_else(|| template.base_audit.get(field))
        {
            base[field] = value.clone();
        }
    }
    if base.get("scout_comparison_role").is_some() {
        let comparison_id = format!("scout-pair:{}", probe_id(group_id));
        base["scout_comparison_id"] = serde_json::json!(comparison_id);
        if let Some(decision) = base.get_mut("strategy_decision") {
            decision["scout_comparison_id"] = serde_json::json!(comparison_id);
        }
    }
    let source_id = template
        .base_audit
        .get("decision_id")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("scout");
    let id_prefix = source_id.split("|exit:").next().unwrap_or(source_id);
    let group_fingerprint = probe_id(group_id);
    if let Some(level) = decision
        .get("primary_level")
        .and_then(serde_json::Value::as_f64)
        .filter(|_| strategy.starts_with("donchian-"))
    {
        base["scout_entry_price"] =
            serde_json::json!(level * (1.0 - template.level_percent / 100.0));
    } else if let Some(price) = decision
        .get("observed_close")
        .and_then(serde_json::Value::as_f64)
    {
        base["scout_entry_price"] = serde_json::json!(price);
    }
    let scenarios: &[(&str, f64)] = if template.strategy_only {
        &[("strategy", 1.0)]
    } else {
        &[("strategy", 1.0), ("tight", 0.5), ("wide", 1.5)]
    };
    scenarios
        .iter()
        .enumerate()
        .map(|(index, (scenario, multiplier))| {
            let mut audit = base.clone();
            let timestamp = template
                .base_audit
                .get("timestamp")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default();
            audit["timestamp"] = serde_json::json!(timestamp.saturating_add(index as u64));
            audit["stop_loss_percent"] = serde_json::json!(template.stop_loss_percent * multiplier);
            audit["take_profit_percent"] =
                serde_json::json!(template.take_profit_percent * multiplier);
            audit["scout_exit_scenario"] = serde_json::json!(scenario);
            audit["scout_sample_group_id"] = serde_json::json!(group_id);
            audit["scout"] = serde_json::json!(true);
            let learning_context = if template.learning_context.ends_with(":scenario") {
                format!("{}:{scenario}:trade", template.learning_context)
            } else {
                template.learning_context.clone()
            };
            audit["scout_learning_context"] = serde_json::json!(learning_context);
            let role = audit
                .get("scout_comparison_role")
                .and_then(serde_json::Value::as_str);
            let comparison_suffix =
                role.map_or_else(String::new, |role| format!("|comparison:{role}"));
            let decision_id = format!(
                "{id_prefix}|sample:{group_fingerprint}|exit:{scenario}{comparison_suffix}"
            );
            audit["decision_id"] = serde_json::json!(decision_id);
            audit["trade_id"] = serde_json::json!(decision_id);
            audit
        })
        .collect()
}

fn persist_scout_state(
    path: &std::path::Path,
    state: &PersistedState,
) -> Result<(), (StatusCode, String)> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    std::fs::rename(temporary, path)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

async fn flush_scout_outbox(
    scout: &ScoutState,
    client: &reqwest::Client,
    learning_url: &str,
) -> Result<(), (StatusCode, String)> {
    let _flush = scout.flush_lock.lock().await;
    loop {
        let item = scout.data.lock().await.outbox.first().cloned();
        let Some(item) = item else {
            return Ok(());
        };
        if matches!(item, ScoutOutboxItem::Audit(_)) {
            let mut state = scout.data.lock().await;
            state.outbox.remove(0);
            persist_scout_state(&scout.path, &state)?;
            continue;
        }
        let (url, payload) = match &item {
            ScoutOutboxItem::Learning(event) => (
                format!("{}/feedback", learning_url.trim_end_matches('/')),
                serde_json::to_value(event).unwrap_or_default(),
            ),
            ScoutOutboxItem::Audit(_) => unreachable!("audit items are removed without Insights delivery"),
            ScoutOutboxItem::Registration(registration) => {
                let base = std::env::var("INSIGHTS_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:3004".into());
                (
                    format!(
                        "{}/internal/scouts/register",
                        base.trim_end_matches('/')
                    ),
                    registration.clone(),
                )
            }
        };
        client
            .post(url)
            .json(&payload)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status)
            .map_err(|error| {
                (
                    StatusCode::BAD_GATEWAY,
                    format!("Scout outcome delivery failed: {error}"),
                )
            })?;
        let mut state = scout.data.lock().await;
        state.outbox.remove(0);
        persist_scout_state(&scout.path, &state)?;
    }
}

fn feedback_event_id(event: &FeedbackEvent) -> &str {
    match event {
        FeedbackEvent::ScoutOutcome { event_id, .. }
        | FeedbackEvent::TradeOutcome { event_id, .. } => event_id,
    }
}

fn outbox_has_learning(state: &PersistedState, event: &FeedbackEvent) -> bool {
    state.outbox.iter().any(|item| matches!(item,
        ScoutOutboxItem::Learning(pending) if feedback_event_id(pending) == feedback_event_id(event)))
}

