// Paper-only dated futures cash-and-carry. It never submits exchange orders.
#[cfg(test)]
const CARRY_COST_GATE_PERCENT: f64 = strategies::DATED_CARRY_COST_BUDGET_PERCENT;
const CARRY_ENTRY_SLIPPAGE: f64 = 0.0005;
const CARRY_DAY_MS: u64 = 86_400_000;

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryPosition {
    spot_symbol: String,
    future_symbol: String,
    expiry_ms: u64,
    entry_ms: u64,
    quantity: f64,
    spot_entry_price: f64,
    future_entry_price: f64,
    entry_fees: f64,
    status: String,
    #[serde(default)]
    scout_parameter_key: Option<String>,
    #[serde(default)]
    scout_parameter_value: Option<f64>,
    #[serde(default)]
    spot_exit_twap: Option<f64>,
    #[serde(default)]
    settlement_price: Option<f64>,
    #[serde(default)]
    net_pnl_usdt: Option<f64>,
    #[serde(default)]
    settlement_source: Option<String>,
    #[serde(default)]
    spot_mark_bid: Option<f64>,
    #[serde(default)]
    future_mark_ask: Option<f64>,
    #[serde(default)]
    spot_leg_pnl_usdt: Option<f64>,
    #[serde(default)]
    future_leg_pnl_usdt: Option<f64>,
    #[serde(default)]
    estimated_exit_fees_usdt: Option<f64>,
    #[serde(default)]
    estimated_exit_slippage_usdt: Option<f64>,
    #[serde(default)]
    marked_net_pnl_usdt: Option<f64>,
    #[serde(default)]
    mark_checked_at_ms: Option<u64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryScoutShadow {
    candidate: DatedCarryCandidate,
    opened_at_ms: u64,
    parameter_key: String,
    parameter_value: f64,
    #[serde(default)]
    parameters: BTreeMap<String, f64>,
    #[serde(default)]
    tested_parameters: BTreeMap<String, f64>,
    would_enter: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct DatedCarryBook {
    enabled: bool,
    max_spot_notional_usdt: f64,
    #[serde(default)]
    candidates: Vec<DatedCarryCandidate>,
    #[serde(default)]
    positions: Vec<DatedCarryPosition>,
    #[serde(default)]
    checked_at_ms: u64,
    #[serde(default)]
    last_error: Option<String>,
    // Read once to transfer pre-migration samples/cooldowns to Scouts. Empty
    // values are omitted from subsequent saves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    scout_shadow_probes: Vec<DatedCarryScoutShadow>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    scout_shadow_last_started: BTreeMap<String, u64>,
}

impl Default for DatedCarryBook {
    fn default() -> Self {
        Self {
            enabled: false,
            max_spot_notional_usdt: 1_000.0,
            candidates: Vec::new(),
            positions: Vec::new(),
            checked_at_ms: 0,
            last_error: None,
            scout_shadow_probes: Vec::new(),
            scout_shadow_last_started: BTreeMap::new(),
        }
    }
}


fn dated_carry_shadow_key(candidate: &DatedCarryCandidate, key: &str, value: f64) -> String {
    format!("{}:{key}:{value:.8}", candidate.spot_symbol)
}

#[cfg(test)]
fn dated_carry_shadow_return_percent(
    entry: &DatedCarryCandidate,
    exit: &DatedCarryCandidate,
) -> Option<f64> {
    let quantity = entry.quantity;
    let entry_spot = entry.spot_ask * (1.0 + CARRY_ENTRY_SLIPPAGE);
    let entry_future = entry.future_bid * (1.0 - CARRY_ENTRY_SLIPPAGE);
    let exit_spot = exit.spot_bid * (1.0 - CARRY_ENTRY_SLIPPAGE);
    let exit_future = exit.future_ask * (1.0 + CARRY_ENTRY_SLIPPAGE);
    let entry_fees = quantity * (0.001 * entry_spot + 0.0005 * entry_future);
    let exit_fees = quantity * (0.001 * exit_spot + 0.0005 * exit_future);
    let net = quantity * (exit_spot - entry_spot + entry_future - exit_future)
        - entry_fees
        - exit_fees;
    let entry_notional = quantity * entry_spot;
    (quantity.is_finite()
        && quantity > 0.0
        && [entry_spot, entry_future, exit_spot, exit_future, entry_notional]
            .iter()
            .all(|value| value.is_finite() && *value > 0.0)
        && net.is_finite())
    .then_some(net / entry_notional * 100.0)
}

fn shared_dated_carry_candidate(
    candidate: &DatedCarryCandidate,
) -> strategies::DatedCarryEntryCandidate {
    strategies::DatedCarryEntryCandidate {
        spot_symbol: candidate.spot_symbol.clone(),
        future_symbol: candidate.future_symbol.clone(),
        expiry_ms: candidate.expiry_ms,
        days_to_expiry: candidate.days_to_expiry,
        quantity: candidate.quantity,
        spot_ask: candidate.spot_ask,
        future_bid: candidate.future_bid,
        spot_bid: candidate.spot_bid,
        future_ask: candidate.future_ask,
        basis_percent: candidate.basis_percent,
        net_after_budget_percent: candidate.net_after_budget_percent,
        eligible: candidate.eligible,
    }
}

fn dated_carry_probe_plan_candidate(
    candidate: &DatedCarryCandidate,
    opened_at_ms: u64,
    parameter_key: &str,
    parameter_value: f64,
    parameters: &BTreeMap<String, f64>,
    tested_parameters: &BTreeMap<String, f64>,
    cooldown_ms: u64,
) -> scouts::ShadowPlanCandidate {
    let profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
    let sample_id = format!(
        "dated-carry-scout:{}:{}:{parameter_key}:{parameter_value:.8}",
        candidate.future_symbol, opened_at_ms,
    );
    let shared_candidate = shared_dated_carry_candidate(candidate);
    let minimum_days = strategy_parameter_value(
        parameters,
        StrategyParameterKey::CarryMinimumDaysToExpiry,
    )
    .unwrap_or(f64::NAN);
    let cost_gate_percent = strategy_parameter_value(
        parameters,
        StrategyParameterKey::CarryCostGatePercent,
    )
    .unwrap_or(f64::NAN);
    let would_enter = strategies::dated_carry_entry_is_eligible(
        &shared_candidate,
        opened_at_ms,
        opened_at_ms,
        minimum_days,
        cost_gate_percent,
    );
    let parameter_specs = scout_parameter_specs(Strategy::DatedCarry)
        .into_iter()
        .filter(|spec| tested_parameters.contains_key(spec.key.as_str()))
        .collect();

    scouts::ShadowPlanCandidate {
        generate_group_id: false,
        sample_group_id: sample_id.clone(),
        cooldown_key: dated_carry_shadow_key(candidate, parameter_key, parameter_value),
        profile_key: format!("{profile_symbol}:{}", Strategy::DatedCarry.as_str()),
        eligible: true,
        parameter_variant: true,
        cooldown_ms,
        audit_plans: Vec::new(),
        audit_templates: Vec::new(),
        daily_policy: None,
        live_policy: None,
        sample_payload: Some(scouts::ShadowSamplePayload::DatedCarryProbe(
            scouts::DatedCarryProbe {
                sample_id,
                parameter_key: parameter_key.to_owned(),
                parameter_value,
                opened_at_ms,
                candidate: shared_candidate,
                strategy: Strategy::DatedCarry.as_str().to_owned(),
                profile_symbol,
                parameters: parameters.clone(),
                tested_parameters: tested_parameters.clone(),
                parameter_specs,
                would_enter,
            },
        )),
    }
}

fn carry_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn update_dated_carry_mark(
    position: &mut DatedCarryPosition,
    spot_mark_bid: f64,
    future_mark_ask: f64,
    checked_at_ms: u64,
) {
    if !spot_mark_bid.is_finite() || spot_mark_bid <= 0.0
        || !future_mark_ask.is_finite() || future_mark_ask <= 0.0
    {
        return;
    }
    let quantity = position.quantity;
    let spot_leg_pnl = quantity * (spot_mark_bid - position.spot_entry_price);
    let future_leg_pnl = quantity * (position.future_entry_price - future_mark_ask);
    let exit_fees = quantity * (0.001 * spot_mark_bid + 0.0005 * future_mark_ask);
    let exit_slippage = quantity * CARRY_ENTRY_SLIPPAGE * (spot_mark_bid + future_mark_ask);
    let net = spot_leg_pnl + future_leg_pnl - position.entry_fees - exit_fees - exit_slippage;
    if [spot_leg_pnl, future_leg_pnl, exit_fees, exit_slippage, net].iter().any(|value| !value.is_finite()) {
        return;
    }
    position.spot_mark_bid = Some(spot_mark_bid);
    position.future_mark_ask = Some(future_mark_ask);
    position.spot_leg_pnl_usdt = Some(spot_leg_pnl);
    position.future_leg_pnl_usdt = Some(future_leg_pnl);
    position.estimated_exit_fees_usdt = Some(exit_fees);
    position.estimated_exit_slippage_usdt = Some(exit_slippage);
    position.marked_net_pnl_usdt = Some(net);
    position.mark_checked_at_ms = Some(checked_at_ms);
}

fn dated_carry_path(settings_path: &PathBuf) -> PathBuf {
    settings_path.with_extension("dated_carry.json")
}

fn save_dated_carry(path: &PathBuf, book: &DatedCarryBook) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(book).map_err(|error| error.to_string())?;
    let temporary = path.with_extension("dated_carry.tmp");
    std::fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    std::fs::rename(&temporary, path).map_err(|error| error.to_string())
}

async fn get_dated_carry(State(state): State<AutoTraderState>) -> Json<DatedCarryBook> {
    let mut book = state.dated_carry.lock().await.clone();
    let settings = state.settings.lock().await;
    book.enabled = state.enabled.load(Ordering::Relaxed) && settings.strategies.contains(&Strategy::DatedCarry);
    Json(book)
}

#[derive(Deserialize)]
struct DatedCarryUpdate {
    max_spot_notional_usdt: f64,
}

async fn update_dated_carry(
    State(state): State<AutoTraderState>,
    Json(update): Json<DatedCarryUpdate>,
) -> Result<Json<DatedCarryBook>, (StatusCode, String)> {
    if !update.max_spot_notional_usdt.is_finite()
        || !(100.0..=10_000.0).contains(&update.max_spot_notional_usdt)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Paper spot notional must be between 100 and 10,000 USDT".into(),
        ));
    }
    let _work = state.dated_carry_work_lock.lock().await;
    let mut book = state.dated_carry.lock().await;
    book.max_spot_notional_usdt = update.max_spot_notional_usdt;
    save_dated_carry(&state.dated_carry_path, &book)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    let mut response = book.clone();
    drop(book);
    let settings = state.settings.lock().await;
    response.enabled = state.enabled.load(Ordering::Relaxed) && settings.strategies.contains(&Strategy::DatedCarry);
    Ok(Json(response))
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum DatedCarrySignal {
    Scan {
        emitted_at_ms: u64,
        candidates: Vec<DatedCarryCandidate>,
        last_error: Option<String>,
    },
    Entry {
        emitted_at_ms: u64,
        candidate: DatedCarryCandidate,
    },
    Settlement {
        emitted_at_ms: u64,
        spot_symbol: String,
        future_symbol: String,
        expiry_ms: u64,
        spot_exit_twap: f64,
        settlement_price: f64,
        settlement_source: DatedCarrySettlementSource,
    },
}

