fn signal_strategy_parameters(signal: &Signal) -> BTreeMap<String, f64> {
    signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("strategy_parameters"))
        .and_then(serde_json::Value::as_object)
        .into_iter().flatten()
        .filter_map(|(key, value)| Some((key.clone(), value.as_f64()?)))
        .collect()
}

fn inject_learned_parameters(settings: &Settings, signal: &mut Signal) {
    let Some(strategy_name) = signal.strategy.as_deref() else { return; };
    let Some(strategy) = Strategy::from_name(strategy_name) else { return; };
    let profile_symbol = signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("scout_parameter_profile_symbol"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&signal.symbol)
        .to_owned();
    let Some(profile) = settings.learned_profiles.get(&format!("{profile_symbol}:{strategy_name}")) else { return; };
    let mut parameters = profile.parameters.clone();
    for spec in strategy.parameter_specs() {
        let override_key = strategy_parameter_key(&profile_symbol, strategy, spec.key.as_str());
        let configured = settings.strategy_parameter_overrides.get(&override_key).copied()
            .or_else(|| legacy_strategy_parameter_name(spec.key.as_str()).and_then(|legacy| {
                settings.strategy_parameter_overrides.get(&strategy_parameter_key(&profile_symbol, strategy, legacy)).copied()
            }));
        if let Some(value) = configured.filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max) {
            parameters.insert(spec.key.as_str().to_owned(), value);
        }
    }
    if let Some(decision) = signal.strategy_decision.as_ref() {
        if let (Some(key), Some(value)) = (
            decision.get("scout_parameter_key").and_then(serde_json::Value::as_str),
            decision.get("scout_parameter_value").and_then(serde_json::Value::as_f64),
        ) {
            if key.starts_with("combo:") {
                if let Some(variant) = parse_scout_combination(key, strategy) {
                    parameters.extend(variant);
                }
            } else {
                parameters.insert(key.to_owned(), value);
            }
        }
    }
    let profile_id = strategy_core::profile_id(&profile_symbol, strategy_name, &parameters);
    let Some(decision) = signal.strategy_decision.as_mut().and_then(serde_json::Value::as_object_mut) else { return; };
    decision.insert("strategy_parameters".into(), serde_json::json!(parameters));
    decision.insert("learning_profile_id".into(), serde_json::json!(profile_id));
}

fn strategy_parameter_key(symbol: &str, strategy: Strategy, parameter: &str) -> String {
    format!("{symbol}:{}:{parameter}", strategy.as_str())
}

fn legacy_strategy_parameter_name(parameter: &str) -> Option<&'static str> {
    match parameter {
        "profit_protection_activation_percent" => Some("shadow_profit_activation_percent"),
        "profit_protection_giveback_percent" => Some("shadow_profit_giveback_percent"),
        "profit_protection_minimum_lock_percent" => Some("shadow_profit_minimum_lock_percent"),
        _ => None,
    }
}

fn strategy_learning_stats(settings: &Settings, symbol: &str, strategy: Strategy) -> LearningStats {
    let now_ms = current_time_ms();
    let mut stats = settings
        .learning
        .get(&strategy_trade_learning_key(symbol, strategy.as_str()))
        .cloned()
        .unwrap_or_default();
    if let Some(profile) = settings.learned_profiles.get(&format!("{symbol}:{}", strategy.as_str())) {
        stats.samples = stats.samples.saturating_add(profile.learning_samples);
        stats.positive = stats.positive.saturating_add(profile.positive_samples);
        stats.realized_pnl += profile.realized_pnl;
    }
    decay_learning_stats(&mut stats, now_ms);
    stats
}

// A parameter probe only becomes the active profile after a meaningful paired
// sample and positive improvement over its baseline beyond sampling noise.
const MIN_SCOUT_PARAMETER_VARIANT_SAMPLES: usize = 20;
const MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES: usize = 6;

fn minimum_scout_parameter_variant_samples(strategy: Strategy) -> usize {
    if strategy.is_live_insight() {
        MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES
    } else {
        MIN_SCOUT_PARAMETER_VARIANT_SAMPLES
    }
}

#[cfg(test)]
fn next_scout_parameter_probe(
    symbol: &str,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
    day: u64,
) -> Option<(String, f64)> {
    scouts::next_parameter_probe(symbol, strategy.as_str(), parameters, &scout_parameter_specs(strategy), day)
}

// Pair scouts measure interactions while keeping exploration bounded to two
// tunables at a time. The day selects a deterministic pair and direction so
// retries generate the same variant.
#[cfg(test)]
fn next_scout_parameter_combination(
    symbol: &str,
    strategy: Strategy,
    parameters: &BTreeMap<String, f64>,
    day: u64,
) -> Option<BTreeMap<String, f64>> {
    scouts::next_parameter_combination(symbol, strategy.as_str(), parameters, &scout_parameter_specs(strategy), day)
}

fn scout_context_hash(parameters: &BTreeMap<String, f64>, excluded: &BTreeMap<String, f64>) -> String {
    scouts::context_hash(parameters, excluded)
}

fn scout_combination_key(
    values: &BTreeMap<String, f64>, full_profile: &BTreeMap<String, f64>,
) -> Option<String> {
    scouts::combination_key(values, full_profile)
}

fn scout_combination_context(key: &str) -> Option<&str> {
    scouts::combination_context(key)
}

fn parse_scout_combination(key: &str, strategy: Strategy) -> Option<BTreeMap<String, f64>> {
    scouts::parse_combination(key, &scout_parameter_specs(strategy))
}

fn scout_parameter_specs(strategy: Strategy) -> Vec<strategy_core::ParameterSpec> {
    all_strategy_parameter_specs(strategy).into_iter().map(|spec| strategy_core::ParameterSpec {
        key: spec.key.as_str().to_owned(), label: spec.label.to_owned(), default: spec.default,
        min: spec.min, max: spec.max, step: spec.step, unit: spec.unit.to_owned(),
        learning_direction: spec.learning_direction,
    }).collect()
}

fn scout_parameter_learning_key(symbol: &str, strategy: Strategy, parameter: &str, value: f64) -> String {
    if parameter.starts_with("combo:") {
        format!("probe_combo:{symbol}:{}:{parameter}", strategy.as_str())
    } else {
        strategy_parameter_probe_learning_key(symbol, strategy.as_str(), parameter, value)
    }
}

