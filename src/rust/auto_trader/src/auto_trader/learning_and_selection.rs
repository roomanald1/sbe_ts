fn market_context(signal: &Signal) -> String {
    let regime = if signal.research_strategy_candidate {
        "trend"
    } else if signal.ranging_probability.unwrap_or(0.0) >= CONTEXT_RANGE_MIN_PROBABILITY {
        "range"
    } else if signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs()
        >= CONTEXT_TREND_MIN_VOLATILITY_UNITS
    {
        "trend"
    } else if signal.components.volatility_1m_percent.unwrap_or(0.0).abs()
        >= CONTEXT_VOLATILE_MIN_PERCENT
    {
        "volatile"
    } else {
        "mixed"
    };
    let direction = if signal.direction == "bullish" {
        "long"
    } else {
        "short"
    };
    if let Some(strategy) = signal.research_strategy.as_deref() {
        format!("{direction}:{regime}:{strategy}")
    } else {
        format!("{direction}:{regime}")
    }
}

const ALL_ORDER_TYPES: [AutoOrderType; 12] = [
    AutoOrderType::Market,
    AutoOrderType::Ioc,
    AutoOrderType::Fok,
    AutoOrderType::Limit,
    AutoOrderType::Gtd,
    AutoOrderType::PostOnlyLimit,
    AutoOrderType::Barrier,
    AutoOrderType::StopMarket,
    AutoOrderType::StopLimit,
    AutoOrderType::TrailingStop,
    AutoOrderType::Vwap,
    AutoOrderType::Twap,
];
fn context_learning_stats(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
) -> LearningStats {
    let prefix = format!("{context}:");
    learning
        .iter()
        .filter(|(key, _)| key.starts_with(&prefix))
        .map(|(_, stats)| stats)
        .fold(LearningStats::default(), |mut total, stats| {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
            total
        })
}

fn adaptive_scout_thresholds(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    active_scout_count: usize,
) -> (f64, f64) {
    let stats = context_learning_stats(learning, &format!("scout:{context}"));
    let win_rate = stats.positive as f64 / stats.samples.max(1) as f64;
    let density = active_scout_count.min(SCOUT_MAX_ACTIVE_POSITIONS) as f64;
    let mut strength =
        (SCOUT_MIN_STRENGTH - 0.15 + density * 0.025).clamp(SCOUT_MIN_STRENGTH_FLOOR, 0.25);
    let mut score =
        (SCOUT_MIN_ABS_SCORE - 0.15 + density * 0.05).clamp(SCOUT_MIN_SCORE_FLOOR, 0.40);
    if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES {
        if win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE {
            strength = (strength + 0.05).min(0.30);
            score = (score + 0.10).min(0.50);
        } else if win_rate >= 0.60 {
            strength = (strength - 0.02).max(SCOUT_MIN_STRENGTH_FLOOR);
            score = (score - 0.03).max(SCOUT_MIN_SCORE_FLOOR);
        }
    }
    (strength, score)
}

fn risk_floor_scale(risk_appetite: f64) -> f64 {
    1.5 - risk_appetite.clamp(0.0, 100.0) / 100.0
}

fn risk_size_scale(risk_appetite: f64) -> f64 {
    0.5 + 2.5 * risk_appetite.clamp(0.0, 100.0) / 100.0
}

fn adaptive_strength_floor(learning: &BTreeMap<String, LearningStats>, context: &str) -> f64 {
    let stats = context_learning_stats(learning, context);
    let win_rate = stats.positive as f64 / stats.samples.max(1) as f64;
    if stats.samples >= STRONG_NEGATIVE_LEARNING_SAMPLES
        && win_rate < STRONG_NEGATIVE_LEARNING_WIN_RATE
    {
        STRONG_NEGATIVE_STRENGTH_FLOOR
    } else if stats.samples >= LEARNED_STRENGTH_ADJUSTMENT_SAMPLES
        && win_rate < LEARNED_STRENGTH_ADJUSTMENT_WIN_RATE
    {
        LEARNED_STRENGTH_FLOOR
    } else {
        MIN_STRENGTH
    }
}

