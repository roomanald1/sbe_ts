use super::*;

pub(super) async fn execution_decision(
    State(service): State<ServiceState>,
    Json(request): Json<ExecutionLearningRequest>,
) -> Result<Json<ExecutionLearningResponse>, (StatusCode, String)> {
    if request.context.is_empty()
        || request.context.len() > 256
        || request.order_types.len() > 32
        || request.profile_count > 16
        || !request.risk_appetite.is_finite()
        || request.learning.len() > 100_000
        || request.learning.values().any(|stats| {
            !stats.fill_rate_total.is_finite()
                || !stats.cost_notional.is_finite()
                || !stats.all_in_cost_notional_bps.is_finite()
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid execution-learning request".into(),
        ));
    }
    let mut state = service.state.lock().await;
    let mut imported = false;
    for (key, stats) in &request.learning {
        if !state.execution_learning.contains_key(key) {
            state.execution_learning.insert(key.clone(), stats.clone());
            imported = true;
        }
    }
    if imported {
        persist(&service.path, &state)?;
    }
    let mut request = request;
    request.learning = state.execution_learning.clone();
    drop(state);
    let learned_order_type = learned_order_type(&request);
    let (schedule_depth_threshold, schedule_threshold_samples) =
        learned_schedule_threshold(&request);
    let parameter_profiles = request
        .order_types
        .iter()
        .map(|order_type| {
            (
                order_type.clone(),
                choose_execution_profile(
                    &request,
                    order_type,
                    request.explore_profile_order_types.contains(order_type),
                ),
            )
        })
        .collect();
    Ok(Json(ExecutionLearningResponse {
        learned_order_type,
        schedule_depth_threshold,
        schedule_threshold_samples,
        parameter_profiles,
    }))
}

//TODO remove all hardcoded values. 
pub(super) async fn execution_outcome(
    State(service): State<ServiceState>,
    Json(event): Json<ExecutionOutcomeEvent>,
) -> Result<StatusCode, (StatusCode, String)> {
    if event.event_id.is_empty()
        || event.event_id.len() > 240
        || event.context.is_empty()
        || event.context.len() > 256
        || event.order_type.is_empty()
        || event.order_type.len() > 40
        || !event.fill_rate.is_finite()
        || !(0.0..=1.0).contains(&event.fill_rate)
        || !event.cost_notional.is_finite()
        || event.cost_notional < 0.0
        || !event.all_in_cost_notional_bps.is_finite()
        || event.observed_at_ms > now_ms().saturating_add(5_000)
    {
        return Err((StatusCode::BAD_REQUEST, "invalid execution outcome".into()));
    }
    let mut state = service.state.lock().await;
    if state.seen_execution_event_ids.contains(&event.event_id) {
        return Ok(StatusCode::NO_CONTENT);
    }
    let key = format!("{}:exec:{}", event.context, event.order_type);
    let stats = state
        .execution_learning
        .entry(key)
        .or_insert(ExecutionLearningStat {
            samples: 0,
            filled_samples: 0,
            fill_rate_total: 0.0,
            cost_notional: 0.0,
            all_in_cost_notional_bps: 0.0,
            updated_at_ms: event.observed_at_ms,
        });
    if stats.updated_at_ms != 0 {
        let weight = 0.5_f64.powf(
            event.observed_at_ms.saturating_sub(stats.updated_at_ms) as f64
                / (7.0 * 24.0 * 60.0 * 60.0 * 1_000.0),
        );
        stats.samples = (stats.samples as f64 * weight).round() as usize;
        stats.filled_samples =
            ((stats.filled_samples as f64 * weight).round() as usize).min(stats.samples);
        stats.fill_rate_total *= weight;
        stats.cost_notional *= weight;
        stats.all_in_cost_notional_bps *= weight;
    }
    stats.samples = stats.samples.saturating_add(1);
    stats.fill_rate_total += event.fill_rate;
    if event.filled {
        stats.filled_samples = stats.filled_samples.saturating_add(1);
        stats.cost_notional += event.cost_notional;
        stats.all_in_cost_notional_bps += event.all_in_cost_notional_bps;
    }
    stats.updated_at_ms = event.observed_at_ms;
    state.seen_execution_event_ids.insert(event.event_id);
    while state.seen_execution_event_ids.len() > 100_000 {
        if let Some(oldest) = state.seen_execution_event_ids.iter().next().cloned() {
            state.seen_execution_event_ids.remove(&oldest);
        }
    }
    persist(&service.path, &state)?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) fn decayed_execution_stat(
    mut stats: ExecutionLearningStat,
    now: u64,
) -> ExecutionLearningStat {
    if stats.updated_at_ms != 0 {
        let weight = 0.5_f64.powf(
            now.saturating_sub(stats.updated_at_ms) as f64 / (7.0 * 24.0 * 60.0 * 60.0 * 1_000.0),
        );
        stats.samples = (stats.samples as f64 * weight).round() as usize;
        stats.filled_samples =
            ((stats.filled_samples as f64 * weight).round() as usize).min(stats.samples);
        stats.fill_rate_total *= weight;
        stats.cost_notional *= weight;
        stats.all_in_cost_notional_bps *= weight;
    }
    stats
}

