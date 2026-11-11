use super::*;

pub(super) const MINIMUM_SCOUT_PARAMETER_SAMPLES: usize = 2;
pub(super) const MAXIMUM_SCOUT_PARAMETER_SAMPLES: usize = 100;
pub(super) const PARAMETER_STEP_RELATIVE_TOLERANCE: f64 = 1.0e-6;

pub(super) async fn accept_feedback(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<Json<FeedbackResult>, (StatusCode, String)> {
    let (event_id, symbol, strategy, parameters, profile_id) = event_parts(&event);
    validate_feedback_identity(event_id, parameters)?;
    let mut state = service.state.lock().await;
    if state.seen_event_ids.contains(event_id) {
        return Ok(Json(FeedbackResult {
            accepted: true,
            duplicate: true,
            profile: None,
        }));
    }

    let profile_key = key(symbol, strategy);
    let minimum_parameter_samples = state.minimum_live_scout_parameter_samples;
    let source_profile = StrategyProfile {
        profile_id: profile_id.clone(),
        version: 0,
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    register_feedback_variant(&mut state, &event, source_profile, parameters, &profile_id)?;
    let update = append_profile_outcome(&mut state, &event, symbol, strategy, parameters)?;
    update_active_profile(
        &mut state,
        &event,
        symbol,
        strategy,
        parameters,
        minimum_parameter_samples,
        &profile_key,
        &update,
    );
    train_threshold_parameters(&mut state, &event, symbol, strategy, &profile_key, &update);
    finalize_feedback(&mut state, &event, event_id);
    persist(&service.path, &state)?;
    Ok(Json(FeedbackResult {
        accepted: true,
        duplicate: false,
        profile: Some(update.profile),
    }))
}

fn validate_feedback_identity(
    event_id: &str,
    parameters: &BTreeMap<String, f64>,
) -> Result<(), (StatusCode, String)> {
    if event_id.is_empty() || parameters.values().any(|value| !value.is_finite()) {
        Err((StatusCode::BAD_REQUEST, "invalid feedback event".into()))
    } else {
        Ok(())
    }
}

fn register_feedback_variant(
    state: &mut LearningState,
    event: &FeedbackEvent,
    source_profile: StrategyProfile,
    parameters: &BTreeMap<String, f64>,
    profile_id: &str,
) -> Result<(), (StatusCode, String)> {
    let registered = state
        .variants
        .entry(variant_key(&source_profile))
        .or_insert_with(|| RegisteredProfile {
            profile: source_profile,
            specs: event_specs(event).to_vec(),
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes: Vec::new(),
        });
    if registered
        .specs
        .iter()
        .any(|spec| !valid_spec(spec) || !parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::CONFLICT,
            "feedback does not include valid parameter bounds".into(),
        ));
    }
    if event_specs(event).iter().any(|spec| {
        !valid_spec(spec) || !parameters.contains_key(&spec.key) || !registered.specs.contains(spec)
    }) {
        return Err((
            StatusCode::CONFLICT,
            "Scout feedback references an unregistered parameter probe".into(),
        ));
    }
    if registered.profile.profile_id != profile_id || &registered.profile.parameters != parameters {
        return Err((
            StatusCode::CONFLICT,
            "feedback profile does not match registered variant".into(),
        ));
    }
    Ok(())
}

struct ProfileOutcomeUpdate {
    profile: StrategyProfile,
    samples: usize,
    positive: usize,
    realized_pnl: f64,
    registered_specs: Vec<ParameterSpec>,
    registered_profile: StrategyProfile,
}

fn append_profile_outcome(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
) -> Result<ProfileOutcomeUpdate, (StatusCode, String)> {
    let (_, _, _, _, profile_id) = event_parts(event);
    let variant = StrategyProfile {
        profile_id,
        version: 0,
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    let registered = state
        .variants
        .get_mut(&variant_key(&variant))
        .expect("feedback profile is registered before recording its outcome");
    record_learning_outcome(registered, event);
    refresh_profile_statistics(registered);
    let profile = update_profile(&registered.profile, event_specs(event), event).ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        "feedback did not match profile".into(),
    ))?;
    Ok(ProfileOutcomeUpdate {
        profile,
        samples: registered.samples,
        positive: registered.positive,
        realized_pnl: registered.realized_pnl,
        registered_specs: registered.specs.clone(),
        registered_profile: registered.profile.clone(),
    })
}