async fn receive_dated_carry_signal(
    State(state): State<AutoTraderState>,
    Json(signal): Json<DatedCarrySignal>,
) -> Result<StatusCode, (StatusCode, String)> {
    let _work = state.dated_carry_work_lock.lock().await;
    let now_ms = carry_now_ms();
    let mut book = state.dated_carry.lock().await;
    let mut changed = false;
    let mut audit_payload = None;
    match signal {
        DatedCarrySignal::Scan { emitted_at_ms, candidates, last_error } => {
            if emitted_at_ms > now_ms + 5_000 || now_ms.saturating_sub(emitted_at_ms) > 120_000 {
                return Err((StatusCode::BAD_REQUEST, "Carry scan update is stale".into()));
            }
            book.checked_at_ms = emitted_at_ms;
            book.last_error = last_error;
            if book.last_error.is_none() {
                for position in book.positions.iter_mut().filter(|position| position.status == "open") {
                    if let Some(candidate) = candidates.iter().find(|candidate| {
                        candidate.future_symbol == position.future_symbol
                            && candidate.spot_symbol == position.spot_symbol
                    }) {
                        update_dated_carry_mark(position, candidate.spot_bid, candidate.future_ask, emitted_at_ms);
                    }
                }

                let strategy_settings = {
                    let settings = state.settings.lock().await;
                    settings.clone()
                };
                let enabled = state.enabled.load(Ordering::Relaxed)
                    && strategy_settings.strategies.contains(&Strategy::DatedCarry);

                let scouts_url = std::env::var("SCOUTS_SERVICE_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:3011".into());
                let legacy_cooldowns = book.scout_shadow_last_started.clone();
                if !legacy_cooldowns.is_empty() {
                    {
                        let mut settings = state.settings.lock().await;
                        for (key, started_day) in &legacy_cooldowns {
                            settings.shadow_sample_at_ms.entry(key.clone()).or_insert_with(|| {
                                started_day.saturating_mul(CARRY_DAY_MS)
                            });
                        }
                    }
                    let cooldowns = legacy_cooldowns.iter()
                        .map(|(key, started_day)| (key.clone(), started_day.saturating_mul(CARRY_DAY_MS)))
                        .collect::<BTreeMap<_, _>>();
                    let mut cooldown_ids = cooldowns.keys().cloned().collect::<Vec<_>>();
                    cooldown_ids.sort();
                    let import_request = scouts::ShadowPlanRequest {
                        request_id: format!("dated-carry-cooldown-import:{}", scouts::probe_id(&cooldown_ids.join("|"))),
                        now_ms: emitted_at_ms,
                        max_active_batches: 5_000,
                        max_active_variants_per_profile: 1_000,
                        legacy_cooldowns: cooldowns,
                        legacy_comparisons: Vec::new(),
                        candidates: Vec::new(),
                    };
                    if scouts::request_shadow_plan(&scouts_url, &import_request).await.is_ok() {
                        book.scout_shadow_last_started.clear();
                        if let Err(error) = request_settings_persist(&state) {
                            crate::service_log!("WARN", "Could not persist imported dated-carry Scout cooldowns: {}", error.1);
                        }
                    } else {
                        crate::service_log!("WARN", "Could not import legacy dated-carry Scout cooldowns; retaining them for retry");
                    }
                }

                let observed_candidates = candidates.iter().map(shared_dated_carry_candidate).collect::<Vec<_>>();
                let observe_request = scouts::ShadowLifecycleObserveRequest {
                    request_id: format!("dated-carry-mark:{emitted_at_ms}:{}", scouts::probe_id(
                        &serde_json::to_string(&observed_candidates).unwrap_or_default(),
                    )),
                    now_ms: emitted_at_ms,
                    candidates: observed_candidates,
                };
                if let Err(error) = scouts::request_shadow_lifecycle_observe(&scouts_url, &observe_request).await {
                    crate::service_log!("WARN", "Scout service unavailable for dated-carry markout updates: {error}");
                }

                // Transfer probes persisted by older AutoTrader versions before
                // creating new ones. The stable request ID makes a retry safe if
                // the process stops after Scout admission but before this book saves.
                if !book.scout_shadow_probes.is_empty() {
                    let mut legacy_candidates = Vec::new();
                    for shadow in &book.scout_shadow_probes {
                        let mut planned = dated_carry_probe_plan_candidate(
                            &shadow.candidate,
                            shadow.opened_at_ms,
                            &shadow.parameter_key,
                            shadow.parameter_value,
                            &shadow.parameters,
                            &shadow.tested_parameters,
                            0,
                        );
                        if let Some(scouts::ShadowSamplePayload::DatedCarryProbe(probe)) =
                            planned.sample_payload.as_mut()
                        {
                            probe.would_enter = shadow.would_enter;
                        }
                        legacy_candidates.push(planned);
                    }
                    let mut ids = legacy_candidates.iter().map(|candidate| candidate.sample_group_id.clone()).collect::<Vec<_>>();
                    ids.sort();
                    let legacy_request = scouts::ShadowPlanRequest {
                        request_id: format!("dated-carry-legacy-import:{}", scouts::probe_id(&ids.join("|"))),
                        now_ms: emitted_at_ms,
                        max_active_batches: 5_000,
                        max_active_variants_per_profile: 1_000,
                        legacy_cooldowns: BTreeMap::new(),
                        legacy_comparisons: Vec::new(),
                        candidates: legacy_candidates,
                    };
                    match scouts::request_shadow_plan(&scouts_url, &legacy_request).await {
                        Ok(response) => {
                            let accepted = response.selected_groups.into_iter().collect::<BTreeSet<_>>();
                            book.scout_shadow_probes.retain(|shadow| {
                                let id = format!("dated-carry-scout:{}:{}:{}:{:.8}",
                                    shadow.candidate.future_symbol, shadow.opened_at_ms,
                                    shadow.parameter_key, shadow.parameter_value);
                                !accepted.contains(&id)
                            });
                        }
                        Err(error) => crate::service_log!("WARN", "Could not migrate legacy dated-carry Scout probes: {error}"),
                    }
                }

                if enabled {
                    let day = emitted_at_ms / CARRY_DAY_MS;
                    let mut plan_candidates = Vec::new();
                    for candidate in &candidates {
                        if !candidate.eligible {
                            continue;
                        }
                        let profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
                        let strategy_parameters = strategy_parameter_values(
                            &strategy_settings, &profile_symbol, Strategy::DatedCarry,
                        );
                        if candidate.expiry_ms <= emitted_at_ms.saturating_add(CARRY_DAY_MS) {
                            continue;
                        }
                        let variants = match scouts::request_parameter_variants(
                            &scouts_url,
                            &scouts::ParameterVariantsRequest {
                                parameters: strategy_parameters.clone(),
                                specs: scout_parameter_specs(Strategy::DatedCarry),
                                day,
                            },
                        )
                        .await
                        {
                            Ok(Some(variants)) => variants,
                            Ok(None) => Vec::new(),
                            Err(error) => {
                                crate::service_log!("WARN", "Scout service unavailable for dated-carry parameter sweep: {error}");
                                Vec::new()
                            }
                        };
                        for variant in variants {
                            let parameter_key = variant.key;
                            let parameter_value = variant.value;
                            let changes = variant.changes;
                            let cooldown_key = dated_carry_shadow_key(candidate, &parameter_key, parameter_value);
                            if legacy_cooldowns.get(&cooldown_key).is_some_and(|started_day| *started_day >= day)
                                || book.scout_shadow_probes.iter().any(|shadow| {
                                    dated_carry_shadow_key(&shadow.candidate, &shadow.parameter_key, shadow.parameter_value) == cooldown_key
                                })
                            {
                                continue;
                            }
                            let mut probe_parameters = strategy_parameters.clone();
                            probe_parameters.extend(changes.iter().map(|(key, value)| (key.clone(), *value)));
                            plan_candidates.push(dated_carry_probe_plan_candidate(
                                candidate,
                                emitted_at_ms,
                                &parameter_key,
                                parameter_value,
                                &probe_parameters,
                                &changes,
                                CARRY_DAY_MS,
                            ));
                        }
                    }
                    if !plan_candidates.is_empty() {
                        let request_hash = scouts::probe_id(
                            &serde_json::to_string(&plan_candidates).unwrap_or_default(),
                        );
                        let request = scouts::ShadowPlanRequest {
                            request_id: format!("dated-carry-scan:{emitted_at_ms}:{request_hash}"),
                            now_ms: emitted_at_ms,
                            max_active_batches: 5_000,
                            max_active_variants_per_profile: 1_000,
                            legacy_cooldowns: BTreeMap::new(),
                            legacy_comparisons: Vec::new(),
                            candidates: plan_candidates,
                        };
                        if let Err(error) = scouts::request_shadow_plan(&scouts_url, &request).await {
                            crate::service_log!("WARN", "Could not schedule dated-carry Scout probes: {error}");
                        }
                    }
                }
                book.candidates = candidates;
            }
            changed = true;
        }
        DatedCarrySignal::Entry { emitted_at_ms, candidate } => {
            if emitted_at_ms > now_ms + 5_000 || now_ms.saturating_sub(emitted_at_ms) > 120_000 {
                return Err((StatusCode::BAD_REQUEST, "Carry entry signal is stale".into()));
            }
            let spot_profile_symbol = candidate.spot_symbol.replace("USDT", "-USDT");
            let strategy_parameters = {
                let settings = state.settings.lock().await;
                strategy_parameter_values(&settings, &spot_profile_symbol, Strategy::DatedCarry)
            };
            let Some(minimum_days) = strategy_parameter_value(
                &strategy_parameters, StrategyParameterKey::CarryMinimumDaysToExpiry,
            ) else {
                return Err((StatusCode::BAD_REQUEST, "Missing learned carry expiry parameter".into()));
            };
            let entry_decision = Strategy::DatedCarry
                .evaluate(&StrategySignal::dated_carry_entry(
                    &candidate, emitted_at_ms, now_ms, &strategy_parameters,
                ))
                .unwrap_or(StrategyDecision::Reject);
            if !candidate.quantity.is_finite()
                || !candidate.spot_ask.is_finite()
                || !candidate.future_bid.is_finite()
                || !candidate.spot_bid.is_finite()
                || !candidate.future_ask.is_finite()
                || !candidate.basis_percent.is_finite()
                || !candidate.net_after_budget_percent.is_finite()
                || candidate.quantity <= 0.0
                || candidate.spot_ask <= 0.0
                || candidate.future_bid <= 0.0
                || candidate.spot_bid <= 0.0
                || candidate.future_ask <= 0.0
                || candidate.expiry_ms <= now_ms
                || candidate.spot_symbol != "BTCUSDT" && candidate.spot_symbol != "ETHUSDT"
                || !candidate.future_symbol.starts_with(&format!("{}_", candidate.spot_symbol))
            {
                return Err((StatusCode::BAD_REQUEST, "Invalid dated carry entry signal".into()));
            }
            let days = (candidate.expiry_ms - now_ms) as f64 / CARRY_DAY_MS as f64;
            if days < minimum_days {
                return Ok(StatusCode::ACCEPTED);
            }
            book.checked_at_ms = emitted_at_ms;
            book.last_error = None;
            book.candidates.retain(|item| item.future_symbol != candidate.future_symbol);
            book.candidates.push(candidate.clone());
            let selected = {
                let settings = state.settings.lock().await;
                state.enabled.load(Ordering::Relaxed)
                    && settings.strategies.contains(&Strategy::DatedCarry)
            };
            if selected
                && entry_decision == StrategyDecision::Enter
                && !book.positions.iter().any(|position| {
                    position.spot_symbol == candidate.spot_symbol && position.status == "open"
                })
            {
                if state.audit_health.load(Ordering::Relaxed) != 3
                    || state.audit_failure_latched.load(Ordering::Relaxed)
                {
                    book.last_error = Some(
                        "Audit persistence unavailable; new dated-carry entries are paused".into(),
                    );
                    changed = true;
                } else {
                    let spot_entry = candidate.spot_ask * (1.0 + CARRY_ENTRY_SLIPPAGE);
                    let future_entry = candidate.future_bid * (1.0 - CARRY_ENTRY_SLIPPAGE);
                    if candidate.quantity * spot_entry > book.max_spot_notional_usdt + 0.01 {
                        return Err((StatusCode::BAD_REQUEST, "Carry signal exceeds the paper size limit".into()));
                    }
                    let fees = candidate.quantity * (0.001 * spot_entry + 0.0005 * future_entry);
                    audit_payload = Some(serde_json::json!({
                        "decision_id": format!("dated-carry:{}", candidate.future_symbol),
                        "stage": "dated_carry_entry",
                        "timestamp": now_ms,
                        "symbol": candidate.spot_symbol,
                        "future_symbol": candidate.future_symbol,
                        "quantity": candidate.quantity,
                        "spot_entry_price": spot_entry,
                        "future_entry_price": future_entry,
                        "entry_fees": fees,
                        "signal_basis_percent": candidate.basis_percent,
                    }));
                    let mut position = DatedCarryPosition {
                        spot_symbol: candidate.spot_symbol,
                        future_symbol: candidate.future_symbol,
                        expiry_ms: candidate.expiry_ms,
                        entry_ms: now_ms,
                        quantity: candidate.quantity,
                        spot_entry_price: spot_entry,
                        future_entry_price: future_entry,
                        entry_fees: fees,
                        status: "open".into(),
                        scout_parameter_key: None,
                        scout_parameter_value: None,
                        spot_exit_twap: None,
                        settlement_price: None,
                        net_pnl_usdt: None,
                        settlement_source: None,
                        spot_mark_bid: None,
                        future_mark_ask: None,
                        spot_leg_pnl_usdt: None,
                        future_leg_pnl_usdt: None,
                        estimated_exit_fees_usdt: None,
                        estimated_exit_slippage_usdt: None,
                        marked_net_pnl_usdt: None,
                        mark_checked_at_ms: None,
                    };
                    update_dated_carry_mark(&mut position, candidate.spot_bid, candidate.future_ask, emitted_at_ms);
                    book.positions.push(position);
                    changed = true;
                }
            }
        }
        DatedCarrySignal::Settlement {
            emitted_at_ms,
            spot_symbol,
            future_symbol,
            expiry_ms,
            spot_exit_twap,
            settlement_price,
            settlement_source,
        } => {
            let strategy_parameters = BTreeMap::new();
            let exit_decision = Strategy::DatedCarry
                .evaluate(&StrategySignal::dated_carry_settlement(
                    emitted_at_ms,
                    now_ms,
                    &spot_symbol,
                    &future_symbol,
                    expiry_ms,
                    spot_exit_twap,
                    settlement_price,
                    Some(settlement_source),
                    &strategy_parameters,
                ))
                .unwrap_or(StrategyDecision::Hold);
            if exit_decision != StrategyDecision::Exit {
                return Err((StatusCode::BAD_REQUEST, "Invalid dated carry settlement signal".into()));
            }
            if let Some(position) = book.positions.iter_mut().find(|position| {
                position.future_symbol == future_symbol
                    && position.spot_symbol == spot_symbol
                    && position.expiry_ms == expiry_ms
                    && !position.status.starts_with("closed")
            }) {
                let spot_exit = spot_exit_twap * (1.0 - CARRY_ENTRY_SLIPPAGE);
                let q = position.quantity;
                let exit_fees = q * (0.001 * spot_exit + 0.0005 * settlement_price
                    + CARRY_ENTRY_SLIPPAGE * settlement_price);
                let net = q * (position.future_entry_price - position.spot_entry_price
                    + spot_exit - settlement_price) - position.entry_fees - exit_fees;
                position.spot_exit_twap = Some(spot_exit);
                position.settlement_price = Some(settlement_price);
                position.settlement_source = Some(settlement_source.as_str().into());
                position.net_pnl_usdt = Some(net);
                audit_payload = Some(serde_json::json!({
                    "decision_id": format!("dated-carry:{}", position.future_symbol),
                    "stage": "dated_carry_exit",
                    "timestamp": now_ms,
                    "symbol": position.spot_symbol,
                    "future_symbol": position.future_symbol,
                    "quantity": q,
                    "spot_exit_twap": spot_exit,
                    "settlement_price": settlement_price,
                    "settlement_source": settlement_source,
                    "net_pnl_usdt": net,
                }));
                position.status = if settlement_source == DatedCarrySettlementSource::Official { "closed" }
                    else { "closed_estimated" }.into();
                changed = true;
            }
        }
    }
    if changed {
        save_dated_carry(&state.dated_carry_path, &book)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    }
    drop(book);
    if let Some(payload) = audit_payload {
        queue_auto_trader_audit(&state, payload);
    }
    Ok(StatusCode::ACCEPTED)
}

