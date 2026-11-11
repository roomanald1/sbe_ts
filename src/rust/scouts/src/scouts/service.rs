#[derive(Clone)]
struct ServiceState {
    learning_rsocket_url: String,
    insights_rsocket_url: String,
    auto_trader_rsocket_url: String,
    strategy_outcomes_by_profile:
        Arc<std::sync::RwLock<BTreeMap<String, Vec<StrategyOutcomeProfileIndexEntry>>>>,
    portfolio_instances_by_group:
        Arc<std::sync::RwLock<BTreeMap<String, Vec<PortfolioInstanceEvidence>>>>,
    strategy_outcome_profile_index_ready: Arc<std::sync::atomic::AtomicBool>,
    live_scout_maximum_duration_ms: Arc<std::sync::RwLock<BTreeMap<String, u64>>>,
    scout_outcomes_cache: Arc<Mutex<BTreeMap<String, (std::time::Instant, axum::body::Bytes)>>>,
    scout: ScoutState,
}

const LIVE_SCOUT_HOLD_PRIOR_SAMPLES: usize = 10;
const LIVE_SCOUT_DEFAULT_HOLD_MS: u64 = 30 * 60 * 1_000;
const LIVE_SCOUT_MIN_DURATION_MS: u64 = 15 * 60 * 1_000;
const LIVE_SCOUT_MAX_DURATION_MS: u64 = 6 * 60 * 60 * 1_000;

fn estimate_live_scout_maximum_durations(
    activity: &serde_json::Value,
) -> BTreeMap<String, u64> {
    let mut by_strategy = BTreeMap::<String, Vec<u64>>::new();
    for row in activity["closed_ledger_rows"].as_array().into_iter().flatten() {
        let strategy = row["strategy"].as_str().unwrap_or_default();
        if !strategy.starts_with("live-")
            || row["scout"].as_bool().unwrap_or(false)
            || !row["rowId"].as_str().unwrap_or_default().starts_with("ledger-summary:")
        {
            continue;
        }
        let (Some(opened), Some(closed)) = (
            row["openedAt"].as_u64(),
            row["closedAt"].as_u64(),
        ) else {
            continue;
        };
        let duration = closed.saturating_sub(opened);
        if duration > 0 && duration <= 7 * 86_400_000 {
            by_strategy
                .entry(strategy.to_owned())
                .or_default()
                .push(duration);
        }
    }
    let mut all_durations = by_strategy
        .values()
        .flat_map(|durations| durations.iter().copied())
        .collect::<Vec<_>>();
    all_durations.sort_unstable();
    let pooled_median = all_durations
        .get(all_durations.len().saturating_sub(1) / 2)
        .copied()
        .unwrap_or(LIVE_SCOUT_DEFAULT_HOLD_MS);
    let winsor_limit = pooled_median
        .saturating_mul(3)
        .clamp(LIVE_SCOUT_MIN_DURATION_MS, LIVE_SCOUT_MAX_DURATION_MS);
    let pooled_mean = if all_durations.is_empty() {
        LIVE_SCOUT_DEFAULT_HOLD_MS
    } else {
        all_durations
            .iter()
            .map(|duration| (*duration).min(winsor_limit))
            .sum::<u64>()
            / all_durations.len() as u64
    };
    by_strategy
        .into_iter()
        .map(|(strategy, durations)| {
            let sample_count = durations.len();
            let observed_total = durations
                .iter()
                .map(|duration| (*duration).min(winsor_limit))
                .sum::<u64>();
            let shrinkage_weight = LIVE_SCOUT_HOLD_PRIOR_SAMPLES as u64;
            let shrunk_mean = observed_total
                .saturating_add(pooled_mean.saturating_mul(shrinkage_weight))
                / (sample_count as u64 + shrinkage_weight);
            let maximum_duration = shrunk_mean
                .saturating_mul(2)
                .clamp(LIVE_SCOUT_MIN_DURATION_MS, LIVE_SCOUT_MAX_DURATION_MS);
            (strategy, maximum_duration)
        })
        .collect()
}

fn schedule_scout_outbox_flush(service: &ServiceState) {
    schedule_scout_outbox_flush_with(
        service.scout.clone(),
        service.learning_rsocket_url.clone(),
        service.insights_rsocket_url.clone(),
    );
}

fn schedule_scout_outbox_flush_with(
    scout: ScoutState,
    learning_rsocket_url: String,
    insights_rsocket_url: String,
) {
    if scout
        .flush_scheduled
        .compare_exchange(false, true, std::sync::atomic::Ordering::AcqRel, std::sync::atomic::Ordering::Acquire)
        .is_err()
    {
        return;
    }
    tokio::spawn(async move {
        let result = flush_scout_outbox(&scout, &learning_rsocket_url, &insights_rsocket_url).await;
        scout
            .flush_scheduled
            .store(false, std::sync::atomic::Ordering::Release);
        if let Err((_, error)) = result {
            eprintln!("WARN Scout outbox delivery deferred: {error}");
            return;
        }
        let pending = !scout.data.lock().await.outbox.is_empty();
        if pending {
            schedule_scout_outbox_flush_with(scout, learning_rsocket_url, insights_rsocket_url);
        }
    });
}

pub fn router() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
}

pub fn strategy_worker_router(strategy: strategies::Strategy, state_path: PathBuf) -> Router {
    router_with_state(state_path, strategy)
}

/// Explicit offline migration for the former shared JSON/SQLite state file.
/// Existing worker databases are never overwritten.
pub fn import_strategy_partition(
    source_path: &std::path::Path,
    destination_path: &std::path::Path,
    strategy: strategies::Strategy,
) -> anyhow::Result<usize> {
    if destination_path.exists() {
        anyhow::bail!("refusing to overwrite existing strategy state {}", destination_path.display());
    }
    let state = load_state_for_import(source_path)?;
    import_partition_state(state, destination_path, strategy)
}

/// One-pass partition import for large SQLite state files. Source state is
/// read once, then cloned and filtered into each configured strategy database.
pub fn import_strategy_partitions(
    source_path: &std::path::Path,
    destination_directory: &std::path::Path,
) -> anyhow::Result<Vec<(String, usize)>> {
    let source_state = load_state_for_import(source_path)?;
    let strategies = source_state
        .scout_configuration
        .strategies
        .iter()
        .map(|name| {
            strategies::Strategy::from_name(name)
                .ok_or_else(|| anyhow::anyhow!("unknown strategy in source config: {name}"))
        })
        .collect::<anyhow::Result<Vec<_>>>()?;
    if strategies.is_empty() {
        anyhow::bail!("source Scout state has no configured strategies");
    }
    std::fs::create_dir_all(destination_directory)?;
    for strategy in &strategies {
        let destination = destination_directory.join(format!("{}.sqlite", strategy.as_str()));
        if destination.exists() {
            anyhow::bail!("refusing to overwrite existing strategy state {}", destination.display());
        }
    }
    strategies
        .into_iter()
        .map(|strategy| {
            let destination = destination_directory.join(format!("{}.sqlite", strategy.as_str()));
            let count = import_partition_state(source_state.clone(), &destination, strategy)?;
            Ok((strategy.as_str().to_owned(), count))
        })
        .collect()
}

fn load_state_for_import(source_path: &std::path::Path) -> anyhow::Result<PersistedState> {
    let mut source = std::fs::File::open(source_path)?;
    let mut prefix = [0u8; 32];
    let bytes_read = std::io::Read::read(&mut source, &mut prefix)?;
    let is_json = prefix[..bytes_read]
        .iter()
        .copied()
        .find(|byte| !byte.is_ascii_whitespace())
        == Some(b'{');
    if is_json {
        Ok(serde_json::from_slice(&std::fs::read(source_path)?)?)
    } else {
        let mut value = load_scout_state_value(source_path)?;
        normalize_import_outbox(&mut value);
        Ok(serde_json::from_value(value)?)
    }
}

fn import_partition_state(
    mut state: PersistedState,
    destination_path: &std::path::Path,
    strategy: strategies::Strategy,
) -> anyhow::Result<usize> {
    if destination_path.exists() {
        anyhow::bail!("refusing to overwrite existing strategy state {}", destination_path.display());
    }
    let strategy_name = strategy.as_str();
    let includes_strategy = |value: &serde_json::Value| json_has_strategy(value, strategy_name);
    state.scout_configuration.strategies = vec![strategy_name.to_owned()];
    state.outcomes.retain(|_, outcome| outcome.strategy == strategy_name);
    state.scout_instances.retain(|_, row| includes_strategy(row));
    state.pairs.retain(|_, pair| {
        serde_json::to_value(pair).is_ok_and(|value| includes_strategy(&value))
    });
    state.outbox.retain(|item| {
        serde_json::to_value(item).is_ok_and(|value| includes_strategy(&value))
    });
    state.seen.retain(|key| key.contains(strategy_name) || (strategy == strategies::Strategy::DatedCarry && key.contains("dated-carry")));
    state.scout_sample_at_ms.retain(|key, _| key.contains(strategy_name) || (strategy == strategies::Strategy::DatedCarry && key.contains("dated-carry")));
    state.scout_sample_active_scenarios.retain(|key, _| key.contains(strategy_name) || (strategy == strategies::Strategy::DatedCarry && key.contains("dated-carry")));
    state.scout_plan_profiles.retain(|key, value| key.contains(strategy_name) || value == strategy_name);
    state.scout_plan_results.retain(|key, response| {
        response.audits.retain(|audit| includes_strategy(audit));
        response.selected_groups.retain(|group| group.contains(strategy_name));
        !response.audits.is_empty()
            || !response.selected_groups.is_empty()
            || key.contains(strategy_name)
    });
    state.scout_pending_samples.retain(|key, sample| key.contains(strategy_name) || serde_json::to_value(sample).is_ok_and(|value| includes_strategy(&value)));
    state.scout_observation_results.retain(|key, result| key.contains(strategy_name) || (strategy == strategies::Strategy::DatedCarry && key.contains("dated-carry")) || serde_json::to_value(result).is_ok_and(|value| includes_strategy(&value)));
    state.scout_active_audits.retain(|key, audits| {
        audits.retain(|audit| includes_strategy(audit));
        !audits.is_empty() || key.contains(strategy_name)
    });
    let count = state.outcomes.len() + state.scout_instances.len() + state.scout_active_audits.values().map(Vec::len).sum::<usize>();
    persist_scout_state(
        destination_path,
        &state,
        &["scout_configuration", "seen", "outcomes", "scout_instances", "pairs", "outbox", "scout_sample_at_ms", "scout_sample_active_scenarios", "scout_plan_profiles", "scout_plan_results", "scout_pending_samples", "scout_observation_results", "scout_active_audits"],
    ).map_err(|(_, error)| anyhow::anyhow!(error))?;
    Ok(count)
}

fn normalize_import_outbox(state: &mut serde_json::Value) {
    for item in state["outbox"].as_array_mut().into_iter().flatten() {
        let payload = &mut item["payload"];
        if payload["kind"] == "scout_outcome" && payload.get("comparison").is_none() {
            payload["comparison"] = serde_json::json!({"status":"independent"});
        }
    }
}

fn json_has_strategy(value: &serde_json::Value, strategy: &str) -> bool {
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            (matches!(key.as_str(), "strategy" | "strategy_name" | "scout_strategy")
                && value.as_str().is_some_and(|name| name == strategy || (strategy == "dated-futures-carry" && name == "dated-carry")))
                || json_has_strategy(value, strategy)
        }),
        serde_json::Value::Array(values) => values.iter().any(|value| json_has_strategy(value, strategy)),
        _ => false,
    }
}