fn record_learning_outcome(registered: &mut RegisteredProfile, event: &FeedbackEvent) {
    registered.outcomes.push(learning_outcome(event));
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    registered
        .outcomes
        .retain(|outcome| outcome.observed_at_ms >= cutoff);
}

// Outcome history stores the complete event; threshold samples are added by the
// caller once the state borrow is available. This helper constructs that record.
fn learning_outcome(event: &FeedbackEvent) -> LearningOutcome {
    let (tested_parameter_keys, baseline_parameters) = match event {
        FeedbackEvent::ScoutOutcome {
            specs,
            baseline_parameters,
            ..
        } => (
            specs.iter().map(|spec| spec.key.clone()).collect(),
            baseline_parameters.clone(),
        ),
        FeedbackEvent::TradeOutcome { .. } => (Vec::new(), BTreeMap::new()),
    };
    LearningOutcome {
        observed_at_ms: event_observed_at(event),
        net_return_percent: event_net_return_percent(event),
        evidence_group_id: event_evidence_group_id(event),
        learning_context: match event {
            FeedbackEvent::TradeOutcome { scout: false, .. } => event_context(event)
                .and_then(|context| context.strip_suffix(":trade").map(str::to_owned)),
            _ => None,
        },
        source: match event {
            FeedbackEvent::ScoutOutcome { .. } => LearningOutcomeSource::Scout,
            FeedbackEvent::TradeOutcome { scout: true, .. } => LearningOutcomeSource::Scout,
            FeedbackEvent::TradeOutcome { scout: false, .. } => LearningOutcomeSource::Trade,
        },
        realized_pnl_usd: match event {
            FeedbackEvent::TradeOutcome {
                realized_pnl_usd, ..
            } => Some(*realized_pnl_usd),
            FeedbackEvent::ScoutOutcome { .. } => None,
        },
        tested_parameter_keys,
        baseline_parameters,
        paired_comparison: matches!(
            event,
            FeedbackEvent::ScoutOutcome {
                comparison: ScoutComparisonMetadata::Paired { .. },
                ..
            }
        ),
    }
}

fn refresh_profile_statistics(registered: &mut RegisteredProfile) {
    // Profile gates and statistics use actual closed-trade evidence only.
    registered.samples = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade)
        .count();
    registered.positive = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade && o.net_return_percent > 0.0)
        .count();
    registered.realized_pnl = registered
        .outcomes
        .iter()
        .filter(|o| o.source == LearningOutcomeSource::Trade)
        .filter_map(|o| o.realized_pnl_usd)
        .sum();
}

fn update_active_profile(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    parameters: &BTreeMap<String, f64>,
    minimum_parameter_samples: usize,
    profile_key: &str,
    update: &ProfileOutcomeUpdate,
) {
    let specs = match event {
        FeedbackEvent::ScoutOutcome { specs, .. } => specs.as_slice(),
        _ => &[],
    };
    let empty_baseline = BTreeMap::new();
    let baseline = match event {
        FeedbackEvent::ScoutOutcome {
            baseline_parameters,
            ..
        } => baseline_parameters,
        _ => &empty_baseline,
    };
    let active_matches_baseline = state
        .active_profiles
        .get(profile_key)
        .is_some_and(|active| {
            active.parameters.len() == parameters.len()
                && parameters.iter().all(|(key, value)| {
                    if specs.iter().any(|spec| &spec.key == key) {
                        baseline
                            .get(key)
                            .is_some_and(|base| active.parameters.get(key) == Some(base))
                    } else {
                        active.parameters.get(key) == Some(value)
                    }
                })
        });
    let paired = if specs.is_empty() || !active_matches_baseline {
        Vec::new()
    } else {
        paired_parameter_outcomes(
            &state.variants,
            symbol,
            strategy,
            parameters,
            specs,
            baseline,
        )
    };
    let promote_candidate = matches!(event, FeedbackEvent::ScoutOutcome { .. })
        && !specs.is_empty()
        && paired.len() >= minimum_parameter_samples
        && one_sided_90_lower_bound(&paired).is_some_and(|bound| bound > 0.0);
    if promote_candidate {
        promote_paired_parameters(
            &state.variants,
            &mut state.active_profiles,
            symbol,
            strategy,
            specs,
            parameters,
        );
    } else if matches!(event, FeedbackEvent::TradeOutcome { .. })
        && should_publish_trade_profile(update.samples, update.positive, update.realized_pnl)
    {
        let active = state
            .active_profiles
            .entry(profile_key.to_owned())
            .or_insert(update.profile.clone());
        // Trade feedback reports its entry profile without undoing newer Scout promotion.
        let mut promoted = active.clone();
        promoted.version = active.version.saturating_add(1);
        promoted.profile_id =
            strategies::profile_id(&promoted.symbol, &promoted.strategy, &promoted.parameters);
        promoted.learning_samples = update.samples;
        promoted.positive_samples = update.positive;
        promoted.realized_pnl = update.realized_pnl;
        *active = promoted;
    }
}