fn adaptive_exploration_score_floor(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
    missed_opportunity_bands: &[MissedOpportunityGateBand],
) -> f64 {
    let stats = context_learning_stats(learning, context);
    let win_rate = stats.positive as f64 / stats.samples.max(1) as f64;
    let learned_floor = if stats.samples >= STRONG_NEGATIVE_LEARNING_SAMPLES
        && win_rate < STRONG_NEGATIVE_LEARNING_WIN_RATE
    {
        STRONG_NEGATIVE_SCORE_FLOOR
    } else if stats.samples >= EXPLORATION_SCORE_HIGH_SAMPLES
        && win_rate < EXPLORATION_SCORE_HIGH_WIN_RATE
    {
        EXPLORATION_SCORE_HIGH_FLOOR
    } else if stats.samples >= EXPLORATION_SCORE_MID_SAMPLES
        && win_rate < EXPLORATION_SCORE_MID_WIN_RATE
    {
        EXPLORATION_SCORE_MID_FLOOR
    } else {
        MIN_EXPLORATION_SCORE
    };
    // Five-minute counterfactual stop/target outcomes may relax gates when they
    // are profitable. Longer terminal outcomes also inform the score floor when
    // mature cohorts show that otherwise-rejected signals need more time to turn
    // net positive. This changes entry selection, not live position exit timing.
    let short_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    let longer_horizon_floor = missed_opportunity_bands
        .iter()
        .filter(|band| {
            band.gate == "score"
                && matches!(band.horizon_seconds, 900 | 3_600)
                && band.threshold >= COUNTERFACTUAL_SCORE_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.positive_net_rate.is_some_and(|value| value >= 0.55)
                // Reject a later recovery if the path would already have hit
                // the configured stop before the longer horizon elapsed.
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
                && band.stop_target_positive_net_rate.is_some_and(|value| value >= 0.55)
                && band.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band.time_to_profit_rate.is_some_and(|value| value >= 0.50)
                && band.average_time_to_profit_seconds.is_some_and(|value| {
                    value > 300.0 && value <= band.horizon_seconds as f64
                })
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor);
    short_horizon_floor.min(longer_horizon_floor)
}

fn longer_horizon_score_multiplier(score: f64, bands: &[MissedOpportunityGateBand]) -> f64 {
    if !score.is_finite() {
        return 1.0;
    }
    let absolute_score = score.abs();
    let short_horizon = bands
        .iter()
        .filter(|band| {
            band.gate == "score" && band.horizon_seconds == 300 && band.threshold <= absolute_score
        })
        .max_by(|left, right| left.threshold.total_cmp(&right.threshold));
    let Some(short) = short_horizon else {
        return 1.0;
    };
    let Some(short_average) = short.average_net_return_percent else {
        return 1.0;
    };
    let Some(short_positive_rate) = short.positive_net_rate else {
        return 1.0;
    };
    let supported_longer_horizon = bands.iter().any(|long| {
        long.gate == "score"
            && (long.threshold - short.threshold).abs() < 1e-9
            && matches!(long.horizon_seconds, 900 | 3_600)
            && long.sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.time_to_profit_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.average_net_return_percent.is_some_and(|average| {
                average >= short_average + 0.10 && average > 0.0
            })
            && long.positive_net_rate.is_some_and(|rate| {
                rate >= 0.70 && rate + 0.05 >= short_positive_rate
            })
            // The stop-target path model treats a stop breach as the exit,
            // even if the terminal mark later recovers above entry.
            && long.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
            && long.stop_target_average_net_return_percent.is_some_and(|value| value > 0.0)
            && long.stop_target_positive_net_rate.is_some_and(|rate| rate >= 0.70)
            && long.time_to_profit_rate.is_some_and(|rate| rate >= 0.70)
            && long.average_time_to_profit_seconds.is_some_and(|seconds| {
                seconds > 300.0 && seconds <= long.horizon_seconds as f64
            })
    });
    if supported_longer_horizon { 1.5 } else { 1.0 }
}

fn adaptive_missed_opportunity_floor(
    learned_floor: f64,
    gate: &str,
    bands: &[MissedOpportunityGateBand],
) -> f64 {
    bands
        .iter()
        .filter(|band| {
            band.gate == gate
                && band.horizon_seconds == 300
                && band.threshold >= COUNTERFACTUAL_STRENGTH_MIN_THRESHOLD
                && band.threshold < learned_floor
                && band.stop_target_sample_count >= COUNTERFACTUAL_RELAXATION_SAMPLES
                && band
                    .stop_target_average_net_return_percent
                    .is_some_and(|value| value > 0.0)
                && band
                    .stop_target_positive_net_rate
                    .is_some_and(|value| value >= 0.55)
        })
        .map(|band| band.threshold)
        .min_by(f64::total_cmp)
        .unwrap_or(learned_floor)
}
fn order_type_learning_stats(
    context: &str,
    kind: AutoOrderType,
    learning: &BTreeMap<String, LearningStats>,
) -> LearningStats {
    // Paper Market and IOC both execute immediately against the same book
    // levels with the same taker fee and no resting remainder. Treat their
    // historical cohorts as one execution behavior so evidence is not split
    // across two labels for identical fills.
    let equivalent_kinds: &[AutoOrderType] = match kind {
        AutoOrderType::Market | AutoOrderType::Ioc => &[AutoOrderType::Market, AutoOrderType::Ioc],
        _ => std::slice::from_ref(&kind),
    };
    let mut total = LearningStats::default();
    for equivalent_kind in equivalent_kinds {
        let prefix = format!("{context}:P");
        let suffix = format!(":{equivalent_kind:?}");
        for stats in learning
            .iter()
            .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(&suffix))
            .map(|(_, stats)| stats)
        {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
        }
        if let Some(stats) = learning.get(&format!("{context}:{equivalent_kind:?}")) {
            total.samples += stats.samples;
            total.positive += stats.positive;
            total.realized_pnl += stats.realized_pnl;
        }
    }
    total
}