fn router_with_state(state_path: PathBuf, worker_strategy: strategies::Strategy) -> Router {
    let mut persisted = load_scout_state(&state_path).unwrap_or_default();
    persisted.scout_configuration.strategies = vec![worker_strategy.as_str().to_owned()];
    let _ = persist_scout_state(&state_path, &persisted, &["scout_configuration"]);
    let (changed_instances, changed_audit_groups, changed_plan_results) =
        compact_persisted_scout_payloads(&mut persisted);
    if let Err(error) = persist_compacted_scout_payloads(
        &state_path,
        &persisted,
        &changed_instances,
        &changed_audit_groups,
        &changed_plan_results,
        &[],
    ) {
        eprintln!("WARN Could not compact persisted Scout payloads: {error}");
    }
    if let Err(error) = compact_scout_instances_in_db(&state_path) {
        eprintln!("WARN Could not compact persisted Scout instances: {error}");
    }
    rebuild_strategy_outcome_profile_index(&mut persisted);
    if let Err(error) = rebuild_portfolio_instance_index_from_db(&state_path, &mut persisted) {
        eprintln!("WARN Could not rebuild portfolio instance evidence index: {error}");
    }
    let strategy_outcomes_by_profile = Arc::clone(&persisted.strategy_outcomes_by_profile);
    let portfolio_instances_by_group = Arc::clone(&persisted.portfolio_instances_by_group);
    let strategy_outcome_profile_index_ready = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let learning_rsocket_url = std::env::var("LEARNING_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10010".into());
    let insights_rsocket_url = std::env::var("INSIGHTS_RPC_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10007".into());
    let auto_trader_rsocket_url = std::env::var("AUTO_TRADER_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10006".into());
    let scout = ScoutState {
        data: Arc::new(Mutex::new(persisted)),
        path: state_path,
        flush_lock: Arc::new(Mutex::new(())),
        flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    // Retry durable pending work after startup or a transient downstream
    // failure, even if no new Scout observations arrive.
    let retry_scout = scout.clone();
    let retry_learning_rsocket_url = learning_rsocket_url.clone();
    let retry_insights_rsocket_url = insights_rsocket_url.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(10));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if !retry_scout.data.lock().await.outbox.is_empty() {
                schedule_scout_outbox_flush_with(
                    retry_scout.clone(),
                    retry_learning_rsocket_url.clone(),
                    retry_insights_rsocket_url.clone(),
                );
            }
        }
    });
    let live_scout_maximum_duration_ms =
        Arc::new(std::sync::RwLock::new(BTreeMap::new()));
    let hold_refresh_cache = Arc::clone(&live_scout_maximum_duration_ms);
    let hold_refresh_rsocket_url = auto_trader_rsocket_url.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(300));
        loop {
            ticker.tick().await;
            let activity = strategies::request_scout_service_route(
                &hold_refresh_rsocket_url,
                &strategies::ScoutCoordinatorRouteCommand {
                    request_id: format!("scout-activity:{}", scout_now_ms()),
                    strategy: None,
                    method: strategies::ScoutWorkerHttpMethod::Get,
                    path: "/positions/auto-trader/activity".into(),
                    payload: serde_json::Value::Null,
                },
            )
            .await;
            let Ok(response) = activity else {
                continue;
            };
            if response.status >= 400 {
                continue;
            }
            let estimates = estimate_live_scout_maximum_durations(&response.body);
            if !estimates.is_empty() {
                *hold_refresh_cache
                    .write()
                    .unwrap_or_else(|error| error.into_inner()) = estimates;
            }
        }
    });
    let replay_insights_rsocket_url = insights_rsocket_url.clone();
    let replay_scout = scout.clone();
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
        loop {
            ticker.tick().await;
            let registrations = {
                let state = replay_scout.data.lock().await;
                active_scout_registrations(&state)
            };
            let mut retry_replay = false;
            for registration in registrations {
                let response = strategies::request_scout_service_route(
                    &replay_insights_rsocket_url,
                    &strategies::ScoutCoordinatorRouteCommand {
                        request_id: registration["decision_id"]
                            .as_str()
                            .or_else(|| registration["id"].as_str())
                            .unwrap_or("replay-registration")
                            .to_owned(),
                        strategy: None,
                        method: strategies::ScoutWorkerHttpMethod::Post,
                        path: "/internal/scouts/register".into(),
                        payload: registration,
                    },
                )
                .await;
                match response {
                    Ok(response) if (200..300).contains(&response.status) => {}
                    Ok(response) if (400..500).contains(&response.status) => {
                        eprintln!(
                            "WARN Insights rejected a replayed Scout registration: status={} detail={}",
                            response.status,
                            response.body.to_string().chars().take(512).collect::<String>()
                        );
                    }
                    Ok(response) => {
                        eprintln!(
                            "WARN Scout registration replay deferred: Insights returned {}",
                            response.status
                        );
                        retry_replay = true;
                    }
                    Err(error) => {
                        eprintln!("WARN Scout registration replay deferred: {error}");
                        retry_replay = true;
                    }
                }
            }
            if retry_replay {
                eprintln!("WARN Scout registration replay will retry after the next interval");
            }
        }
    });
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
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
        .route("/strategy-walk-forward", post(get_strategy_walk_forward))
        .with_state(ServiceState {
            learning_rsocket_url,
            insights_rsocket_url,
            auto_trader_rsocket_url,
            strategy_outcomes_by_profile,
            portfolio_instances_by_group,
            strategy_outcome_profile_index_ready,
            live_scout_maximum_duration_ms,
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout,
        })
}

async fn process_cpu_seconds() -> String {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return "0".to_owned();
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    (user + system).to_string()
}

async fn process_memory_bytes() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find_map(|line| line.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse::<u64>().ok()))
        .map(|kilobytes| (kilobytes * 1024).to_string())
        .unwrap_or_else(|| "0".to_owned())
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
        || configuration.max_active_positions > 10_000
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
    persist_scout_state(&service.scout.path, &state, &["scout_configuration"])?;
    Ok(StatusCode::ACCEPTED)
}




// Keep live exploration bounded while giving each symbol/profile enough arms
// to collect evidence faster. Profiles remain keyed by strategy, symbol and
// direction, so probes from unrelated instruments are never pooled.
const MAX_PARAMETER_VARIANTS_PER_BATCH: usize = 5;
const MAX_LIVE_PROFILES_PER_SYMBOL_DIRECTION: usize = 2;
const SCOUT_PARAMETER_SAMPLE_INTERVAL_MS: u64 = 4 * 60 * 60 * 1_000;

fn ranked_parameter_variants(
    snapshot: &serde_json::Value,
    profile_symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    day: u64,
) -> Vec<ParameterVariant> {
    let candidates = single_parameter_variants(parameters, specs, day)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|changes| {
            let (key, value) = changes.iter().next()?;
            Some(ParameterVariant {
                key: key.clone(),
                value: *value,
                changes,
            })
        })
        .collect::<Vec<_>>();
    let mut known = Vec::<(f64, ParameterVariant)>::new();
    let mut unexplored = Vec::<ParameterVariant>::new();
    for candidate in candidates {
        let evidence = snapshot
            .get("scout_parameter_variants")
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .find(|variant| {
                variant.get("symbol").and_then(serde_json::Value::as_str)
                    == Some(profile_symbol)
                    && variant.get("strategy").and_then(serde_json::Value::as_str)
                        == Some(strategy)
                    && variant.get("parameter").and_then(serde_json::Value::as_str)
                        == Some(candidate.key.as_str())
                    && variant.get("candidate_value").and_then(serde_json::Value::as_f64)
                        .is_some_and(|value| (value - candidate.value).abs() <= 1e-8)
            });
        match evidence {
            Some(variant)
                if variant.get("rejected").and_then(serde_json::Value::as_bool)
                    != Some(true) =>
            {
                if let Some(score) = variant
                    .get("average_net_improvement_percent")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|score| score.is_finite())
                {
                    known.push((score, candidate));
                } else {
                    unexplored.push(candidate);
                }
            }
            Some(_) => {}
            None => unexplored.push(candidate),
        }
    }
    known.sort_by(|left, right| {
        right.0.total_cmp(&left.0).then_with(|| left.1.key.cmp(&right.1.key))
    });
    unexplored.sort_by(|left, right| left.key.cmp(&right.key));
    if !unexplored.is_empty() {
        let offset = day as usize % unexplored.len();
        unexplored.rotate_left(offset);
    }
    let direction_of = |candidate: &ParameterVariant| {
        candidate.changes.iter().next().and_then(|(key, value)| {
            parameters
                .get(key)
                .copied()
                .or_else(|| specs.iter().find(|spec| spec.key == *key).map(|spec| spec.default))
                .map(|baseline| if *value < baseline { -1_i8 } else { 1_i8 })
        })
    };
    let candidate_id = |candidate: &ParameterVariant| {
        (candidate.key.clone(), format!("{:.8}", candidate.value))
    };
    let mut selected = Vec::new();
    let mut selected_ids = BTreeSet::new();

    // Keep both sides represented in every live batch whenever each side has
    // an available candidate. Prefer an unexplored value on each side; use
    // the best known result only when that side has no unexplored value.
    for direction in [1_i8, -1_i8] {
        let unexplored_index = unexplored
            .iter()
            .position(|candidate| direction_of(candidate) == Some(direction));
        let candidate = unexplored_index
            .map(|index| unexplored.remove(index))
            .or_else(|| {
                let known_index = known
                    .iter()
                    .position(|(_, candidate)| direction_of(candidate) == Some(direction))?;
                Some(known.remove(known_index).1)
            });
        if let Some(candidate) = candidate {
            selected_ids.insert(candidate_id(&candidate));
            selected.push(candidate);
        }
    }

    for (_, candidate) in known {
        if selected.len() >= MAX_PARAMETER_VARIANTS_PER_BATCH {
            break;
        }
        if selected_ids.insert(candidate_id(&candidate)) {
            selected.push(candidate);
        }
    }
    for candidate in unexplored {
        if selected.len() >= MAX_PARAMETER_VARIANTS_PER_BATCH {
            break;
        }
        if selected_ids.insert(candidate_id(&candidate)) {
            selected.push(candidate);
        }
    }
    selected
}

fn daily_parameter_batch_key(signal: &serde_json::Value) -> String {
    let decision = &signal["strategy_decision"];
    let baseline = decision
        .get("scout_parameter_baseline_decision")
        .unwrap_or(decision);
    format!(
        "{}|{}|{}|{}|{}|window:{}",
        signal["strategy"].as_str().unwrap_or_default(),
        signal["symbol"].as_str().unwrap_or_default(),
        signal["direction"].as_str().unwrap_or_default(),
        baseline["bar_close_timestamp"]
            .as_u64()
            .or_else(|| baseline["monthly_anchor_close_time_ms"].as_u64())
            .or_else(|| signal["timestamp"].as_u64())
            .unwrap_or_default(),
        baseline["strategy_parameters"],
        signal["_scout_probe_window"].as_u64().unwrap_or_default()
    )
}

