fn inject_learned_parameters(settings: &Settings, signal: &mut Signal) {
    let Some(strategy_name) = signal.strategy.as_deref() else { return; };
    let Some(strategy) = Strategy::from_name(strategy_name) else { return; };
    let profile_symbol = signal.strategy_decision.as_ref()
        .and_then(|decision| decision.get("scout_parameter_profile_symbol"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&signal.symbol)
        .to_owned();
    let Some(profile) = settings.learned_profiles.get(&format!("{profile_symbol}:{strategy_name}")) else { return; };
    let probe = signal.strategy_decision.as_ref().and_then(|decision| {
        Some((
            decision.get("scout_parameter_key")?.as_str()?,
            decision.get("scout_parameter_value")?.as_f64()?,
        ))
    });
    let parameters = effective_parameter_values(
        strategy,
        &profile_symbol,
        &profile.parameters,
        &settings.strategy_parameter_overrides,
        probe,
    );
    let profile_id = strategy::profile_id(&profile_symbol, strategy_name, &parameters);
    let Some(decision) = signal.strategy_decision.as_mut().and_then(serde_json::Value::as_object_mut) else { return; };
    decision.insert("strategy_parameters".into(), serde_json::json!(parameters));
    decision.insert("learning_profile_id".into(), serde_json::json!(profile_id));
    decision.insert("learning_profile_symbol".into(), serde_json::json!(profile_symbol));
    decision.insert("learning_profile_version".into(), serde_json::json!(profile.version));
}

#[cfg(test)]
fn strategy_learning_stats(settings: &Settings, symbol: &str, strategy: Strategy) -> LearningStats {
    settings.learned_profiles.get(&format!("{symbol}:{}", strategy.as_str()))
        .map(|profile| LearningStats {
            samples: profile.learning_samples,
            positive: profile.positive_samples,
            realized_pnl: profile.realized_pnl,
            ..LearningStats::default()
        })
        .unwrap_or_default()
}

// A parameter probe only becomes the active profile after a meaningful paired
// sample and positive improvement over its baseline beyond sampling noise.
#[cfg(test)]
const MIN_SCOUT_PARAMETER_VARIANT_SAMPLES: usize = 20;
#[cfg(test)]
const MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES: usize = 6;

#[cfg(test)]
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
    strategy::next_parameter_probe(symbol, strategy.as_str(), parameters, &scout_parameter_specs(strategy), day)
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
    strategy::next_parameter_combination(symbol, strategy.as_str(), parameters, &scout_parameter_specs(strategy), day)
}

#[cfg(test)]
fn scout_context_hash(parameters: &BTreeMap<String, f64>, excluded: &BTreeMap<String, f64>) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for (key, value) in parameters.iter().filter(|(key, _)| !excluded.contains_key(*key)) {
        for byte in format!("{key}={value:.8};").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    format!("{hash:016x}")
}

#[cfg(test)]
fn scout_combination_key(
    values: &BTreeMap<String, f64>, full_profile: &BTreeMap<String, f64>,
) -> Option<String> {
    strategy::combination_key(values, full_profile)
}

#[cfg(test)]
fn scout_combination_context(key: &str) -> Option<&str> {
    strategy::combination_context(key)
}

#[cfg(test)]
fn best_scout_parameter_combination(
    settings: &Settings, symbol: &str, strategy: Strategy, baseline: &BTreeMap<String, f64>,
) -> Option<BTreeMap<String, f64>> {
    let learning_prefix = if strategy == Strategy::DatedCarry {
        "probe_combo_scout"
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
        let critical = one_sided_90_t_critical(returns.len())?;
        let lower_90 = mean - critical * (variance / returns.len() as f64).sqrt();
        (lower_90 > 0.0).then_some((values, mean))
    }).max_by(|left, right| left.1.total_cmp(&right.1)).map(|(values, _)| values)
}