pub(super) fn aggregate_execution_stats(
    request: &ExecutionLearningRequest,
    prefix: &str,
    suffix: &str,
) -> ExecutionLearningStat {
    let now = now_ms();
    request
        .learning
        .iter() 
        //TODO do not iterate every time we add something new. 
        //Need a way to have the data structure determine the bucket and simply append this learning outcome to that bucket of information i.e. strategy + symbol = Bucket[outcome1, outcome2]. 
        .filter(|(key, _)| key.starts_with(prefix) && key.ends_with(suffix))
        .map(|(_, stats)| decayed_execution_stat(stats.clone(), now))
        .fold(
            ExecutionLearningStat {
                samples: 0,
                filled_samples: 0,
                fill_rate_total: 0.0,
                cost_notional: 0.0,
                all_in_cost_notional_bps: 0.0,
                updated_at_ms: now,
            },
            |mut total, stats| {
                total.samples += stats.samples;
                total.filled_samples += stats.filled_samples;
                total.fill_rate_total += stats.fill_rate_total;
                total.cost_notional += stats.cost_notional;
                total.all_in_cost_notional_bps += stats.all_in_cost_notional_bps;
                total
            },
        )
}

pub(super) fn learned_order_type(request: &ExecutionLearningRequest) -> Option<String> {
    const MIN_SAMPLES: usize = 10;
    const MIN_FILLS: usize = 5;
    if !request.allow_order_type_adaptation {
        return Some(request.baseline_order_type.clone());
    }
    let baseline = aggregate_execution_stats(
        request,
        &format!("{}:P", request.context),
        &format!(":exec:{}", request.baseline_order_type),
    );
    if baseline.samples < MIN_SAMPLES
        || baseline.filled_samples < MIN_FILLS
        || baseline.cost_notional <= 0.0
    {
        return Some(request.baseline_order_type.clone());
    }
    let baseline_cost = baseline.all_in_cost_notional_bps / baseline.cost_notional;
    let baseline_fill = baseline.fill_rate_total / baseline.samples as f64;
    request
        .order_types
        .iter()
        .filter(|kind| *kind != &request.baseline_order_type)
        .filter_map(|kind| {
            let stats = aggregate_execution_stats(
                request,
                &format!("{}:P", request.context), //TODO remove prefix, postfix - lets instead have a composite key and multiple different maps to look up the items by different ids 
                &format!(":exec:{kind}"),
            );
            if stats.samples < MIN_SAMPLES
                || stats.filled_samples < MIN_FILLS
                || stats.cost_notional <= 0.0
            {
                return None;
            }
            let cost = stats.all_in_cost_notional_bps / stats.cost_notional;
            let fill = stats.fill_rate_total / stats.samples as f64;
            (fill + 0.05 >= baseline_fill && cost + 0.5 < baseline_cost)
                .then_some((kind.clone(), cost))
        })
        .min_by(|left, right| left.1.total_cmp(&right.1))
        .map(|(kind, _)| kind)
        .or_else(|| Some(request.baseline_order_type.clone()))
}