fn best_scout_parameter_combination(
    settings: &Settings, symbol: &str, strategy: Strategy, baseline: &BTreeMap<String, f64>,
) -> Option<BTreeMap<String, f64>> {
    let learning_prefix = if strategy == Strategy::DatedCarry {
        "probe_combo_shadow"
    } else {
        "paired_probe_combo"
    };
    let prefix = format!("{learning_prefix}:{symbol}:{}:combo:", strategy.as_str());
    settings.learning.iter().filter_map(|(key, stored)| {
        let candidate = format!("combo:{}", key.strip_prefix(&prefix)?);
        let values = parse_scout_combination(&candidate, strategy)?;
        if scout_combination_context(&candidate)? != scout_context_hash(baseline, &values) { return None; }
        let mut stats = stored.clone();
        decay_learning_stats(&mut stats, current_time_ms());
        let minimum_samples = minimum_scout_parameter_variant_samples(strategy);
        if stats.samples < minimum_samples
            || stats.recent_trade_pnls.len() < minimum_samples { return None; }
        let returns = stats.recent_trade_pnls.iter().map(|(_, pnl)| *pnl).collect::<Vec<_>>();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns.iter().map(|pnl| (pnl - mean).powi(2)).sum::<f64>() / (returns.len()-1) as f64;
        let lower_95 = mean - 1.96 * (variance / returns.len() as f64).sqrt();
        (lower_95 > 0.0).then_some((values, mean))
    }).max_by(|left, right| left.1.total_cmp(&right.1)).map(|(values, _)| values)
}

fn best_scout_parameter_value(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
    spec: &StrategyParameterSpec,
) -> Option<f64> {
    let learning_prefix = if strategy == Strategy::DatedCarry {
        "probe_shadow"
    } else {
        "paired_probe"
    };
    let prefix = format!("{learning_prefix}:{symbol}:{}:{}:", strategy.as_str(), spec.key.as_str());
    settings.learning.iter().filter_map(|(key, stored)| {
        let value = key.strip_prefix(&prefix)?.parse::<f64>().ok()?;
        if !value.is_finite() || value < spec.min || value > spec.max {
            return None;
        }
        let mut stats = stored.clone();
        decay_learning_stats(&mut stats, current_time_ms());
        let minimum_samples = minimum_scout_parameter_variant_samples(strategy);
        if stats.samples < minimum_samples
            || stats.recent_trade_pnls.len() < minimum_samples
        {
            return None;
        }
        let returns = stats.recent_trade_pnls.iter().map(|(_, pnl)| *pnl).collect::<Vec<_>>();
        let mean = returns.iter().sum::<f64>() / returns.len() as f64;
        let variance = returns.iter().map(|pnl| (pnl - mean).powi(2)).sum::<f64>()
            / (returns.len() - 1) as f64;
        let lower_95 = mean - 1.96 * (variance / returns.len() as f64).sqrt();
        (lower_95 > 0.0).then_some((value, mean))
    }).max_by(|left, right| left.1.total_cmp(&right.1)).map(|(value, _)| value)
}

fn strategy_parameter_profile(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> StrategyParameterProfile {
    let mut learning = strategy_learning_stats(settings, symbol, strategy);
    let legacy_shadow_key = format!("shadow:{symbol}:{}", strategy.as_str());
    let scenario_shadow_prefix = format!("{legacy_shadow_key}:");
    for (key, stored_shadow) in &settings.learning {
        if key != &legacy_shadow_key && !key.starts_with(&scenario_shadow_prefix) {
            continue;
        }
        let mut shadow = stored_shadow.clone();
        decay_learning_stats(&mut shadow, current_time_ms());
        learning.samples += shadow.samples;
        learning.positive += shadow.positive;
        learning.realized_pnl += shadow.realized_pnl;
        learning.recent_trade_pnls.extend(shadow.recent_trade_pnls);
    }
    learning.recent_trade_pnls.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    learning.recent_trade_pnls.truncate(STRATEGY_SIZING_RECENT_TRADES);
    let positive_rate =
        (learning.samples > 0).then_some(learning.positive as f64 / learning.samples as f64);
    let performance = recent_net_pnl_edge(&learning)
        .unwrap_or_else(|| confident_win_rate_edge(learning.positive, learning.samples));
    let pooled_learning = strategy.is_live_insight()
        .then(|| strategy_learning_stats_across_symbols(settings, strategy));
    let pooled_threshold_samples = strategy.is_live_insight()
        .then(|| strategy_threshold_samples_across_symbols(settings, strategy))
        .unwrap_or_default();
    let learned_profile = settings
        .learned_profiles
        .get(&format!("{symbol}:{}", strategy.as_str()));
    let specs = all_strategy_parameter_specs(strategy);
    let learning_key = strategy_parameter_key(symbol, strategy, "learning_rate");
    let learning_rate = settings
        .strategy_parameter_overrides
        .get(&learning_key)
        .copied()
        .unwrap_or(0.05)
        .clamp(0.0, 0.25);
    let baseline_values = specs.iter().map(|spec| {
        let key = strategy_parameter_key(symbol, strategy, spec.key.as_str());
        let configured = settings.strategy_parameter_overrides.get(&key).copied().or_else(|| {
            legacy_strategy_parameter_name(spec.key.as_str()).and_then(|legacy| {
                settings.strategy_parameter_overrides.get(&strategy_parameter_key(symbol, strategy, legacy)).copied()
            })
        });
        let base = configured.unwrap_or(spec.default);
        let learned_value = if configured.is_none() {
            learned_profile
                .and_then(|profile| profile.parameters.get(spec.key.as_str()))
                .copied()
                .filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max)
        } else {
            None
        };
        let value = learned_value
            .or_else(|| best_scout_parameter_value(settings, symbol, strategy, spec))
            .unwrap_or_else(|| {
            if is_walk_forward_threshold(spec.key) {
                let history = if strategy.is_live_insight() {
                    &pooled_threshold_samples
                } else {
                    &learning.threshold_samples
                };
                if let Some(validated_threshold) = walk_forward_threshold(spec, base, history) {
                    return validated_threshold;
                }
                // Share same-strategy threshold evidence across symbols. When
                // mature pooled outcomes are negative and no candidate clears
                // the positive holdout test, tighten the learned entry
                // parameter to its most selective registered value. Scout
                // variants remain free to produce new evidence for promotion.
                let pooled_edge_is_negative = pooled_learning.as_ref().is_some_and(|outcomes| {
                    outcomes.samples >= MIN_THRESHOLD_TRAIN_SAMPLES
                        && recent_net_pnl_edge(outcomes).is_some_and(|edge| edge < 0.0)
                });
                if strategy.is_live_insight() && pooled_edge_is_negative {
                    return restrictive_threshold(spec);
                }
                walk_forward_threshold(spec, base, &learning.threshold_samples).unwrap_or(base)
            } else {
                let adaptive_step = if spec.key == StrategyParameterKey::LearningRate { 0.0 }
                    else { (spec.max - spec.min) * learning_rate * performance * spec.learning_direction };
                let candidate = (base + adaptive_step).clamp(spec.min, spec.max);
                (spec.min + ((candidate - spec.min) / spec.step).round() * spec.step).clamp(spec.min, spec.max)
            }
        });
        (spec.key.as_str().to_owned(), value)
    }).collect::<BTreeMap<_, _>>();
    let learned_combination = if learned_profile.is_some() {
        None
    } else {
        best_scout_parameter_combination(settings, symbol, strategy, &baseline_values)
    };
    let parameters = specs
        .iter()
        .map(|spec| {
            let key = strategy_parameter_key(symbol, strategy, spec.key.as_str());
            let configured = settings
                .strategy_parameter_overrides
                .get(&key)
                .copied()
                .or_else(|| {
                    legacy_strategy_parameter_name(spec.key.as_str()).and_then(|legacy| {
                        settings
                            .strategy_parameter_overrides
                            .get(&strategy_parameter_key(symbol, strategy, legacy))
                            .copied()
                    })
                });
            let default_value = spec.default;
            let base = configured.unwrap_or(default_value);
            let learned_value = learned_combination.as_ref()
                .and_then(|values| values.get(spec.key.as_str()).copied())
                .or_else(|| baseline_values.get(spec.key.as_str()).copied())
                .unwrap_or(base);
            StrategyParameterValue {
                key: spec.key.as_str().into(),
                label: spec.label.into(),
                value: learned_value,
                learned_value,
                configured_value: configured,
                default_value,
                min: spec.min,
                max: spec.max,
                step: spec.step,
                unit: spec.unit.into(),
                overridden: configured.is_some(),
            }
        })
        .collect();
    StrategyParameterProfile {
        symbol: symbol.into(),
        strategy: strategy.as_str().into(),
        learning_samples: learning.samples,
        positive_rate,
        realized_pnl: learning.realized_pnl,
        parameters,
    }
}