fn group_daily_parameter_variants(
    signals: Vec<serde_json::Value>,
) -> Vec<(usize, serde_json::Value, Vec<serde_json::Value>, String)> {
    let mut groups = BTreeMap::<String, (usize, Vec<serde_json::Value>)>::new();
    for (index, signal) in signals.into_iter().enumerate() {
        let is_probe = signal
            .get("strategy_decision")
            .and_then(|decision| decision.get("scout_parameter_probe"))
            .and_then(serde_json::Value::as_bool)
            == Some(true);
        let key = if is_probe {
            format!("probe:{}", daily_parameter_batch_key(&signal))
        } else {
            format!("single:{index}")
        };
        let entry = groups.entry(key).or_insert_with(|| (index, Vec::new()));
        entry.0 = entry.0.min(index);
        entry.1.push(signal);
    }
    let mut batches = groups
        .into_iter()
        .map(|(key, (index, mut signals))| {
            signals.sort_by(|left, right| {
                left["strategy_decision"]["scout_parameter_key"]
                    .as_str()
                    .cmp(&right["strategy_decision"]["scout_parameter_key"].as_str())
                    .then_with(|| {
                        left["strategy_decision"]["scout_parameter_value"]
                            .as_f64()
                            .partial_cmp(&right["strategy_decision"]["scout_parameter_value"].as_f64())
                            .unwrap_or(std::cmp::Ordering::Equal)
                    })
            });
            if key.starts_with("probe:") && signals.len() > MAX_PARAMETER_VARIANTS_PER_BATCH {
                let day = signals[0]["timestamp"].as_u64().unwrap_or_default() / 86_400_000;
                let offset = day as usize % signals.len();
                signals.rotate_left(offset);
                signals.truncate(MAX_PARAMETER_VARIANTS_PER_BATCH);
            }
            let batch_id = probe_id(&key);
            (index, signals[0].clone(), signals, batch_id)
        })
        .collect::<Vec<_>>();
    batches.sort_by_key(|batch| batch.0);
    batches
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
    let mut portfolio_templates = Vec::new();
    let probe_window = request.now_ms / SCOUT_PARAMETER_SAMPLE_INTERVAL_MS;
    let signal_batches = group_daily_parameter_variants(
        request
            .signals
            .into_iter()
            .map(|mut signal| {
                signal["_scout_probe_window"] = serde_json::json!(probe_window);
                signal
            })
            .collect(),
    );
    for (index, signal, probe_signals, batch_id) in signal_batches {
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
        let decision_id = format!("{}|{}|{}|{}|scout:daily:{}:{index}", timestamp, symbol, direction, strategy_name, probe_window);
        let mut audit = signal.clone();
        audit
            .as_object_mut()
            .map(|object| object.remove("_scout_probe_window"));
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
        let template = ScoutAuditTemplate {
            base_audit: audit,
            notional_usd: configuration.max_sample_notional,
            learning_context: format!("scout:daily:{strategy_name}:{symbol}:{direction}:scenario"),
            stop_loss_percent: stop,
            take_profit_percent: target,
            level_percent: 0.0,
            strategy_only: false,
            parameter_entry_eligible: probe.map(|_| matches!(decision, strategies::StrategyDecision::Enter | strategies::StrategyDecision::Scout)),
            parameter_specs: strategies::scout_parameter_specs(strategy),
        };
        if strategy == strategies::Strategy::RiskAdjustedTrendPortfolio
            && decision == strategies::StrategyDecision::Enter
            && probe.is_none()
        {
            portfolio_templates.push(template);
            continue;
        }
        let (audit_templates, entry_eligible, sample_group_id) = if let Some((_key, _value)) = probe {
            let Some(baseline_decision) = signal
                .get("strategy_decision")
                .and_then(|decision| decision.get("scout_parameter_baseline_decision"))
                .cloned()
            else {
                continue;
            };
            let baseline_parameters = baseline_decision
                .get("strategy_parameters")
                .cloned()
                .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok())
                .unwrap_or_default();
            let mut baseline_signal = signal.clone();
            baseline_signal["strategy_decision"] = baseline_decision.clone();
            let baseline_input = strategies::StrategyInput::from_candidate_json(
                baseline_signal,
                baseline_parameters.clone(),
            )
            .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
            let baseline_decision_result = strategy
                .evaluate(&baseline_input)
                .unwrap_or(strategies::StrategyDecision::Reject);
            let baseline_eligible = matches!(
                baseline_decision_result,
                strategies::StrategyDecision::Enter | strategies::StrategyDecision::Scout
            );
            let mut eligible_variants = Vec::new();
            for probe_signal in &probe_signals {
                let variant_decision = probe_signal
                    .get("strategy_decision")
                    .cloned()
                    .unwrap_or_default();
                let variant_parameters = variant_decision
                    .get("strategy_parameters")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok())
                    .unwrap_or_default();
                let variant_input = strategies::StrategyInput::from_candidate_json(
                    probe_signal.clone(),
                    variant_parameters.clone(),
                )
                .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
                let variant_result = strategy
                    .evaluate(&variant_input)
                    .unwrap_or(strategies::StrategyDecision::Reject);
                let Some((variant_key, variant_value)) = variant_decision
                    .get("scout_parameter_key").and_then(serde_json::Value::as_str)
                    .zip(variant_decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64))
                else { continue; };
                let variant_key = variant_key.to_owned();
                let variant_eligible = matches!(variant_result, strategies::StrategyDecision::Enter | strategies::StrategyDecision::Scout);
                eligible_variants.push((
                    variant_decision,
                    variant_parameters,
                    variant_key,
                    variant_value,
                    variant_eligible,
                ));
            }
            let has_eligible_variant = eligible_variants.iter().any(|(_, _, _, _, eligible)| *eligible);
            if !paired_probe_has_entering_side(baseline_eligible, has_eligible_variant) {
                continue;
            }
            let profile_symbol = signal
                .get("strategy_decision")
                .and_then(|decision| decision.get("scout_parameter_profile_symbol"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(symbol);
            let mut templates = Vec::with_capacity(eligible_variants.len() + 1);
            let comparison_ids = eligible_variants.iter().map(|(_, _, variant_key, variant_value, _)| {
                format!("scout-pair:{}", probe_id(&format!("{batch_id}|{variant_key}|{variant_value:.8}")))
            }).collect::<Vec<_>>();
            if baseline_eligible {
                let mut baseline_template = template.clone();
                baseline_template.strategy_only = true;
                baseline_template.parameter_entry_eligible = Some(true);
                baseline_template.base_audit["strategy_decision"] = baseline_decision;
                baseline_template.base_audit["strategy_parameters"] =
                    serde_json::to_value(&baseline_parameters).unwrap_or_default();
                baseline_template.base_audit["stop_loss_percent"] = serde_json::json!(
                    baseline_parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent)
                );
                baseline_template.base_audit["take_profit_percent"] = serde_json::json!(
                    baseline_parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent)
                );
                baseline_template.base_audit["scout_comparison_role"] = serde_json::json!("baseline");
                baseline_template.base_audit["scout_comparison_ids"] = serde_json::json!(comparison_ids);
                baseline_template.base_audit["scout_comparison_parameter_profile_symbol"] = serde_json::json!(profile_symbol);
                let zero_sides = eligible_variants.iter().filter_map(|(_, _, variant_key, variant_value, eligible)| {
                    (!eligible).then(|| serde_json::json!({
                        "comparison_id": format!("scout-pair:{}", probe_id(&format!("{batch_id}|{variant_key}|{variant_value:.8}"))),
                        "parameter_key": variant_key,
                        "parameter_value": variant_value,
                        "profile_symbol": profile_symbol,
                    }))
                }).collect::<Vec<_>>();
                baseline_template.base_audit["scout_comparison_zero_sides"] = serde_json::json!(zero_sides);
                baseline_template.base_audit.as_object_mut().map(|audit| {
                    audit.remove("scout_parameter_key");
                    audit.remove("scout_parameter_value");
                    audit.remove("scout_parameter_profile_symbol");
                    audit.remove("scout_parameter_entry_eligible");
                });
                templates.push(baseline_template);
            }
            for (variant_decision, variant_parameters, variant_key, variant_value, variant_eligible) in eligible_variants {
                if !variant_eligible { continue; }
                let comparison_id = format!("scout-pair:{}", probe_id(&format!("{batch_id}|{variant_key}|{variant_value:.8}")));
                let mut variant_template = template.clone();
                variant_template.strategy_only = true;
                variant_template.parameter_entry_eligible = Some(true);
                variant_template.base_audit["strategy_decision"] = variant_decision;
                variant_template.base_audit["strategy_parameters"] = serde_json::to_value(&variant_parameters).unwrap_or_default();
                variant_template.base_audit["stop_loss_percent"] = serde_json::json!(variant_parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent));
                variant_template.base_audit["take_profit_percent"] = serde_json::json!(variant_parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent));
                variant_template.base_audit["scout_parameter_key"] = serde_json::json!(variant_key);
                variant_template.base_audit["scout_parameter_value"] = serde_json::json!(variant_value);
                variant_template.base_audit["scout_parameter_profile_symbol"] = serde_json::json!(profile_symbol);
                variant_template.base_audit["scout_comparison_role"] = serde_json::json!("variant");
                variant_template.base_audit["scout_comparison_id"] = serde_json::json!(comparison_id);
                variant_template.base_audit["scout_comparison_parameter_key"] = serde_json::json!(variant_key);
                variant_template.base_audit["scout_comparison_parameter_value"] = serde_json::json!(variant_value);
                variant_template.base_audit["scout_comparison_parameter_profile_symbol"] = serde_json::json!(profile_symbol);
                if !baseline_eligible {
                    variant_template.base_audit["scout_comparison_zero_role"] = serde_json::json!("baseline");
                }
                templates.push(variant_template);
            }
            (templates, true, format!("daily-parameter-batch:{batch_id}"))
        } else {
            (vec![template], true, String::new())
        };
        let cooldown_key = if sample_group_id.is_empty() {
            String::new()
        } else {
            format!("daily|paired-batch:{batch_id}")
        };
        candidates.push(ScoutPlanCandidate {
            generate_group_id: sample_group_id.is_empty(),
            sample_group_id,
            cooldown_key,
            profile_key: format!("{strategy_name}|{symbol}"),
            eligible: entry_eligible,
            parameter_variant: probe.is_some(),
            cooldown_ms: if probe.is_some() { SCOUT_PARAMETER_SAMPLE_INTERVAL_MS } else { 300_000 },
            audit_plans: Vec::new(),
            audit_templates,
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
    if !portfolio_templates.is_empty()
        && portfolio_templates.len() <= configuration.max_active_positions
    {
        let sample_group_id = format!("risk-adjusted-trend-portfolio:{}", request.now_ms);
        candidates.insert(0, ScoutPlanCandidate {
            generate_group_id: false,
            sample_group_id,
            cooldown_key: "risk-adjusted-trend-portfolio".into(),
            profile_key: "risk-adjusted-trend-portfolio".into(),
            eligible: true,
            parameter_variant: false,
            cooldown_ms: 86_400_000,
            audit_plans: Vec::new(),
            audit_templates: portfolio_templates,
            sample_payload: None,
            daily_policy: None,
            live_policy: None,
        });
    }
    if candidates.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let plan = ScoutPlanRequest {
        request_id: request.request_id,
        now_ms: request.now_ms,
        max_active_batches: configuration.max_active_samples,
        max_active_variants_per_profile: 6,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates,
    };
    let _ = create_scout_plan(State(service), Json(plan)).await?;
    Ok(StatusCode::ACCEPTED)
}

fn paired_probe_has_entering_side(baseline_eligible: bool, variant_eligible: bool) -> bool {
    baseline_eligible || variant_eligible
}

#[derive(Default)]
struct LiveScoutProbeBatch {
    strategy: String,
    symbol: String,
    direction: String,
    signal_timestamp_ms: u64,
    baseline_profile_key: String,
    baseline_parameters: BTreeMap<String, f64>,
    baseline_signal: Option<serde_json::Value>,
    variants: BTreeMap<(String, String), (serde_json::Value, ParameterVariant)>,
}

fn live_profile_arm_key(profile: &LiveScoutProfile) -> String {
    format!(
        "{}|{}|{}",
        profile.strategy,
        profile.symbol,
        serde_json::to_string(&profile.parameters).unwrap_or_default()
    )
}

#[cfg(test)]
mod paired_probe_tests {
    use super::{
        daily_parameter_batch_key, group_daily_parameter_variants, paired_probe_has_entering_side,
        ranked_parameter_variants, BTreeMap, BTreeSet, ParameterSpec,
        MAX_PARAMETER_VARIANTS_PER_BATCH,
    };
    use crate::scouts::probe_id;

    #[test]
    fn parameter_probe_skips_only_when_both_profiles_reject_the_signal() {
        assert!(paired_probe_has_entering_side(true, true));
        assert!(paired_probe_has_entering_side(true, false));
        assert!(paired_probe_has_entering_side(false, true));
        assert!(!paired_probe_has_entering_side(false, false));
    }

    #[test]
    fn cross_path_variant_ranking_caps_at_five_and_reserves_exploration() {
        let specs = ["a", "b", "c", "d"]
            .into_iter()
            .map(|key| ParameterSpec {
                key: key.into(),
                label: key.into(),
                default: 5.0,
                min: 0.0,
                max: 10.0,
                step: 1.0,
                unit: "units".into(),
                learning_direction: 1.0,
            })
            .collect::<Vec<_>>();
        let baseline = ["a", "b", "c", "d"]
            .into_iter()
            .map(|key| (key.to_owned(), 5.0))
            .collect::<BTreeMap<_, _>>();
        let snapshot = serde_json::json!({
            "scout_parameter_variants": [
                {"symbol":"BTC-USDT", "strategy":"live-momentum", "parameter":"a", "candidate_value":6.0, "average_net_improvement_percent":6.0, "rejected":false},
                {"symbol":"BTC-USDT", "strategy":"live-momentum", "parameter":"b", "candidate_value":6.0, "average_net_improvement_percent":3.0, "rejected":false},
                {"symbol":"BTC-USDT", "strategy":"live-momentum", "parameter":"c", "candidate_value":6.0, "average_net_improvement_percent":1.0, "rejected":false}
            ]
        });
        let selected = ranked_parameter_variants(
            &snapshot,
            "BTC-USDT",
            "live-momentum",
            &baseline,
            &specs,
            0,
        );
        assert_eq!(selected.len(), MAX_PARAMETER_VARIANTS_PER_BATCH);
        let directions = selected
            .iter()
            .map(|variant| {
                let value = variant.changes.values().next().unwrap();
                if value < &5.0 { -1_i8 } else { 1_i8 }
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(directions, BTreeSet::from([-1, 1]));
        assert!(selected.iter().any(|variant| variant.key == "a"));
    }

    #[test]
    fn same_signal_variants_form_one_bounded_batch() {
        let baseline = serde_json::json!({
            "strategy_parameters": {"threshold": 0.5},
            "bar_close_timestamp": 100,
        });
        let signals = (0..5)
            .map(|index| serde_json::json!({
                "strategy": "test-strategy",
                "symbol": "BTC-USDT",
                "direction": "bullish",
                "timestamp": 100,
                "strategy_decision": {
                    "scout_parameter_probe": true,
                    "scout_parameter_key": format!("parameter_{index}"),
                    "scout_parameter_value": index as f64,
                    "scout_parameter_baseline_decision": baseline,
                }
            }))
            .collect();
        let batches = group_daily_parameter_variants(signals);
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].2.len(), MAX_PARAMETER_VARIANTS_PER_BATCH);
        assert_eq!(batches[0].3, probe_id(&format!("probe:{}", daily_parameter_batch_key(&batches[0].1))));
    }
}

#[derive(Clone, Copy, Default)]
struct LiveThesisSignalObservation {
    timestamp_ms: u64,
    bid: f64,
    ask: f64,
    taker_fee_rate: f64,
    bullish_strength: Option<f64>,
    bearish_strength: Option<f64>,
}

fn record_live_thesis_failure_observations(
    instances: &mut BTreeMap<String, serde_json::Value>,
    signals: &[serde_json::Value],
) -> Vec<String> {
    let mut observations = BTreeMap::<(String, String), LiveThesisSignalObservation>::new();
    for signal in signals {
        let (Some(strategy), Some(symbol), Some(direction), Some(timestamp_ms), Some(bid), Some(ask)) = (
            signal.get("strategy").and_then(serde_json::Value::as_str),
            signal.get("symbol").and_then(serde_json::Value::as_str),
            signal.get("direction").and_then(serde_json::Value::as_str),
            signal.get("book_updated_at_ms").and_then(serde_json::Value::as_u64),
            signal.get("best_bid").and_then(serde_json::Value::as_f64),
            signal.get("best_ask").and_then(serde_json::Value::as_f64),
        ) else {
            continue;
        };
        let Some(strength) = signal
            .get("strength")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
        else {
            continue;
        };
        if !matches!(direction, "bullish" | "bearish")
            || !bid.is_finite()
            || !ask.is_finite()
            || bid <= 0.0
            || ask < bid
        {
            continue;
        }
        let fee_rate = signal
            .get("scout_taker_fee_rate")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && (0.0..=0.02).contains(value))
            .unwrap_or(0.001);
        let observation = observations
            .entry((strategy.to_owned(), symbol.to_owned()))
            .or_default();
        if timestamp_ms > observation.timestamp_ms {
            *observation = LiveThesisSignalObservation {
                timestamp_ms,
                bid,
                ask,
                taker_fee_rate: fee_rate,
                bullish_strength: None,
                bearish_strength: None,
            };
        }
        if timestamp_ms == observation.timestamp_ms {
            let same_direction_strength = if direction == "bullish" {
                &mut observation.bullish_strength
            } else {
                &mut observation.bearish_strength
            };
            *same_direction_strength = Some(
                same_direction_strength.map_or(strength, |previous| previous.max(strength)),
            );
        }
    }

    let mut changed_ids = Vec::new();
    for (decision_id, instance) in instances {
        if instance["scout_exit_scenario"].as_str().unwrap_or("strategy") != "strategy"
            || instance["scout_net_return_percent"].is_number()
            || instance["scout_thesis_failure_exit_net_return_percent"].is_number()
            || instance["scout_thesis_failure_stop_timestamp_ms"].is_number()
        {
            continue;
        }
        let (Some(strategy), Some(symbol), Some(direction)) = (
            instance["strategy"].as_str(),
            instance["symbol"].as_str(),
            instance["direction"].as_str(),
        ) else {
            continue;
        };
        let Some(observation) = observations.get(&(strategy.to_owned(), symbol.to_owned())) else {
            continue;
        };
        let last_timestamp = instance["scout_thesis_failure_last_signal_timestamp_ms"]
            .as_u64()
            .unwrap_or_default();
        if observation.timestamp_ms <= last_timestamp {
            continue;
        }
        let parameters = instance["strategy_parameters"]
            .as_object()
            .cloned()
            .unwrap_or_default();
        let Some(loss_activation) = parameters
            .get("thesis_failure_loss_activation_percent")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value > 0.0)
        else {
            continue;
        };
        let Some(confirmation_samples) = parameters
            .get("thesis_failure_confirmation_samples")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
            .map(|value| value.round().clamp(1.0, 4.0) as u32)
        else {
            continue;
        };
        let Some(minimum_strength) = parameters
            .get("thesis_failure_minimum_signal_strength")
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite())
        else {
            continue;
        };
        let stop_loss_percent = instance["stop_loss_percent"]
            .as_f64()
            .or_else(|| {
                parameters
                    .get("stop_loss_percent")
                    .and_then(serde_json::Value::as_f64)
            })
            .filter(|value| value.is_finite() && *value > 0.0);
        let (opposing_strength, exit_price, direction_sign) = match direction {
            "bullish" => (observation.bearish_strength, observation.bid, 1.0),
            "bearish" => (observation.bullish_strength, observation.ask, -1.0),
            _ => continue,
        };
        let entry_price = match direction {
            "bullish" => instance["scout_entry_ask"]
                .as_f64()
                .or_else(|| instance["scout_entry_price"].as_f64()),
            "bearish" => instance["scout_entry_bid"]
                .as_f64()
                .or_else(|| instance["scout_entry_price"].as_f64()),
            _ => None,
        };
        let Some(entry_price) = entry_price.filter(|value| value.is_finite() && *value > 0.0) else {
            continue;
        };
        let net_return_percent = ((exit_price / entry_price - 1.0) * direction_sign
            - 2.0 * observation.taker_fee_rate)
            * 100.0;
        if stop_loss_percent.is_some_and(|stop| net_return_percent <= -stop) {
            instance["scout_thesis_failure_confirmation_count"] = serde_json::json!(0);
            instance["scout_thesis_failure_last_signal_timestamp_ms"] =
                serde_json::json!(observation.timestamp_ms);
            instance["scout_thesis_failure_stop_timestamp_ms"] =
                serde_json::json!(observation.timestamp_ms);
            instance["scout_taker_fee_rate"] = serde_json::json!(observation.taker_fee_rate);
            changed_ids.push(decision_id.clone());
            continue;
        }
        let invalidation_confirmed = opposing_strength
            .is_some_and(|strength| strength >= minimum_strength)
            && net_return_percent <= -loss_activation
            && stop_loss_percent.is_none_or(|stop| net_return_percent > -stop);
        let current_count = instance["scout_thesis_failure_confirmation_count"]
            .as_u64()
            .unwrap_or_default() as u32;
        let next_count = if invalidation_confirmed {
            current_count.saturating_add(1).min(confirmation_samples)
        } else {
            0
        };
        instance["scout_thesis_failure_confirmation_count"] = serde_json::json!(next_count);
        instance["scout_thesis_failure_last_signal_timestamp_ms"] =
            serde_json::json!(observation.timestamp_ms);
        instance["scout_taker_fee_rate"] = serde_json::json!(observation.taker_fee_rate);
        if next_count >= confirmation_samples {
            instance["scout_thesis_failure_exit_net_return_percent"] =
                serde_json::json!(net_return_percent);
            instance["scout_thesis_failure_exit_timestamp_ms"] =
                serde_json::json!(observation.timestamp_ms);
        }
        changed_ids.push(decision_id.clone());
    }
    changed_ids
}