#[cfg(test)]
mod dated_carry_probe_contract_tests {
    use super::*;

    fn candidate() -> DatedCarryCandidate {
        DatedCarryCandidate {
            spot_symbol: "BTCUSDT".into(),
            future_symbol: "BTCUSDT_Q1".into(),
            expiry_ms: 1_700_000_000_000 + 10 * CARRY_DAY_MS,
            days_to_expiry: 10.0,
            quantity: 0.01,
            spot_ask: 100.0,
            future_bid: 100.95,
            spot_bid: 99.9,
            future_ask: 101.05,
            basis_percent: 0.95,
            net_after_budget_percent: 0.95 - CARRY_COST_GATE_PERCENT,
            eligible: true,
            reason: "eligible".into(),
        }
    }

    #[test]
    fn dated_carry_probe_uses_shared_entry_rule_and_round_trips_through_scouts_contract() {
        let candidate = candidate();
        let parameters = BTreeMap::from([
            ("carry_minimum_days_to_expiry".into(), 7.0),
            ("carry_cost_gate_percent".into(), 0.4),
        ]);
        let tested = BTreeMap::from([("carry_cost_gate_percent".into(), 0.4)]);
        let planned = dated_carry_probe_plan_candidate(
            &candidate,
            1_700_000_000_000,
            "carry_cost_gate_percent",
            0.4,
            &parameters,
            &tested,
            CARRY_DAY_MS,
        );
        assert_eq!(planned.sample_group_id, "dated-carry-scout:BTCUSDT_Q1:1700000000000:carry_cost_gate_percent:0.40000000");
        assert!(planned.eligible);
        let Some(scouts::ShadowSamplePayload::DatedCarryProbe(probe)) = planned.sample_payload.as_ref() else {
            panic!("dated-carry plan should carry a typed pending sample");
        };
        assert_eq!(probe.parameter_key, "carry_cost_gate_percent");
        assert_eq!(probe.parameter_value, 0.4);
        assert_eq!(probe.tested_parameters, tested);
        assert!(probe.would_enter);

        let encoded = serde_json::to_vec(&planned).unwrap();
        let decoded: scouts::ShadowPlanCandidate = serde_json::from_slice(&encoded).unwrap();
        let Some(scouts::ShadowSamplePayload::DatedCarryProbe(decoded_probe)) = decoded.sample_payload else {
            panic!("dated-carry payload should deserialize through the shared Scouts contract");
        };
        assert_eq!(decoded_probe.sample_id, probe.sample_id);
        assert_eq!(decoded_probe.parameters, parameters);
        assert_eq!(decoded_probe.tested_parameters, tested);
    }