fn strategy_threshold_samples_across_symbols(
    settings: &Settings,
    strategy: Strategy,
) -> Vec<StrategyThresholdSample> {
    let suffix = format!(":{}", strategy.as_str());
    let mut samples = settings
        .learning
        .iter()
        .filter(|(key, _)| {
            key.starts_with("trade:") && key.ends_with(&suffix)
        })
        .flat_map(|(_, stats)| stats.threshold_samples.iter().cloned())
        .filter(|sample| {
            sample.feature_value.is_finite()
                && sample.threshold.is_finite()
                && sample.net_return_percent.is_finite()
        })
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.timestamp_ms);
    samples
}

fn restrictive_threshold(spec: &StrategyParameterSpec) -> f64 {
    if spec.key == StrategyParameterKey::RsiEntryMax {
        spec.min
    } else {
        spec.max
    }
}

fn threshold_observation(
    strategy: &str,
    decision: &serde_json::Value,
) -> Option<(StrategyParameterKey, f64, f64)> {
    let kind = Strategy::from_name(strategy)?;
    let key = match kind {
        Strategy::LiveMomentum => StrategyParameterKey::MinimumMomentum,
        Strategy::LiveTradeFlow => StrategyParameterKey::MinimumFlowImbalance,
        Strategy::LiveOrderBook => StrategyParameterKey::MinimumBookImbalance,
        Strategy::LiveVwapDislocation => StrategyParameterKey::MinimumVwapDislocationBps,
        Strategy::LiveAbsorptionPressure => StrategyParameterKey::MinimumAbsorptionPressure,
        Strategy::Rsi2OversoldRebound => StrategyParameterKey::RsiEntryMax,
        _ => return None,
    };
    let threshold = decision
        .get("strategy_parameters")?
        .get(key.as_str())?
        .as_f64()?;
    if !threshold.is_finite() {
        return None;
    }
    let feature = if key == StrategyParameterKey::RsiEntryMax {
        decision.get("primary_metric_value")?.as_f64()?
    } else {
        let side = match SignalDirection::from_wire(decision.get("direction")?.as_str()?)? {
            SignalDirection::Long => 1.0,
            SignalDirection::Short => -1.0,
        };
        let components = decision.get("observed_insight")?;
        let read = |name: &str| {
            components
                .get(name)?
                .as_f64()
                .filter(|value| value.is_finite())
        };
        match key {
            StrategyParameterKey::MinimumMomentum => {
                let momentum = read("momentum_10s_percent").map(|value| side * value);
                let trend = read("trend_5m_volatility_units").map(|value| side * value);
                momentum.into_iter().chain(trend).max_by(f64::total_cmp)?
            }
            StrategyParameterKey::MinimumFlowImbalance => side * read("trade_flow_imbalance")?,
            StrategyParameterKey::MinimumBookImbalance => side * read("book_imbalance")?,
            StrategyParameterKey::MinimumVwapDislocationBps => {
                side * read("mispricing_vs_vwap_bps")?
            }
            StrategyParameterKey::MinimumAbsorptionPressure => {
                -side * read("pressure_without_price_move")?
            }
            _ => return None,
        }
    };
    feature.is_finite().then_some((key, threshold, feature))
}

const MIN_THRESHOLD_TRAIN_SAMPLES: usize = 20;
const MIN_THRESHOLD_VALIDATION_SAMPLES: usize = 10;
const MAX_THRESHOLD_VALIDATION_SAMPLES: usize = 64;
const MIN_THRESHOLD_VALIDATION_GAIN_PERCENT: f64 = 0.05;

fn is_walk_forward_threshold(key: StrategyParameterKey) -> bool {
    matches!(
        key,
        StrategyParameterKey::MinimumMomentum
            | StrategyParameterKey::MinimumFlowImbalance
            | StrategyParameterKey::MinimumBookImbalance
            | StrategyParameterKey::MinimumVwapDislocationBps
            | StrategyParameterKey::MinimumAbsorptionPressure
            | StrategyParameterKey::RsiEntryMax
    )
}

fn threshold_accepts(key: StrategyParameterKey, feature: f64, threshold: f64) -> bool {
    if key == StrategyParameterKey::RsiEntryMax {
        feature <= threshold
    } else if key == StrategyParameterKey::MinimumMomentum {
        feature > threshold
    } else {
        feature >= threshold
    }
}