pub(super) fn choose_execution_profile(
    request: &ExecutionLearningRequest,
    order_type: &str,
    explore: bool,
) -> usize {
    const MIN_SAMPLES: usize = 10;
    const MIN_FILLS: usize = 5;
    let profiles = (0..request.profile_count)
        .map(|profile| {
            let prefix = format!("{}:P{profile}:", request.context);
            let suffix = format!(":exec:{order_type}");
            (
                profile,
                aggregate_execution_stats(request, &prefix, &suffix),
            )
        })
        .collect::<Vec<_>>();
    if explore {
        if let Some((profile, _)) = profiles
            .iter()
            .filter(|(_, stats)| stats.samples < MIN_SAMPLES || stats.filled_samples < MIN_FILLS)
            .min_by_key(|(_, stats)| stats.samples)
        {
            return *profile;
        }
    }
    profiles
        .iter()
        .filter(|(_, stats)| {
            stats.samples >= MIN_SAMPLES
                && stats.filled_samples >= MIN_FILLS
                && stats.cost_notional > 0.0
                && stats.fill_rate_total / stats.samples as f64 >= 0.60
        })
        .min_by(|left, right| {
            (left.1.all_in_cost_notional_bps / left.1.cost_notional)
                .total_cmp(&(right.1.all_in_cost_notional_bps / right.1.cost_notional))
        })
        .map(|(profile, _)| *profile)
        .unwrap_or(0)
}