#[cfg(test)]
fn best_scout_parameter_value(
    settings: &Settings,
    _symbol: &str,
    strategy: Strategy,
    spec: &strategy::StrategyParameterSpec,
) -> Option<f64> {
    scout_parameter_variants(settings, strategy, spec)
        .into_iter()
        .filter(|variant| {
            variant.sample_count >= variant.minimum_samples
                && variant
                    .lower_90_one_sided_improvement_percent
                    .is_some_and(|lower_90| lower_90 > 0.0)
        })
        .max_by(|left, right| {
            left.average_net_improvement_percent
                .total_cmp(&right.average_net_improvement_percent)
        })
        .map(|variant| variant.candidate_value)
}

#[cfg(test)]
fn scout_parameter_variants(
    settings: &Settings,
    strategy: Strategy,
    spec: &strategy::StrategyParameterSpec,
) -> Vec<StrategyParameterScoutVariant> {
    let mut candidates: BTreeMap<String, (f64, Vec<(u64, f64)>)> = BTreeMap::new();
    for (key, stored) in &settings.learning {
        let parts = key.split(':').collect::<Vec<_>>();
        if parts.len() != 5
            || parts[0] != "paired_probe"
            || parts[2] != strategy.as_str()
            || parts[3] != spec.key.as_str()
        {
            continue;
        }
        let Ok(value) = parts[4].parse::<f64>() else {
            continue;
        };
        if !value.is_finite() || value < spec.min || value > spec.max {
            continue;
        }
        let mut stats = stored.clone();
        decay_learning_stats(&mut stats, current_time_ms());
        let candidate = candidates.entry(format!("{value:.8}")).or_insert((value, Vec::new()));
        candidate.1.extend(stats.recent_trade_pnls);
    }
    let minimum_samples = minimum_scout_parameter_variant_samples(strategy);
    candidates
        .into_values()
        .filter_map(|(candidate_value, returns)| {
            if returns.is_empty() {
                return None;
            }
            let sample_count = returns.len();
            let mean = returns.iter().map(|(_, delta)| *delta).sum::<f64>() / sample_count as f64;
            let lower_90 = if sample_count >= minimum_samples {
                let variance = returns
                    .iter()
                    .map(|(_, delta)| (delta - mean).powi(2))
                    .sum::<f64>()
                    / (sample_count - 1) as f64;
                one_sided_90_t_critical(sample_count)
                    .map(|critical| mean - critical * (variance / sample_count as f64).sqrt())
            } else {
                None
            };
            let upper_90 = lower_90.map(|lower| 2.0 * mean - lower);
            let promoted = sample_count >= minimum_samples
                && lower_90.is_some_and(|bound| bound > 0.0);
            let rejected = sample_count >= minimum_samples
                && upper_90.is_some_and(|bound| bound < 0.0);
            Some(StrategyParameterScoutVariant {
                candidate_value,
                sample_count,
                minimum_samples,
                average_net_improvement_percent: mean,
                lower_90_one_sided_improvement_percent: lower_90,
                upper_90_one_sided_improvement_percent: upper_90,
                paired_baseline_increase_samples: 0,
                paired_baseline_decrease_samples: 0,
                paired_baseline_same_samples: 0,
                paired_baseline_unknown_samples: sample_count,
                paired_baseline_delta_mean: None,
                paired_baseline_delta_samples: 0,
                promoted,
                rejected,
            })
        })
        .collect()
}

/// Cornish-Fisher approximation to the 90th percentile of Student's t.
/// Promotion evidence starts at six samples, where the expansion is accurate
/// enough for this bounded lower-confidence test.
#[cfg(test)]
fn one_sided_90_t_critical(sample_count: usize) -> Option<f64> {
    let degrees = sample_count.checked_sub(1)? as f64;
    if degrees < 5.0 {
        return None;
    }
    let z = 1.281_551_565_544_600_4;
    let z2 = z * z;
    let z3 = z2 * z;
    let z5 = z3 * z2;
    let z7 = z5 * z2;
    Some(
        z + (z3 + z) / (4.0 * degrees)
            + (5.0 * z5 + 16.0 * z3 + 3.0 * z) / (96.0 * degrees.powi(2))
            + (3.0 * z7 + 19.0 * z5 + 17.0 * z3 - 15.0 * z)
                / (384.0 * degrees.powi(3)),
    )
}