fn train_threshold_parameters(
    state: &mut LearningState,
    event: &FeedbackEvent,
    symbol: &str,
    strategy: &str,
    profile_key: &str,
    update: &ProfileOutcomeUpdate,
) {
    let FeedbackEvent::TradeOutcome {
        threshold_observations,
        net_return_percent,
        ..
    } = event
    else {
        return;
    };
    if !net_return_percent.is_finite() {
        return;
    }
    let valid_observations = threshold_observations
        .iter()
        .filter(|observation| {
            observation.parameter_key.len() <= 128
                && observation.threshold.is_finite()
                && observation.feature_value.is_finite()
                && update
                    .registered_profile
                    .parameters
                    .get(&observation.parameter_key)
                    .is_some_and(|value| (*value - observation.threshold).abs() <= 1e-8)
                && update
                    .registered_specs
                    .iter()
                    .any(|spec| spec.key == observation.parameter_key)
        })
        .collect::<Vec<_>>();
    if valid_observations.is_empty() {
        return;
    }
    for observation in valid_observations {
        state.threshold_samples.push(ThresholdTrainingSample {
            symbol: symbol.to_owned(),
            strategy: strategy.to_owned(),
            parameter_key: observation.parameter_key.clone(),
            observed_at_ms: event_observed_at(event),
            threshold: observation.threshold,
            feature_value: observation.feature_value,
            net_return_percent: *net_return_percent,
        });
    }
    if state.threshold_samples.len() > 100_000 {
        state
            .threshold_samples
            .sort_by_key(|sample| sample.observed_at_ms);
        state
            .threshold_samples
            .drain(..state.threshold_samples.len() - 100_000);
    }
    for spec in &update.registered_specs {
        let parameter_key = spec.key.as_str();
        if !state
            .threshold_samples
            .iter()
            .any(|sample| sample.strategy == strategy && sample.parameter_key == parameter_key)
        {
            continue;
        }
        let active_value = state
            .active_profiles
            .get(profile_key)
            .and_then(|active| active.parameters.get(parameter_key))
            .copied()
            .or_else(|| {
                update
                    .registered_profile
                    .parameters
                    .get(parameter_key)
                    .copied()
            });
        let Some(base) = active_value else { continue };
        let Some(tuned) = walk_forward_threshold(spec, base, strategy, &state.threshold_samples)
        else {
            continue;
        };
        if !tuning_delta_is_effective(spec.step, base, tuned) {
            continue;
        }
        let active = state
            .active_profiles
            .entry(profile_key.to_owned())
            .or_insert_with(|| update.registered_profile.clone());
        active.parameters.insert(parameter_key.to_owned(), tuned);
        active.version = active.version.saturating_add(1);
        active.profile_id =
            strategies::profile_id(&active.symbol, &active.strategy, &active.parameters);
    }
}