pub(super) fn learned_schedule_threshold(request: &ExecutionLearningRequest) -> (f64, usize) {
    const MIN_SAMPLES: usize = 10;
    const MIN_FILLS: usize = 5;
    let mut threshold: f64 = 0.25;
    let mut paired_samples = 0;
    //TODO remove hardcoding
    for (bucket, lower, upper) in [
        ("05_10", 0.05_f64, 0.10_f64),
        ("10_15", 0.10, 0.15),
        ("15_25", 0.15, 0.25),
        ("25_40", 0.25, 0.40),
        ("ge40", 0.40, 1.0),
    ] {
        let marker = format!(":depth:{bucket}:");
        let mut scheduled = ExecutionLearningStat {
            samples: 0,
            filled_samples: 0,
            fill_rate_total: 0.0,
            cost_notional: 0.0,
            all_in_cost_notional_bps: 0.0,
            updated_at_ms: 0,
        };
        let mut alternatives = scheduled.clone();
        for (key, raw) in request.learning.iter().filter(|(key, _)| {
            key.starts_with(&format!("{}:P", request.context)) && key.contains(&marker)
        }) {
            let stats = decayed_execution_stat(raw.clone(), now_ms());
            let target = if key.ends_with(":exec:Vwap") || key.ends_with(":exec:Twap") {
                &mut scheduled
            } else {
                &mut alternatives
            };
            target.samples += stats.samples;
            target.filled_samples += stats.filled_samples;
            target.fill_rate_total += stats.fill_rate_total;
            target.cost_notional += stats.cost_notional;
            target.all_in_cost_notional_bps += stats.all_in_cost_notional_bps;
        }
        if scheduled.samples < MIN_SAMPLES
            || scheduled.filled_samples < MIN_FILLS
            || scheduled.cost_notional <= 0.0
        {
            continue;
        }
        paired_samples += scheduled.samples + alternatives.samples;
        if alternatives.samples >= MIN_SAMPLES
            && alternatives.filled_samples >= MIN_FILLS
            && alternatives.cost_notional > 0.0
        {
            let s_cost = scheduled.all_in_cost_notional_bps / scheduled.cost_notional;
            let a_cost = alternatives.all_in_cost_notional_bps / alternatives.cost_notional;
            let s_fill = scheduled.fill_rate_total / scheduled.samples as f64;
            let a_fill = alternatives.fill_rate_total / alternatives.samples as f64;
            if s_cost + 0.5 < a_cost && s_fill + 0.05 >= a_fill {
                threshold = threshold.min(lower.max(0.05));
            } else if s_cost > a_cost + 0.5 && s_fill + 0.05 < a_fill {
                threshold = threshold.max(upper.min(1.0));
            }
        }
    }
    (threshold.clamp(0.05, 1.0), paired_samples)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> ExecutionLearningRequest {
        ExecutionLearningRequest {
            context: "ctx".into(),
            baseline_order_type: "Limit".into(),
            order_types: vec!["Limit".into(), "Market".into()],
            profile_count: 2,
            risk_appetite: 0.5,
            allow_order_type_adaptation: true,
            explore_profile_order_types: Vec::new(),
            learning: BTreeMap::new(),
        }
    }

    fn stat(
        samples: usize,
        fills: usize,
        rate_total: f64,
        notional: f64,
        bps_total: f64,
    ) -> ExecutionLearningStat {
        ExecutionLearningStat {
            samples,
            filled_samples: fills,
            fill_rate_total: rate_total,
            cost_notional: notional,
            all_in_cost_notional_bps: bps_total,
            updated_at_ms: 0,
        }
    }

    #[test]
    fn execution_stat_decay_and_aggregation_preserve_recent_counts() {
        let decayed = decayed_execution_stat(stat(20, 10, 12.0, 100.0, 300.0), 0);
        assert_eq!(decayed.samples, 20);
        let mut req = request();
        req.learning
            .insert("ctx:P0:exec:Limit".into(), stat(4, 3, 2.0, 10.0, 30.0));
        req.learning
            .insert("ctx:P1:exec:Limit".into(), stat(6, 2, 4.0, 20.0, 40.0));
        req.learning.insert(
            "other:P0:exec:Limit".into(),
            stat(100, 100, 100.0, 100.0, 0.0),
        );
        let total = aggregate_execution_stats(&req, "ctx:P", ":exec:Limit");
        assert_eq!(total.samples, 10);
        assert_eq!(total.filled_samples, 5);
        assert_eq!(total.cost_notional, 30.0);
    }

    #[test]
    fn order_type_adaptation_requires_evidence_and_prefers_lower_cost() {
        let mut req = request();
        assert_eq!(learned_order_type(&req).as_deref(), Some("Limit"));
        req.allow_order_type_adaptation = false;
        assert_eq!(learned_order_type(&req).as_deref(), Some("Limit"));
        req.allow_order_type_adaptation = true;
        req.learning
            .insert("ctx:P0:exec:Limit".into(), stat(12, 8, 10.0, 100.0, 500.0));
        req.learning
            .insert("ctx:P0:exec:Market".into(), stat(12, 8, 10.0, 100.0, 100.0));
        assert_eq!(learned_order_type(&req).as_deref(), Some("Market"));
    }

    #[test]
    fn profile_selection_explores_under_sampled_profiles_then_minimizes_cost() {
        let mut req = request();
        req.learning
            .insert("ctx:P0:exec:Limit".into(), stat(12, 8, 10.0, 100.0, 300.0));
        assert_eq!(choose_execution_profile(&req, "Limit", true), 1);
        req.learning
            .insert("ctx:P1:exec:Limit".into(), stat(12, 8, 10.0, 100.0, 100.0));
        assert_eq!(choose_execution_profile(&req, "Limit", false), 1);
    }

    #[test]
    fn schedule_threshold_changes_only_when_paired_bucket_evidence_supports_it() {
        let mut req = request();
        assert_eq!(learned_schedule_threshold(&req), (0.25, 0));
        req.learning.insert(
            "ctx:P0:depth:05_10:exec:Vwap".into(),
            stat(10, 8, 8.0, 10.0, 10.0),
        );
        req.learning.insert(
            "ctx:P0:depth:05_10:exec:Limit".into(),
            stat(10, 8, 8.0, 10.0, 100.0),
        );
        let (threshold, samples) = learned_schedule_threshold(&req);
        assert_eq!(threshold, 0.05);
        assert_eq!(samples, 20);
    }
}