#[cfg(test)]
fn strategy_parameter_profile(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> StrategyParameterProfile {
    let mut learning = strategy_learning_stats(settings, symbol, strategy);
    let legacy_scout_key = format!("scout:{symbol}:{}", strategy.as_str());
    let scenario_scout_prefix = format!("{legacy_scout_key}:");
    for (key, stored_scout) in &settings.learning {
        if key != &legacy_scout_key && !key.starts_with(&scenario_scout_prefix) {
            continue;
        }
        let mut scout = stored_scout.clone();
        decay_learning_stats(&mut scout, current_time_ms());
        learning.samples += scout.samples;
        learning.positive += scout.positive;
        learning.realized_pnl += scout.realized_pnl;
        learning.recent_trade_pnls.extend(scout.recent_trade_pnls);
    }
    learning.recent_trade_pnls.sort_unstable_by(|left, right| right.0.cmp(&left.0));
    learning.recent_trade_pnls.truncate(STRATEGY_SIZING_RECENT_TRADES);
    let positive_rate =
        (learning.samples > 0).then_some(learning.positive as f64 / learning.samples as f64);
    let learned_profile = settings
        .learned_profiles
        .get(&format!("{symbol}:{}", strategy.as_str()));
    let specs = strategy.parameter_specs();
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
        let value = best_scout_parameter_value(settings, symbol, strategy, spec)
            .or(learned_value)
            .unwrap_or(base);
        (spec.key.as_str().to_owned(), value)
    }).collect::<BTreeMap<_, _>>();
    let learned_combination =
        best_scout_parameter_combination(settings, symbol, strategy, &baseline_values);
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
                scout_variants: scout_parameter_variants(settings, strategy, spec)
                    .into_iter()
                    .map(|mut variant| {
                        variant.promoted = variant.promoted
                            && (variant.candidate_value - learned_value).abs() < spec.step * 1e-6;
                        variant
                    })
                    .collect(),
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

