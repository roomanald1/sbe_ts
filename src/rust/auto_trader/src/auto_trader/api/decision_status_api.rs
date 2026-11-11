async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DecisionParametersResponse> {
    let (max_entries_per_auto_position, risk_appetite, settings_snapshot) = {
        let settings = state.settings.lock().await;
        (
            settings.max_entries_per_auto_position,
            settings.risk_appetite,
            settings.clone(),
        )
    };
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
    let thresholds = request_learning_context_thresholds_for_contexts(
        &state,
        BTreeSet::new(),
        &bands,
    )
    .await;
    let floor_scale = risk_floor_scale(risk_appetite);
    let contexts = thresholds
        .into_iter()
        .map(|(context, thresholds)| ContextDecisionParameters {
            exploration_allowed: !is_mixed_market_context(&context),
            context,
            learning_samples: thresholds.learning_samples,
            positive_rate: thresholds.positive_rate,
            scout_learning_samples: thresholds.scout_outcome_samples,
            scout_positive_rate: thresholds.scout_outcome_positive_rate,
            strength_floor: (adaptive_missed_opportunity_floor(
                thresholds.strength_floor,
                MissedOpportunityGate::Strength,
                &bands,
            ) * floor_scale)
                .clamp(0.0, 1.0),
            exploration_score_floor: (adaptive_missed_opportunity_floor(
                thresholds.exploration_score_floor,
                MissedOpportunityGate::Score,
                &bands,
            ) * floor_scale)
                .max(0.0),
        })
        .collect();
    let thresholds = build_decision_thresholds(risk_appetite, max_entries_per_auto_position);
    let latest_execution = state.latest_execution_parameters.lock().await.clone();
    let refreshed_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    Json(DecisionParametersResponse {
        refreshed_at,
        thresholds,
        contexts,
        latest_execution,
        strategy_profiles: strategy_parameter_profiles(&settings_snapshot),
    })
}

#[cfg(test)]
fn build_context_decision_parameters(
    learning: &BTreeMap<String, LearningStats>,
    bands: &[MissedOpportunityGateBand],
    risk_appetite: f64,
) -> Vec<ContextDecisionParameters> {
    let floor_scale = risk_floor_scale(risk_appetite);
    let contexts = learning
        .keys()
        .filter_map(|key| learning_context_prefix(key))
        .collect::<BTreeSet<_>>();
    contexts
        .into_iter()
        .map(|context| {
            let stats = context_learning_stats(learning, &context);
            let scout_stats = context_learning_stats(learning, &format!("scout:{context}"));
            let positive_rate =
                (stats.samples > 0).then_some(stats.positive as f64 / stats.samples as f64);
            let scout_positive_rate = (scout_stats.samples > 0)
                .then_some(scout_stats.positive as f64 / scout_stats.samples as f64);
            let strength_floor = (adaptive_missed_opportunity_floor(
                adaptive_strength_floor(learning, &context),
                MissedOpportunityGate::Strength,
                bands,
            ) * floor_scale)
                .clamp(0.0, 1.0);
            let exploration_score_floor =
                (adaptive_exploration_score_floor(learning, &context, bands) * floor_scale)
                    .max(0.0);
            ContextDecisionParameters {
                exploration_allowed: !is_mixed_market_context(&context),
                context,
                learning_samples: stats.samples,
                positive_rate,
                scout_learning_samples: scout_stats.samples,
                scout_positive_rate,
                strength_floor,
                exploration_score_floor,
            }
        })
        .collect()
}

#[cfg(test)]
fn learning_context_prefix(key: &str) -> Option<String> {
    let context = key.split(":P").next()?;
    let parts = context.split(':').collect::<Vec<_>>();
    let offset = usize::from(parts.first() == Some(&"scout"));
    if parts.len() < offset + 3 || !matches!(parts[offset], "long" | "short") {
        return None;
    }
    // Preserve the dimensions used by market_context: side, regime, symbol,
    // and (when present) strategy. This makes the UI reflect actual profiles.
    let dimensions = (offset + 3).min(parts.len());
    let dimensions = if parts.len() > offset + 3 {
        offset + 4
    } else {
        dimensions
    };
    let prefix = parts[offset..dimensions].join(":");
    Some(prefix)
}

fn build_decision_thresholds(
    risk_appetite: f64,
    max_entries_per_auto_position: u32,
) -> Vec<DecisionParameter> {
    let mut thresholds = Vec::new();
    thresholds.extend(signal_qualification_thresholds(risk_appetite));
    thresholds.extend(historical_evidence_thresholds());
    thresholds.extend(counterfactual_relaxation_thresholds());
    thresholds.extend(order_selection_thresholds(risk_appetite));
    thresholds.extend(market_context_thresholds());
    thresholds.extend(learning_thresholds());
    thresholds.extend(risk_and_hedging_thresholds());
    thresholds.extend(risk_and_re_entry_thresholds(max_entries_per_auto_position));
    thresholds.extend(exit_policy_thresholds());
    thresholds
}

