async fn get_status(State(state): State<AutoTraderState>) -> Json<StatusResponse> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    Json(status(&state, &settings))
}

async fn get_strategy_open_position_counts(
    State(state): State<AutoTraderState>,
) -> Json<BTreeMap<String, usize>> {
    let settings = state.settings.lock().await;
    let mut counts = settings.strategies.iter()
        .map(|strategy| (strategy.as_str().to_owned(), 0usize))
        .collect::<BTreeMap<_, _>>();
    for strategies in settings.strategy_positions.values() {
        for (name, position) in strategies {
            if position.quantity.abs() > 1e-12 {
                *counts.entry(name.clone()).or_default() += 1;
            }
        }
    }
    let carry_positions = state.dated_carry.lock().await;
    let open_carry_count = carry_positions.positions.iter()
        .filter(|position| position.status.eq_ignore_ascii_case("open"))
        .count();
    *counts.entry("dated-futures-carry".to_owned()).or_default() += open_carry_count;
    Json(counts)
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
    let profile_id = strategy::profile_id(&profile.symbol, strategy.as_str(), &parameters);
    strategy::LiveStrategyProfile {
        profile_id: profile_id.clone(),
        walk_forward_evidence: walk_forward_evidence_for_profile(
            settings,
            &profile_id,
            &profile.symbol,
            strategy,
        ),
        walk_forward_evidence_by_regime: walk_forward_evidence_by_regime_for_profile(
            settings,
            &profile_id,
            &profile.symbol,
            strategy,
        ),
        profile_symbol: profile.symbol,
        version,
        parameters,
    }
}

fn walk_forward_evidence_for_profile(
    settings: &Settings,
    profile_id: &str,
    profile_symbol: &str,
    strategy: Strategy,
) -> Option<strategy::StrategyWalkForwardEvidence> {
    settings
        .learned_walk_forward_evidence
        .get(profile_id)
        .or_else(|| {
            settings
                .learned_walk_forward_evidence
                .get(&format!("{profile_symbol}|{}", strategy.as_str()))
        })
        .cloned()
}