/// Read the active profile maintained by the Learning service. This snapshot
/// path is used by live decision and execution code; local learning history is
/// not re-scored inside AutoTrader.
fn learned_strategy_parameter_profile(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> StrategyParameterProfile {
    let specs = strategy.parameter_specs();
    let learned = settings.learned_profiles.get(&format!("{symbol}:{}", strategy.as_str()));
    let parameters = specs.iter().map(|spec| {
        let key = strategy_parameter_key(symbol, strategy, spec.key.as_str());
        let configured = settings.strategy_parameter_overrides.get(&key).copied()
            .or_else(|| legacy_strategy_parameter_name(spec.key.as_str()).and_then(|legacy| {
                settings.strategy_parameter_overrides.get(&strategy_parameter_key(symbol, strategy, legacy)).copied()
            }))
            .filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max);
        let learned_value = learned.and_then(|profile| profile.parameters.get(spec.key.as_str()).copied())
            .filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max)
            .unwrap_or(spec.default);
        let value = configured.unwrap_or(learned_value);
        StrategyParameterValue {
            key: spec.key.as_str().to_owned(),
            label: spec.label.to_owned(),
            value,
            learned_value,
            configured_value: configured,
            default_value: spec.default,
            min: spec.min,
            max: spec.max,
            step: spec.step,
            unit: spec.unit.to_owned(),
            overridden: configured.is_some(),
            scout_variants: settings.learned_scout_parameter_variants.iter()
                .filter(|variant| variant.symbol == symbol
                    && variant.strategy == strategy.as_str()
                    && variant.parameter == spec.key.as_str())
                .map(|variant| strategy::StrategyParameterScoutVariant {
                    candidate_value: variant.candidate_value,
                    sample_count: variant.sample_count,
                    minimum_samples: variant.minimum_samples,
                    average_net_improvement_percent: variant.average_net_improvement_percent,
                    lower_90_one_sided_improvement_percent: variant.lower_90_one_sided_improvement_percent,
                    upper_90_one_sided_improvement_percent: variant.upper_90_one_sided_improvement_percent,
                    paired_baseline_increase_samples: variant.paired_baseline_increase_samples,
                    paired_baseline_decrease_samples: variant.paired_baseline_decrease_samples,
                    paired_baseline_same_samples: variant.paired_baseline_same_samples,
                    paired_baseline_unknown_samples: variant.paired_baseline_unknown_samples,
                    paired_baseline_delta_mean: variant.paired_baseline_delta_mean,
                    paired_baseline_delta_samples: variant.paired_baseline_delta_samples,
                    promoted: variant.promoted && (variant.candidate_value - value).abs() < spec.step * 1e-6,
                    rejected: variant.rejected,
                }).collect(),
        }
    }).collect();
    StrategyParameterProfile {
        symbol: symbol.to_owned(),
        strategy: strategy.as_str().to_owned(),
        learning_samples: learned.map_or(0, |profile| profile.learning_samples),
        positive_rate: learned.and_then(|profile| (profile.learning_samples > 0)
            .then_some(profile.positive_samples as f64 / profile.learning_samples as f64)),
        realized_pnl: learned.map_or(0.0, |profile| profile.realized_pnl),
        parameters,
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
        let side = match strategy::SignalDirection::from_wire(decision.get("direction")?.as_str()?)? {
            strategy::SignalDirection::Long => 1.0,
            strategy::SignalDirection::Short => -1.0,
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

fn strategy_parameter_values(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> BTreeMap<String, f64> {
    learned_strategy_parameter_profile(settings, symbol, strategy)
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
    let profile_id = strategy::profile_id(symbol, strategy.as_str(), &parameters);
    if settings.registered_profile_ids.contains(&profile_id) { return; }
    if settings.profile_registration_outbox.iter().any(|registration| {
        registration.profile.profile_id == profile_id
            && registration.profile.symbol == symbol
            && registration.profile.strategy == strategy.as_str()
    }) { return; }
    let version = settings.learned_profiles
        .get(&format!("{symbol}:{}", strategy.as_str()))
        .map_or(0, |learned| learned.version);
    let specs = strategy.parameter_specs().into_iter().map(|spec| strategy::ParameterSpec {
        key: spec.key.as_str().to_owned(), label: spec.label.to_owned(), default: spec.default,
        min: spec.min, max: spec.max, step: spec.step, unit: spec.unit.to_owned(),
        learning_direction: spec.learning_direction,
    }).filter(|spec| parameters.contains_key(&spec.key)).collect();
    settings.profile_registration_outbox.push(strategy::StrategyProfileRegistration {
        profile: strategy::StrategyProfile {
            profile_id, version, symbol: symbol.to_owned(), strategy: strategy.as_str().to_owned(),
            parameters, learning_samples: profile.learning_samples,
            positive_samples: (profile.learning_samples as f64 * profile.positive_rate.unwrap_or(0.0)).round() as usize,
            realized_pnl: profile.realized_pnl,
        },
        specs,
    });
}

#[cfg(test)]
mod strategy_parameter_bounds_tests {
    use super::*;

    #[test]
    fn hot_path_overlays_the_latest_learning_profile_on_signal_parameters() {
        let mut settings = Settings::default();
        let parameters = BTreeMap::from([("minimum_momentum".to_owned(), 0.003)]);
        let profile_id = strategy::profile_id("BTC-USDT", "live-momentum", &parameters);
        settings.learned_profiles.insert("BTC-USDT:live-momentum".into(), strategy::StrategyProfile {
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
        assert_eq!(signal.strategy_decision.as_ref().unwrap()["learning_profile_id"],
            strategy::profile_id("BTC-USDT", "live-momentum", &signal_strategy_parameters(&signal)));
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
        assert_eq!(registration.profile.profile_id, strategy::profile_id(
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
            (Strategy::RiskAdjustedTrendPortfolio, StrategyParameterKey::TrendPortfolioExitWeightFloor),
            (Strategy::RiskAdjustedMonthlyMomentum, StrategyParameterKey::MonthlyMomentumLookbackDays),
            (Strategy::RiskAdjustedMonthlyMomentum, StrategyParameterKey::MonthlyMomentumVolatilityTarget),
            (Strategy::DatedCarry, StrategyParameterKey::CarryMinimumDaysToExpiry),
        ];
        for (strategy, key) in cases {
            assert!(strategy.parameter_specs().iter().any(|spec| spec.key == key),
                "{} must expose {}", strategy.as_str(), key.as_str());
        }
    }

    #[test]
    fn every_registered_strategy_accepts_bounds_and_rejects_values_outside_them() {
        let mut strategies_checked = 0;

        for strategy in Strategy::all() {
            strategies_checked += 1;
            let specs = strategy.parameter_specs();
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
mod threshold_profile_source_tests {
    use super::*;

    #[test]
    fn extracts_threshold_evidence_for_learning_feedback() {
        let decision = serde_json::json!({
            "direction":"bullish",
            "observed_insight":{"trade_flow_imbalance":0.2},
            "strategy_parameters":{"minimum_flow_imbalance":0.15}
        });
        assert_eq!(
            threshold_observation("live-trade-flow", &decision),
            Some((StrategyParameterKey::MinimumFlowImbalance, 0.15, 0.2))
        );
    }

    #[test]
    fn learned_threshold_comes_from_learning_snapshot_and_configured_override_wins() {
        let strategy = Strategy::LiveTradeFlow;
        let threshold_key = StrategyParameterKey::MinimumFlowImbalance.as_str();
        let profile_key = format!("BTC-USDT:{}", strategy.as_str());
        let learned_value = 0.08;
        let mut settings = Settings::default();
        settings.learned_profiles.insert(profile_key, strategy::StrategyProfile {
            profile_id: strategy::profile_id("BTC-USDT", strategy.as_str(), &BTreeMap::from([
                (threshold_key.to_owned(), learned_value),
            ])),
            version: 7,
            symbol: "BTC-USDT".into(),
            strategy: strategy.as_str().into(),
            parameters: BTreeMap::from([(threshold_key.to_owned(), learned_value)]),
            learning_samples: 40,
            positive_samples: 24,
            realized_pnl: 4.2,
        });

        let active = learned_strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let threshold = active.parameters.iter().find(|p| p.key == threshold_key).unwrap();
        assert_eq!(threshold.learned_value, learned_value);
        assert_eq!(threshold.value, learned_value);
        assert_eq!(threshold.scout_variants.len(), 0);

        settings.strategy_parameter_overrides.insert(
            strategy_parameter_key("BTC-USDT", strategy, threshold_key), 0.12,
        );
        let overridden = learned_strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let threshold = overridden.parameters.iter().find(|p| p.key == threshold_key).unwrap();
        assert_eq!(threshold.learned_value, learned_value);
        assert_eq!(threshold.configured_value, Some(0.12));
        assert_eq!(threshold.value, 0.12);
        assert!(threshold.overridden);
    }

    #[test]
    fn active_profile_uses_profile_symbol_and_filters_out_of_bounds_learning() {
        let strategy = Strategy::LiveTradeFlow;
        let key = format!("ETH-USDT:{}", strategy.as_str());
        let mut settings = Settings::default();
        settings.learned_profiles.insert(key, strategy::StrategyProfile {
            profile_id: "learning-profile".into(),
            version: 3,
            symbol: "ETH-USDT".into(),
            strategy: strategy.as_str().into(),
            parameters: BTreeMap::from([(
                StrategyParameterKey::MinimumFlowImbalance.as_str().into(), 2.0,
            )]),
            learning_samples: 10,
            positive_samples: 5,
            realized_pnl: 1.0,
        });

        // The query key follows the profile symbol (which can differ from the
        // signal symbol for scout variants); invalid learned values fall back to
        // the strategy default while the active profile's metadata is retained.
        let profile = learned_strategy_parameter_profile(&settings, "ETH-USDT", strategy);
        let threshold = profile.parameters.iter()
            .find(|p| p.key == StrategyParameterKey::MinimumFlowImbalance.as_str())
            .unwrap();
        assert_eq!(threshold.value, threshold.default_value);
        assert_eq!(profile.learning_samples, 10);
    }

    #[test]
    fn scout_probe_overrides_configured_threshold_and_identity_uses_its_profile_symbol() {
        let strategy = Strategy::LiveTradeFlow;
        let profile_symbol = "ETH-USDT";
        let mut settings = Settings::default();
        settings.learned_profiles.insert(
            format!("{profile_symbol}:{}", strategy.as_str()),
            strategy::StrategyProfile {
                profile_id: "base-profile".into(),
                version: 4,
                symbol: profile_symbol.into(),
                strategy: strategy.as_str().into(),
                parameters: BTreeMap::from([(
                    StrategyParameterKey::MinimumFlowImbalance.as_str().into(), 0.07,
                )]),
                learning_samples: 20,
                positive_samples: 12,
                realized_pnl: 2.0,
            },
        );
        settings.strategy_parameter_overrides.insert(
            strategy_parameter_key(
                profile_symbol,
                strategy,
                StrategyParameterKey::MinimumFlowImbalance.as_str(),
            ),
            0.10,
        );
        let mut signal = Signal {
            symbol: "BTC-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 1,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.as_str().into()),
            strategy_decision: Some(serde_json::json!({
                "scout_parameter_profile_symbol": profile_symbol,
                "scout_parameter_key": "minimum_flow_imbalance",
                "scout_parameter_value": 0.15,
                "strategy_parameters": { "minimum_flow_imbalance": 0.02 }
            })),
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        };

        inject_learned_parameters(&settings, &mut signal);
        let parameters = signal_strategy_parameters(&signal);
        assert_eq!(parameters["minimum_flow_imbalance"], 0.15);
        let decision = signal.strategy_decision.as_ref().unwrap();
        assert_eq!(decision["learning_profile_symbol"], profile_symbol);
        assert_eq!(
            decision["learning_profile_id"],
            strategy::profile_id(profile_symbol, strategy.as_str(), &parameters),
        );
    }
}

#[cfg(test)]
mod scout_parameter_learning_tests {
    use super::*;

    #[test]
    fn paired_scout_results_from_other_symbols_promote_only_the_tested_parameter() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian55Btc200;
        let spec = strategy.parameter_specs()
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::BreakoutEntryDistancePercent)
            .unwrap();
        let candidate = spec.default + spec.step;
        let key = format!(
            "paired_probe:ETH-USDT:{}:{}:{candidate:.8}",
            strategy.as_str(), spec.key.as_str()
        );
        for _ in 0..MIN_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(
                settings.learning.entry(key.clone()).or_default(),
                0.1,
            );
        }

        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let entry_distance = profile
            .parameters
            .iter()
            .find(|parameter| {
                parameter.key == StrategyParameterKey::BreakoutEntryDistancePercent.as_str()
            })
            .unwrap();
        assert_eq!(entry_distance.learned_value, candidate);
        let lookback = profile
            .parameters
            .iter()
            .find(|parameter| parameter.key == StrategyParameterKey::BreakoutLookbackDays.as_str())
            .unwrap();
        assert_eq!(lookback.learned_value, lookback.default_value);
    }

    #[test]
    fn scout_outcomes_contribute_to_strategy_parameter_learning() {
        let mut settings = Settings::default();
        let strategy = Strategy::Donchian55Btc200;
        let key = strategy_scout_learning_key("BTC-USDT", strategy.as_str(), "strategy");
        let scout = settings.learning.entry(key).or_default();
        scout.samples = 20;
        scout.positive = 15;
        scout.realized_pnl = 1.5;
        scout.recent_trade_pnls = (0..20)
            .map(|index| (current_time_ms(), if index < 15 { 0.1 } else { -0.05 }))
            .collect();
        let tight_key = strategy_scout_learning_key("BTC-USDT", strategy.as_str(), "tight");
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
        assert_eq!(lookback.learned_value, lookback.default_value);
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
        assert_eq!(eth_distance.learned_value, 10.0);
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
mod dated_carry_scout_probe_learning_tests {
    use super::*;

    #[test]
    fn promotion_uses_one_day_scout_markouts_for_dated_carry_variants() {
        let mut settings = Settings::default();
        let strategy = Strategy::DatedCarry;
        let scout_key = strategy_parameter_scout_probe_learning_key(
            "BTC-USDT", strategy.as_str(), "carry_minimum_days_to_expiry", 6.0,
        );
        for _ in 0..20 {
            record_learning_outcome(settings.learning.entry(scout_key.clone()).or_default(), 0.02);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let minimum_days = profile.parameters.iter()
            .find(|parameter| parameter.key == "carry_minimum_days_to_expiry")
            .unwrap();
        assert_eq!(minimum_days.learned_value, 7.0);

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
        assert_eq!(minimum_days.learned_value, 7.0);
    }

    #[test]
    fn dated_carry_scout_markouts_promote_the_tested_pair_in_its_profile_context() {
        let mut settings = Settings::default();
        let strategy = Strategy::DatedCarry;
        let baseline = strategy.parameter_specs().into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default)).collect::<BTreeMap<_, _>>();
        let combination = next_scout_parameter_combination("BTC-USDT", strategy, &baseline, 3).unwrap();
        let mut probe_profile = baseline.clone();
        for (key, value) in &combination { probe_profile.insert(key.clone(), *value); }
        let combination_key = scout_combination_key(&combination, &probe_profile).unwrap();
        let learning_key = strategy_parameter_scout_combination_learning_key(
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
        let spec = strategy.parameter_specs()
            .into_iter()
            .find(|spec| spec.key == StrategyParameterKey::PostOnlyTimeoutMinutes)
            .unwrap();
        assert_eq!(spec.default, 5.0);
        assert_eq!((spec.min, spec.max, spec.step), (1.0, 15.0, 1.0));

        let baseline = strategy.parameter_specs()
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
        let spec = strategy.parameter_specs().into_iter()
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
        let parameters = strategy.parameter_specs().into_iter()
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
    fn promotion_bound_uses_one_sided_ninety_percent_student_t_critical_values() {
        assert!((one_sided_90_t_critical(6).unwrap() - 1.4759).abs() < 0.002);
        assert!((one_sided_90_t_critical(20).unwrap() - 1.3277).abs() < 0.002);
        assert!(one_sided_90_t_critical(5).is_none());
    }

    #[test]
    fn live_parameter_promotion_requires_positive_paired_improvement() {
        let strategy = Strategy::LiveTradeFlow;
        let spec = strategy.parameter_specs().into_iter()
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

        for _ in 0..MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES / 2 {
            record_learning_outcome(settings.learning.entry(paired_key.clone()).or_default(), 0.02);
        }
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let parameter = profile.parameters.iter().find(|parameter| parameter.key == spec.key.as_str()).unwrap();
        assert_eq!(parameter.scout_variants[0].sample_count, MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES / 2);
        assert!(!parameter.scout_variants[0].promoted);

        for _ in MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES / 2..MIN_LIVE_SCOUT_PARAMETER_VARIANT_SAMPLES {
            record_learning_outcome(settings.learning.entry(paired_key.clone()).or_default(), 0.02);
        }
        assert_eq!(best_scout_parameter_value(&settings, "BTC-USDT", strategy, &spec), Some(candidate));
        let profile = strategy_parameter_profile(&settings, "BTC-USDT", strategy);
        let parameter = profile.parameters.iter().find(|parameter| parameter.key == spec.key.as_str()).unwrap();
        assert!(parameter.scout_variants[0].promoted);
    }
}
