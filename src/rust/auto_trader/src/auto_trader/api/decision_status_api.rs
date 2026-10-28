async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DecisionParametersResponse> {
    let (
        learning,
        max_entries_per_auto_position,
        risk_appetite,
        settings_snapshot,
    ) = {
        let settings = state.settings.lock().await;
        (
            {
                let mut learning = settings.learning.clone();
                decay_learning_map(&mut learning, current_time_ms());
                learning
            },
            settings.max_entries_per_auto_position,
            settings.risk_appetite,
            settings.clone(),
        )
    };
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
    let contexts = build_context_decision_parameters(&learning, &bands, risk_appetite);
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
            let positive_rate =
                (stats.samples > 0).then_some(stats.positive as f64 / stats.samples as f64);
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
                strength_floor,
                exploration_score_floor,
            }
        })
        .collect()
}

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
    let prefix = parts[..dimensions].join(":");
    Some(if offset == 1 {
        format!("scout:{prefix}")
    } else {
        prefix
    })
}

fn build_decision_thresholds(
    risk_appetite: f64,
    max_entries_per_auto_position: u32,
) -> Vec<DecisionParameter> {
    let mut thresholds = Vec::new();
    thresholds.extend(signal_qualification_thresholds(risk_appetite));
    thresholds.extend(scout_qualification_thresholds(risk_appetite));
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
    let symbols = [
        "BTC-USDT",
        "ETH-USDT",
        "BNB-USDT",
        "XRP-USDT",
        "SOL-USDT",
        "ADA-USDT",
        "DOGE-USDT",
    ];
    let strategies = Strategy::all().filter(|strategy| !strategy.is_legacy_composite()).collect::<Vec<_>>();
    symbols
        .into_iter()
        .flat_map(|symbol| {
            strategies.iter().copied()
                .into_iter()
                .map(move |strategy| strategy_parameter_profile(settings, symbol, strategy))
        })
        .collect()
}
