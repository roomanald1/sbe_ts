#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_trader_profiles_fill_missing_live_learning_profiles() {
        let snapshot = serde_json::json!({
            "strategy_profiles": [
                {
                    "symbol": "BTC-USDT", "strategy": "live-momentum",
                    "parameters": [{"key": "minimum_momentum", "value": 0.01}]
                },
                {
                    "symbol": "BTC-USDT", "strategy": "live-order-book",
                    "parameters": [{"key": "minimum_book_imbalance", "value": 0.2}]
                },
                {
                    "symbol": "ETH-USDT", "strategy": "live-momentum",
                    "parameters": [{"key": "minimum_momentum", "value": 0.01}]
                },
                {
                    "symbol": "BTC-USDT", "strategy": "trend-50-200",
                    "parameters": [{"key": "minimum_momentum", "value": 0.01}]
                }
            ]
        });
        let profiles = auto_trader_live_profiles(
            &snapshot,
            &BTreeSet::from(["BTC-USDT"]),
            &["live-momentum".into()],
        );

        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].symbol, "BTC-USDT");
        assert_eq!(profiles[0].strategy, "live-momentum");
        assert_eq!(profiles[0].parameters["minimum_momentum"], 0.01);
    }

    #[test]
    fn live_candidate_entry_uses_shared_strategy_evaluator_and_requires_profile_parameters() {
        let candidate = serde_json::json!({
            "symbol": "BTC-USDT",
            "direction": "bullish",
            "components": { "momentum_10s_percent": 0.75 }
        });
        assert!(live_strategy_enters(
            "live-momentum",
            candidate.clone(),
            "bullish",
            BTreeMap::from([("minimum_momentum".into(), 0.5)])
        ));
        assert!(!live_strategy_enters(
            "live-momentum",
            candidate.clone(),
            "bullish",
            BTreeMap::from([("minimum_momentum".into(), 0.8)])
        ));
        assert!(!live_strategy_enters(
            "live-momentum",
            candidate,
            "bullish",
            BTreeMap::new()
        ));
    }

    #[tokio::test]
    async fn generated_live_candidates_keep_the_strategy_for_plan_registration() {
        let strategy = strategies::Strategy::LiveMomentum;
        let parameters = strategy
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let source = serde_json::json!({
            "symbol":"BTC-USDT", "direction":"neutral", "timestamp":1_000_000,
            "score":0.5, "strength":0.5,
            "components":{
                "momentum_by_window":[{"window_seconds":10,"value":0.01}],
                "trend_volatility_units_by_window":[{"window_seconds":300,"value":0.5}]
            }
        });
        let Json(candidates) = generate_live_candidates(Json(LiveScoutCandidatesRequest {
            signals: vec![source],
            profiles: vec![LiveScoutProfile {
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                parameters,
            }],
        }))
        .await
        .unwrap();

        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0]["strategy"], "live-momentum");
    }

    fn specs() -> Vec<ParameterSpec> {
        vec![
            ParameterSpec {
                key: "minimum_momentum".into(),
                label: "Momentum".into(),
                default: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
                unit: "ratio".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "lookback_days".into(),
                label: "Lookback".into(),
                default: 20.0,
                min: 10.0,
                max: 30.0,
                step: 1.0,
                unit: "days".into(),
                learning_direction: 1.0,
            },
            ParameterSpec {
                key: "learning_rate".into(),
                label: "Learning rate".into(),
                default: 0.05,
                min: 0.0,
                max: 0.25,
                step: 0.01,
                unit: "ratio".into(),
                learning_direction: 1.0,
            },
        ]
    }

    #[test]
    fn probe_generation_is_deterministic_bounded_and_excludes_meta_parameters() {
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 1.0),
            ("lookback_days".into(), 10.0),
        ]);
        let first =
            next_parameter_probe("BTC-USDT", "live-momentum", &parameters, &specs(), 2).unwrap();
        assert_eq!(
            first,
            next_parameter_probe("BTC-USDT", "live-momentum", &parameters, &specs(), 2).unwrap()
        );
        assert_ne!(first.0, "learning_rate");
        assert!(
            specs()
                .iter()
                .find(|spec| spec.key == first.0)
                .is_some_and(|spec| first.1 >= spec.min && first.1 <= spec.max)
        );
        assert_eq!(
            next_parameter_combination("BTC-USDT", "live-momentum", &parameters, &specs(), 2)
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn single_parameter_sweep_includes_both_bounded_directions() {
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 0.5),
            ("lookback_days".into(), 20.0),
            ("learning_rate".into(), 0.05),
        ]);
        let variants = single_parameter_variants(&parameters, &specs(), 2).unwrap();
        for key in ["minimum_momentum", "lookback_days"] {
            let values = variants
                .iter()
                .filter_map(|variant| variant.get(key).copied())
                .collect::<Vec<_>>();
            assert_eq!(values.len(), 2, "{key} should be tested up and down");
            assert!(values.iter().any(|value| value < &parameters[key]));
            assert!(values.iter().any(|value| value > &parameters[key]));
            let spec = specs().into_iter().find(|spec| spec.key == key).unwrap();
            assert!(values.iter().all(|value| *value >= spec.min && *value <= spec.max));
        }
        assert!(variants.iter().all(|variant| !variant.contains_key("learning_rate")));
    }

    #[test]
    fn subset_sweep_includes_every_arity_without_waiting_for_later_days() {
        let mut parameter_specs = specs();
        parameter_specs.push(ParameterSpec {
            key: "take_profit_percent".into(),
            label: "Take profit".into(),
            default: 1.0,
            min: 0.1,
            max: 2.0,
            step: 0.1,
            unit: "%".into(),
            learning_direction: 1.0,
        });
        let parameters = BTreeMap::from([
            ("minimum_momentum".into(), 1.0),
            ("lookback_days".into(), 10.0),
            ("take_profit_percent".into(), 1.0),
        ]);
        let variants = all_parameter_variants(&parameters, &parameter_specs, 2).unwrap();
        assert_eq!(variants.len(), 7);
        let sizes = variants
            .iter()
            .map(BTreeMap::len)
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(sizes, std::collections::BTreeSet::from([1, 2, 3]));
        assert!(variants.iter().any(|variant| variant.len() == 3));
        for variant in variants {
            for (key, value) in variant {
                let spec = parameter_specs.iter().find(|spec| spec.key == key).unwrap();
                assert!((value - parameters[&key]).abs() <= spec.step + 1e-9);
                assert!(value >= spec.min && value <= spec.max);
            }
        }
    }

    #[test]
    fn ten_parameters_produce_the_entire_one_step_subset_sweep() {
        let specs = (0..10)
            .map(|index| ParameterSpec {
                key: format!("parameter_{index}"),
                label: format!("Parameter {index}"),
                default: 5.0,
                min: 0.0,
                max: 10.0,
                step: 1.0,
                unit: "units".into(),
                learning_direction: 1.0,
            })
            .collect::<Vec<_>>();
        let parameters = specs
            .iter()
            .map(|spec| (spec.key.clone(), spec.default))
            .collect::<BTreeMap<_, _>>();

        let variants = all_parameter_variants(&parameters, &specs, 0).unwrap();
        assert_eq!(variants.len(), 1_023);
        assert!(variants.iter().any(|variant| variant.len() == 1));
        assert!(variants.iter().any(|variant| variant.len() == 2));
        assert!(variants.iter().any(|variant| variant.len() == 10));
    }

    #[test]
    fn paired_probe_key_round_trips_with_variant_bounds() {
        let values = BTreeMap::from([
            ("minimum_momentum".into(), 0.6),
            ("lookback_days".into(), 21.0),
        ]);
        let profile = BTreeMap::from([
            ("minimum_momentum".into(), 0.5),
            ("lookback_days".into(), 20.0),
        ]);
        let key = combination_key(&values, &profile).unwrap();
        assert_eq!(parse_combination(&key, &specs()).unwrap(), values);
        assert!(combination_context(&key).is_some());
    }

    #[test]
    fn sample_preserves_profile_and_computes_net_return() {
        let sample = ScoutSample {
            event_id: "scout-1".into(),
            opened_at_ms: 10,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "profile-7".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.4)]),
            specs: vec![],
            entry_price: 100.0,
            side: 1,
        };
        let event = sample.finalize(20, 102.0, 0.5).unwrap();
        let FeedbackEvent::ScoutOutcome {
            event_id,
            profile_id,
            parameters,
            net_return_percent,
            ..
        } = event
        else {
            panic!("wrong event")
        };
        assert_eq!(profile_id, "profile-7");
        assert_eq!(event_id, "scout-1");
        assert_eq!(parameters["minimum_momentum"], 0.4);
        assert!((net_return_percent - 1.5).abs() < 1e-12);
    }
}