fn threshold_return_stats(
    samples: &[StrategyThresholdSample],
    key: StrategyParameterKey,
    threshold: f64,
) -> Option<(usize, f64, f64)> {
    let outcomes = samples
        .iter()
        .filter_map(|sample| {
            if sample.parameter_key != key {
                return None;
            }
            let supported = if key == StrategyParameterKey::RsiEntryMax {
                threshold <= sample.threshold
            } else {
                threshold >= sample.threshold
            };
            (supported && threshold_accepts(key, sample.feature_value, threshold))
                .then_some(sample.net_return_percent)
        })
        .collect::<Vec<_>>();
    if outcomes.is_empty() {
        return None;
    }
    let count = outcomes.len();
    let average = outcomes.iter().sum::<f64>() / count as f64;
    let variance = if count > 1 {
        outcomes
            .iter()
            .map(|value| (value - average).powi(2))
            .sum::<f64>()
            / (count - 1) as f64
    } else {
        return Some((count, average, f64::NEG_INFINITY));
    };
    let lower_95 = average - 1.96 * (variance / count as f64).sqrt();
    Some((count, average, lower_95))
}

fn walk_forward_threshold(
    spec: &StrategyParameterSpec,
    base: f64,
    history: &[StrategyThresholdSample],
) -> Option<f64> {
    if history.len() < MIN_THRESHOLD_TRAIN_SAMPLES + MIN_THRESHOLD_VALIDATION_SAMPLES {
        return None;
    }
    let mut samples = history
        .iter()
        .filter(|sample| {
            sample.parameter_key == spec.key
                && sample.threshold.is_finite()
                && sample.feature_value.is_finite()
                && sample.net_return_percent.is_finite()
        })
        .collect::<Vec<_>>();
    samples.sort_by_key(|sample| sample.timestamp_ms);
    if samples.len() < MIN_THRESHOLD_TRAIN_SAMPLES + MIN_THRESHOLD_VALIDATION_SAMPLES {
        return None;
    }
    if samples.len() > MIN_THRESHOLD_VALIDATION_SAMPLES + MAX_THRESHOLD_VALIDATION_SAMPLES {
        samples.drain(
            ..samples.len() - MIN_THRESHOLD_VALIDATION_SAMPLES - MAX_THRESHOLD_VALIDATION_SAMPLES,
        );
    }
    let split = samples.len() * 2 / 3;
    let split_timestamp = samples[split].timestamp_ms;
    let split = samples.partition_point(|sample| sample.timestamp_ms < split_timestamp);
    let train = samples[..split]
        .iter()
        .map(|sample| (*sample).clone())
        .collect::<Vec<_>>();
    let validation = samples[split..]
        .iter()
        .map(|sample| (*sample).clone())
        .collect::<Vec<_>>();
    if train.len() < MIN_THRESHOLD_TRAIN_SAMPLES
        || validation.len() < MIN_THRESHOLD_VALIDATION_SAMPLES
    {
        return None;
    }
    // Keep holdout observations out of the candidate search boundary too.
    let supported_boundary = if spec.key == StrategyParameterKey::RsiEntryMax {
        train
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::min)
    } else {
        train
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::max)
    };
    let mut best: Option<(f64, f64)> = None;
    let steps = ((spec.max - spec.min) / spec.step).round() as usize;
    for index in 0..=steps {
        let threshold = (spec.min + index as f64 * spec.step).clamp(spec.min, spec.max);
        let tighter_than_recorded = if spec.key == StrategyParameterKey::RsiEntryMax {
            threshold <= supported_boundary
        } else {
            threshold >= supported_boundary
        };
        if !tighter_than_recorded {
            continue;
        }
        let Some((count, _average, lower_95)) = threshold_return_stats(&train, spec.key, threshold)
        else {
            continue;
        };
        if count >= MIN_THRESHOLD_TRAIN_SAMPLES
            && best.is_none_or(|(_, best_lower_95)| lower_95 > best_lower_95)
        {
            best = Some((threshold, lower_95));
        }
    }
    let (threshold, _) = best?;
    let (validation_count, validation_average, validation_lower_95) =
        threshold_return_stats(&validation, spec.key, threshold)?;
    let (baseline_count, _, baseline_lower_95) =
        threshold_return_stats(&validation, spec.key, base)?;
    (validation_count >= MIN_THRESHOLD_VALIDATION_SAMPLES
        && baseline_count >= MIN_THRESHOLD_VALIDATION_SAMPLES
        && validation_average > 0.0
        && validation_lower_95 >= baseline_lower_95 + MIN_THRESHOLD_VALIDATION_GAIN_PERCENT)
        .then_some(threshold)
}