fn finalize_feedback(state: &mut LearningState, event: &FeedbackEvent, event_id: &str) {
    state.seen_event_ids.insert(event_id.to_owned());
    while state.seen_event_ids.len() > 100_000 {
        let Some(oldest) = state.seen_event_ids.iter().next().cloned() else {
            break;
        };
        state.seen_event_ids.remove(&oldest);
    }
    state.accepted_events = state.accepted_events.saturating_add(1);
    if let Some(context) = event_context(event) {
        record_context_outcome(
            &mut state.context_learning,
            &context,
            event_observed_at(event),
            event_was_profitable(event),
            event_net_return_percent(event),
        );
    }
}

#[derive(Deserialize)]
pub(super) struct ScoutParameterSamplesRequest {
    minimum_samples: usize,
}

pub(super) async fn update_scout_parameter_samples(
    State(service): State<ServiceState>,
    Json(request): Json<ScoutParameterSamplesRequest>,
) -> Result<Json<usize>, (StatusCode, String)> {
    if !(MINIMUM_SCOUT_PARAMETER_SAMPLES..=MAXIMUM_SCOUT_PARAMETER_SAMPLES)
        .contains(&request.minimum_samples)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "minimum_samples must be between {MINIMUM_SCOUT_PARAMETER_SAMPLES} and {MAXIMUM_SCOUT_PARAMETER_SAMPLES}"
            ),
        ));
    }
    let mut state = service.state.lock().await;
    state.minimum_live_scout_parameter_samples = request.minimum_samples;
    persist(&service.path, &state)?;
    Ok(Json(state.minimum_live_scout_parameter_samples))
}

fn tuning_delta_is_effective(step: f64, base: f64, tuned: f64) -> bool {
    (tuned - base).abs() >= step * PARAMETER_STEP_RELATIVE_TOLERANCE
}

fn paired_parameter_outcomes(
    variants: &BTreeMap<String, RegisteredProfile>,
    symbol: &str,
    strategy: &str,
    candidate_parameters: &BTreeMap<String, f64>,
    specs: &[ParameterSpec],
    baseline_parameters: &BTreeMap<String, f64>,
) -> Vec<f64> {
    let mut grouped_outcomes = BTreeMap::<String, Vec<f64>>::new();
    for (profile_id, variant) in variants.iter().filter(|(_, variant)| {
        variant.profile.symbol == symbol && variant.profile.strategy == strategy
    }) {
        let matches_candidate = &variant.profile.parameters == candidate_parameters;
        if !matches_candidate {
            continue;
        }
        for (index, outcome) in variant.outcomes.iter().enumerate() {
            if outcome.source == LearningOutcomeSource::Scout
                && outcome.paired_comparison
                && outcome.baseline_parameters == *baseline_parameters
                && specs
                    .iter()
                    .all(|spec| outcome.tested_parameter_keys.contains(&spec.key))
            {
                let group = outcome
                    .evidence_group_id
                    .clone()
                    .unwrap_or_else(|| format!("{profile_id}:{index}"));
                grouped_outcomes
                    .entry(group)
                    .or_default()
                    .push(outcome.net_return_percent);
            }
        }
    }
    grouped_outcomes
        .values()
        .map(|returns| returns.iter().sum::<f64>() / returns.len() as f64)
        .collect()
}

fn promote_paired_parameters(
    variants: &BTreeMap<String, RegisteredProfile>,
    active_profiles: &mut BTreeMap<String, StrategyProfile>,
    symbol: &str,
    strategy: &str,
    specs: &[ParameterSpec],
    candidate_parameters: &BTreeMap<String, f64>,
) {
    let candidates = variants
        .values()
        .filter(|variant| {
            variant.profile.strategy == strategy
                && variant.profile.symbol == symbol
                && &variant.profile.parameters == candidate_parameters
                && variant.outcomes.iter().any(|outcome| {
                    specs
                        .iter()
                        .all(|spec| outcome.tested_parameter_keys.contains(&spec.key))
                })
        })
        .map(|variant| variant.profile.clone())
        .collect::<Vec<_>>();

    for candidate in candidates {
        let profile_key = key(&candidate.symbol, strategy);
        let active = active_profiles
            .get(&profile_key)
            .cloned()
            .unwrap_or_else(|| candidate.clone());
        let mut promoted = active.clone();
        let mut changed = false;
        for spec in specs {
            let Some(value) = candidate.parameters.get(&spec.key).copied() else {
                continue;
            };
            if promoted.parameters.get(&spec.key) != Some(&value) {
                promoted.parameters.insert(spec.key.clone(), value);
                changed = true;
            }
        }
        if changed {
            promoted.version = active.version.saturating_add(1);
            promoted.profile_id =
                strategies::profile_id(&promoted.symbol, &promoted.strategy, &promoted.parameters);
            active_profiles.insert(profile_key, promoted);
        }
    }
}

