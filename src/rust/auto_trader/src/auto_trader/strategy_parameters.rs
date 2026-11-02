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
    decay_learning_stats(&mut stats, now_ms);
    stats
}

fn strategy_parameter_profile(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> StrategyParameterProfile {
    let learning = strategy_learning_stats(settings, symbol, strategy);
    let positive_rate =
        (learning.samples > 0).then_some(learning.positive as f64 / learning.samples as f64);
    let performance = recent_net_pnl_edge(&learning)
        .unwrap_or_else(|| confident_win_rate_edge(learning.positive, learning.samples));
    let specs = all_strategy_parameter_specs(strategy);
    let learning_key = strategy_parameter_key(symbol, strategy, "learning_rate");
    let learning_rate = settings
        .strategy_parameter_overrides
        .get(&learning_key)
        .copied()
        .unwrap_or(0.05)
        .clamp(0.0, 0.25);
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
            // Win rate or realized PnL alone cannot safely tune stop and target
            // distances. The entry planner may use a wider walk-forward policy
            // only after its held-out results beat the conservative baseline.
            let learned_value = if is_walk_forward_threshold(spec.key) {
                walk_forward_threshold(spec, base, &learning.threshold_samples).unwrap_or(base)
            } else {
                let adaptive_step = if matches!(
                    spec.key,
                    StrategyParameterKey::LearningRate
                        | StrategyParameterKey::StopLossPercent
                        | StrategyParameterKey::TakeProfitPercent
                ) {
                    0.0
                } else {
                    (spec.max - spec.min) * learning_rate * performance * spec.learning_direction
                };
                let candidate = (base + adaptive_step).clamp(spec.min, spec.max);
                let step_position = (candidate - spec.min) / spec.step;
                (spec.min + step_position.round() * spec.step).clamp(spec.min, spec.max)
            };
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
    let supported_boundary = if spec.key == StrategyParameterKey::RsiEntryMax {
        samples
            .iter()
            .map(|sample| sample.threshold)
            .fold(base, f64::min)
    } else {
        samples
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

fn strategy_parameter_value(
    parameters: &BTreeMap<String, f64>,
    key: StrategyParameterKey,
) -> Option<f64> {
    parameters.get(key.as_str()).copied().filter(|value| value.is_finite())
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