fn adapt_order_type(
    baseline: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, LearningStats>,
    risk_appetite: f64,
) -> Option<AutoOrderType> {
    let favor_non_post_only = risk_appetite >= AGGRESSIVE_ORDER_PRIORITY_APPETITE;
    // A post-only entry is intentionally allowed as a cold-start experiment in
    // calm, costly-to-aggress conditions. Do not let the default IOC fallback
    // erase that signal before the maker cohort has enough outcomes to learn.
    if matches!(baseline, AutoOrderType::PostOnlyLimit) && !favor_non_post_only {
        let maker_stats =
            order_type_learning_stats(context, AutoOrderType::PostOnlyLimit, learning);
        if maker_stats.samples < MIN_EXECUTION_LEARNING_SAMPLES {
            return Some(AutoOrderType::PostOnlyLimit);
        }
    }
    let candidates = ALL_ORDER_TYPES
        .iter()
        .copied()
        .filter_map(|kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            let average_pnl = stats.realized_pnl / stats.samples.max(1) as f64;
            let positive_rate = stats.positive as f64 / stats.samples.max(1) as f64;
            (stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && !(favor_non_post_only && matches!(kind, AutoOrderType::PostOnlyLimit))
                && average_pnl > 0.0
                && positive_rate >= 0.55)
                .then_some((kind, average_pnl))
        })
        .collect::<Vec<_>>();
    if candidates.is_empty() {
        // A losing record rules out that order type, not the signal. Keep
        // trying the alternate execution type while its own evidence is
        // immature; only stop when both choices have mature losing records.
        let is_mature_unprofitable = |kind| {
            let stats = order_type_learning_stats(context, kind, learning);
            stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && (stats.realized_pnl / stats.samples as f64 <= 0.0
                    || (stats.positive as f64 / stats.samples as f64) < 0.55)
        };
        let baseline_unprofitable = is_mature_unprofitable(baseline);
        let ioc_unprofitable = is_mature_unprofitable(AutoOrderType::Ioc);
        if baseline_unprofitable && ioc_unprofitable {
            return None;
        }
        return Some(if baseline_unprofitable {
            AutoOrderType::Ioc
        } else if ioc_unprofitable {
            baseline
        } else {
            AutoOrderType::Ioc
        });
    }
    candidates
        .into_iter()
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(kind, _)| kind)
}

fn choose_parameter_profile(
    kind: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, LearningStats>,
    explore: bool,
) -> usize {
    let profiles = (0..PARAMETER_PROFILE_COUNT)
        .map(|profile| {
            let prefix = format!("{context}:P{profile}:");
            let suffix = format!(":{kind:?}");
            let stats = learning.iter()
                .filter(|(key, _)| key.starts_with(&prefix) && key.ends_with(&suffix))
                .fold(LearningStats::default(), |mut total, (_, item)| {
                    total.samples += item.samples;
                    total.positive += item.positive;
                    total.realized_pnl += item.realized_pnl;
                    total
                });
            (profile, stats)
        })
        .collect::<Vec<_>>();
    let proven = profiles
        .iter()
        .filter(|(_, stats)| {
            stats.samples >= MIN_EXECUTION_LEARNING_SAMPLES
                && stats.realized_pnl > 0.0
                && stats.positive as f64 / stats.samples as f64 >= 0.55
        })
        .max_by(|a, b| {
            (a.1.realized_pnl / a.1.samples as f64)
                .total_cmp(&(b.1.realized_pnl / b.1.samples as f64))
        });
    if let Some((profile, _)) = proven {
        return *profile;
    }
    if explore {
        return profiles
            .iter()
            .min_by_key(|(_, stats)| stats.samples)
            .map(|(profile, _)| *profile)
            .unwrap_or(0);
    }
    0
}