//TODO comment this - lots of random numbers!!
pub(super) fn one_sided_90_t_critical(sample_count: usize) -> Option<f64> {
    let degrees = sample_count.checked_sub(1)? as f64;
    if degrees < 1.0 {
        return None;
    }
    match degrees as usize {
        1 => return Some(3.077_683_537),
        2 => return Some(1.885_618_083),
        3 => return Some(1.637_744_354),
        4 => return Some(1.533_206_275),
        _ => {}
    }
    let z = 1.281_551_565_544_600_4;
    let z2 = z * z;
    let z3 = z2 * z;
    let z5 = z3 * z2;
    let z7 = z5 * z2;
    Some(
        z + (z3 + z) / (4.0 * degrees)
            + (5.0 * z5 + 16.0 * z3 + 3.0 * z) / (96.0 * degrees.powi(2))
            + (3.0 * z7 + 19.0 * z5 + 17.0 * z3 - 15.0 * z) / (384.0 * degrees.powi(3)),
    )
}

//TODO comment this - what is it really doing
fn one_sided_90_lower_bound(outcomes: &[f64]) -> Option<f64> {
    if outcomes.len() < 2 || outcomes.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let sample_count = outcomes.len();
    let mean = outcomes.iter().sum::<f64>() / sample_count as f64;
    let variance = outcomes
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (sample_count - 1) as f64;
    let critical = one_sided_90_t_critical(sample_count)?;
    Some(mean - critical * (variance / sample_count as f64).sqrt())
}

pub(super) fn should_publish_trade_profile(
    samples: usize,
    positive: usize,
    net_outcome: f64,
) -> bool {
    samples >= 5 && positive <= samples && net_outcome.is_finite()
}

pub(super) fn persist(path: &Path, state: &LearningState) -> Result<(), (StatusCode, String)> {
    //TODO - could we just keep the db open ?
    let connection =
        super::open_state_db(path).map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
    super::persist_normalized(&connection, state).map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("could not persist learning state: {error}"),
        )
    })?;
    Ok(())
}

pub(super) fn valid_spec(spec: &ParameterSpec) -> bool {
    spec.default.is_finite()
        && spec.min.is_finite()
        && spec.max.is_finite()
        && spec.step.is_finite()
        && spec.learning_direction.is_finite()
        && spec.min <= spec.default
        && spec.default <= spec.max
        && spec.min < spec.max
        && spec.step > 0.0
}

pub(super) fn event_specs(event: &FeedbackEvent) -> &[ParameterSpec] {
    match event {
        FeedbackEvent::ScoutOutcome { specs, .. } | FeedbackEvent::TradeOutcome { specs, .. } => {
            specs
        }
    }
}

//TODO rename - is positive is vague.
pub(super) fn event_was_profitable(event: &FeedbackEvent) -> bool {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        } => *net_return_percent > 0.0,
        FeedbackEvent::TradeOutcome {
            realized_pnl_usd, ..
        } => *realized_pnl_usd > 0.0,
    }
}

pub(super) fn event_net_return_percent(event: &FeedbackEvent) -> f64 {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        }
        | FeedbackEvent::TradeOutcome {
            net_return_percent, ..
        } => *net_return_percent,
    }
}

fn event_evidence_group_id(event: &FeedbackEvent) -> Option<String> {
    let FeedbackEvent::ScoutOutcome {
        comparison: ScoutComparisonMetadata::Paired { group_id, .. },
        ..
    } = event
    else {
        return None;
    };
    Some(group_id.clone())
}

