async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_missed_opportunity_bands(
    State(state): State<AutoTraderState>,
) -> Json<Vec<MissedOpportunityGateBand>> {
    Json(
        state
            .latest_missed_opportunity_bands
            .lock()
            .await
            .as_ref()
            .clone(),
    )
}

fn live_profile_from_api(
    settings: &Settings,
    profile: StrategyParameterProfile,
    strategy: Strategy,
) -> strategy::LiveStrategyProfile {
    let parameters = profile.parameters.into_iter()
        .map(|parameter| (parameter.key, parameter.value))
        .collect::<BTreeMap<_, _>>();
    let version = settings.learned_profiles
        .get(&format!("{}:{}", profile.symbol, strategy.as_str()))
        .map_or(0, |profile| profile.version);
    strategy::LiveStrategyProfile {
        profile_id: strategy::profile_id(&profile.symbol, strategy.as_str(), &parameters),
        walk_forward_evidence: settings.learned_walk_forward_evidence
            .get(&strategy::profile_id(&profile.symbol, strategy.as_str(), &parameters))
            .cloned(),
        profile_symbol: profile.symbol,
        version,
        parameters,
    }
}

fn live_profile_for_symbol(
    settings: &Settings,
    symbol: &str,
    strategy: Strategy,
) -> strategy::LiveStrategyProfile {
    live_profile_from_api(settings, learned_strategy_parameter_profile(settings, symbol, strategy), strategy)
}