fn choose_order_type(signal: &Signal, risk_appetite: f64) -> AutoOrderType {
    let ranging = signal.ranging_probability.unwrap_or(0.0);
    let volatility = signal.components.volatility_1m_percent.unwrap_or(0.0).abs();
    let trend_units = signal
        .components
        .trend_5m_volatility_units
        .unwrap_or(0.0)
        .abs();
    let momentum = signal.components.momentum_10s_percent.unwrap_or(0.0).abs();
    let mispricing = signal
        .components
        .mispricing_vs_vwap_bps
        .unwrap_or(0.0)
        .abs();
    let pressure = signal
        .components
        .pressure_without_price_move
        .unwrap_or(0.0)
        .abs();
    let aggressor_cost = signal.components.aggressor_cost_bps.unwrap_or(0.0).abs();
    let strength_scale = risk_floor_scale(risk_appetite);
    if risk_appetite >= AGGRESSIVE_ORDER_PRIORITY_APPETITE {
        if trend_units >= ORDER_STOP_MARKET_TREND_UNITS
            && signal.strength >= ORDER_STOP_MARKET_STRENGTH * strength_scale
        {
            return AutoOrderType::StopMarket;
        }
        if pressure >= ORDER_MARKET_PRESSURE
            && signal.strength >= ORDER_MARKET_STRENGTH * strength_scale
        {
            return AutoOrderType::Market;
        }
        if momentum >= ORDER_IOC_MOMENTUM_PERCENT
            && signal.strength >= ORDER_IOC_STRENGTH * strength_scale
        {
            return AutoOrderType::Ioc;
        }
    }
    let post_only_range = (ORDER_POST_ONLY_RANGE + 0.20 * risk_appetite / 100.0).clamp(0.0, 1.0);
    let gtd_range = (ORDER_GTD_RANGE + 0.30 * risk_appetite / 100.0).clamp(0.0, 1.0);
    let limit_range = (ORDER_LIMIT_RANGE + 0.30 * risk_appetite / 100.0).clamp(0.0, 1.0);
    if ranging >= post_only_range && aggressor_cost >= ORDER_POST_ONLY_AGGRESSOR_COST_BPS {
        AutoOrderType::PostOnlyLimit
    } else if ranging >= gtd_range
        && mispricing >= ORDER_GTD_MIN_MISPRICING_BPS
        && mispricing < ORDER_GTD_MAX_MISPRICING_BPS
        && aggressor_cost < ORDER_POST_ONLY_AGGRESSOR_COST_BPS
    {
        AutoOrderType::Gtd
    } else if ranging >= limit_range && mispricing >= ORDER_GTD_MAX_MISPRICING_BPS {
        AutoOrderType::Limit
    } else if volatility >= ORDER_STOP_LIMIT_VOLATILITY_PERCENT
        && trend_units >= ORDER_STOP_LIMIT_TREND_UNITS
    {
        AutoOrderType::StopLimit
    } else if trend_units >= ORDER_STOP_MARKET_TREND_UNITS
        && signal.strength >= ORDER_STOP_MARKET_STRENGTH * strength_scale
    {
        AutoOrderType::StopMarket
    } else if pressure >= ORDER_MARKET_PRESSURE
        && signal.strength >= ORDER_MARKET_STRENGTH * strength_scale
    {
        AutoOrderType::Market
    } else if momentum >= ORDER_IOC_MOMENTUM_PERCENT
        && signal.strength >= ORDER_IOC_STRENGTH * strength_scale
    {
        AutoOrderType::Ioc
    } else if signal.strength >= ORDER_VWAP_STRENGTH * strength_scale {
        AutoOrderType::Vwap
    } else if ranging >= ORDER_TWAP_RANGE {
        AutoOrderType::Twap
    } else if volatility >= ORDER_TRAILING_VOLATILITY_PERCENT {
        AutoOrderType::TrailingStop
    } else if signal.strength >= ORDER_FOK_STRENGTH * strength_scale {
        AutoOrderType::Fok
    } else {
        AutoOrderType::Ioc
    }
}