pub(super) fn event_context(event: &FeedbackEvent) -> Option<String> {
    match event {
        FeedbackEvent::ScoutOutcome {
            learning_context, ..
        } => learning_context.clone(),
        FeedbackEvent::TradeOutcome {
            learning_context, ..
        } => {
            let raw = learning_context.as_deref()?;
            let (scout_prefix, context) = raw
                .strip_prefix("scout:")
                .map_or(("", raw), |context| ("scout:", context));
            let context = context.split_once(":P")?.0;
            (context.split(':').count() >= 3).then(|| format!("{scout_prefix}{context}:trade"))
        }
    }
}

pub(super) fn record_context_outcome(
    contexts: &mut BTreeMap<String, ContextLearningStat>,
    context: &str,
    observed_at_ms: u64,
    positive: bool,
    value: f64,
) {
    if context.is_empty() || context.len() > 256 || !value.is_finite() {
        return;
    }
    let stats = contexts
        .entry(context.to_owned())
        .or_insert(ContextLearningStat {
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            updated_at_ms: observed_at_ms,
        });
    if stats.updated_at_ms != 0 {
        let elapsed = observed_at_ms.saturating_sub(stats.updated_at_ms);
        //TODO remove hardcoding
        let weight = 0.5_f64.powf(elapsed as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1_000.0));
        stats.samples = (stats.samples as f64 * weight).round() as usize;
        stats.positive = ((stats.positive as f64 * weight).round() as usize).min(stats.samples);
        stats.realized_pnl *= weight;
    }
    stats.samples = stats.samples.saturating_add(1);
    if positive {
        stats.positive = stats.positive.saturating_add(1);
    }
    stats.realized_pnl += value;
    stats.updated_at_ms = observed_at_ms;
}