fn strategy_parameter_values(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> BTreeMap<String, f64> {
    strategy_parameter_profile(settings, symbol, strategy)
        .parameters
        .into_iter()
        .map(|parameter| (parameter.key, parameter.value))
        .collect()
}

fn queue_strategy_profile_registration(
    settings: &mut Settings,
    symbol: &str,
    strategy: Strategy,
    profile: &StrategyParameterProfile,
) {
    let parameters = profile.parameters.iter()
        .map(|parameter| (parameter.key.clone(), parameter.value))
        .collect::<BTreeMap<_, _>>();
    let profile_id = strategy_core::profile_id(symbol, strategy.as_str(), &parameters);
    if settings.registered_profile_ids.contains(&profile_id) { return; }
    if settings.profile_registration_outbox.iter().any(|registration| {
        registration.profile.profile_id == profile_id
            && registration.profile.symbol == symbol
            && registration.profile.strategy == strategy.as_str()
    }) { return; }
    let version = settings.learned_profiles
        .get(&format!("{symbol}:{}", strategy.as_str()))
        .map_or(0, |learned| learned.version);
    let specs = strategy.parameter_specs().into_iter().map(|spec| strategy_core::ParameterSpec {
        key: spec.key.as_str().to_owned(), label: spec.label.to_owned(), default: spec.default,
        min: spec.min, max: spec.max, step: spec.step, unit: spec.unit.to_owned(),
        learning_direction: spec.learning_direction,
    }).filter(|spec| parameters.contains_key(&spec.key)).collect();
    settings.profile_registration_outbox.push(strategy_core::StrategyProfileRegistration {
        profile: strategy_core::StrategyProfile {
            profile_id, version, symbol: symbol.to_owned(), strategy: strategy.as_str().to_owned(),
            parameters, learning_samples: profile.learning_samples,
            positive_samples: (profile.learning_samples as f64 * profile.positive_rate.unwrap_or(0.0)).round() as usize,
            realized_pnl: profile.realized_pnl,
        },
        specs,
    });
}

fn validate_strategy_parameter_overrides(values: &BTreeMap<String, f64>) -> Result<(), String> {
    for (key, value) in values {
        if !value.is_finite() {
            return Err(format!("Strategy parameter {key} must be finite"));
        }
        let mut parts = key.splitn(3, ':');
        let Some(symbol) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(strategy_name) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(parameter_name) = parts.next() else {
            return Err(format!("Invalid strategy parameter key {key}"));
        };
        let Some(strategy) = Strategy::from_name(strategy_name) else {
            return Err(format!("Unknown strategy in parameter key {key}"));
        };
        let specs = all_strategy_parameter_specs(strategy);
        let Some(spec) = specs.iter().find(|spec| {
            spec.key.as_str() == parameter_name
                || legacy_strategy_parameter_name(spec.key.as_str()) == Some(parameter_name)
        }) else {
            return Err(format!("Unknown strategy parameter {key}"));
        };
        let step_position = (*value - spec.min) / spec.step;
        if symbol.is_empty()
            || *value < spec.min
            || *value > spec.max
            || (step_position - step_position.round()).abs() > 1e-6
        {
            return Err(format!(
                "Strategy parameter {key} must be between {} and {} in steps of {}",
                spec.min, spec.max, spec.step
            ));
        }
    }
    Ok(())
}

fn confident_win_rate_edge(positive: usize, samples: usize) -> f64 {
    if samples == 0 || positive > samples {
        return 0.0;
    }
    // Use a light Beta(1, 1) prior and a short confidence ramp so the recent
    // sizing window can move exposure promptly without letting one trade
    // dominate the estimate.
    let n = samples as f64;
    let posterior_rate = (positive as f64 + 1.0) / (n + 2.0);
    let confidence = n / (n + 2.0);
    ((posterior_rate - 0.5) * 2.0 * confidence).clamp(-1.0, 1.0)
}

const WALK_FORWARD_POLICY_MIN_SAMPLES: usize = 30;

pub(crate) fn strategy_parameter_value(
    parameters: &BTreeMap<String, f64>,
    key: StrategyParameterKey,
) -> Option<f64> {
    parameters
        .get(key.as_str())
        .copied()
        .filter(|value| value.is_finite())
}

fn strategy_profile_value(profile: &StrategyParameterProfile, key: &str) -> Option<f64> {
    profile
        .parameters
        .iter()
        .find(|parameter| parameter.key == key)
        .map(|parameter| parameter.value)
}

#[cfg(test)]
mod strategy_parameter_bounds_tests {
    use super::*;

    #[test]
    fn hot_path_overlays_the_latest_learning_profile_on_signal_parameters() {
        let mut settings = Settings::default();
        let parameters = BTreeMap::from([("minimum_momentum".to_owned(), 0.003)]);
        let profile_id = strategy_core::profile_id("BTC-USDT", "live-momentum", &parameters);
        settings.learned_profiles.insert("BTC-USDT:live-momentum".into(), strategy_core::StrategyProfile {
            profile_id: profile_id.clone(), version: 5, symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(), parameters,
            learning_samples: 8, positive_samples: 6, realized_pnl: 4.0,
        });
        let mut signal = Signal {
            symbol: "BTC-USDT".into(), direction: "bullish".into(), strength: 1.0,
            timestamp: 1, stale: false, score: 1.0, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: true,
            strategy: Some("live-momentum".into()),
            strategy_decision: Some(serde_json::json!({ "strategy_parameters": { "minimum_momentum": 0.0 } })),
            market_regime: None, news_attribution: None, entry_gate: None,
        };
        inject_learned_parameters(&settings, &mut signal);
        assert_eq!(signal_strategy_parameters(&signal)["minimum_momentum"], 0.003);
        assert_eq!(signal.strategy_decision.as_ref().unwrap()["learning_profile_id"], profile_id);
        settings.strategy_parameter_overrides.insert(
            "BTC-USDT:live-momentum:minimum_momentum".into(), 0.004,
        );
        inject_learned_parameters(&settings, &mut signal);
        assert_eq!(signal_strategy_parameters(&signal)["minimum_momentum"], 0.004);
    }

    #[test]
    fn profile_registration_outbox_deduplicates_exact_variants() {
        let mut settings = Settings::default();
        let strategy = Strategy::LiveMomentum;
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        queue_strategy_profile_registration(&mut settings, "BTC-USDT", strategy, &profile);
        queue_strategy_profile_registration(&mut settings, "BTC-USDT", strategy, &profile);
        assert_eq!(settings.profile_registration_outbox.len(), 1);
        let registration = &settings.profile_registration_outbox[0];
        assert_eq!(registration.profile.profile_id, strategy_core::profile_id(
            "BTC-USDT", strategy.as_str(), &registration.profile.parameters,
        ));
        assert!(!registration.specs.is_empty());
    }

    #[test]
    fn daily_rule_inputs_are_registered_for_learning_profiles() {
        let cases = [
            (Strategy::Donchian10Btc200, StrategyParameterKey::BreakoutLookbackDays),
            (Strategy::Donchian20Btc200, StrategyParameterKey::BreakoutLookbackDays),
            (Strategy::Donchian20Btc200Hold3, StrategyParameterKey::MinimumHoldDays),
            (Strategy::RiskAdjustedTrendPortfolio, StrategyParameterKey::TrendPortfolioLookbackDays),
            (Strategy::RiskAdjustedTrendPortfolio, StrategyParameterKey::TrendPortfolioWeightCap),
            (Strategy::RiskAdjustedMonthlyMomentum, StrategyParameterKey::MonthlyMomentumLookbackDays),
            (Strategy::RiskAdjustedMonthlyMomentum, StrategyParameterKey::MonthlyMomentumVolatilityTarget),
            (Strategy::DatedCarry, StrategyParameterKey::CarryMinimumDaysToExpiry),
        ];
        for (strategy, key) in cases {
            assert!(all_strategy_parameter_specs(strategy).iter().any(|spec| spec.key == key),
                "{} must expose {}", strategy.as_str(), key.as_str());
        }
    }

    #[test]
    fn every_registered_strategy_accepts_bounds_and_rejects_values_outside_them() {
        let mut strategies_checked = 0;

        for strategy in Strategy::all() {
            strategies_checked += 1;
            let specs = all_strategy_parameter_specs(strategy);
            assert!(
                !specs.is_empty(),
                "{} has no parameter specs",
                strategy.as_str()
            );

            for spec in specs {
                let key = format!("BTC-USDT:{}:{}", strategy.as_str(), spec.key.as_str());
                assert!(spec.min.is_finite() && spec.max.is_finite());
                assert!(spec.step.is_finite() && spec.step > 0.0);
                assert!(
                    spec.min <= spec.default && spec.default <= spec.max,
                    "{}:{} default is outside its bounds",
                    strategy.as_str(),
                    spec.key.as_str()
                );
                assert!(
                    spec.min <= spec.max,
                    "{}:{} has inverted bounds",
                    strategy.as_str(),
                    spec.key.as_str()
                );

                let mut override_value = BTreeMap::from([(key.clone(), spec.min)]);
                assert!(
                    validate_strategy_parameter_overrides(&override_value).is_ok(),
                    "{}:{} rejected its minimum",
                    strategy.as_str(),
                    spec.key.as_str()
                );

                override_value.insert(key.clone(), spec.max);
                assert!(
                    validate_strategy_parameter_overrides(&override_value).is_ok(),
                    "{}:{} rejected its maximum",
                    strategy.as_str(),
                    spec.key.as_str()
                );

                override_value.insert(key.clone(), spec.min - spec.step);
                assert!(
                    validate_strategy_parameter_overrides(&override_value).is_err(),
                    "{}:{} accepted a value below its minimum",
                    strategy.as_str(),
                    spec.key.as_str()
                );

                override_value.insert(key, spec.max + spec.step);
                assert!(
                    validate_strategy_parameter_overrides(&override_value).is_err(),
                    "{}:{} accepted a value above its maximum",
                    strategy.as_str(),
                    spec.key.as_str()
                );
            }
        }

        assert_eq!(strategies_checked, Strategy::all().count());
    }
}

#[cfg(test)]
mod threshold_walk_forward_tests {
    use super::*;

    fn flow_sample(
        timestamp: u64,
        feature: f64,
        pnl: f64,
        threshold: f64,
    ) -> StrategyThresholdSample {
        StrategyThresholdSample {
            timestamp_ms: timestamp,
            parameter_key: StrategyParameterKey::MinimumFlowImbalance,
            threshold,
            feature_value: feature,
            net_return_percent: pnl,
        }
    }

    #[test]
    fn extracts_the_signal_feature_for_each_learned_threshold() {
        let cases = [
            (
                "live-momentum",
                serde_json::json!({
                    "direction":"bearish", "observed_insight":{"momentum_10s_percent":0.001,"trend_5m_volatility_units":0.02},
                    "strategy_parameters":{"minimum_momentum":0.0}
                }),
                StrategyParameterKey::MinimumMomentum,
                -0.001,
            ),
            (
                "live-trade-flow",
                serde_json::json!({
                    "direction":"bullish", "observed_insight":{"trade_flow_imbalance":0.2},
                    "strategy_parameters":{"minimum_flow_imbalance":0.15}
                }),
                StrategyParameterKey::MinimumFlowImbalance,
                0.2,
            ),
            (
                "live-order-book",
                serde_json::json!({
                    "direction":"bearish", "observed_insight":{"book_imbalance":-0.3},
                    "strategy_parameters":{"minimum_book_imbalance":0.15}
                }),
                StrategyParameterKey::MinimumBookImbalance,
                0.3,
            ),
            (
                "live-vwap-dislocation",
                serde_json::json!({
                    "direction":"bullish", "observed_insight":{"mispricing_vs_vwap_bps":2.0},
                    "strategy_parameters":{"minimum_vwap_dislocation_bps":1.0}
                }),
                StrategyParameterKey::MinimumVwapDislocationBps,
                2.0,
            ),
            (
                "live-absorption-pressure",
                serde_json::json!({
                    "direction":"bearish", "observed_insight":{"pressure_without_price_move":0.4},
                    "strategy_parameters":{"minimum_absorption_pressure":0.2}
                }),
                StrategyParameterKey::MinimumAbsorptionPressure,
                0.4,
            ),
        ];
        for (strategy, decision, key, feature) in cases {
            assert_eq!(
                threshold_observation(strategy, &decision),
                Some((
                    key,
                    decision["strategy_parameters"][key.as_str()]
                        .as_f64()
                        .unwrap(),
                    feature
                ))
            );
        }
    }

    #[test]
    fn tightens_a_threshold_only_when_later_holdout_edge_improves() {
        let spec = all_strategy_parameter_specs(Strategy::LiveTradeFlow)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MinimumFlowImbalance)
            .unwrap();
        let mut samples = Vec::new();
        for index in 0..70_u64 {
            let (feature, pnl) = if index % 2 == 0 {
                (0.025, -0.2)
            } else {
                (0.075, 1.0)
            };
            samples.push(flow_sample(index, feature, pnl, 0.02));
        }

        let tuned = walk_forward_threshold(&spec, 0.02, &samples);
        assert_eq!(tuned, Some(0.03));
    }

    #[test]
    fn leaves_threshold_unchanged_without_enough_holdout_samples() {
        let spec = all_strategy_parameter_specs(Strategy::LiveMomentum)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MinimumMomentum)
            .unwrap();
        let samples = (0..MIN_THRESHOLD_TRAIN_SAMPLES + MIN_THRESHOLD_VALIDATION_SAMPLES - 1)
            .map(|index| StrategyThresholdSample {
                timestamp_ms: index as u64,
                parameter_key: StrategyParameterKey::MinimumMomentum,
                threshold: 0.0,
                feature_value: 0.01,
                net_return_percent: 0.5,
            })
            .collect::<Vec<_>>();

        assert_eq!(walk_forward_threshold(&spec, 0.0, &samples), None);
    }

    #[test]
    fn rsi_entry_ceiling_uses_daily_rsi_feature_without_directional_wire_data() {
        let spec = all_strategy_parameter_specs(Strategy::Rsi2OversoldRebound)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::RsiEntryMax)
            .unwrap();
        let decision = serde_json::json!({
            "primary_metric_value": 7.0,
            "strategy_parameters": { "rsi_entry_max": 10.0 }
        });
        assert_eq!(
            threshold_observation("rsi2-oversold-rebound", &decision),
            Some((StrategyParameterKey::RsiEntryMax, 10.0, 7.0))
        );
        assert!(walk_forward_threshold(&spec, 10.0, &[]).is_none());
    }

    #[test]
    fn profitable_scout_outcomes_relax_scout_walk_forward_thresholds_within_bounds() {
        let strategy = Strategy::LiveTradeFlow;
        let mut settings = Settings::default();
        settings.strategy_parameter_overrides.insert(
            strategy_parameter_key("BTC-USDT", strategy, "learning_rate"),
            0.25,
        );
        let key = strategy_trade_learning_key("BTC-USDT", strategy.as_str());
        let stats = settings.learning.entry(key).or_default();
        for _ in 0..30 {
            record_learning_outcome(stats, 1.0);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let samples = profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::ScoutMinimumWalkForwardSamples.as_str())
            .unwrap();
        let positive_rate = profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::ScoutMinimumPositiveNetRate.as_str())
            .unwrap();
        assert!(samples.learned_value < samples.default_value);
        assert!(positive_rate.learned_value < positive_rate.default_value);
        assert!((samples.min..=samples.max).contains(&samples.learned_value));
        assert!((positive_rate.min..=positive_rate.max).contains(&positive_rate.learned_value));
    }

    #[test]
    fn negative_pooled_live_results_tighten_parameter_when_no_threshold_validates() {
        let strategy = Strategy::LiveTradeFlow;
        let spec = all_strategy_parameter_specs(strategy)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MinimumFlowImbalance)
            .unwrap();
        let mut settings = Settings::default();
        for symbol in ["BTC-USDT", "ETH-USDT"] {
            let stats = settings
                .learning
                .entry(strategy_trade_learning_key(symbol, strategy.as_str()))
                .or_default();
            for index in 0..24 {
                record_learning_outcome(stats, -1.0);
                stats.threshold_samples.push(StrategyThresholdSample {
                    timestamp_ms: index,
                    parameter_key: spec.key,
                    threshold: spec.default,
                    feature_value: 0.20,
                    net_return_percent: -0.25,
                });
            }
        }

        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let threshold = profile
            .parameters
            .iter()
            .find(|parameter| parameter.key == spec.key.as_str())
            .unwrap();
        assert_eq!(threshold.learned_value, spec.max);
        assert!(threshold.learned_value > threshold.default_value);
    }

    #[test]
    fn parameter_profile_uses_validated_walk_forward_threshold() {
        let strategy = Strategy::LiveTradeFlow;
        let spec = all_strategy_parameter_specs(strategy)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::MinimumFlowImbalance)
            .unwrap();
        let mut settings = Settings::default();
        let key = strategy_trade_learning_key("BTC-USDT", strategy.as_str());
        let stats = settings.learning.entry(key).or_default();
        for index in 0..70_u64 {
            let (feature_value, net_return_percent) = if index % 2 == 0 {
                (0.16, -0.2)
            } else {
                (0.25, 1.0)
            };
            stats.threshold_samples.push(StrategyThresholdSample {
                timestamp_ms: index,
                parameter_key: spec.key,
                threshold: 0.15,
                feature_value,
                net_return_percent,
            });
        }

        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let threshold = profile
            .parameters
            .iter()
            .find(|parameter| parameter.key == StrategyParameterKey::MinimumFlowImbalance.as_str())
            .unwrap();
        assert!((threshold.learned_value - 0.17).abs() < 1e-9);
        assert!(
            threshold.min <= threshold.learned_value && threshold.learned_value <= threshold.max
        );
    }
}