fn load_scout_instances_for_signals(
    path: &std::path::Path,
    signals: &[serde_json::Value],
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    let targets = signals
        .iter()
        .filter_map(|signal| {
            Some((
                signal.get("strategy")?.as_str()?.to_owned(),
                signal.get("symbol")?.as_str()?.to_owned(),
            ))
        })
        .collect::<BTreeSet<_>>();
    let connection = rusqlite::Connection::open(path)?;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='scout_instances' ORDER BY record_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut instances = BTreeMap::new();
    for row in rows {
        let (decision_id, payload) = row?;
        let instance: serde_json::Value = serde_json::from_str(&payload)?;
        let target = instance["strategy"]
            .as_str()
            .zip(instance["symbol"].as_str())
            .map(|(strategy, symbol)| (strategy.to_owned(), symbol.to_owned()));
        if target.is_some_and(|target| targets.contains(&target)) {
            instances.insert(decision_id, instance);
        }
    }
    Ok(instances)
}

fn persist_scout_instance_records(
    path: &std::path::Path,
    instances: &BTreeMap<String, serde_json::Value>,
    ids: &[String],
) -> anyhow::Result<()> {
    if ids.is_empty() {
        return Ok(());
    }
    let mut connection = rusqlite::Connection::open(path)?;
    let tx = connection.transaction()?;
    for id in ids {
        if let Some(instance) = instances.get(id) {
            tx.execute(
                "UPDATE scout_records SET payload=?1 WHERE collection='scout_instances' AND record_key=?2",
                rusqlite::params![serde_json::to_string(instance)?, id],
            )?;
        }
    }
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod live_thesis_failure_replay_tests {
    use super::*;

    fn instance(decision_id: &str, loss_activation: f64) -> serde_json::Value {
        serde_json::json!({
            "decision_id": decision_id,
            "strategy": "live-momentum",
            "symbol": "BTC-USDT",
            "direction": "bullish",
            "scout_exit_scenario": "strategy",
            "scout_entry_price": 100.0,
            "scout_entry_bid": 100.0,
            "scout_entry_ask": 100.1,
            "scout_taker_fee_rate": 0.001,
            "stop_loss_percent": 0.5,
            "strategy_parameters": {
                "thesis_failure_loss_activation_percent": loss_activation,
                "thesis_failure_confirmation_samples": 2.0,
                "thesis_failure_minimum_signal_strength": 0.6
            }
        })
    }

    fn opposing_signal(timestamp_ms: u64, bid: f64) -> serde_json::Value {
        serde_json::json!({
            "strategy": "live-momentum",
            "symbol": "BTC-USDT",
            "direction": "bearish",
            "strength": 0.8,
            "timestamp": timestamp_ms,
            "book_updated_at_ms": timestamp_ms,
            "best_bid": bid,
            "best_ask": bid + 0.1,
            "scout_taker_fee_rate": 0.001
        })
    }

    #[test]
    fn records_candidate_specific_early_exit_returns_from_the_same_signal_path() {
        let mut state = PersistedState::default();
        state
            .scout_instances
            .insert("baseline".into(), instance("baseline", 0.2));
        state
            .scout_instances
            .insert("higher_threshold".into(), instance("higher_threshold", 0.7));
        let mut hard_stop = instance("hard_stop", 0.2);
        hard_stop["stop_loss_percent"] = serde_json::json!(0.4);
        state.scout_instances.insert("hard_stop".into(), hard_stop);

        let first = vec![opposing_signal(1_000, 99.85)];
        assert_eq!(record_live_thesis_failure_observations(&mut state.scout_instances, &first).len(), 3);
        assert_eq!(state.scout_instances["baseline"]["scout_thesis_failure_confirmation_count"], 1);
        assert_eq!(state.scout_instances["higher_threshold"]["scout_thesis_failure_confirmation_count"], 0);

        let second = vec![opposing_signal(2_000, 99.85)];
        assert_eq!(record_live_thesis_failure_observations(&mut state.scout_instances, &second).len(), 2);
        assert!(state.scout_instances["baseline"]["scout_thesis_failure_exit_net_return_percent"].is_number());
        assert!(state.scout_instances["higher_threshold"]["scout_thesis_failure_exit_net_return_percent"].is_null());
        assert!(state.scout_instances["hard_stop"]["scout_thesis_failure_exit_net_return_percent"].is_null());
        assert_eq!(state.scout_instances["hard_stop"]["scout_thesis_failure_stop_timestamp_ms"], 1_000);
        assert_eq!(
            state.scout_instances["baseline"]["scout_thesis_failure_exit_timestamp_ms"],
            2_000
        );
    }
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
    let snapshot = strategies::request_scout_profile_snapshot(&service.learning_rsocket_url)
        .await
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
    let live_strategies = configuration.strategies.iter().filter(|strategy| {
        matches!(strategy.as_str(), "live-momentum" | "live-trade-flow" | "live-order-book" | "live-vwap-dislocation" | "live-absorption-pressure")
    }).map(String::as_str).collect::<Vec<_>>();
    let has_missing_profiles = symbols.iter().any(|symbol| live_strategies.iter().any(|strategy| {
        !profiles.iter().any(|profile| profile.symbol == *symbol && profile.strategy == *strategy)
    }));
    if has_missing_profiles {
        let response = strategies::request_scout_service_route(
            &service.auto_trader_rsocket_url,
            &strategies::ScoutCoordinatorRouteCommand {
                request_id: format!("scout-parameters:{now_ms}"),
                strategy: None,
                method: strategies::ScoutWorkerHttpMethod::Get,
                path: "/positions/auto-trader/parameters".into(),
                payload: serde_json::Value::Null,
            },
        )
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
        if response.status >= 400 {
            return Err((StatusCode::BAD_GATEWAY, "AutoTrader rejected Scout profile lookup".into()));
        }
        let fallback_profiles = auto_trader_live_profiles(&response.body, &symbols, &configuration.strategies);
        for profile in fallback_profiles {
            if !profiles.iter().any(|existing| existing.symbol == profile.symbol && existing.strategy == profile.strategy) {
                profiles.push(profile);
            }
        }
    }
    if profiles.is_empty() { return Ok(StatusCode::NO_CONTENT); }
    let day = now_ms / 86_400_000;
    let mut expanded_profiles = Vec::new();
    let mut arm_metadata = BTreeMap::<String, (LiveScoutProfile, Option<ParameterVariant>)>::new();
    let mut selected_variants_by_profile = BTreeMap::<String, Vec<ParameterVariant>>::new();
    for baseline in profiles {
        let Some(strategy_kind) = strategies::Strategy::from_name(&baseline.strategy) else { continue; };
        let specs = strategies::scout_parameter_specs(strategy_kind);
        let variants = ranked_parameter_variants(
            &snapshot,
            &baseline.symbol,
            &baseline.strategy,
            &baseline.parameters,
            &specs,
            day,
        );
        let baseline_key = live_profile_arm_key(&baseline);
        selected_variants_by_profile.insert(baseline_key.clone(), variants.clone());
        arm_metadata.insert(baseline_key, (baseline.clone(), None));
        expanded_profiles.push(baseline.clone());
        for variant in variants {
            let mut arm = baseline.clone();
            arm.parameters.extend(variant.changes.iter().map(|(key, value)| (key.clone(), *value)));
            arm_metadata.insert(live_profile_arm_key(&arm), (baseline.clone(), Some(variant)));
            expanded_profiles.push(arm);
        }
    }
    let mut generated = Vec::new();
    for profile_batch in expanded_profiles.chunks(100) {
        generated.extend(generate_live_candidates(Json(LiveScoutCandidatesRequest {
            signals: signals.clone(),
            profiles: profile_batch.to_vec(),
        })).await?.0);
    }
    let thesis_signal_candidates = generated
        .iter()
        .filter_map(|signal| {
            let strategy = signal.get("strategy")?.as_str()?;
            let symbol = signal.get("symbol")?.as_str()?;
            let parameters = signal
                .get("strategy_decision")?
                .get("strategy_parameters")?
                .clone();
            let parameters = serde_json::from_value::<BTreeMap<String, f64>>(parameters).ok()?;
            let arm_key = live_profile_arm_key(&LiveScoutProfile {
                strategy: strategy.to_owned(),
                symbol: symbol.to_owned(),
                parameters,
            });
            arm_metadata
                .get(&arm_key)
                .is_some_and(|(_, variant)| variant.is_none())
                .then(|| signal.clone())
        })
        .collect::<Vec<_>>();
    {
        let mut state = service.scout.data.lock().await;
        let mut candidate_instances =
            load_scout_instances_for_signals(&service.scout.path, &thesis_signal_candidates)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        let changed_instances = record_live_thesis_failure_observations(
            &mut candidate_instances,
            &thesis_signal_candidates,
        );
        persist_scout_instance_records(
            &service.scout.path,
            &candidate_instances,
            &changed_instances,
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        for decision_id in changed_instances {
            if let Some(instance) = candidate_instances.get(&decision_id) {
                cache_scout_instance(&mut state, &decision_id, instance.clone());
            }
        }
    }
    let mut batches = BTreeMap::<String, LiveScoutProbeBatch>::new();
    let mut unpaired = Vec::new();
    for signal in generated {
        let (Some(strategy), Some(symbol), Some(direction)) = (
            signal.get("strategy").and_then(serde_json::Value::as_str).map(str::to_owned),
            signal.get("symbol").and_then(serde_json::Value::as_str).map(str::to_owned),
            signal.get("direction").and_then(serde_json::Value::as_str).map(str::to_owned),
        ) else { continue };
        let parameters = signal.get("strategy_decision").and_then(|decision| decision.get("strategy_parameters"))
            .cloned().and_then(|value| serde_json::from_value::<BTreeMap<String, f64>>(value).ok()).unwrap_or_default();
        let arm_key = live_profile_arm_key(&LiveScoutProfile {
            symbol: symbol.clone(),
            strategy: strategy.clone(),
            parameters: parameters.clone(),
        });
        let Some((baseline, variant)) = arm_metadata.get(&arm_key).cloned() else { continue; };
        let signal_timestamp_ms = signal.get("book_updated_at_ms").and_then(serde_json::Value::as_u64)
            .or_else(|| signal.get("timestamp").and_then(serde_json::Value::as_u64))
            .unwrap_or(now_ms);
        let batch_key = format!(
            "{}|{}|{}|{}|{}",
            strategy,
            symbol,
            direction,
            signal_timestamp_ms,
            serde_json::to_string(&baseline.parameters).unwrap_or_default()
        );
        if variant.is_none() && ranked_parameter_variants(
            &snapshot,
            &baseline.symbol,
            &baseline.strategy,
            &baseline.parameters,
            &strategies::scout_parameter_specs(strategies::Strategy::from_name(&baseline.strategy).unwrap()),
            day,
        ).is_empty() {
            unpaired.push((signal, baseline));
            continue;
        }
        let baseline_profile_key = live_profile_arm_key(&baseline);
        let batch = batches.entry(batch_key).or_insert_with(|| LiveScoutProbeBatch {
            strategy: strategy.clone(),
            symbol: symbol.clone(),
            direction: direction.clone(),
            signal_timestamp_ms,
            baseline_profile_key,
            baseline_parameters: baseline.parameters.clone(),
            baseline_signal: None,
            variants: BTreeMap::new(),
        });
        if let Some(variant) = variant {
            batch.variants.insert((variant.key.clone(), format!("{:.8}", variant.value)), (signal, variant));
        } else {
            batch.baseline_signal = Some(signal);
        }
    }
    let mut candidates = Vec::new();
    let mut ordered_batches = batches.into_iter().collect::<Vec<_>>();
    ordered_batches.sort_by(|(_, left), (_, right)| {
        right.signal_timestamp_ms.cmp(&left.signal_timestamp_ms)
    });
    let mut selected_live_profiles = BTreeMap::<String, usize>::new();
    for (batch_key, mut batch) in ordered_batches {
        let symbol_direction_key = format!("{}|{}|{}", batch.strategy, batch.symbol, batch.direction);
        let selected_count = selected_live_profiles.entry(symbol_direction_key).or_default();
        if *selected_count >= MAX_LIVE_PROFILES_PER_SYMBOL_DIRECTION {
            continue;
        }
        *selected_count += 1;
        let selected_variants = selected_variants_by_profile
            .get(&batch.baseline_profile_key)
            .cloned()
            .unwrap_or_default();
        if selected_variants.is_empty() {
            if let Some(baseline_signal) = batch.baseline_signal.take() {
                unpaired.push((baseline_signal, LiveScoutProfile {
                    symbol: batch.symbol,
                    strategy: batch.strategy,
                    parameters: batch.baseline_parameters,
                }));
            }
            continue;
        }
        let Some(mut baseline_signal) = batch.baseline_signal.take()
            .or_else(|| batch.variants.values().next().map(|(signal, _)| signal.clone()))
        else { continue; };
        let baseline_eligible = live_strategy_enters(
            &batch.strategy,
            baseline_signal.clone(),
            &batch.direction,
            batch.baseline_parameters.clone(),
        );
        let mut evaluated_variants = Vec::new();
        for variant in selected_variants {
            let mut parameters = batch.baseline_parameters.clone();
            parameters.extend(variant.changes.iter().map(|(key, value)| (key.clone(), *value)));
            let eligible = live_strategy_enters(
                &batch.strategy,
                baseline_signal.clone(),
                &batch.direction,
                parameters.clone(),
            );
            let mut variant_signal = baseline_signal.clone();
            if let Some(decision) = variant_signal.get_mut("strategy_decision") {
                decision["strategy_parameters"] = serde_json::to_value(&parameters).unwrap_or_default();
                decision["scout_parameter_key"] = serde_json::json!(variant.key);
                decision["scout_parameter_value"] = serde_json::json!(variant.value);
                decision["scout_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
            }
            evaluated_variants.push((variant_signal, variant, parameters, eligible));
        }
        if !paired_probe_has_entering_side(
            baseline_eligible,
            evaluated_variants.iter().any(|(_, _, _, eligible)| *eligible),
        ) {
            continue;
        }
        let strategy_kind = strategies::Strategy::from_name(&batch.strategy).unwrap();
        let specs = strategies::scout_parameter_specs(strategy_kind);
        let batch_id = probe_id(&batch_key);
        let comparison_ids = evaluated_variants.iter().map(|(_, variant, _, _)| {
            format!("scout-pair:{}", probe_id(&format!("{batch_id}|{}|{:.8}", variant.key, variant.value)))
        }).collect::<Vec<_>>();
        let sample_group_id = format!("live-parameter-batch:{batch_id}");
        let cooldown_key = format!("live-parameter|{}|{}|{}", batch.strategy, batch.symbol, batch.direction);
        let learning_context = format!("scout:live:{}:{}:{}:scenario", batch.strategy, batch.symbol, batch.direction);
        let base_stop = batch.baseline_parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent);
        let base_target = batch.baseline_parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent);
        let maximum_scout_duration_ms = service
            .live_scout_maximum_duration_ms
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(&batch.strategy)
            .copied()
            .unwrap_or(LIVE_SCOUT_DEFAULT_HOLD_MS.saturating_mul(2));
        let entry_price = baseline_signal.get("price").cloned()
            .or_else(|| baseline_signal.get("observed_close").cloned())
            .or_else(|| baseline_signal.get("strategy_decision").and_then(|decision| decision.get("observed_close")).cloned())
            .unwrap_or(serde_json::Value::Null);
        if let Some(decision) = baseline_signal.get_mut("strategy_decision") {
            decision["strategy_parameters"] = serde_json::to_value(&batch.baseline_parameters).unwrap_or_default();
        }
        baseline_signal["decision_id"] = serde_json::json!(format!("{sample_group_id}|baseline"));
        baseline_signal["trade_id"] = baseline_signal["decision_id"].clone();
        baseline_signal["timestamp"] = serde_json::json!(now_ms);
        baseline_signal["signal_timestamp"] = serde_json::json!(batch.signal_timestamp_ms);
        baseline_signal["stage"] = serde_json::json!("decision");
        baseline_signal["status"] = serde_json::json!("scouted");
        baseline_signal["scout"] = serde_json::json!(true);
        baseline_signal["scout_notional_usd"] = serde_json::json!(configuration.max_sample_notional);
        baseline_signal["scout_entry_price"] = entry_price.clone();
        baseline_signal["scout_entry_bid"] = baseline_signal
            .get("best_bid")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        baseline_signal["scout_entry_ask"] = baseline_signal
            .get("best_ask")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        baseline_signal["scout_learning_context"] = serde_json::json!(format!("scout:live:{}:{}:{}:trade", batch.strategy, batch.symbol, batch.direction));
        baseline_signal["scout_sample_group_id"] = serde_json::json!(sample_group_id);
        baseline_signal["scout_maximum_duration_ms"] = serde_json::json!(maximum_scout_duration_ms);
        baseline_signal["scout_cooldown_key"] = serde_json::json!(cooldown_key);
        baseline_signal["strategy_parameters"] = serde_json::to_value(&batch.baseline_parameters).unwrap_or_default();
        baseline_signal["strategy_parameter_specs"] = serde_json::to_value(&specs).unwrap_or_default();
        baseline_signal["stop_loss_percent"] = serde_json::json!(base_stop);
        baseline_signal["take_profit_percent"] = serde_json::json!(base_target);
        baseline_signal["scout_comparison_role"] = serde_json::json!("baseline");
        baseline_signal["scout_comparison_ids"] = serde_json::json!(comparison_ids);
        baseline_signal["scout_comparison_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
        let mut templates = Vec::with_capacity(evaluated_variants.len() + usize::from(baseline_eligible));
        if baseline_eligible {
            baseline_signal["scout_comparison_role"] = serde_json::json!("baseline");
            baseline_signal["scout_comparison_ids"] = serde_json::json!(comparison_ids);
            baseline_signal["scout_comparison_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
            let zero_sides = evaluated_variants.iter().filter_map(|(_, variant, _, eligible)| {
                (!eligible).then(|| serde_json::json!({
                    "comparison_id": format!("scout-pair:{}", probe_id(&format!("{batch_id}|{}|{:.8}", variant.key, variant.value))),
                    "parameter_key": variant.key,
                    "parameter_value": variant.value,
                    "profile_symbol": batch.symbol,
                }))
            }).collect::<Vec<_>>();
            baseline_signal["scout_comparison_zero_sides"] = serde_json::json!(zero_sides);
            baseline_signal.as_object_mut().map(|audit| {
                audit.remove("scout_parameter_key");
                audit.remove("scout_parameter_value");
                audit.remove("scout_parameter_profile_symbol");
            });
            templates.push(ScoutAuditTemplate {
                base_audit: baseline_signal.clone(),
                notional_usd: configuration.max_sample_notional,
                learning_context: learning_context.clone(),
                stop_loss_percent: base_stop,
                take_profit_percent: base_target,
                level_percent: 0.0,
                strategy_only: true,
                parameter_entry_eligible: Some(true),
                parameter_specs: specs.clone(),
            });
        }
        for (mut variant_signal, variant, variant_parameters, eligible) in evaluated_variants {
            if !eligible { continue; }
            let comparison_id = format!("scout-pair:{}", probe_id(&format!("{batch_id}|{}|{:.8}", variant.key, variant.value)));
            let stop = variant_parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent);
            let target = variant_parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent);
            variant_signal["decision_id"] = serde_json::json!(format!("{sample_group_id}|variant:{}:{:.8}", variant.key, variant.value));
            variant_signal["trade_id"] = variant_signal["decision_id"].clone();
            variant_signal["timestamp"] = serde_json::json!(now_ms);
            variant_signal["signal_timestamp"] = serde_json::json!(batch.signal_timestamp_ms);
            variant_signal["stage"] = serde_json::json!("decision");
            variant_signal["status"] = serde_json::json!("scouted");
            variant_signal["scout"] = serde_json::json!(true);
            variant_signal["scout_notional_usd"] = serde_json::json!(configuration.max_sample_notional);
            variant_signal["scout_entry_price"] = entry_price.clone();
            variant_signal["scout_entry_bid"] = variant_signal
                .get("best_bid")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            variant_signal["scout_entry_ask"] = variant_signal
                .get("best_ask")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            variant_signal["scout_learning_context"] = serde_json::json!(format!("scout:live:{}:{}:{}:trade", batch.strategy, batch.symbol, batch.direction));
            variant_signal["scout_sample_group_id"] = serde_json::json!(sample_group_id);
            variant_signal["scout_maximum_duration_ms"] = serde_json::json!(maximum_scout_duration_ms);
            variant_signal["scout_cooldown_key"] = serde_json::json!(cooldown_key);
            variant_signal["strategy_parameters"] = serde_json::to_value(&variant_parameters).unwrap_or_default();
            variant_signal["strategy_parameter_specs"] = serde_json::to_value(&specs).unwrap_or_default();
            variant_signal["stop_loss_percent"] = serde_json::json!(stop);
            variant_signal["take_profit_percent"] = serde_json::json!(target);
            variant_signal["scout_parameter_key"] = serde_json::json!(variant.key);
            variant_signal["scout_parameter_value"] = serde_json::json!(variant.value);
            variant_signal["scout_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
            if let Some(decision) = variant_signal.get_mut("strategy_decision") {
                decision["scout_parameter_probe"] = serde_json::json!(true);
                decision["scout_parameter_key"] = serde_json::json!(variant.key);
                decision["scout_parameter_value"] = serde_json::json!(variant.value);
                decision["scout_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
            }
            variant_signal["scout_comparison_role"] = serde_json::json!("variant");
            variant_signal["scout_comparison_id"] = serde_json::json!(comparison_id);
            variant_signal["scout_comparison_parameter_key"] = serde_json::json!(variant.key);
            variant_signal["scout_comparison_parameter_value"] = serde_json::json!(variant.value);
            variant_signal["scout_comparison_parameter_profile_symbol"] = serde_json::json!(batch.symbol);
            if !baseline_eligible {
                variant_signal["scout_comparison_zero_role"] = serde_json::json!("baseline");
            }
            templates.push(ScoutAuditTemplate {
                base_audit: variant_signal,
                notional_usd: configuration.max_sample_notional,
                learning_context: learning_context.clone(),
                stop_loss_percent: stop,
                take_profit_percent: target,
                level_percent: 0.0,
                strategy_only: true,
                parameter_entry_eligible: Some(true),
                parameter_specs: specs.clone(),
            });
        }
        candidates.push(ScoutPlanCandidate {
            generate_group_id: false,
            sample_group_id,
            cooldown_key,
            profile_key: format!("{}|{}", batch.strategy, batch.symbol),
            eligible: true,
            parameter_variant: true,
            cooldown_ms: 300_000,
            audit_plans: Vec::new(),
            audit_templates: templates,
            sample_payload: None,
            daily_policy: None,
            live_policy: None,
        });
    }
    for (index, (mut signal, profile)) in unpaired.into_iter().enumerate() {
        let strategy = profile.strategy;
        let symbol = profile.symbol;
        let direction = signal.get("direction").and_then(serde_json::Value::as_str).unwrap_or("bullish").to_owned();
        let parameters = profile.parameters;
        let maximum_scout_duration_ms = service
            .live_scout_maximum_duration_ms
            .read()
            .unwrap_or_else(|error| error.into_inner())
            .get(&strategy)
            .copied()
            .unwrap_or(LIVE_SCOUT_DEFAULT_HOLD_MS.saturating_mul(2));
        let stop = parameters.get("stop_loss_percent").copied().unwrap_or(configuration.stop_loss_percent);
        let target = parameters.get("take_profit_percent").copied().unwrap_or(configuration.take_profit_percent);
        let decision_id = format!("live-scout:{}:{}:{}:{}", strategy, symbol, direction, now_ms.saturating_add(index as u64));
        signal["decision_id"] = serde_json::json!(decision_id);
        signal["trade_id"] = signal["decision_id"].clone();
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
        signal["scout_entry_bid"] = signal
            .get("best_bid")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        signal["scout_entry_ask"] = signal
            .get("best_ask")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        let group = format!("live-scout:{strategy}|{symbol}|{direction}");
        signal["scout_sample_group_id"] = serde_json::json!(group);
        signal["scout_maximum_duration_ms"] = serde_json::json!(maximum_scout_duration_ms);
        signal["scout_cooldown_key"] = serde_json::json!(group);
        signal["scout_exit_scenario"] = serde_json::json!("strategy");
        signal["strategy_parameters"] = serde_json::to_value(&parameters).unwrap_or_default();
        let strategy_kind = strategies::Strategy::from_name(&strategy).unwrap();
        signal["strategy_parameter_specs"] = serde_json::to_value(strategies::scout_parameter_specs(strategy_kind)).unwrap_or_default();
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
        max_active_variants_per_profile: 6,
        legacy_cooldowns: BTreeMap::new(),
        legacy_comparisons: Vec::new(),
        candidates,
    };
    let _ = create_scout_plan(State(service), Json(plan)).await?;
    Ok(StatusCode::ACCEPTED)
}

fn auto_trader_live_profiles(
    response: &serde_json::Value,
    symbols: &BTreeSet<&str>,
    enabled_strategies: &[String],
) -> Vec<LiveScoutProfile> {
    response.get("strategy_profiles")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|profile| {
            let symbol = profile.get("symbol")?.as_str()?;
            let strategy = profile.get("strategy")?.as_str()?;
            if !symbols.contains(symbol)
                || !enabled_strategies.iter().any(|enabled| enabled == strategy)
                || !matches!(strategy, "live-momentum" | "live-trade-flow" | "live-order-book" | "live-vwap-dislocation" | "live-absorption-pressure")
            {
                return None;
            }
            let parameters = profile.get("parameters")?.as_array()?.iter().filter_map(|parameter| {
                Some((
                    parameter.get("key")?.as_str()?.to_owned(),
                    parameter.get("value")?.as_f64()?,
                ))
            }).collect::<BTreeMap<_, _>>();
            (!parameters.is_empty() && parameters.values().all(|value| value.is_finite())).then(|| LiveScoutProfile {
                symbol: symbol.to_owned(),
                strategy: strategy.to_owned(),
                parameters,
            })
        })
        .collect()
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
        let (_, changed_records) =
            apply_scout_lifecycle_observation_with_changes(&mut state, &observation);
        if !changed_records.is_empty() {
            let mut changed_sections = vec!["outbox".to_owned()];
            changed_sections.extend(changed_records);
            let changed_sections = changed_sections.iter().map(String::as_str).collect::<Vec<_>>();
            persist_scout_state(&service.scout.path, &state, &changed_sections)?;
        }
    }
    schedule_scout_outbox_flush(&service);

    let snapshot = strategies::request_scout_profile_snapshot(&service.learning_rsocket_url)
        .await
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
        let variants = ranked_parameter_variants(
            &snapshot,
            &profile_symbol,
            "dated-futures-carry",
            &baseline,
            &specs,
            day,
        );
        if variants.is_empty() { continue; }
        let baseline_minimum_days = baseline.get("carry_minimum_days_to_expiry").copied().unwrap_or(f64::NAN);
        let baseline_cost_gate = baseline.get("carry_cost_gate_percent").copied().unwrap_or(f64::NAN);
        let baseline_would_enter = strategies::dated_carry_entry_is_eligible(
            candidate,
            request.now_ms,
            request.now_ms,
            baseline_minimum_days,
            baseline_cost_gate,
        );
        let mut evaluated_variants = Vec::new();
        for variant in variants {
            let mut parameters = baseline.clone();
            parameters.extend(variant.changes.iter().map(|(key, value)| (key.clone(), *value)));
            let minimum_days = parameters.get("carry_minimum_days_to_expiry").copied().unwrap_or(f64::NAN);
            let cost_gate = parameters.get("carry_cost_gate_percent").copied().unwrap_or(f64::NAN);
            let would_enter = strategies::dated_carry_entry_is_eligible(candidate, request.now_ms, request.now_ms, minimum_days, cost_gate);
            if baseline_would_enter || would_enter {
                evaluated_variants.push((variant, parameters, would_enter));
            }
        }
        if evaluated_variants.is_empty() { continue; }
        let batch_id = probe_id(&format!(
            "dated-carry|{}|{}|{day}|{}",
            profile_symbol,
            candidate.future_symbol,
            serde_json::to_string(&baseline).unwrap_or_default(),
        ));
        let baseline_sample_id = format!("dated-carry-batch:{batch_id}|baseline");
        let comparison_ids = evaluated_variants.iter().map(|(variant, _, _)| {
            format!("scout-pair:{}", probe_id(&format!("{batch_id}|{}|{:.8}", variant.key, variant.value)))
        }).collect::<Vec<_>>();
        plans.push(ScoutPlanCandidate {
            generate_group_id: false,
            sample_group_id: baseline_sample_id.clone(),
            cooldown_key: format!("dated-carry-base|{profile_symbol}|{}|{day}", candidate.future_symbol),
            profile_key: format!("{profile_symbol}:dated-futures-carry"),
            eligible: true,
            parameter_variant: false,
            cooldown_ms: 86_400_000,
            audit_plans: Vec::new(),
            audit_templates: Vec::new(),
            sample_payload: Some(ScoutSamplePayload::DatedCarryProbe(DatedCarryProbe {
                sample_id: baseline_sample_id,
                parameter_key: String::new(),
                parameter_value: 0.0,
                comparison_role: Some("baseline".into()),
                comparison_id: None,
                comparison_ids,
                opened_at_ms: request.now_ms,
                candidate: candidate.clone(),
                strategy: "dated-futures-carry".into(),
                profile_symbol: profile_symbol.clone(),
                parameters: baseline.clone(),
                tested_parameters: BTreeMap::new(),
                parameter_specs: specs.clone(),
                would_enter: baseline_would_enter,
            })),
            daily_policy: None,
            live_policy: None,
        });
        for (variant, parameters, would_enter) in evaluated_variants {
            let comparison_id = format!("scout-pair:{}", probe_id(&format!("{batch_id}|{}|{:.8}", variant.key, variant.value)));
            let sample_id = format!("dated-carry-batch:{batch_id}|variant:{}:{:.8}", variant.key, variant.value);
            plans.push(ScoutPlanCandidate {
                generate_group_id: false,
                sample_group_id: sample_id.clone(),
                cooldown_key: format!("dated-carry-variant|{profile_symbol}|{}|{}|{:.8}", candidate.future_symbol, variant.key, variant.value),
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
                    comparison_role: Some("variant".into()),
                    comparison_id: Some(comparison_id),
                    comparison_ids: Vec::new(),
                    opened_at_ms: request.now_ms,
                    candidate: candidate.clone(),
                    strategy: "dated-futures-carry".into(),
                    profile_symbol: profile_symbol.clone(),
                    parameters,
                    tested_parameters: variant.changes,
                    parameter_specs: specs.clone(),
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
    if service.scout.data.lock().await.seen.contains(&event_id)
        || scout_record_exists(&service.scout.path, "seen", &event_id)? {
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
        let mut persisted = service.scout.data.lock().await;
        persisted.seen.insert(event_id.clone());
        while persisted.seen.len() > 100_000 {
            if let Some(oldest) = persisted.seen.iter().next().cloned() {
                persisted.seen.remove(&oldest);
            }
        }
        persist_scout_state(
            &service.scout.path,
            &persisted,
            &[&format!("seen@{event_id}")],
        )?;
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
    let mut zero_side_ids = BTreeSet::new();
    let has_duplicate_zero_sides = outcome
        .scout_comparison_zero_sides
        .iter()
        .any(|side| !zero_side_ids.insert(side.comparison_id.as_str()));
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
        || !(matches!(outcome.exit_scenario.as_str(), "strategy" | "tight" | "wide")
            || outcome
                .exit_scenario
                .strip_prefix("horizon-")
                .and_then(|seconds| seconds.parse::<u64>().ok())
                .is_some_and(|seconds| matches!(seconds, 60 | 300 | 900 | 3_600)))
        || (outcome.scout_comparison_role.is_none()
            != (outcome.scout_comparison_id.is_none() && outcome.scout_comparison_ids.is_empty()))
        || (!outcome.scout_comparison_ids.is_empty()
            && (outcome.scout_comparison_role.as_deref() != Some("baseline")
                || outcome.scout_comparison_ids.len() > MAX_PARAMETER_VARIANTS_PER_BATCH
                || outcome.scout_comparison_id.is_some()
                || outcome.scout_comparison_ids.iter().any(|id| id.is_empty() || id.len() > 240)))
        || (!outcome.scout_comparison_zero_sides.is_empty()
            && (outcome.scout_comparison_role.as_deref() != Some("baseline")
                || has_duplicate_zero_sides
                || outcome.scout_comparison_zero_sides.len() > MAX_PARAMETER_VARIANTS_PER_BATCH
                || outcome.scout_comparison_zero_sides.iter().any(|side| {
                    side.comparison_id.is_empty()
                        || side.comparison_id.len() > 240
                        || !outcome.scout_comparison_ids.contains(&side.comparison_id)
                        || side.parameter_key.is_empty()
                        || !side.parameter_value.is_finite()
                        || side.profile_symbol.is_empty()
                        || !outcome.strategy_parameter_specs.iter().any(|spec| {
                            spec.key == side.parameter_key
                                && side.parameter_value >= spec.min
                                && side.parameter_value <= spec.max
                        })
                })))
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
        baseline_parameters: BTreeMap::new(),
        comparison: strategies::ScoutComparisonMetadata::Independent,
        specs,
        net_return_percent: outcome.net_return_percent,
    }
}

fn paired_event(
    comparison_id: &str,
    baseline: &ScoutOutcomeRequest,
    variant: &ScoutOutcomeRequest,
) -> Option<FeedbackEvent> {
    // A comparison is meaningful only when both arms describe the same
    // opportunity. In particular, never let a reused comparison identifier
    // mix observations from different markets or directions.
    if baseline.symbol != variant.symbol
        || baseline.strategy != variant.strategy
        || baseline.learning_context != variant.learning_context
        || baseline.signal_timestamp_ms.abs_diff(variant.signal_timestamp_ms) > 10_000
    {
        return None;
    }
    let tested = parameter_keys(variant)?;
    let mut parameters = baseline.strategy_parameters.clone();
    parameters.extend(tested.clone());
    let baseline_parameters = tested
        .keys()
        .filter_map(|key| {
            baseline
                .strategy_parameters
                .get(key)
                .copied()
                .map(|value| (key.clone(), value))
        })
        .collect();
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
    let horizon_seconds = variant
        .exit_scenario
        .strip_prefix("horizon-")
        .and_then(|seconds| seconds.parse::<u64>().ok());
    let comparison_group = comparison_id
        .split_once("|scenario:")
        .map_or(comparison_id, |(group, _)| group);
    let event_id = format!("scout-comparison:{comparison_id}");
    Some(FeedbackEvent::ScoutOutcome {
        event_id,
        observed_at_ms: baseline
            .evaluated_timestamp_ms
            .max(variant.evaluated_timestamp_ms),
        learning_context: None,
        symbol: profile_symbol.to_owned(),
        strategy: variant.strategy.clone(),
        profile_id: strategies::profile_id(profile_symbol, &variant.strategy, &parameters),
        parameters,
        baseline_parameters,
        comparison: strategies::ScoutComparisonMetadata::Paired {
            group_id: comparison_group.to_owned(),
            horizon_seconds,
        },
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
    let mut event = paired_event(comparison_id, &baseline, &variant)?;
    if pair.baseline.is_none() {
        // A legacy pair can retain its return but not the original control
        // profile parameters. Don't fabricate direction from the variant.
        if let FeedbackEvent::ScoutOutcome {
            baseline_parameters,
            ..
        } = &mut event
        {
            baseline_parameters.clear();
        }
    }
    Some(event)
}

fn record_paired_scout_outcome(
    state: &mut PersistedState,
    outcome: &ScoutOutcomeRequest,
) -> (Vec<(String, f64)>, Vec<String>) {
    let Some(role) = outcome.scout_comparison_role.as_deref() else {
        return (Vec::new(), Vec::new());
    };
    let comparison_ids = outcome_comparison_ids(outcome);
    let mut paired_updates = Vec::new();
    let mut touched_ids = comparison_ids.clone();
    for comparison_id in comparison_ids {
        // A parameter comparison can produce more than one exit-scenario
        // outcome. Keep each scenario's baseline and variant together; sharing
        // one pair slot lets whichever side closes last overwrite a different
        // scenario and silently loses (or misattributes) the other result.
        let pair_key = format!("{comparison_id}|scenario:{}", outcome.exit_scenario);
        let pair = state.pairs.entry(pair_key.clone()).or_default();
        pair.updated_at_ms = outcome.evaluated_timestamp_ms;
        match role {
            "baseline" => {
                let mut baseline = outcome.clone();
                baseline.scout_comparison_id = Some(comparison_id.clone());
                baseline.scout_comparison_ids.clear();
                baseline.scout_comparison_zero_sides.clear();
                pair.baseline = Some(baseline);
            }
            "variant" => pair.variant = Some(outcome.clone()),
            _ => unreachable!(),
        }
        if let Some(zero_role) = outcome.scout_comparison_zero_role.as_deref() {
            let mut zero = outcome.clone();
            zero.scout_comparison_role = Some(zero_role.to_owned());
            zero.scout_comparison_zero_role = None;
            zero.scout_comparison_ids.clear();
            zero.scout_comparison_zero_sides.clear();
            zero.net_return_percent = 0.0;
            zero.simulated_net_pnl_usd = 0.0;
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
        for zero_side in outcome
            .scout_comparison_zero_sides
            .iter()
            .filter(|zero_side| zero_side.comparison_id == comparison_id)
        {
            let mut zero_variant = outcome.clone();
            zero_variant.scout_comparison_id = Some(comparison_id.clone());
            zero_variant.scout_comparison_ids.clear();
            zero_variant.scout_comparison_zero_sides.clear();
            zero_variant.scout_comparison_role = Some("variant".into());
            zero_variant.scout_comparison_zero_role = None;
            zero_variant.scout_parameter_key = Some(zero_side.parameter_key.clone());
            zero_variant.scout_parameter_value = Some(zero_side.parameter_value);
            zero_variant.scout_parameter_profile_symbol = Some(zero_side.profile_symbol.clone());
            zero_variant.scout_comparison_parameter_key = Some(zero_side.parameter_key.clone());
            zero_variant.scout_comparison_parameter_value = Some(zero_side.parameter_value);
            zero_variant.scout_comparison_parameter_profile_symbol = Some(zero_side.profile_symbol.clone());
            zero_variant.net_return_percent = 0.0;
            zero_variant.simulated_net_pnl_usd = 0.0;
            pair.variant = Some(zero_variant);
        }
        if let Some(event) = paired_event_with_legacy_side(&pair_key, pair, outcome) {
            if let FeedbackEvent::ScoutOutcome { net_return_percent, .. } = &event {
                if let Some(variant) = pair.variant.as_ref() {
                    paired_updates.push((variant.decision_id.clone(), *net_return_percent));
                }
            }
            if !outbox_has_learning(state, &event) {
                state.outbox.push(ScoutOutboxItem::Learning(event));
            }
            state.pairs.remove(&pair_key);
            touched_ids.push(pair_key);
        }
    }
    let expired = state
        .pairs
        .iter()
        .filter_map(|(key, pair)| {
            (outcome.evaluated_timestamp_ms.saturating_sub(pair.updated_at_ms) > 7 * 86_400_000)
                .then_some(key.clone())
        })
        .collect::<Vec<_>>();
    for key in expired {
        state.pairs.remove(&key);
        touched_ids.push(key);
    }
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
        touched_ids.push(oldest);
    }
    touched_ids.sort();
    touched_ids.dedup();
    (paired_updates, touched_ids)
}

fn scout_registration(audit: &serde_json::Value) -> Option<serde_json::Value> {
    let decision = audit.get("strategy_decision")?;
    let decision_id = audit.get("decision_id")?.as_str()?;
    let strategy = audit.get("strategy")?.as_str()?;
    let symbol = audit.get("symbol")?.as_str()?;
    let direction = audit.get("direction")?.as_str()?;
    let timestamp = audit.get("timestamp")?.as_u64()?;
    // Insights only tracks price paths for the last seven days. Historical
    // strategy samples are useful for learning but cannot be registered there.
    let now = scout_now_ms();
    if timestamp > now.saturating_add(5_000)
        || now.saturating_sub(timestamp) > 7 * 86_400_000
    {
        return None;
    }
    let entry_price = audit.get("scout_entry_price")?.as_f64()?;
    let parameters = decision.get("strategy_parameters");
    let parameter = |key: &str, default: f64| {
        parameters
            .and_then(|p| p.get(key))
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(default)
    };
    let strategy_exit_enabled =
        !strategy.starts_with("live-") && strategy != "dated-carry" && strategy != "unattributed";
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
        "scout_maximum_duration_ms": audit.get("scout_maximum_duration_ms"),
        "scout_comparison_id": audit.get("scout_comparison_id"),
        "scout_comparison_ids": audit.get("scout_comparison_ids")
            .and_then(serde_json::Value::as_array)
            .cloned()
            .unwrap_or_default(),
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
        "scout_comparison_ids",
        "scout_comparison_zero_sides",
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
    if base.get("scout_comparison_role").is_some()
        && base.get("scout_comparison_id").is_none()
        && base.get("scout_comparison_ids").is_none()
    {
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
            let comparison_suffix = role.map_or_else(String::new, |role| {
                let comparison_id = audit
                    .get("scout_comparison_id")
                    .and_then(serde_json::Value::as_str);
                comparison_id.map_or_else(
                    || format!("|comparison:{role}"),
                    |id| format!("|comparison:{role}:{}", probe_id(id)),
                )
            });
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
    changed_sections: &[&str],
) -> Result<(), (StatusCode, String)> {
    let mut changes = serde_json::Map::new();
    let mut sync_outbox = false;
    let mut sync_seen = false;
    for section in changed_sections {
        if *section == "outbox" {
            sync_outbox = true;
            continue;
        }
        if *section == "seen" || section.starts_with("seen@") {
            sync_seen = true;
        }
        if let Some((collection, key)) = section.split_once('@') {
            let value = scout_state_record(state, collection, key)?;
            changes.insert(format!("{collection}@{key}"), value);
        } else {
            changes.insert((*section).to_owned(), scout_state_section(state, section)?);
        }
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    let mut connection = rusqlite::Connection::open(path)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;\
         CREATE TABLE IF NOT EXISTS scout_records (collection TEXT NOT NULL, record_key TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(collection, record_key));\
         CREATE TABLE IF NOT EXISTS scout_configuration (id INTEGER PRIMARY KEY CHECK(id=1), strategies TEXT NOT NULL, max_active_samples INTEGER NOT NULL, max_active_positions INTEGER NOT NULL, max_sample_notional REAL NOT NULL, stop_loss_percent REAL NOT NULL, take_profit_percent REAL NOT NULL);\
         CREATE TABLE IF NOT EXISTS scout_metadata (id INTEGER PRIMARY KEY CHECK(id=1), variant_rotation_cursor INTEGER NOT NULL, lifecycle_updated_at_ms INTEGER NOT NULL);",
    ).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let tx = connection.transaction()
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let configuration = scout_state_section(state, "scout_configuration")?;
    sync_configuration(&tx, &configuration)?;
    sync_metadata(&tx, state)?;
    for (section, current) in &changes {
        if let Some((collection, key)) = section.split_once('@') {
            sync_record(&tx, collection, key, current)?;
        } else if matches!(section.as_str(), "scout_variant_rotation_cursor" | "scout_lifecycle_updated_at_ms" | "scout_configuration") {
            continue;
        } else if section == "seen" {
            sync_record_array(&tx, section, current, true)?;
        } else if section != "scout_configuration" {
            sync_record_map(&tx, section, current)?;
        }
    }
    if sync_outbox {
        sync_outbox_delta(&tx, &state.outbox)?;
    }
    if sync_seen {
        tx.execute(
            "DELETE FROM scout_records WHERE collection='seen' AND record_key IN (SELECT record_key FROM scout_records WHERE collection='seen' ORDER BY record_key LIMIT MAX(0, (SELECT COUNT(*) FROM scout_records WHERE collection='seen') - 100000))",
            [],
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    tx.commit().map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(())
}

fn sync_outbox_delta(
    tx: &rusqlite::Transaction<'_>,
    outbox: &[ScoutOutboxItem],
) -> Result<(), (StatusCode, String)> {
    let existing_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM scout_records WHERE collection='outbox'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let existing_count = existing_count.max(0) as usize;
    if outbox.len() < existing_count {
        let remove_count = existing_count - outbox.len();
        let oldest_keys = {
            let mut statement = tx
                .prepare("SELECT record_key FROM scout_records WHERE collection='outbox' ORDER BY record_key LIMIT ?1")
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            statement
                .query_map([remove_count as i64], |row| row.get::<_, String>(0))
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        };
        for key in oldest_keys {
            tx.execute(
                "DELETE FROM scout_records WHERE collection='outbox' AND record_key=?1",
                [key],
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }
    } else if outbox.len() > existing_count {
        let mut next_key: i64 = tx
            .query_row(
                "SELECT COALESCE(MAX(CAST(record_key AS INTEGER)), -1) + 1 FROM scout_records WHERE collection='outbox'",
                [],
                |row| row.get(0),
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        for item in &outbox[existing_count..] {
            let payload = serde_json::to_string(item)
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            tx.execute(
                "INSERT INTO scout_records(collection,record_key,payload) VALUES('outbox',?1,?2)",
                rusqlite::params![format!("{next_key:020}"), payload],
            )
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
            next_key += 1;
        }
    }
    Ok(())
}

fn scout_state_record(
    state: &PersistedState,
    collection: &str,
    key: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let value = match collection {
        "scout_plan_results" => state.scout_plan_results.get(key).map(serde_json::to_value).transpose(),
        "scout_instances" => state.scout_instances.get(key).map(serde_json::to_value).transpose(),
        "scout_pending_samples" => state.scout_pending_samples.get(key).map(serde_json::to_value).transpose(),
        "scout_sample_at_ms" => state.scout_sample_at_ms.get(key).map(serde_json::to_value).transpose(),
        "scout_plan_profiles" => state.scout_plan_profiles.get(key).map(serde_json::to_value).transpose(),
        "scout_active_audits" => state.scout_active_audits.get(key).map(serde_json::to_value).transpose(),
        "scout_sample_active_scenarios" => state.scout_sample_active_scenarios.get(key).map(serde_json::to_value).transpose(),
        "scout_observation_results" => state.scout_observation_results.get(key).map(serde_json::to_value).transpose(),
        "outcomes" => state.outcomes.get(key).map(serde_json::to_value).transpose(),
        "pairs" => state.pairs.get(key).map(serde_json::to_value).transpose(),
        "seen" => Ok(state.seen.contains(key).then(|| serde_json::Value::String(key.to_owned()))),
        _ => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("unknown Scout persistence record collection {collection}"))),
    };
    value
        .map(Option::unwrap_or_default)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn scout_state_section(
    state: &PersistedState,
    section: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let value = match section {
        "scout_configuration" => serde_json::to_value(&state.scout_configuration),
        "scout_variant_rotation_cursor" => serde_json::to_value(state.scout_variant_rotation_cursor),
        "scout_lifecycle_updated_at_ms" => serde_json::to_value(state.scout_lifecycle_updated_at_ms),
        "seen" => serde_json::to_value(&state.seen),
        "outcomes" => serde_json::to_value(&state.outcomes),
        "scout_instances" => serde_json::to_value(&state.scout_instances),
        "pairs" => serde_json::to_value(&state.pairs),
        "outbox" => serde_json::to_value(&state.outbox),
        "scout_sample_at_ms" => serde_json::to_value(&state.scout_sample_at_ms),
        "scout_sample_active_scenarios" => serde_json::to_value(&state.scout_sample_active_scenarios),
        "scout_plan_profiles" => serde_json::to_value(&state.scout_plan_profiles),
        "scout_plan_results" => serde_json::to_value(&state.scout_plan_results),
        "scout_pending_samples" => serde_json::to_value(&state.scout_pending_samples),
        "scout_observation_results" => serde_json::to_value(&state.scout_observation_results),
        "scout_active_audits" => serde_json::to_value(&state.scout_active_audits),
        _ if section.starts_with("scout_plan_results@") => {
            let key = section.split_once('@').map(|(_, key)| key).unwrap_or_default();
            state.scout_plan_results.get(key).map(serde_json::to_value)
                .transpose().map(|value| value.unwrap_or(serde_json::Value::Null))
        }
        _ => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("unknown Scout persistence section {section}"))),
    };
    value.map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn sync_configuration(
    tx: &rusqlite::Transaction<'_>,
    current: &serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    let config = current;
    let strategies = serde_json::to_string(&config["strategies"])
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    tx.execute(
        "INSERT INTO scout_configuration VALUES(1, ?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(id) DO UPDATE SET strategies=excluded.strategies, max_active_samples=excluded.max_active_samples, max_active_positions=excluded.max_active_positions, max_sample_notional=excluded.max_sample_notional, stop_loss_percent=excluded.stop_loss_percent, take_profit_percent=excluded.take_profit_percent WHERE scout_configuration.strategies IS NOT excluded.strategies OR scout_configuration.max_active_samples IS NOT excluded.max_active_samples OR scout_configuration.max_active_positions IS NOT excluded.max_active_positions OR scout_configuration.max_sample_notional IS NOT excluded.max_sample_notional OR scout_configuration.stop_loss_percent IS NOT excluded.stop_loss_percent OR scout_configuration.take_profit_percent IS NOT excluded.take_profit_percent",
        rusqlite::params![strategies, config["max_active_samples"].as_u64().unwrap_or_default() as i64, config["max_active_positions"].as_u64().unwrap_or_default() as i64, config["max_sample_notional"].as_f64().unwrap_or_default(), config["stop_loss_percent"].as_f64().unwrap_or_default(), config["take_profit_percent"].as_f64().unwrap_or_default()],
    ).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(())
}

fn sync_metadata(
    tx: &rusqlite::Transaction<'_>,
    state: &PersistedState,
) -> Result<(), (StatusCode, String)> {
    let cursor = state.scout_variant_rotation_cursor;
    let lifecycle = state.scout_lifecycle_updated_at_ms;
    tx.execute(
        "INSERT INTO scout_metadata VALUES(1, ?1, ?2) ON CONFLICT(id) DO UPDATE SET variant_rotation_cursor=excluded.variant_rotation_cursor, lifecycle_updated_at_ms=excluded.lifecycle_updated_at_ms WHERE scout_metadata.variant_rotation_cursor IS NOT excluded.variant_rotation_cursor OR scout_metadata.lifecycle_updated_at_ms IS NOT excluded.lifecycle_updated_at_ms",
        rusqlite::params![cursor as i64, lifecycle as i64],
    ).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    Ok(())
}

fn sync_record(
    tx: &rusqlite::Transaction<'_>,
    collection: &str,
    key: &str,
    current: &serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    if current.is_null() {
        tx.execute("DELETE FROM scout_records WHERE collection=?1 AND record_key=?2", rusqlite::params![collection, key])
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    } else {
        tx.execute("INSERT INTO scout_records(collection,record_key,payload) VALUES(?1,?2,?3) ON CONFLICT(collection,record_key) DO UPDATE SET payload=excluded.payload", rusqlite::params![collection, key, current.to_string()])
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    Ok(())
}

fn sync_record_map(
    tx: &rusqlite::Transaction<'_>,
    collection: &str,
    current: &serde_json::Value,
) -> Result<(), (StatusCode, String)> {
    let entries = current.as_object();
    let existing = {
        let mut statement = tx.prepare("SELECT record_key FROM scout_records WHERE collection=?1")
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        statement.query_map([collection], |row| row.get::<_, String>(0))
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
            .collect::<Result<BTreeSet<_>, _>>()
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
    };
    if let Some(entries) = entries {
    for (key, payload) in entries {
        tx.execute("INSERT INTO scout_records(collection,record_key,payload) VALUES(?1,?2,?3) ON CONFLICT(collection,record_key) DO UPDATE SET payload=excluded.payload WHERE scout_records.payload IS NOT excluded.payload", rusqlite::params![collection, key, payload.to_string()])
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    }
    if let Some(entries) = entries {
        for key in existing.iter().filter(|key| !entries.contains_key(*key)) {
            tx.execute("DELETE FROM scout_records WHERE collection=?1 AND record_key=?2", rusqlite::params![collection, key])
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }
    } else {
        tx.execute("DELETE FROM scout_records WHERE collection=?1", [collection])
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    }
    Ok(())
}

fn sync_record_array(
    tx: &rusqlite::Transaction<'_>,
    collection: &str,
    current: &serde_json::Value,
    keyed_by_value: bool,
) -> Result<(), (StatusCode, String)> {
    tx.execute("DELETE FROM scout_records WHERE collection=?1", [collection])
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    if let Some(items) = current.as_array() {
        for (index, payload) in items.iter().enumerate() {
            let key = if keyed_by_value { payload.as_str().unwrap_or_default().to_owned() } else { format!("{index:020}") };
            let serialized = if keyed_by_value { "null".to_owned() } else { payload.to_string() };
            tx.execute("INSERT INTO scout_records(collection,record_key,payload) VALUES(?1,?2,?3)", rusqlite::params![collection, key, serialized])
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
        }
    }
    Ok(())
}

fn load_scout_state(path: &std::path::Path) -> anyhow::Result<PersistedState> {
    // Decode each persisted record directly into its final collection. Building
    // one giant serde_json::Value tree first duplicated the full database in
    // memory during startup (and again while converting that tree into state).
    // The offline importer still uses load_scout_state_value when it needs a
    // whole-file transformation; runtime workers do not.
    let connection = rusqlite::Connection::open(path)?;
    let config = connection.query_row(
        "SELECT strategies, max_active_samples, max_active_positions, max_sample_notional, stop_loss_percent, take_profit_percent FROM scout_configuration WHERE id=1",
        [],
        |row| Ok(serde_json::json!({
            "strategies": serde_json::from_str::<Vec<String>>(&row.get::<_, String>(0)?).unwrap_or_default(),
            "max_active_samples": row.get::<_, i64>(1)?,
            "max_active_positions": row.get::<_, i64>(2)?,
            "max_sample_notional": row.get::<_, f64>(3)?,
            "stop_loss_percent": row.get::<_, f64>(4)?,
            "take_profit_percent": row.get::<_, f64>(5)?,
        })),
    )?;
    let (cursor, lifecycle): (i64, i64) = connection.query_row(
        "SELECT variant_rotation_cursor, lifecycle_updated_at_ms FROM scout_metadata WHERE id=1",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut state = PersistedState {
        scout_configuration: serde_json::from_value(config)?,
        scout_variant_rotation_cursor: cursor.max(0) as usize,
        scout_lifecycle_updated_at_ms: lifecycle.max(0) as u64,
        ..PersistedState::default()
    };
    // Seen IDs are an exact SQLite-backed set. Loading up to 100k string keys
    // into runtime state duplicated memory without helping membership checks.
    state.outcomes = load_scout_collection(&connection, "outcomes", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    // Full instance history is accessed through SQLite-backed report and
    // point-lookup helpers. `scout_instances` in PersistedState is a bounded
    // cache of recently touched decision IDs only.
    state.pairs = load_scout_collection(&connection, "pairs", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    state.outbox = load_scout_collection(&connection, "outbox", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?
    .into_values()
    .collect();
    state.scout_sample_at_ms = load_scout_collection(&connection, "scout_sample_at_ms", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    state.scout_sample_active_scenarios = load_scout_collection(
        &connection,
        "scout_sample_active_scenarios",
        |_, payload| Ok(serde_json::from_str(payload)?),
    )?;
    state.scout_plan_profiles = load_scout_collection(&connection, "scout_plan_profiles", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    state.scout_plan_results = load_scout_collection(&connection, "scout_plan_results", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    state.scout_pending_samples = load_scout_collection(&connection, "scout_pending_samples", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    state.scout_observation_results = load_scout_collection(
        &connection,
        "scout_observation_results",
        |_, payload| Ok(serde_json::from_str(payload)?),
    )?;
    state.scout_active_audits = load_scout_collection(&connection, "scout_active_audits", |_, payload| {
        Ok(serde_json::from_str(payload)?)
    })?;
    Ok(state)
}

fn load_scout_collection<T>(
    connection: &rusqlite::Connection,
    collection: &str,
    mut decode: impl FnMut(&str, &str) -> anyhow::Result<T>,
) -> anyhow::Result<BTreeMap<String, T>> {
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection=?1 ORDER BY record_key",
    )?;
    let rows = statement.query_map([collection], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut values = BTreeMap::new();
    for row in rows {
        let (key, payload) = row?;
        values.insert(key.clone(), decode(&key, &payload)?);
    }
    Ok(values)
}

const MAX_CACHED_SCOUT_INSTANCES: usize = 4_096;

fn cache_scout_instance(state: &mut PersistedState, key: &str, instance: serde_json::Value) {
    let instance = compact_scout_instance(&instance);
    state.scout_instances.insert(key.to_owned(), instance.clone());
    remove_portfolio_instance_index_entry_by_id(state, key);
    add_portfolio_instance_index_entry(state, key, &instance);
    trim_scout_instance_cache(state);
}

// Cache eviction removes only the full JSON value. Compact portfolio evidence
// remains indexed separately and all history remains in SQLite.
fn trim_scout_instance_cache(state: &mut PersistedState) {
    while state.scout_instances.len() > MAX_CACHED_SCOUT_INSTANCES {
        let oldest = state
            .scout_instances
            .iter()
            .min_by_key(|(id, value)| {
                (value["timestamp"].as_u64().unwrap_or_default(), (*id).clone())
            })
            .map(|(id, _)| id.clone());
        let Some(oldest) = oldest else { break };
        state.scout_instances.remove(&oldest);
    }
}

fn load_scout_instance(
    path: &std::path::Path,
    decision_id: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    let connection = rusqlite::Connection::open(path)?;
    connection
        .query_row(
            "SELECT payload FROM scout_records WHERE collection='scout_instances' AND record_key=?1",
            [decision_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .map(|payload| serde_json::from_str(&payload).map_err(Into::into))
        .transpose()
}

fn ensure_scout_instance_loaded(
    state: &mut PersistedState,
    path: &std::path::Path,
    decision_id: &str,
) -> anyhow::Result<Option<serde_json::Value>> {
    if let Some(instance) = state.scout_instances.get(decision_id) {
        return Ok(Some(instance.clone()));
    }
    let Some(instance) = load_scout_instance(path, decision_id)? else {
        return Ok(None);
    };
    cache_scout_instance(state, decision_id, instance.clone());
    Ok(Some(instance))
}

fn load_scout_instances_since(
    path: &std::path::Path,
    since_ms: u64,
) -> anyhow::Result<BTreeMap<String, serde_json::Value>> {
    let connection = rusqlite::Connection::open(path)?;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='scout_instances' ORDER BY record_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut instances = BTreeMap::new();
    for row in rows {
        let (key, payload) = row?;
        let instance: serde_json::Value = serde_json::from_str(&payload)?;
        if instance["timestamp"].as_u64().unwrap_or_default() >= since_ms {
            instances.insert(key, instance);
        }
    }
    Ok(instances)
}

fn compact_scout_instances_in_db(path: &std::path::Path) -> anyhow::Result<()> {
    let connection = rusqlite::Connection::open(path)?;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='scout_instances' ORDER BY record_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut update = connection.prepare(
        "UPDATE scout_records SET payload=?1 WHERE collection='scout_instances' AND record_key=?2",
    )?;
    for row in rows {
        let (key, payload) = row?;
        let instance: serde_json::Value = serde_json::from_str(&payload)?;
        let compact = compact_scout_instance(&instance);
        if compact != instance {
            update.execute(rusqlite::params![serde_json::to_string(&compact)?, key])?;
        }
    }
    Ok(())
}

fn rebuild_portfolio_instance_index_from_db(
    path: &std::path::Path,
    state: &mut PersistedState,
) -> anyhow::Result<()> {
    let connection = rusqlite::Connection::open(path)?;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='scout_instances' ORDER BY record_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut rebuilt = BTreeMap::<String, Vec<PortfolioInstanceEvidence>>::new();
    for row in rows {
        let (decision_id, payload) = row?;
        let instance: serde_json::Value = serde_json::from_str(&payload)?;
        if let Some((group, evidence)) = portfolio_instance_entry(&decision_id, &instance) {
            rebuilt.entry(group).or_default().push(evidence);
        }
    }
    for instances in rebuilt.values_mut() {
        instances.sort_by(|left, right| left.decision_id.cmp(&right.decision_id));
    }
    *state
        .portfolio_instances_by_group
        .write()
        .unwrap_or_else(|error| error.into_inner()) = rebuilt;
    Ok(())
}

fn scout_record_exists(
    path: &std::path::Path,
    collection: &str,
    key: &str,
) -> Result<bool, (StatusCode, String)> {
    let connection = rusqlite::Connection::open(path)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    connection
        .query_row(
            "SELECT 1 FROM scout_records WHERE collection=?1 AND record_key=?2",
            rusqlite::params![collection, key],
            |_| Ok(()),
        )
        .optional()
        .map(|value| value.is_some())
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

fn load_scout_state_value(path: &std::path::Path) -> anyhow::Result<serde_json::Value> {
    let connection = rusqlite::Connection::open(path)?;
    let mut state = serde_json::Map::new();
    let configuration = connection.query_row(
        "SELECT strategies, max_active_samples, max_active_positions, max_sample_notional, stop_loss_percent, take_profit_percent FROM scout_configuration WHERE id=1", [],
        |row| Ok(serde_json::json!({"strategies": serde_json::from_str::<Vec<String>>(&row.get::<_, String>(0)?).unwrap_or_default(), "max_active_samples": row.get::<_, i64>(1)?, "max_active_positions": row.get::<_, i64>(2)?, "max_sample_notional": row.get::<_, f64>(3)?, "stop_loss_percent": row.get::<_, f64>(4)?, "take_profit_percent": row.get::<_, f64>(5)?})),
    )?;
    state.insert("scout_configuration".into(), configuration);
    let (cursor, lifecycle): (i64, i64) = connection.query_row("SELECT variant_rotation_cursor, lifecycle_updated_at_ms FROM scout_metadata WHERE id=1", [], |row| Ok((row.get(0)?, row.get(1)?)))?;
    state.insert("scout_variant_rotation_cursor".into(), serde_json::json!(cursor));
    state.insert("scout_lifecycle_updated_at_ms".into(), serde_json::json!(lifecycle));
    for collection in ["seen", "outcomes", "scout_instances", "pairs", "outbox", "scout_sample_at_ms", "scout_sample_active_scenarios", "scout_plan_profiles", "scout_plan_results", "scout_pending_samples", "scout_observation_results", "scout_active_audits"] {
        let mut statement = connection.prepare("SELECT record_key, payload FROM scout_records WHERE collection=?1 ORDER BY record_key")?;
        let rows = statement.query_map([collection], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))?;
        if collection == "seen" || collection == "outbox" {
            let mut values = Vec::new();
            for row in rows {
                let (key, payload) = row?;
                values.push(if collection == "seen" { serde_json::Value::String(key) } else { serde_json::from_str(&payload)? });
            }
            state.insert(collection.into(), serde_json::Value::Array(values));
        } else {
            let mut values = serde_json::Map::new();
            for row in rows {
                let (key, payload) = row?;
                values.insert(key, serde_json::from_str(&payload)?);
            }
            state.insert(collection.into(), serde_json::Value::Object(values));
        }
    }
    Ok(serde_json::Value::Object(state))
}

fn active_scout_registrations(state: &PersistedState) -> Vec<serde_json::Value> {
    state
        .scout_active_audits
        .values()
        .flatten()
        .filter_map(scout_registration)
        .collect()
}

const SCOUT_OUTBOX_ACK_BATCH_SIZE: usize = 64;

async fn flush_scout_outbox(
    scout: &ScoutState,
    learning_rsocket_url: &str,
    insights_rsocket_url: &str,
) -> Result<(), (StatusCode, String)> {
    let _flush = scout.flush_lock.lock().await;
    let mut acknowledged = 0usize;
    loop {
        let item = scout.data.lock().await.outbox.get(acknowledged).cloned();
        let Some(item) = item else {
            acknowledge_scout_outbox(scout, acknowledged).await?;
            return Ok(());
        };
        if matches!(item, ScoutOutboxItem::Audit(_)) {
            acknowledged += 1;
            if acknowledged >= SCOUT_OUTBOX_ACK_BATCH_SIZE {
                acknowledge_scout_outbox(scout, acknowledged).await?;
                acknowledged = 0;
            }
            continue;
        }
        let delivery = match &item {
            ScoutOutboxItem::Learning(event) => {
                strategies::request_scout_feedback(learning_rsocket_url, event)
                    .await
                    .map(|ack| (ack.status, ack.detail.unwrap_or_default()))
            }
            ScoutOutboxItem::Registration(registration) => {
                let request_id = registration["decision_id"]
                    .as_str()
                    .or_else(|| registration["id"].as_str())
                    .unwrap_or("registration")
                    .to_owned();
                strategies::request_scout_service_route(
                    insights_rsocket_url,
                    &strategies::ScoutCoordinatorRouteCommand {
                        request_id,
                        strategy: None,
                        method: strategies::ScoutWorkerHttpMethod::Post,
                        path: "/internal/scouts/register".into(),
                        payload: registration.clone(),
                    },
                )
                .await
                .map(|response| (response.status, response.body.to_string()))
            }
            ScoutOutboxItem::Audit(_) => unreachable!("audit items are removed without Insights delivery"),
        };
        let (status, detail) = match delivery {
            Ok(response) => response,
            Err(error) => {
                acknowledge_scout_outbox(scout, acknowledged).await?;
                return Err((
                    StatusCode::BAD_GATEWAY,
                    format!("Scout outbox RSocket delivery failed: {error}"),
                ));
            }
        };
        if (400..500).contains(&status)
            && !(status == StatusCode::REQUEST_TIMEOUT.as_u16()
                || status == StatusCode::TOO_MANY_REQUESTS.as_u16())
        {
            eprintln!(
                "ERROR Discarding permanently rejected Scout outbox item: status={status} kind={} detail={}",
                match item {
                    ScoutOutboxItem::Learning(_) => "learning",
                    ScoutOutboxItem::Registration(_) => "registration",
                    ScoutOutboxItem::Audit(_) => "audit",
                },
                detail.chars().take(512).collect::<String>(),
            );
        } else if status >= 400 {
            acknowledge_scout_outbox(scout, acknowledged).await?;
            return Err((
                StatusCode::BAD_GATEWAY,
                format!("Scout outbox RSocket delivery failed with {status}: {detail}"),
            ));
        }
        acknowledged += 1;
        if acknowledged >= SCOUT_OUTBOX_ACK_BATCH_SIZE {
            acknowledge_scout_outbox(scout, acknowledged).await?;
            acknowledged = 0;
        }
    }
}

async fn acknowledge_scout_outbox(
    scout: &ScoutState,
    acknowledged: usize,
) -> Result<(), (StatusCode, String)> {
    if acknowledged == 0 {
        return Ok(());
    }
    let mut state = scout.data.lock().await;
    let acknowledged = acknowledged.min(state.outbox.len());
    let delivered = state.outbox.drain(..acknowledged).collect::<Vec<_>>();
    if let Err(error) = persist_scout_state(&scout.path, &state, &["outbox"]) {
        state.outbox.splice(0..0, delivered);
        return Err(error);
    }
    if state.outbox.is_empty() {
        state.outbox.shrink_to_fit();
    }
    Ok(())
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