fn schedule_depth_ratio_bucket(ratio: f64) -> &'static str {
    match ratio {
        value if value < 0.05 => "lt05",
        value if value < 0.10 => "05_10",
        value if value < 0.15 => "10_15",
        value if value < 0.25 => "15_25",
        value if value < 0.40 => "25_40",
        _ => "ge40",
    }
}

fn learned_schedule_depth_threshold(
    learning: &BTreeMap<String, LearningStats>,
    context: &str,
) -> (f64, usize) {
    let mut threshold = DEFAULT_SCHEDULE_DEPTH_RATIO;
    let mut paired_samples = 0;
    let cohorts: [(&str, f64, f64); 5] = [
        ("05_10", 0.05, 0.10),
        ("10_15", 0.10, 0.15),
        ("15_25", 0.15, 0.25),
        ("25_40", 0.25, 0.40),
        ("ge40", 0.40, MAX_SCHEDULE_DEPTH_RATIO),
    ];
    for (bucket, lower, upper) in cohorts {
        let marker = format!(":depth:{bucket}:");
        let mut scheduled = LearningStats::default();
        let mut alternatives = LearningStats::default();
        for (key, stats) in learning.iter().filter(|(key, _)| {
            key.starts_with(&format!("{context}:P")) && key.contains(&marker)
        }) {
            if key.ends_with(":Vwap") || key.ends_with(":Twap") {
                scheduled.samples += stats.samples;
                scheduled.positive += stats.positive;
                scheduled.realized_pnl += stats.realized_pnl;
            } else {
                alternatives.samples += stats.samples;
                alternatives.positive += stats.positive;
                alternatives.realized_pnl += stats.realized_pnl;
            }
        }
        if scheduled.samples < MIN_EXECUTION_LEARNING_SAMPLES {
            continue;
        }
        paired_samples += scheduled.samples + alternatives.samples;
        let scheduled_average = scheduled.realized_pnl / scheduled.samples as f64;
        let scheduled_win_rate = scheduled.positive as f64 / scheduled.samples as f64;
        if alternatives.samples >= MIN_EXECUTION_LEARNING_SAMPLES {
            let alternatives_average = alternatives.realized_pnl / alternatives.samples as f64;
            let alternatives_win_rate = alternatives.positive as f64 / alternatives.samples as f64;
            if scheduled_average > alternatives_average && scheduled_win_rate + 0.03 >= alternatives_win_rate {
                threshold = threshold.min(lower.max(MIN_SCHEDULE_DEPTH_RATIO));
            } else if scheduled_average < alternatives_average && scheduled_win_rate + 0.05 < alternatives_win_rate {
                threshold = threshold.max(upper.min(MAX_SCHEDULE_DEPTH_RATIO));
            }
        } else if lower >= threshold && scheduled_average <= 0.0 && scheduled_win_rate < 0.45 {
            // Large orders are scheduled by policy, so use their own mature,
            // consistently losing outcomes to back the threshold off safely.
            threshold = threshold.max(upper.min(MAX_SCHEDULE_DEPTH_RATIO));
        }
    }
    (threshold.clamp(MIN_SCHEDULE_DEPTH_RATIO, MAX_SCHEDULE_DEPTH_RATIO), paired_samples)
}