fn build_live_strategy_profile_set(
    settings: &Settings,
    trader_enabled: bool,
    strategy: Strategy,
) -> strategy::LiveStrategyProfileSet {
    let mut profiles = if let Some(configured) = settings.active_live_strategy_profiles.get(strategy.as_str()) {
        configured.clone()
    } else {
        strategy_parameter_profiles_for(settings, strategy_profile_symbols(settings), vec![strategy])
            .into_iter()
            .map(|profile| live_profile_from_api(settings, profile, strategy))
            .collect::<Vec<_>>()
    };
    for profile in &mut profiles {
        profile.walk_forward_evidence = settings
            .learned_walk_forward_evidence
            .get(&profile.profile_id)
            .cloned();
    }
    let open_positions = settings.strategy_positions.iter()
        .filter_map(|(symbol, strategies)| {
            let position = strategies.get(strategy.as_str())
                .filter(|position| position.quantity.abs() > 1e-12)?;
            let decision = position.strategy_decision.as_ref();
            let parameters = position_strategy_parameters(position);
            let profile_symbol = decision
                .and_then(|value| value.get("learning_profile_symbol")
                    .or_else(|| value.get("scout_parameter_profile_symbol")))
                .and_then(serde_json::Value::as_str)
                .unwrap_or(symbol);
            let profile_id = decision
                .and_then(|value| value.get("learning_profile_id").or_else(|| value.get("profile_id")))
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
                .unwrap_or_else(|| strategy::profile_id(profile_symbol, strategy.as_str(), &parameters));
            if !profiles.iter().any(|profile| profile.profile_id == profile_id) {
                profiles.push(strategy::LiveStrategyProfile {
                    profile_id: profile_id.clone(),
                    profile_symbol: profile_symbol.to_owned(),
                    version: decision.and_then(|value| value.get("learning_profile_version"))
                        .and_then(serde_json::Value::as_u64).unwrap_or_default(),
                    parameters,
                    walk_forward_evidence: settings
                        .learned_walk_forward_evidence
                        .get(&profile_id)
                        .cloned(),
                });
            }
            Some(strategy::LiveStrategyOpenPosition {
                symbol: symbol.clone(),
                profile_id,
                profile_symbol: profile_symbol.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    let enabled = trader_enabled
        && (settings.strategies.contains(&strategy) || !open_positions.is_empty());
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    use std::hash::{Hash, Hasher};
    enabled.hash(&mut hasher);
    for profile in &profiles {
        profile.profile_id.hash(&mut hasher);
        profile.version.hash(&mut hasher);
        if let Some(evidence) = &profile.walk_forward_evidence {
            evidence.sample_count.hash(&mut hasher);
            evidence.last_sample_timestamp_ms.hash(&mut hasher);
            evidence
                .positive_net_rate
                .map(f64::to_bits)
                .hash(&mut hasher);
            evidence
                .average_net_return_percent
                .map(f64::to_bits)
                .hash(&mut hasher);
        }
    }
    for position in &open_positions {
        position.symbol.hash(&mut hasher);
        position.profile_id.hash(&mut hasher);
    }
    strategy::LiveStrategyProfileSet {
        contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
        set_version: hasher.finish(),
        strategy,
        enabled,
        profiles,
        open_positions,
    }
}

impl AutoTraderSocketApi {
    pub fn subscribe_live_profile_updates(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.state.live_profile_updates.subscribe()
    }

    pub async fn live_profile_sets(&self) -> Vec<strategy::LiveStrategyProfileSet> {
        let settings = self.state.settings.lock().await;
        Strategy::all()
            .filter(|strategy| !strategy.is_legacy_composite())
            .map(|strategy| {
                build_live_strategy_profile_set(
                    &settings,
                    self.state.enabled.load(Ordering::Relaxed),
                    strategy,
                )
            })
            .collect()
    }

    pub async fn report_decision(
        &self,
        report: strategy::LiveStrategyDecisionReport,
    ) -> Result<u16, (u16, String)> {
        receive_strategy_decision(State(self.state.clone()), Json(report))
            .await
            .map(|status| status.as_u16())
            .map_err(|(status, message)| (status.as_u16(), message))
    }
    pub async fn apply_learning_snapshot(&self, snapshot: strategy::LearnedProfileSnapshot) {
        apply_learning_snapshot(&self.state, snapshot).await;
    }
}

async fn get_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DecisionParametersResponse> {
    let (max_entries_per_auto_position, risk_appetite, settings_snapshot, symbols) = {
        let settings = state.settings.lock().await;
        // This endpoint needs learned profiles and parameter variants, not the
        // full positions, orders, outboxes, and audit state. Cloning Settings
        // here duplicated the trader's entire persisted state on every request.
        let mut profile_settings = Settings::default();
        profile_settings.learned_profiles = settings.learned_profiles.clone();
        profile_settings.strategy_parameter_overrides =
            settings.strategy_parameter_overrides.clone();
        profile_settings.learned_scout_parameter_variants =
            settings.learned_scout_parameter_variants.clone();
        (
            settings.max_entries_per_auto_position,
            settings.risk_appetite,
            profile_settings,
            strategy_profile_symbols(&settings),
        )
    };
    let thresholds = state.settings.lock().await.learned_context_thresholds.clone();
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
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
    let strategies = Strategy::all()
        .filter(|strategy| !strategy.is_legacy_composite())
        .collect::<Vec<_>>();
    Json(DecisionParametersResponse {
        refreshed_at,
        thresholds,
        contexts,
        latest_execution,
        strategy_profiles: strategy_parameter_profiles_for(
            &settings_snapshot,
            symbols,
            strategies,
        ),
    })
}

#[derive(Serialize)]
struct DailyStrategyParametersResponse {
    strategy_profiles: Vec<StrategyParameterProfile>,
}

async fn get_daily_strategy_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DailyStrategyParametersResponse> {
    let settings = state.settings.lock().await;
    Json(DailyStrategyParametersResponse {
        strategy_profiles: daily_strategy_parameter_profiles(&settings),
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
    let strategies = Strategy::all()
        .filter(|strategy| !strategy.is_legacy_composite())
        .collect::<Vec<_>>();
    strategy_parameter_profiles_for(settings, strategy_profile_symbols(settings), strategies)
}

fn strategy_profile_symbols(settings: &Settings) -> BTreeSet<String> {
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
    symbols
}

fn daily_strategy_parameter_profiles(settings: &Settings) -> Vec<StrategyParameterProfile> {
    const DAILY_SYMBOLS: [&str; 7] = [
        "BTC-USDT",
        "ETH-USDT",
        "BNB-USDT",
        "XRP-USDT",
        "SOL-USDT",
        "ADA-USDT",
        "DOGE-USDT",
    ];
    const PORTFOLIO_SYMBOLS: [&str; 16] = [
        "BTC-USDT",
        "ETH-USDT",
        "BNB-USDT",
        "XRP-USDT",
        "ADA-USDT",
        "SOL-USDT",
        "DOGE-USDT",
        "LTC-USDT",
        "TRX-USDT",
        "LINK-USDT",
        "BCH-USDT",
        "XLM-USDT",
        "ETC-USDT",
        "FIL-USDT",
        "AVAX-USDT",
        "DOT-USDT",
    ];
    const NON_PORTFOLIO_STRATEGIES: [&str; 10] = [
        "donchian-55-btc200",
        "donchian-20-sma200",
        "donchian-20-btc200",
        "donchian-10-btc200",
        "donchian-20-btc200-hold3",
        "trend-sleeves-btc200",
        "trend-50-200",
        "relative-strength-90-sma200",
        "risk-adjusted-monthly-momentum",
        "rsi2-oversold-rebound",
    ];
    let symbols = [
        "BTC-USDT", "ETH-USDT", "BNB-USDT", "XRP-USDT", "SOL-USDT", "ADA-USDT",
        "DOGE-USDT", "LTC-USDT", "TRX-USDT", "LINK-USDT", "BCH-USDT", "XLM-USDT",
        "ETC-USDT", "FIL-USDT", "AVAX-USDT", "DOT-USDT",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let strategies = [
        Strategy::Donchian55Btc200,
        Strategy::Donchian20Sma200,
        Strategy::Donchian20Btc200,
        Strategy::Donchian10Btc200,
        Strategy::Donchian20Btc200Hold3,
        Strategy::RiskAdjustedTrendPortfolio,
        Strategy::TrendSleevesBtc200,
        Strategy::Trend50200,
        Strategy::RelativeStrength90Sma200,
        Strategy::RiskAdjustedMonthlyMomentum,
        Strategy::Rsi2OversoldRebound,
    ]
    .into_iter()
    .collect::<Vec<_>>();
    strategy_parameter_profiles_for(settings, symbols, strategies)
        .into_iter()
        .filter(|profile| {
            if profile.strategy == "risk-adjusted-trend-portfolio" {
                PORTFOLIO_SYMBOLS.contains(&profile.symbol.as_str())
            } else {
                DAILY_SYMBOLS.contains(&profile.symbol.as_str())
                    && NON_PORTFOLIO_STRATEGIES.contains(&profile.strategy.as_str())
            }
        })
        .collect()
}

fn strategy_parameter_profiles_for(
    settings: &Settings,
    symbols: BTreeSet<String>,
    strategies: Vec<Strategy>,
) -> Vec<StrategyParameterProfile> {
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
    fn live_profile_refresh_builds_only_the_requested_strategy_profiles() {
        let settings = Settings::default();
        let profiles = strategy_parameter_profiles_for(
            &settings,
            BTreeSet::from(["BTC-USDT".to_owned(), "ETH-USDT".to_owned()]),
            vec![Strategy::LiveMomentum],
        );

        assert_eq!(profiles.len(), 2);
        assert!(profiles
            .iter()
            .all(|profile| profile.strategy == Strategy::LiveMomentum.as_str()));
    }

    #[test]
    fn profiles_include_symbols_with_learned_trade_history() {
        let mut settings = Settings::default();
        let parameters = BTreeMap::new();
        settings.learned_profiles.insert(
            "XRP-USDC:live-momentum".into(),
            strategy::StrategyProfile {
                profile_id: strategy::profile_id("XRP-USDC", "live-momentum", &parameters),
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
            .push(strategy::ScoutParameterVariant {
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