/// Apply one event only to the exact profile variant that generated it.
pub fn update_profile(
    profile: &StrategyProfile,
    specs: &[ParameterSpec],
    event: &FeedbackEvent,
) -> Option<StrategyProfile> {
    let (event_profile, strategy, symbol, parameters, outcome) = match event {
        FeedbackEvent::ScoutOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            net_return_percent,
            ..
        } => (
            profile_id,
            strategy,
            symbol,
            parameters,
            *net_return_percent,
        ),
        FeedbackEvent::TradeOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            realized_pnl_usd,
            ..
        } => (profile_id, strategy, symbol, parameters, *realized_pnl_usd),
    };
    if event_profile != &profile.profile_id
        || strategy != &profile.strategy
        || symbol != &profile.symbol
        || parameters != &profile.parameters
        || !outcome.is_finite()
    {
        return None;
    }
    let mut updated = profile.clone();
    // Feedback changes evidence. Paired scout promotion below chooses the
    // exact tested parameter value; outcome feedback never steps past it.
    let _ = specs;
    updated.version = updated.version.saturating_add(1);
    updated.profile_id =
        strategies::profile_id(&updated.symbol, &updated.strategy, &updated.parameters);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_sided_90_student_t_bounds_match_small_and_mature_samples() {
        assert!((one_sided_90_t_critical(6).unwrap() - 1.4759).abs() < 0.002);
        assert!((one_sided_90_t_critical(20).unwrap() - 1.3277).abs() < 0.002);
        assert!((one_sided_90_t_critical(2).unwrap() - 3.0777).abs() < 0.002);
        assert!((one_sided_90_t_critical(5).unwrap() - 1.5332).abs() < 0.002);
        assert!(one_sided_90_t_critical(1).is_none());
        assert!((one_sided_90_lower_bound(&[0.02; 6]).unwrap() - 0.02).abs() < 1e-12);
    }

    fn spec() -> ParameterSpec {
        ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        }
    }

    fn scout_event() -> FeedbackEvent {
        FeedbackEvent::ScoutOutcome {
            event_id: "e".into(),
            observed_at_ms: 100,
            learning_context: Some("btc:momentum:long".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec()],
            net_return_percent: 1.0,
        }
    }

    #[test]
    fn paired_evidence_does_not_mix_baselines_or_surrounding_profiles() {
        let candidate = BTreeMap::from([("threshold".into(), 0.4), ("stop".into(), 0.5)]);
        let baseline = BTreeMap::from([("threshold".into(), 0.5)]);
        let outcome =
            |baseline: BTreeMap<String, f64>, value: f64, group: Option<&str>| LearningOutcome {
                observed_at_ms: 100,
                net_return_percent: value,
                evidence_group_id: group.map(str::to_owned),
                learning_context: None,
                source: LearningOutcomeSource::Scout,
                realized_pnl_usd: None,
                paired_comparison: true,
                tested_parameter_keys: vec!["threshold".into()],
                baseline_parameters: baseline,
            };
        let profile = |parameters: BTreeMap<String, f64>, outcomes| RegisteredProfile {
            profile: StrategyProfile {
                profile_id: "profile".into(),
                version: 0,
                symbol: "BTC-USDT".into(),
                strategy: "momentum".into(),
                parameters,
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            },
            specs: vec![spec()],
            samples: 0,
            positive: 0,
            realized_pnl: 0.0,
            outcomes,
        };
        let variants = BTreeMap::from([
            (
                "candidate".into(),
                profile(
                    candidate.clone(),
                    vec![
                        outcome(baseline.clone(), 0.1, Some("same-opportunity")),
                        outcome(baseline.clone(), 0.3, Some("same-opportunity")),
                        outcome(BTreeMap::from([("threshold".into(), 0.6)]), 1.0, None),
                    ],
                ),
            ),
            (
                "other".into(),
                profile(
                    BTreeMap::from([("threshold".into(), 0.4), ("stop".into(), 0.8)]),
                    vec![outcome(baseline.clone(), 2.0, None)],
                ),
            ),
        ]);
        assert_eq!(
            paired_parameter_outcomes(
                &variants,
                "BTC-USDT",
                "momentum",
                &candidate,
                &[spec()],
                &baseline
            ),
            vec![0.2]
        );
    }

    #[test]
    fn feedback_event_helpers_extract_fields_and_positive_values() {
        let scout = scout_event();
        assert_eq!(event_specs(&scout).len(), 1);
        assert!(event_was_profitable(&scout));
        assert_eq!(event_observed_at(&scout), 100);
        let mut horizon = scout_event();
        if let FeedbackEvent::ScoutOutcome { comparison, .. } = &mut horizon {
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "comparison".into(),
                horizon_seconds: None,
            };
        }
        assert_eq!(
            event_evidence_group_id(&horizon).as_deref(),
            Some("comparison")
        );
        if let FeedbackEvent::ScoutOutcome { comparison, .. } = &mut horizon {
            *comparison = ScoutComparisonMetadata::Paired {
                group_id: "comparison".into(),
                horizon_seconds: Some(300),
            };
        }
        assert_eq!(
            event_evidence_group_id(&horizon).as_deref(),
            Some("comparison")
        );
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "t".into(),
            observed_at_ms: 200,
            learning_context: Some("scout:btc:momentum:long:P2".into()),
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: true,
            realized_pnl_usd: -2.0,
            net_return_percent: 3.0,
            threshold_observations: Vec::new(),
        };
        assert!(!event_was_profitable(&trade));
        assert_eq!(event_net_return_percent(&trade), 3.0);
        assert_eq!(
            event_context(&trade).as_deref(),
            Some("scout:btc:momentum:long:trade")
        );
    }

    #[test]
    fn tuning_delta_uses_step_relative_tolerance_boundary() {
        let step = 0.5;
        let base = 10.0;
        let tolerance = step * PARAMETER_STEP_RELATIVE_TOLERANCE;
        assert!(!tuning_delta_is_effective(
            step,
            base,
            base + tolerance * 0.5
        ));
        assert!(tuning_delta_is_effective(step, base, base + tolerance));
    }

    #[test]
    fn scout_feedback_requires_explicit_comparison_metadata() {
        let serialized = serde_json::to_value(scout_event()).unwrap();
        assert_eq!(
            serialized["comparison"]["status"],
            serde_json::json!("independent")
        );

        let mut legacy = serialized;
        legacy.as_object_mut().unwrap().remove("comparison");
        assert!(serde_json::from_value::<FeedbackEvent>(legacy).is_err());
    }

    #[test]
    fn event_context_requires_a_complete_trade_learning_context() {
        for context in [None, Some("btc:momentum"), Some("btc:momentum:long")] {
            let event = FeedbackEvent::TradeOutcome {
                event_id: "t".into(),
                observed_at_ms: 1,
                learning_context: context.map(str::to_owned),
                symbol: "BTC".into(),
                strategy: "s".into(),
                profile_id: "p".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: false,
                realized_pnl_usd: 0.0,
                net_return_percent: 0.0,
                threshold_observations: Vec::new(),
            };
            assert!(event_context(&event).is_none());
        }
        let scout = scout_event();
        assert_eq!(event_context(&scout).as_deref(), Some("btc:momentum:long"));
    }

    #[test]
    fn scout_trade_outcomes_share_the_scout_category() {
        let mut scout_trade = match scout_event() {
            FeedbackEvent::ScoutOutcome { .. } => FeedbackEvent::TradeOutcome {
                event_id: "paper-scout".into(),
                observed_at_ms: 1,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "momentum".into(),
                profile_id: "p".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: true,
                realized_pnl_usd: 1.0,
                net_return_percent: 0.5,
                threshold_observations: vec![],
            },
            _ => unreachable!(),
        };
        assert_eq!(
            learning_outcome(&scout_trade).source,
            LearningOutcomeSource::Scout
        );
        if let FeedbackEvent::TradeOutcome { scout, .. } = &mut scout_trade {
            *scout = false;
        }
        assert_eq!(
            learning_outcome(&scout_trade).source,
            LearningOutcomeSource::Trade
        );
    }

    #[test]
    fn parameter_specs_validate_finite_ordered_bounds_and_positive_steps() {
        assert!(valid_spec(&spec()));
        let mut invalid = spec();
        invalid.step = 0.0;
        assert!(!valid_spec(&invalid));
        let mut invalid = spec();
        invalid.default = f64::NAN;
        assert!(!valid_spec(&invalid));
        let mut invalid = spec();
        invalid.min = 0.8;
        assert!(!valid_spec(&invalid));
    }

    #[test]
    fn context_outcomes_ignore_invalid_inputs_and_decay_existing_statistics() {
        let mut contexts = BTreeMap::new();
        record_context_outcome(&mut contexts, "", 100, true, 1.0);
        record_context_outcome(&mut contexts, "valid", 100, true, f64::NAN);
        assert!(contexts.is_empty());
        record_context_outcome(&mut contexts, "valid", 100, true, 2.0);
        record_context_outcome(
            &mut contexts,
            "valid",
            100 + 7 * 24 * 60 * 60 * 1_000,
            false,
            -1.0,
        );
        let stats = &contexts["valid"];
        assert_eq!(stats.samples, 2);
        assert_eq!(stats.positive, 1);
        assert_eq!(stats.realized_pnl, 0.0);
    }

    #[test]
    fn persistence_round_trips_state_atomically() {
        let path = std::env::temp_dir().join(format!(
            "learning-persist-test-{}.sqlite",
            std::process::id()
        ));
        let mut state = LearningState {
            accepted_events: 2,
            minimum_live_scout_parameter_samples: 9,
            seen_event_ids: ["keep".to_owned(), "remove".to_owned()].into(),
            ..LearningState::default()
        };
        persist(&path, &state).unwrap();
        state.accepted_events = 1;
        state.seen_event_ids.remove("remove");
        persist(&path, &state).unwrap();
        let decoded = super::super::load_state(&path).unwrap();
        assert!(decoded.variants.is_empty());
        assert_eq!(decoded.accepted_events, 1);
        assert_eq!(decoded.minimum_live_scout_parameter_samples, 9);
        assert_eq!(decoded.seen_event_ids, ["keep".to_owned()].into());
        let _ = std::fs::remove_file(&path);
    }
}
