fn choose_parameter_profile(
    kind: AutoOrderType,
    context: &str,
    learning: &BTreeMap<String, ExecutionLearningStats>,
    explore: bool,
) -> usize {
    let profiles = (0..PARAMETER_PROFILE_COUNT)
        .map(|profile| {
            let prefix = format!("{context}:P{profile}:");
            let suffix = format!(":exec:{kind:?}");
            let stats = aggregate_execution_quality(learning, &prefix, &suffix);
            (profile, stats)
        })
        .collect::<Vec<_>>();
    let proven = profiles
        .iter()
        .filter(|(_, stats)| {
            stats.samples >= MIN_EXECUTION_QUALITY_SAMPLES
                && stats.filled_samples >= MIN_EXECUTION_QUALITY_FILLS
                && stats.cost_notional > 0.0
                && stats.fill_rate_total / stats.samples as f64 >= 0.60
        })
        .min_by(|a, b| {
            (a.1.all_in_cost_notional_bps / a.1.cost_notional)
                .total_cmp(&(b.1.all_in_cost_notional_bps / b.1.cost_notional))
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
    let post_only_range = (ORDER_POST_ONLY_RANGE
        + ORDER_POST_ONLY_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
        .clamp(0.0, 1.0);
    let gtd_range = (ORDER_GTD_RANGE
        + ORDER_GTD_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
        .clamp(0.0, 1.0);
    let limit_range = (ORDER_LIMIT_RANGE
        + ORDER_LIMIT_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
        .clamp(0.0, 1.0);
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
    learning: &BTreeMap<String, ExecutionLearningStats>,
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
        let mut scheduled = ExecutionLearningStats::default();
        let mut alternatives = ExecutionLearningStats::default();
        for (key, stats) in learning.iter().filter(|(key, _)| {
            key.starts_with(&format!("{context}:P")) && key.contains(&marker)
        }) {
            let mut recent = stats.clone();
            decay_execution_quality(&mut recent, current_time_ms());
            if key.ends_with(":exec:Vwap") || key.ends_with(":exec:Twap") {
                scheduled.samples += recent.samples;
                scheduled.filled_samples += recent.filled_samples;
                scheduled.fill_rate_total += recent.fill_rate_total;
                scheduled.cost_notional += recent.cost_notional;
                scheduled.all_in_cost_notional_bps += recent.all_in_cost_notional_bps;
            } else {
                alternatives.samples += recent.samples;
                alternatives.filled_samples += recent.filled_samples;
                alternatives.fill_rate_total += recent.fill_rate_total;
                alternatives.cost_notional += recent.cost_notional;
                alternatives.all_in_cost_notional_bps += recent.all_in_cost_notional_bps;
            }
        }
        if scheduled.samples < MIN_EXECUTION_QUALITY_SAMPLES
            || scheduled.filled_samples < MIN_EXECUTION_QUALITY_FILLS
            || scheduled.cost_notional <= 0.0
        {
            continue;
        }
        paired_samples += scheduled.samples + alternatives.samples;
        let scheduled_average =
            scheduled.all_in_cost_notional_bps / scheduled.cost_notional;
        let scheduled_fill_rate = scheduled.fill_rate_total / scheduled.samples as f64;
        if alternatives.samples >= MIN_EXECUTION_QUALITY_SAMPLES
            && alternatives.filled_samples >= MIN_EXECUTION_QUALITY_FILLS
            && alternatives.cost_notional > 0.0
        {
            let alternatives_average =
                alternatives.all_in_cost_notional_bps / alternatives.cost_notional;
            let alternatives_fill_rate = alternatives.fill_rate_total / alternatives.samples as f64;
            if scheduled_average + 0.5 < alternatives_average
                && scheduled_fill_rate + 0.05 >= alternatives_fill_rate
            {
                threshold = threshold.min(lower.max(MIN_SCHEDULE_DEPTH_RATIO));
            } else if scheduled_average > alternatives_average + 0.5
                && scheduled_fill_rate + 0.05 < alternatives_fill_rate
            {
                threshold = threshold.max(upper.min(MAX_SCHEDULE_DEPTH_RATIO));
            }
        }
    }
    (threshold.clamp(MIN_SCHEDULE_DEPTH_RATIO, MAX_SCHEDULE_DEPTH_RATIO), paired_samples)
}