#[cfg(test)]
mod scout_shadow_parameter_learning_tests {
    use super::*;

    #[test]
    fn scout_outcomes_contribute_to_strategy_parameter_learning() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian55Btc200;
        let key = strategy_shadow_learning_key("BTC-USDT", strategy.as_str(), "strategy");
        let shadow = settings.learning.entry(key).or_default();
        shadow.samples = 20;
        shadow.positive = 15;
        shadow.realized_pnl = 1.5;
        shadow.recent_trade_pnls = (0..20)
            .map(|index| (current_time_ms(), if index < 15 { 0.1 } else { -0.05 }))
            .collect();
        let tight_key = strategy_shadow_learning_key("BTC-USDT", strategy.as_str(), "tight");
        let tight = settings.learning.entry(tight_key).or_default();
        tight.samples = 10;
        tight.positive = 8;
        tight.realized_pnl = 0.8;
        tight.recent_trade_pnls = (0..10)
            .map(|index| (current_time_ms(), if index < 8 { 0.1 } else { -0.1 }))
            .collect();

        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        assert_eq!(profile.learning_samples, 30);
        let lookback = profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutLookbackDays.as_str())
            .unwrap();
        assert_ne!(lookback.learned_value, lookback.default_value);
        assert!(lookback.min <= lookback.learned_value && lookback.learned_value <= lookback.max);
    }
}

