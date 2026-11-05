#[derive(Clone)]
struct ServiceState {
    client: reqwest::Client,
    learning_url: String,
    insights_url: String,
    seen: Arc<Mutex<BTreeSet<String>>>,
    shadow: ShadowState,
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
    let shadow = ShadowState {
        data: Arc::new(Mutex::new(persisted)),
        path: state_path,
        flush_lock: Arc::new(Mutex::new(())),
    };
    let replay_shadow = shadow.clone();
    let replay_client = client.clone();
    let replay_insights_url = insights_url.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let audits = replay_shadow
                .data
                .lock()
                .await
                .shadow_active_audits
                .values()
                .flat_map(|items| items.iter().filter_map(scout_shadow_registration))
                .collect::<Vec<_>>();
            for registration in audits {
                let _ = replay_client
                    .post(format!(
                        "{}/internal/signals/scout-shadow",
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
        .route("/shadow-outcomes", get(get_shadow_outcomes))
        .route("/samples", post(finalize_sample))
        .route("/shadow-outcome", post(receive_shadow_outcome))
        .route("/shadow-plan", post(create_shadow_plan))
        .route(
            "/shadow-lifecycle/state",
            get(get_shadow_lifecycle_state).post(set_shadow_lifecycle_state),
        )
        .route("/shadow-lifecycle/observe", post(observe_shadow_lifecycle))
        .route("/outcomes", post(forward_outcome))
        .route("/variants", post(generate_variants))
        .route("/variants/batch", post(generate_variants_batch))
        .route("/live-candidates", post(generate_live_candidates))
        .route("/select-entry", post(select_scout_entry))
        .with_state(ServiceState {
            client,
            learning_url,
            insights_url,
            seen: Arc::new(Mutex::new(seen)),
            shadow,
        })
}

async fn get_shadow_outcomes(
    State(service): State<ServiceState>,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let query = uri
        .query()
        .map(|query| format!("?{query}"))
        .unwrap_or_default();
    let response = service
        .client
        .get(format!(
            "{}/internal/scout-shadow/outcomes{query}",
            service.insights_url.trim_end_matches('/'),
        ))
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    if !response.status().is_success() {
        let status = StatusCode::from_u16(response.status().as_u16())
            .unwrap_or(StatusCode::BAD_GATEWAY);
        let message = response.text().await.unwrap_or_else(|error| error.to_string());
        return Err((status, message));
    }
    response
        .json::<serde_json::Value>()
        .await
        .map(Json)
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
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
        let mut persisted = service.shadow.data.lock().await;
        persisted.seen.insert(event_id);
        persisted.seen.extend(seen.iter().cloned());
        persist_shadow_state(&service.shadow.path, &persisted)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

fn shadow_context_learning_key(context: &str) -> Option<String> {
    let context = context.strip_prefix("scout:")?.split(":scenario:").next()?;
    let context = context.strip_suffix(":trade")?;
    (!context.is_empty()).then(|| format!("scout:{context}:trade"))
}

fn shadow_outcome_idempotency_key(outcome: &ShadowOutcomeRequest) -> String {
    format!(
        "{}|{}|{}",
        outcome.decision_id,
        outcome.exit_scenario,
        outcome.scout_comparison_role.as_deref().unwrap_or("single")
    )
}

fn shadow_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u128::from(u64::MAX)) as u64)
        .unwrap_or_default()
}

fn parameter_keys(outcome: &ShadowOutcomeRequest) -> Option<BTreeMap<String, f64>> {
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

fn validate_shadow_outcome(
    outcome: &ShadowOutcomeRequest,
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
        || outcome.evaluated_timestamp_ms > shadow_now_ms().saturating_add(5_000)
        || shadow_now_ms().saturating_sub(outcome.signal_timestamp_ms) > 7 * 86_400_000
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
            "Invalid shadow learning outcome".into(),
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

fn outcome_event(outcome: &ShadowOutcomeRequest, tested: &BTreeMap<String, f64>) -> FeedbackEvent {
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
        .then(|| shadow_context_learning_key(&outcome.learning_context))
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
    baseline: &ShadowOutcomeRequest,
    variant: &ShadowOutcomeRequest,
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
    pair: &ShadowPair,
    latest: &ShadowOutcomeRequest,
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

fn shadow_outcome_audit(
    outcome: &ShadowOutcomeRequest,
    paired_delta: Option<f64>,
) -> serde_json::Value {
    let status = if outcome.net_return_percent > 1e-12 {
        "won"
    } else if outcome.net_return_percent < -1e-12 {
        "lost"
    } else {
        "flat"
    };
    let mut audit = serde_json::json!({
        "record_schema_version": 1,
        "decision_id": format!("{}|outcome", outcome.decision_id),
        "shadow_decision_id": outcome.decision_id,
        "stage": "scout-outcome",
        "status": status,
        "timestamp": outcome.evaluated_timestamp_ms,
        "signal_timestamp": outcome.signal_timestamp_ms,
        "evaluated_timestamp": outcome.evaluated_timestamp_ms,
        "symbol": outcome.symbol,
        "strategy": outcome.strategy,
        "scout": true,
        "shadow_exit_scenario": outcome.exit_scenario,
        "shadow_net_return_percent": outcome.net_return_percent,
        "shadow_taker_fee_rate": outcome.taker_fee_rate,
        "scout_parameter_key": outcome.scout_parameter_key,
        "scout_parameter_value": outcome.scout_parameter_value,
        "scout_parameter_profile_symbol": outcome.scout_parameter_profile_symbol,
        "scout_comparison_id": outcome.scout_comparison_id,
        "scout_comparison_role": outcome.scout_comparison_role,
        "scout_comparison_zero_role": outcome.scout_comparison_zero_role,
        "scout_comparison_parameter_key": outcome.scout_comparison_parameter_key,
        "scout_comparison_parameter_value": outcome.scout_comparison_parameter_value,
        "scout_comparison_parameter_profile_symbol": outcome.scout_comparison_parameter_profile_symbol,
    });
    if let Some(delta) = paired_delta {
        audit["paired_delta_percent"] = serde_json::json!(delta);
    }
    audit
}

fn scout_shadow_registration(audit: &serde_json::Value) -> Option<serde_json::Value> {
    let decision = audit.get("strategy_decision")?;
    let decision_id = audit.get("decision_id")?.as_str()?;
    let strategy = audit.get("strategy")?.as_str()?;
    let symbol = audit.get("symbol")?.as_str()?;
    let direction = audit.get("direction")?.as_str()?;
    let timestamp = audit.get("timestamp")?.as_u64()?;
    let entry_price = audit.get("shadow_entry_price")?.as_f64()?;
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
        "exit_scenario": audit.get("shadow_exit_scenario").and_then(serde_json::Value::as_str).unwrap_or("strategy"),
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

fn build_shadow_audits(template: &ShadowAuditTemplate, group_id: &str) -> Vec<serde_json::Value> {
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
    base["shadow_notional_usd"] = serde_json::json!(template.notional_usd);
    base["strategy_parameters"] = serde_json::json!(parameter_map);
    base["shadow_sample_group_id"] = serde_json::json!(group_id);
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
        base["shadow_entry_price"] =
            serde_json::json!(level * (1.0 - template.level_percent / 100.0));
    } else if let Some(price) = decision
        .get("observed_close")
        .and_then(serde_json::Value::as_f64)
    {
        base["shadow_entry_price"] = serde_json::json!(price);
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
            audit["shadow_exit_scenario"] = serde_json::json!(scenario);
            audit["shadow_sample_group_id"] = serde_json::json!(group_id);
            audit["scout"] = serde_json::json!(true);
            let learning_context = if template.learning_context.ends_with(":scenario") {
                format!("{}:{scenario}:trade", template.learning_context)
            } else {
                template.learning_context.clone()
            };
            audit["shadow_learning_context"] = serde_json::json!(learning_context);
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

fn persist_shadow_state(
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

async fn flush_shadow_outbox(
    shadow: &ShadowState,
    client: &reqwest::Client,
    learning_url: &str,
) -> Result<(), (StatusCode, String)> {
    let _flush = shadow.flush_lock.lock().await;
    loop {
        let item = shadow.data.lock().await.outbox.first().cloned();
        let Some(item) = item else {
            return Ok(());
        };
        let (url, payload) = match &item {
            ShadowOutboxItem::Learning(event) => (
                format!("{}/feedback", learning_url.trim_end_matches('/')),
                serde_json::to_value(event).unwrap_or_default(),
            ),
            ShadowOutboxItem::Audit(audit) => {
                let base = std::env::var("INSIGHTS_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:3004".into());
                (
                    format!("{}/signals/auto-trader-audit", base.trim_end_matches('/')),
                    audit.clone(),
                )
            }
            ShadowOutboxItem::Registration(registration) => {
                let base = std::env::var("INSIGHTS_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:3004".into());
                (
                    format!(
                        "{}/internal/signals/scout-shadow",
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
        let mut state = shadow.data.lock().await;
        state.outbox.remove(0);
        persist_shadow_state(&shadow.path, &state)?;
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
        ShadowOutboxItem::Learning(pending) if feedback_event_id(pending) == feedback_event_id(event)))
}