fn walk_forward_evidence_by_regime_for_profile(
    settings: &Settings,
    profile_id: &str,
    profile_symbol: &str,
    strategy: Strategy,
) -> BTreeMap<String, strategy::StrategyWalkForwardEvidence> {
    ["range", "trend", "volatile", "mixed"]
        .into_iter()
        .filter_map(|regime| {
            settings
                .learned_walk_forward_evidence
                .get(&format!("{profile_id}|regime:{regime}"))
                .or_else(|| {
                    settings
                        .learned_walk_forward_evidence
                        .get(&format!("{profile_symbol}|{}|regime:{regime}", strategy.as_str()))
                })
                .cloned()
                .map(|evidence| (regime.to_owned(), evidence))
        })
        .collect()
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
    profiles.retain(|profile| strategy::is_autotrader_symbol_allowed(&profile.profile_symbol));
    for profile in &mut profiles {
        profile.walk_forward_evidence = walk_forward_evidence_for_profile(
            settings,
            &profile.profile_id,
            &profile.profile_symbol,
            strategy,
        );
        profile.walk_forward_evidence_by_regime = walk_forward_evidence_by_regime_for_profile(
            settings,
            &profile.profile_id,
            &profile.profile_symbol,
            strategy,
        );
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
            if !strategy::is_autotrader_symbol_allowed(symbol)
                || !strategy::is_autotrader_symbol_allowed(profile_symbol)
            {
                return None;
            }
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
                    walk_forward_evidence: walk_forward_evidence_for_profile(
                        settings,
                        &profile_id,
                        profile_symbol,
                        strategy,
                    ),
                    walk_forward_evidence_by_regime: walk_forward_evidence_by_regime_for_profile(
                        settings,
                        &profile_id,
                        profile_symbol,
                        strategy,
                    ),
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
        for (regime, evidence) in &profile.walk_forward_evidence_by_regime {
            regime.hash(&mut hasher);
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
    let (max_entries_per_auto_position, risk_appetite, thresholds, strategy_profiles) = {
        let settings = state.settings.lock().await;
        let symbols = strategy_profile_symbols(&settings);
        let strategies = Strategy::all()
            .filter(|strategy| !strategy.is_legacy_composite())
            .collect::<Vec<_>>();
        let mut strategy_profiles =
            strategy_parameter_profiles_for(&settings, symbols, strategies);
        for profile in &mut strategy_profiles {
            let profile_strategy = Strategy::from_name(&profile.strategy).unwrap_or_default();
            profile.walk_forward_evidence = walk_forward_evidence_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
            profile.walk_forward_evidence_by_regime = walk_forward_evidence_by_regime_for_profile(
                &settings,
                &profile.symbol,
                &profile.symbol,
                profile_strategy,
            );
        }
        (
            settings.max_entries_per_auto_position,
            settings.risk_appetite,
            settings.learned_context_thresholds.clone(),
            strategy_profiles,
        )
    };
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
    Json(DecisionParametersResponse {
        refreshed_at,
        thresholds,
        contexts,
        latest_execution,
        strategy_profiles,
    })
}

#[derive(Serialize)]
struct ParameterCatalogEntry {
    key: String,
    label: String,
    default_value: f64,
    min: f64,
    max: f64,
    step: f64,
    unit: String,
}

#[derive(Serialize)]
struct CompactParameterValue {
    value: f64,
    learned_value: f64,
    configured_value: Option<f64>,
    overridden: bool,
}

#[derive(Serialize)]
struct CompactStrategyProfile {
    symbol: String,
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    realized_pnl: f64,
    walk_forward_evidence: Option<strategy::StrategyWalkForwardEvidence>,
    walk_forward_evidence_by_regime: BTreeMap<String, strategy::StrategyWalkForwardEvidence>,
    parameters: BTreeMap<String, CompactParameterValue>,
}

#[derive(Serialize)]
struct CompactDecisionParametersResponse {
    version: u8,
    refreshed_at: u64,
    thresholds: Vec<DecisionParameter>,
    contexts: Vec<ContextDecisionParameters>,
    latest_execution: Option<LatestExecutionParameters>,
    catalog: BTreeMap<String, Vec<ParameterCatalogEntry>>,
    strategy_profiles: Vec<CompactStrategyProfile>,
}

async fn get_compact_decision_parameters(
    State(state): State<AutoTraderState>,
) -> Json<CompactDecisionParametersResponse> {
    let (max_entries, risk_appetite, thresholds, catalog, strategy_profiles) = {
        let settings = state.settings.lock().await;
        let symbols = strategy_profile_symbols(&settings);
        let strategies = Strategy::all().filter(|strategy| !strategy.is_legacy_composite()).collect::<Vec<_>>();
        let schemas = strategies.iter().map(|strategy| (strategy.as_str().to_owned(),
            strategy.parameter_specs().iter().map(|spec| ParameterCatalogEntry {
                key: spec.key.as_str().to_owned(), label: spec.label.to_owned(), default_value: spec.default,
                min: spec.min, max: spec.max, step: spec.step, unit: spec.unit.to_owned(),
            }).collect::<Vec<_>>())).collect::<BTreeMap<_, _>>();
        let mut profiles = Vec::new();
        for symbol in symbols {
          for strategy in &strategies {
            let learned = settings.learned_profiles.get(&format!("{symbol}:{}", strategy.as_str()));
            let parameters = strategy.parameter_specs().iter().map(|spec| {
                let key = strategy_parameter_key(&symbol, *strategy, spec.key.as_str());
                let configured = settings.strategy_parameter_overrides.get(&key).copied()
                    .or_else(|| legacy_strategy_parameter_name(spec.key.as_str()).and_then(|name|
                        settings.strategy_parameter_overrides.get(&strategy_parameter_key(&symbol, *strategy, name)).copied()))
                    .filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max);
                let learned_value = learned.and_then(|profile| profile.parameters.get(spec.key.as_str()).copied())
                    .filter(|value| value.is_finite() && *value >= spec.min && *value <= spec.max).unwrap_or(spec.default);
                let value = configured.unwrap_or(learned_value);
                (spec.key.as_str().to_owned(), CompactParameterValue { value, learned_value, configured_value: configured, overridden: configured.is_some() })
            }).collect();
            profiles.push(CompactStrategyProfile { symbol: symbol.clone(), strategy: strategy.as_str().to_owned(),
                learning_samples: learned.map_or(0, |profile| profile.learning_samples),
                positive_rate: learned.and_then(|profile| (profile.learning_samples > 0).then_some(profile.positive_samples as f64 / profile.learning_samples as f64)),
                realized_pnl: learned.map_or(0.0, |profile| profile.realized_pnl),
                walk_forward_evidence: walk_forward_evidence_for_profile(&settings, &symbol, &symbol, *strategy),
                walk_forward_evidence_by_regime: walk_forward_evidence_by_regime_for_profile(&settings, &symbol, &symbol, *strategy),
                parameters });
          }
        }
        (settings.max_entries_per_auto_position, settings.risk_appetite, settings.learned_context_thresholds.clone(), schemas, profiles)
    };
    let bands = state.latest_missed_opportunity_bands.lock().await.clone();
    let floor_scale = risk_floor_scale(risk_appetite);
    let contexts = thresholds.into_iter().map(|(context, thresholds)| ContextDecisionParameters {
        exploration_allowed: !is_mixed_market_context(&context), context,
        learning_samples: thresholds.learning_samples, positive_rate: thresholds.positive_rate,
        scout_learning_samples: thresholds.scout_outcome_samples, scout_positive_rate: thresholds.scout_outcome_positive_rate,
        strength_floor: (adaptive_missed_opportunity_floor(thresholds.strength_floor, MissedOpportunityGate::Strength, &bands) * floor_scale).clamp(0.0, 1.0),
        exploration_score_floor: (adaptive_missed_opportunity_floor(thresholds.exploration_score_floor, MissedOpportunityGate::Score, &bands) * floor_scale).max(0.0),
    }).collect();
    let thresholds = build_decision_thresholds(risk_appetite, max_entries);
    let latest_execution = state.latest_execution_parameters.lock().await.clone();
    let refreshed_at = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    Json(CompactDecisionParametersResponse {
        version: 1, refreshed_at, thresholds,
        contexts, latest_execution,
        catalog, strategy_profiles,
    })
}

#[derive(Serialize)]
struct ScoutVariantsResponse {
    scout_variants: BTreeMap<String, Vec<strategy::StrategyParameterScoutVariant>>,
}

async fn get_scout_parameter_variants(
    State(state): State<AutoTraderState>,
) -> Json<ScoutVariantsResponse> {
    let settings = state.settings.lock().await;
    let symbols = strategy_profile_symbols(&settings);
    let strategies = Strategy::all().filter(|strategy| !strategy.is_legacy_composite()).collect::<Vec<_>>();
    let mut variants = BTreeMap::new();
    for profile in strategy_parameter_profiles_for(&settings, symbols, strategies) {
        for parameter in profile.parameters {
            if !parameter.scout_variants.is_empty() {
                variants.insert(format!("{}:{}:{}", profile.symbol, profile.strategy, parameter.key), parameter.scout_variants);
            }
        }
    }
    Json(ScoutVariantsResponse { scout_variants: variants })
}

#[derive(Serialize)]
struct DailyStrategyParametersResponse {
    strategy_profiles: Vec<StrategyParameterProfile>,
}

async fn get_daily_strategy_parameters(
    State(state): State<AutoTraderState>,
) -> Json<DailyStrategyParametersResponse> {
    let settings = state.settings.lock().await;
    let mut strategy_profiles = daily_strategy_parameter_profiles(&settings);
    for profile in &mut strategy_profiles {
        let profile_strategy = Strategy::from_name(&profile.strategy).unwrap_or_default();
        profile.walk_forward_evidence = walk_forward_evidence_for_profile(
            &settings,
            &profile.symbol,
            &profile.symbol,
            profile_strategy,
        );
        profile.walk_forward_evidence_by_regime = walk_forward_evidence_by_regime_for_profile(
            &settings,
            &profile.symbol,
            &profile.symbol,
            profile_strategy,
        );
    }
    Json(DailyStrategyParametersResponse {
        strategy_profiles,
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

fn strategy_profile_symbols(_settings: &Settings) -> BTreeSet<String> {
    let mut symbols = strategy::autotrader_allowed_symbols()
    .iter()
    .copied()
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    // Include symbols with actual profile/outcome state so the audit can show
    // what the strategy used for long-tail assets and non-USDT quote pairs.
    // The snapshot contract only includes symbols accepted by AutoTrader.
    symbols.retain(|symbol| strategy::is_autotrader_symbol_allowed(symbol));
    symbols
}

fn daily_strategy_parameter_profiles(settings: &Settings) -> Vec<StrategyParameterProfile> {
    let allowed_symbols = strategy::autotrader_allowed_symbols()
        .iter()
        .copied()
        .collect::<BTreeSet<_>>();
    let daily_symbols = ["BTC-USDT", "ETH-USDT", "BNB-USDT", "XRP-USDT", "SOL-USDT", "ADA-USDT", "DOGE-USDT"];
    let portfolio_symbols = ["BTC-USDT", "ETH-USDT", "BNB-USDT", "XRP-USDT", "ADA-USDT", "SOL-USDT", "DOGE-USDT", "LTC-USDT", "TRX-USDT", "LINK-USDT", "AVAX-USDT"];
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
    let symbols = allowed_symbols.iter().map(|symbol| (*symbol).to_owned()).collect::<BTreeSet<_>>();
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
                portfolio_symbols.contains(&profile.symbol.as_str())
            } else {
                daily_symbols.contains(&profile.symbol.as_str())
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
    let strategy_names = strategies
        .iter()
        .map(|strategy| strategy.as_str())
        .collect::<BTreeSet<_>>();
    let mut variants_by_profile = BTreeMap::new();
    for variant in &settings.learned_scout_parameter_variants {
        if !symbols.contains(&variant.symbol) || !strategy_names.contains(variant.strategy.as_str()) {
            continue;
        }
        variants_by_profile
            .entry((variant.symbol.clone(), variant.strategy.clone()))
            .or_insert_with(Vec::new)
            .push(variant);
    }
    let variants_by_profile = &variants_by_profile;
    symbols
        .into_iter()
        .flat_map(|symbol| {
            strategies.iter().copied().map(move |strategy| {
                let variants = variants_by_profile
                    .get(&(symbol.clone(), strategy.as_str().to_owned()))
                    .map(Vec::as_slice)
                    .unwrap_or_default();
                learned_strategy_parameter_profile_with_variants(
                    settings,
                    &symbol,
                    strategy,
                    variants,
                )
            })
        })
        .collect()
}

#[cfg(test)]
mod decision_profile_tests {
    use super::*;

    fn evidence(samples: usize) -> strategy::StrategyWalkForwardEvidence {
        strategy::StrategyWalkForwardEvidence {
            sample_count: samples,
            positive_net_rate: Some(0.5),
            average_net_return_percent: Some(0.1),
            last_sample_timestamp_ms: 10,
        }
    }

    #[test]
    fn live_profile_evidence_uses_exact_then_symbol_and_never_strategy_wide() {
        let profile_id = strategy::profile_id(
            "MON-USDT",
            Strategy::LiveMomentum.as_str(),
            &BTreeMap::new(),
        );
        let mut settings = Settings::default();
        assert_eq!(
            walk_forward_evidence_for_profile(
                &settings,
                &profile_id,
                "MON-USDT",
                Strategy::LiveMomentum,
            ),
            None
        );

        settings.learned_walk_forward_evidence.insert(
            "MON-USDT|live-momentum".to_owned(),
            evidence(20),
        );
        assert_eq!(
            walk_forward_evidence_for_profile(
                &settings,
                &profile_id,
                "MON-USDT",
                Strategy::LiveMomentum,
            )
            .unwrap()
            .sample_count,
            20
        );

        settings
            .learned_walk_forward_evidence
            .insert(profile_id.clone(), evidence(10));
        assert_eq!(
            walk_forward_evidence_for_profile(
                &settings,
                &profile_id,
                "MON-USDT",
                Strategy::LiveMomentum,
            )
            .unwrap()
            .sample_count,
            10
        );

        settings
            .learned_walk_forward_evidence
            .insert(Strategy::LiveMomentum.as_str().to_owned(), evidence(30));
        assert_eq!(
            walk_forward_evidence_for_profile(
                &settings,
                "another-profile",
                "OTHER-USDT",
                Strategy::LiveMomentum,
            ),
            None
        );
    }

    #[test]
    fn live_profile_regime_evidence_prefers_exact_profile_then_symbol_and_strategy() {
        let profile_id = strategy::profile_id(
            "BTC-USDT",
            Strategy::LiveMomentum.as_str(),
            &BTreeMap::new(),
        );
        let mut settings = Settings::default();
        settings.learned_walk_forward_evidence.insert(
            "BTC-USDT|live-momentum|regime:trend".into(),
            evidence(7),
        );
        settings.learned_walk_forward_evidence.insert(
            format!("{profile_id}|regime:trend"),
            evidence(11),
        );
        let by_regime = walk_forward_evidence_by_regime_for_profile(
            &settings,
            &profile_id,
            "BTC-USDT",
            Strategy::LiveMomentum,
        );
        assert_eq!(by_regime["trend"].sample_count, 11);

        settings
            .learned_walk_forward_evidence
            .remove(&format!("{profile_id}|regime:trend"));
        let by_regime = walk_forward_evidence_by_regime_for_profile(
            &settings,
            &profile_id,
            "BTC-USDT",
            Strategy::LiveMomentum,
        );
        assert_eq!(by_regime["trend"].sample_count, 7);
    }

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
    fn profiles_include_only_symbols_allowed_by_autotrader() {
        let mut settings = Settings::default();
        let parameters = BTreeMap::new();
        settings.learned_profiles.insert(
            "BTC-USDT:live-momentum".into(),
            strategy::StrategyProfile {
                profile_id: strategy::profile_id("BTC-USDT", "live-momentum", &parameters),
                version: 1,
                symbol: "BTC-USDT".into(),
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
            .find(|profile| profile.symbol == "BTC-USDT" && profile.strategy == "live-momentum")
            .expect("allowed symbol profile");
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