#[cfg(test)]
mod scout_parameter_variant_tests {
    use super::*;

    #[test]
    fn probe_does_not_replace_the_profile_before_minimum_evidence() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian55Btc200;
        for _ in 0..19 {
            record_learning_outcome(
                settings.learning.entry(format!("paired_probe:BTC-USDT:{}:breakout_lookback_days:52.00000000", strategy.as_str())).or_default(),
                2.0,
            );
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let lookback = profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutLookbackDays.as_str())
            .unwrap();
        assert_eq!(lookback.learned_value, lookback.default_value);
    }

    #[test]
    fn breakout_entry_distance_promotes_per_symbol_after_confident_probe_outcomes() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian10Btc200;
        for _ in 0..19 {
            record_learning_outcome(
                settings.learning.entry(format!("paired_probe:BTC-USDT:{}:breakout_entry_distance_percent:10.00000000", strategy.as_str())).or_default(),
                2.0,
            );
        }
        let btc_before_threshold = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let btc_before = btc_before_threshold.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutEntryDistancePercent.as_str())
            .unwrap();
        assert_eq!(btc_before.learned_value, 0.0);

        record_learning_outcome(
            settings.learning.entry(format!("paired_probe:BTC-USDT:{}:breakout_entry_distance_percent:10.00000000", strategy.as_str())).or_default(),
            2.0,
        );
        let btc_after = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let btc_distance = btc_after.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutEntryDistancePercent.as_str())
            .unwrap();
        assert_eq!(btc_distance.learned_value, 10.0);

        let eth_profile = strategy_parameter_profile(&settings, "ETH-USDT", strategy);
        let eth_distance = eth_profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutEntryDistancePercent.as_str())
            .unwrap();
        assert_eq!(eth_distance.learned_value, 0.0);
    }

    #[test]
    fn feedback_selects_the_best_probed_value_without_pooling_it_with_baseline_samples() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian55Btc200;
        for _ in 0..24 {
            record_learning_outcome(
                settings.learning.entry(format!("paired_probe:BTC-USDT:{}:breakout_lookback_days:50.00000000", strategy.as_str())).or_default(),
                -1.0,
            );
            record_learning_outcome(
                settings.learning.entry(format!("paired_probe:BTC-USDT:{}:breakout_lookback_days:52.00000000", strategy.as_str())).or_default(),
                2.0,
            );
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let lookback = profile.parameters.iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutLookbackDays.as_str())
            .unwrap();
        assert_eq!(lookback.learned_value, 52.0);
        assert_eq!(profile.learning_samples, 0);
    }
}

#[cfg(test)]
mod dated_carry_shadow_probe_learning_tests {
    use super::*;