fn score_execution_order_type(
    signal: &Signal,
    risk_appetite: f64,
    notional: f64,
    baseline: AutoOrderType,
    learned: AutoOrderType,
    book: Option<DeepBookAssessment>,
    schedule_depth_threshold: f64,
) -> AutoOrderType {
    let side_depth = book.map(|assessment| {
        if signal.direction == "bullish" { assessment.ask_depth_10bps } else { assessment.bid_depth_10bps }
    }).unwrap_or(0.0);
    let depth_ratio = if side_depth > 0.0 { notional / side_depth } else { 0.0 };
    // Large parent orders use the scheduler to work child slices against volume.
    let large_order = depth_ratio >= schedule_depth_threshold;
    let urgency = (signal.components.momentum_10s_percent.unwrap_or(0.0).abs() / 0.30)
        .max(signal.components.pressure_without_price_move.unwrap_or(0.0).abs())
        .clamp(0.0, 1.0);
    let ranging = signal.ranging_probability.unwrap_or(0.0).clamp(0.0, 1.0);
    let trend = (signal.components.trend_5m_volatility_units.unwrap_or(0.0).abs() / 3.0).clamp(0.0, 1.0);
    let fee_bps = configured_taker_fee_rate() * 10_000.0;
    let impact_bps = book.map(|assessment| assessment.impact_bps).filter(|value| value.is_finite()).unwrap_or(0.0);
    let scaled_impact_bps = if side_depth > 0.0 {
        impact_bps * (notional / (side_depth * DEEP_BOOK_MAX_PARTICIPATION).max(1.0)).clamp(0.25, 4.0)
    } else { impact_bps };
    let candidates = [baseline, learned, AutoOrderType::Vwap, AutoOrderType::Twap,
        AutoOrderType::PostOnlyLimit, AutoOrderType::Gtd, AutoOrderType::Ioc, AutoOrderType::Market];
    let mut best = learned;
    let mut best_score = f64::NEG_INFINITY;
    for (index, candidate) in candidates.into_iter().enumerate() {
        if candidates[..index].contains(&candidate) { continue; }
        if large_order && !matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) { continue; }
        let maker = matches!(candidate, AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd);
        let impact_multiplier = match candidate {
            AutoOrderType::Vwap | AutoOrderType::Twap => 0.45,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => 0.10,
            _ => 1.0,
        };
        let expected_cost_bps = (if maker { configured_fee_rates_maker_bps() } else { fee_bps })
            + scaled_impact_bps * impact_multiplier
            + if maker { 0.0 } else { signal.components.aggressor_cost_bps.unwrap_or(0.0).abs() * 0.25 };
        let mut score = if candidate == baseline { 1.0 } else { 0.0 };
        if candidate == learned { score += 0.35; }
        score += match candidate {
            AutoOrderType::Vwap => signal.strength.clamp(0.0, 1.0) * 0.6,
            AutoOrderType::Twap => ranging * 0.7,
            AutoOrderType::PostOnlyLimit | AutoOrderType::Gtd => ranging * 0.8 - urgency * 0.7,
            AutoOrderType::Market | AutoOrderType::Ioc => urgency * 0.7 + trend * 0.2,
            _ => 0.0,
        };
        if matches!(candidate, AutoOrderType::Vwap | AutoOrderType::Twap) {
            score += ((depth_ratio - MIN_SCHEDULE_DEPTH_RATIO)
                / (schedule_depth_threshold - MIN_SCHEDULE_DEPTH_RATIO).max(0.01))
                .clamp(0.0, 1.0) * 2.0;
            score -= urgency * (risk_appetite / 100.0) * 0.7;
        }
        // Cost is a meaningful score component, but its bounded weight cannot
        // override a strong signal/regime fit by itself.
        score -= (expected_cost_bps / 10.0).clamp(0.0, 2.0) * 0.35;
        if score > best_score { best = candidate; best_score = score; }
    }
    best
}


fn strategy_post_stop_cooldown_ms(strategy: &str, orders: &[TraderOrder]) -> u64 {
    let outcomes = orders
        .iter()
        .rev()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && matches!(
                    order.purpose.as_str(),
                    "stop-loss" | "take-profit" | "strategy-disabled" | "research-regime-exit"
                )
                && order.status == "filled"
                && order.filled_quantity > 0.0
                && order.realized_pnl.is_finite()
        })
        .take(POST_STOP_OUTCOME_WINDOW)
        .collect::<Vec<_>>();

    if outcomes.len() < MIN_POST_STOP_OUTCOMES_FOR_ADAPTATION {
        return DEFAULT_POST_STOP_REENTRY_COOLDOWN_MS;
    }

    let win_rate = outcomes
        .iter()
        .filter(|order| order.realized_pnl > 0.0)
        .count() as f64
        / outcomes.len() as f64;
    let loss_pressure = ((0.55 - win_rate) / 0.35).clamp(0.0, 1.0);
    MIN_POST_STOP_REENTRY_COOLDOWN_MS
        + ((MAX_POST_STOP_REENTRY_COOLDOWN_MS - MIN_POST_STOP_REENTRY_COOLDOWN_MS) as f64
            * loss_pressure) as u64
}
