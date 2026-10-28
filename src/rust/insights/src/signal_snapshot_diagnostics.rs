struct SnapshotReasonInputs<'a> {
    trend: Option<f64>,
    momentum: Option<f64>,
    flow: Option<f64>,
    book_imbalance: Option<f64>,
    order_book_pressure_signal: Option<f64>,
    volatility_breakout_signal: Option<f64>,
    mispricing_vs_vwap_bps: Option<f64>,
    range_bounds: Option<(f64, f64)>,
    mean_reversion_signal: Option<f64>,
    price: f64,
    absorption_imbalance: Option<f64>,
    flow_without_move: Option<f64>,
    aggressor_cost_bps: Option<f64>,
    direction: &'a str,
    components: &'a SignalComponents,
    ranging_probability: Option<f64>,
    prior_low: f64,
    prior_high: f64,
    breakout_clearance_percent: f64,
}

fn snapshot_reasons(input: SnapshotReasonInputs<'_>) -> Vec<String> {
    let SnapshotReasonInputs {
        trend, momentum, flow, book_imbalance, order_book_pressure_signal,
        volatility_breakout_signal, mispricing_vs_vwap_bps, range_bounds, mean_reversion_signal, price,
        absorption_imbalance, flow_without_move, aggressor_cost_bps, direction,
        components, ranging_probability, prior_low, prior_high, breakout_clearance_percent,
    } = input;
    let mut reasons = Vec::new();
    if let Some(v) = trend {
        reasons.push(format!("1m price change {v:.3}%"));
    }
    if let Some(v) = momentum {
        reasons.push(format!("10s price change {v:.3}%"));
    }
    if let Some(v) = flow {
        reasons.push(format!("buy/sell trade imbalance {v:.2}"));
    }
    if let Some(v) = book_imbalance {
        reasons.push(format!("top-book imbalance {v:.2}"));
    }
    if let Some(vote) = order_book_pressure_signal {
        reasons.push(format!(
        "strategy_vote:order_book_pressure;direction={};vote={vote:.2};book_imbalance={:.3};trade_flow={:.3};thresholds=abs_book>=0.600,abs_flow>=0.250",
        if vote > 0.0 { "bullish" } else { "bearish" },
        book_imbalance.unwrap_or_default(),
        flow.unwrap_or_default()
        ));
    }
    if let Some(vote) = volatility_breakout_signal {
        reasons.push(format!(
        "strategy_vote:volatility_breakout;direction={};vote={vote:.2};price={:.8};prior_low={prior_low:.8};prior_high={prior_high:.8};clearance_percent={breakout_clearance_percent:.4};momentum_10s_percent={:.4};trade_flow={:.3};thresholds=prior_samples>=12,clearance>=max(0.20*volatility_1m,0.0200%),abs_trade_flow>=0.300",
        if vote > 0.0 { "bullish" } else { "bearish" },
        price,
        momentum.unwrap_or_default(),
        flow.unwrap_or_default()
        ));
    }
    if let Some(v) = mispricing_vs_vwap_bps.filter(|value| value.abs() >= 8.0) {
        let relation = if v > 0.0 { "above" } else { "below" };
        reasons.push(format!(
        "Price {:.1} bps {relation} 60s trade VWAP",
        v.abs()
        ));
    }
    if let Some((low, high)) = range_bounds {
        if let Some(vote) = mean_reversion_signal {
        let location = (price - low) / (high - low);
        reasons.push(format!(
            "strategy_vote:mean_reversion;direction={};vote={vote:.2};price={:.8};range_low={low:.8};range_high={high:.8};range_location={location:.3};ranging_probability={:.3};thresholds=range_probability>=0.700,edge_fraction=0.120",
            if vote > 0.0 { "bullish" } else { "bearish" },
            price,
            ranging_probability.unwrap_or_default()
        ));
        }
    }
    if let Some(v) = absorption_imbalance.filter(|value| value.abs() >= 0.25) {
        reasons.push(format!(
        "{} aggression met replenishing touch liquidity while price stayed flat",
        if v > 0.0 { "Sell" } else { "Buy" }
        ));
    } else if let Some(v) = flow_without_move {
        let pressure = if v < 0.0 { "buying" } else { "selling" };
        reasons.push(format!(
        "Strong {pressure} pressure met little price movement"
        ));
    }
    if let Some(v) = aggressor_cost_bps.filter(|value| *value >= 5.0) {
        reasons.push(format!(
        "Aggressive traders paid an average {:.1} bps versus midpoint",
        v
        ));
    }
    if direction == "ranging" {
        if let Some((low, high)) = range_bounds {
        reasons.push(format!(
            "Price is ranging between {:.8} and {:.8} over five minutes",
            low, high
        ));
        if let Some((side, pressure)) =
            breakout_watch(&components, range_bounds, ranging_probability)
        {
            reasons.push(format!(
            "Breakout watch: {side} · {:.0}% pressure (heuristic, not calibrated)",
            pressure * 100.0
            ));
        }
        }
    }
    reasons
}