    #[test]
    fn promotion_uses_one_day_shadow_markouts_for_dated_carry_variants() {
        let mut settings = Settings::default();
        let strategy = Strategy::DatedCarry;
        let shadow_key = strategy_parameter_shadow_probe_learning_key(
            "BTC-USDT", strategy.as_str(), "carry_minimum_days_to_expiry", 6.0,
        );
        for _ in 0..20 {
            record_learning_outcome(settings.learning.entry(shadow_key.clone()).or_default(), 0.02);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let minimum_days = profile.parameters.iter()
            .find(|parameter| parameter.key == "carry_minimum_days_to_expiry")
            .unwrap();
        assert_eq!(minimum_days.learned_value, 6.0);

        let settlement_key = strategy_parameter_probe_learning_key(
            "BTC-USDT", strategy.as_str(), "carry_minimum_days_to_expiry", 5.0,
        );
        for _ in 0..25 {
            record_learning_outcome(settings.learning.entry(settlement_key.clone()).or_default(), 1.0);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let minimum_days = profile.parameters.iter()
            .find(|parameter| parameter.key == "carry_minimum_days_to_expiry")
            .unwrap();
        assert_eq!(minimum_days.learned_value, 6.0);
    }

    #[test]
    fn dated_carry_shadow_markouts_promote_the_tested_pair_in_its_profile_context() {
        let mut settings = Settings::default();
        let strategy = Strategy::DatedCarry;
        let baseline = all_strategy_parameter_specs(strategy).into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default)).collect::<BTreeMap<_, _>>();
        let combination = next_scout_parameter_combination("BTC-USDT", strategy, &baseline, 3).unwrap();
        let mut probe_profile = baseline.clone();
        for (key, value) in &combination { probe_profile.insert(key.clone(), *value); }
        let combination_key = scout_combination_key(&combination, &probe_profile).unwrap();
        let learning_key = strategy_parameter_shadow_combination_learning_key(
            "BTC-USDT", strategy.as_str(), &combination_key,
        );
        for _ in 0..MIN_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(settings.learning.entry(learning_key.clone()).or_default(), 0.02);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        for (key, value) in combination {
            let learned = profile.parameters.iter().find(|parameter| parameter.key == key).unwrap();
            assert_eq!(learned.learned_value, value);
        }
    }
}

#[cfg(test)]
mod scout_parameter_scheduling_tests {
    use super::*;

    #[test]
    fn post_only_timeout_is_bounded_and_scoutable() {
        let strategy = Strategy::LiveMomentum;
        let spec = all_strategy_parameter_specs(strategy)
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::PostOnlyTimeoutMinutes)
            .unwrap();
        assert_eq!(spec.default, 5.0);
        assert_eq!((spec.min, spec.max, spec.step), (1.0, 15.0, 1.0));

        let baseline = all_strategy_parameter_specs(strategy)
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let probe = (0..100).find_map(|day| {
            next_scout_parameter_probe("BTC-USDT", strategy, &baseline, day)
                .filter(|(key, _)| key == "post_only_timeout_minutes")
        });
        assert!(probe.is_some(), "scouts should vary post-only timeout");
        assert_ne!(probe.unwrap().1, 5.0);
    }

    #[test]
    fn live_strategy_probe_rotates_one_bounded_parameter_value() {
        let strategy = Strategy::LiveMomentum;
        let parameters = strategy_parameter_values(&Settings::default(), "ETH-USDT", strategy);
        let (key, value) = next_scout_parameter_probe("ETH-USDT", strategy, &parameters, 2).unwrap();
        assert_ne!(key, StrategyParameterKey::LearningRate.as_str());
        let spec = all_strategy_parameter_specs(strategy).into_iter()
            .find(|spec| spec.key.as_str() == key).unwrap();
        assert!(spec.min <= value && value <= spec.max);
        let current = parameters[&key];
        assert!((value - current).abs() <= spec.step + 1e-9);
        assert!((value - current).abs() > 0.0);
    }
}

#[cfg(test)]
mod scout_parameter_combination_tests {
    use super::*;

    #[test]
    fn pair_probes_change_two_bounded_parameters_and_have_stable_keys() {
        let strategy = Strategy::Donchian20Sma200;
        let parameters = all_strategy_parameter_specs(strategy).into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default)).collect::<BTreeMap<_, _>>();
        let combination = next_scout_parameter_combination("BTC-USDT", strategy, &parameters, 8).unwrap();
        assert_eq!(combination.len(), 2);
        let key = scout_combination_key(&combination, &parameters).unwrap();
        let parsed = parse_scout_combination(&key, strategy).unwrap();
        assert_eq!(parsed.keys().collect::<Vec<_>>(), combination.keys().collect::<Vec<_>>());
        for (name, value) in &combination { assert!((parsed[name] - value).abs() < 1e-7); }
        let repeated = next_scout_parameter_combination("BTC-USDT", strategy, &parameters, 8).unwrap();
        assert_eq!(repeated.keys().collect::<Vec<_>>(), combination.keys().collect::<Vec<_>>());
        for (name, value) in &combination { assert!((repeated[name] - value).abs() < 1e-12); }
    }

    #[test]
    fn validated_pair_outcome_promotes_both_values_together() {
        let strategy = Strategy::Donchian20Sma200;
        let mut settings = Settings::default();
        let values = BTreeMap::from([
            ("breakout_lookback_days".to_owned(), 22.0),
            ("breakout_entry_distance_percent".to_owned(), 0.5),
        ]);
        let combination_key = scout_combination_key(&values, &values).unwrap();
        let learning_key = format!("paired_probe_combo:BTC-USDT:{}:{combination_key}", strategy.as_str());
        for _ in 0..MIN_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(settings.learning.entry(learning_key.clone()).or_default(), 1.0);
        }
        assert_eq!(best_scout_parameter_combination(&settings, "BTC-USDT", strategy, &values), Some(values));
    }
}

#[cfg(test)]
mod paired_scout_parameter_tests {
    use super::*;

    #[test]
    fn live_parameter_promotion_requires_positive_paired_improvement() {
        let strategy = Strategy::LiveTradeFlow;
        let spec = all_strategy_parameter_specs(strategy).into_iter()
            .find(|spec| spec.min < spec.max && spec.step > 0.0)
            .expect("live strategy has a tunable parameter");
        let candidate = if spec.default + spec.step <= spec.max {
            spec.default + spec.step
        } else {
            spec.default - spec.step
        };
        let paired_key = format!(
            "paired_probe:BTC-USDT:{}:{}:{candidate:.8}",
            strategy.as_str(), spec.key.as_str(),
        );
        let unpaired_key = format!(
            "probe:BTC-USDT:{}:{}:{candidate:.8}",
            strategy.as_str(), spec.key.as_str(),
        );
        let mut settings = Settings::default();
        for _ in 0..MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(settings.learning.entry(unpaired_key.clone()).or_default(), 5.0);
        }
        assert_eq!(best_scout_parameter_value(&settings, "BTC-USDT", strategy, &spec), None);

        for _ in 0..MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(settings.learning.entry(paired_key.clone()).or_default(), 0.02);
        }
        assert_eq!(best_scout_parameter_value(&settings, "BTC-USDT", strategy, &spec), Some(candidate));
    }
}