fn strategy_parameter_profiles(settings: &Settings) -> Vec<StrategyParameterProfile> {
    let mut symbols = [
        "BTC-USDT",
        "ETH-USDT",
        "BNB-USDT",
        "XRP-USDT",
        "SOL-USDT",
        "ADA-USDT",
        "DOGE-USDT",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    // Include symbols with actual profile/outcome state so the audit can show
    // what the strategy used for long-tail assets and non-USDT quote pairs.
    symbols.extend(settings.positions.keys().cloned());
    symbols.extend(
        settings
            .strategy_positions
            .values()
            .flat_map(|by_symbol| by_symbol.keys().cloned()),
    );
    symbols.extend(
        settings
            .learned_profiles
            .values()
            .map(|profile| profile.symbol.clone()),
    );
    symbols.extend(settings.learned_context_stats.keys().filter_map(|key| {
        let key = key.strip_prefix("scout:").unwrap_or(key);
        let key = key.strip_suffix(":trade").unwrap_or(key);
        let parts = key.split(':').collect::<Vec<_>>();
        (parts.len() >= 3).then(|| parts[2].to_owned())
    }));
    symbols.extend(
        settings
            .strategy_parameter_overrides
            .keys()
            .filter_map(|key| key.split_once(':').map(|(symbol, _)| symbol.to_owned())),
    );
    let strategies = Strategy::all()
        .filter(|strategy| !strategy.is_legacy_composite())
        .collect::<Vec<_>>();
    symbols
        .into_iter()
        .flat_map(|symbol| {
            strategies.iter().copied().into_iter().map(move |strategy| {
                learned_strategy_parameter_profile(settings, &symbol, strategy)
            })
        })
        .collect()
}

#[cfg(test)]
mod decision_profile_tests {
    use super::*;

    #[test]
    fn profiles_include_symbols_with_learned_trade_history() {
        let mut settings = Settings::default();
        let parameters = BTreeMap::new();
        settings.learned_profiles.insert(
            "XRP-USDC:live-momentum".into(),
            strategies::StrategyProfile {
                profile_id: strategies::profile_id("XRP-USDC", "live-momentum", &parameters),
                version: 1,
                symbol: "XRP-USDC".into(),
                strategy: "live-momentum".into(),
                parameters,
                learning_samples: 3,
                positive_samples: 1,
                realized_pnl: -0.2,
            },
        );

        let profiles = strategy_parameter_profiles(&settings);
        let profile = profiles
            .iter()
            .find(|profile| profile.symbol == "XRP-USDC" && profile.strategy == "live-momentum")
            .expect("learned non-default symbol profile");
        assert_eq!(profile.learning_samples, 3);
        assert_eq!(profile.positive_rate, Some(1.0 / 3.0));
    }

    #[test]
    fn decision_profiles_include_paired_scout_variants_from_learning_snapshot() {
        let mut settings = Settings::default();
        let spec = Strategy::LiveMomentum.parameter_specs()[0];
        settings
            .learned_scout_parameter_variants
            .push(strategies::ScoutParameterVariant {
                symbol: "BTC-USDT".into(),
                strategy: Strategy::LiveMomentum.as_str().into(),
                parameter: spec.key.as_str().into(),
                candidate_value: spec.default,
                sample_count: 3,
                minimum_samples: 6,
                average_net_improvement_percent: 0.12,
                lower_90_one_sided_improvement_percent: None,
                upper_90_one_sided_improvement_percent: None,
                paired_baseline_increase_samples: 0,
                paired_baseline_decrease_samples: 2,
                paired_baseline_same_samples: 0,
                paired_baseline_unknown_samples: 0,
                paired_baseline_delta_mean: Some(-0.1),
                paired_baseline_delta_samples: 3,
                promoted: false,
                rejected: false,
            });

        let profiles = strategy_parameter_profiles(&settings);
        let profile = profiles
            .iter()
            .find(|profile| {
                profile.symbol == "BTC-USDT" && profile.strategy == Strategy::LiveMomentum.as_str()
            })
            .expect("live momentum profile");
        let parameter = profile
            .parameters
            .iter()
            .find(|parameter| parameter.key == spec.key.as_str())
            .expect("tested parameter");
        assert_eq!(parameter.scout_variants.len(), 1);
        assert_eq!(parameter.scout_variants[0].sample_count, 3);
        assert_eq!(
            parameter.scout_variants[0].paired_baseline_decrease_samples,
            2
        );
        assert_eq!(
            parameter.scout_variants[0].paired_baseline_delta_mean,
            Some(-0.1)
        );
    }

    #[test]
    fn scout_and_trade_history_share_the_live_context_row() {
        assert_eq!(
            learning_context_prefix("scout:long:range:XRP-USDC:live-momentum:trade"),
            Some("long:range:XRP-USDC:live-momentum".into())
        );
    }
}