    #[test]
    fn ineligible_parameter_variant_remains_a_zero_outcome_probe() {
        let candidate = candidate();
        let parameters = BTreeMap::from([
            ("carry_minimum_days_to_expiry".into(), 7.0),
            ("carry_cost_gate_percent".into(), 0.5),
        ]);
        let tested = BTreeMap::from([("carry_cost_gate_percent".into(), 0.5)]);
        let planned = dated_carry_probe_plan_candidate(
            &candidate,
            1_700_000_000_000,
            "carry_cost_gate_percent",
            0.5,
            &parameters,
            &tested,
            CARRY_DAY_MS,
        );
        assert!(planned.eligible, "the probe should be admitted to record a zero return");
        let Some(scouts::ShadowSamplePayload::DatedCarryProbe(probe)) = planned.sample_payload else {
            panic!("dated-carry plan should carry a typed pending sample");
        };
        assert!(!probe.would_enter);
    }

    #[test]
    fn legacy_pending_probe_and_cooldown_fields_deserialize_then_disappear_when_cleared() {
        let legacy = serde_json::json!({
            "enabled": false,
            "max_spot_notional_usdt": 1000.0,
            "scout_shadow_probes": [{
                "candidate": {
                    "spot_symbol": "BTCUSDT",
                    "future_symbol": "BTCUSDT_Q1",
                    "expiry_ms": 1700864000000_u64,
                    "days_to_expiry": 10.0,
                    "quantity": 0.01,
                    "spot_ask": 100.0,
                    "future_bid": 101.0,
                    "spot_bid": 99.9,
                    "future_ask": 101.1,
                    "basis_percent": 1.0,
                    "net_after_budget_percent": 0.5,
                    "eligible": true,
                    "reason": "eligible"
                },
                "opened_at_ms": 1700000000000_u64,
                "parameter_key": "carry_cost_gate_percent",
                "parameter_value": 0.4,
                "parameters": {"carry_minimum_days_to_expiry": 7.0, "carry_cost_gate_percent": 0.4},
                "tested_parameters": {"carry_cost_gate_percent": 0.4},
                "would_enter": true
            }],
            "scout_shadow_last_started": {"BTCUSDT:carry_cost_gate_percent:0.40000000": 19675_u64}
        });
        let mut book: DatedCarryBook = serde_json::from_value(legacy).unwrap();
        assert_eq!(book.scout_shadow_probes.len(), 1);
        assert_eq!(book.scout_shadow_last_started.len(), 1);

        book.scout_shadow_probes.clear();
        book.scout_shadow_last_started.clear();
        let saved = serde_json::to_value(book).unwrap();
        assert!(saved.get("scout_shadow_probes").is_none());
        assert!(saved.get("scout_shadow_last_started").is_none());
    }
}
