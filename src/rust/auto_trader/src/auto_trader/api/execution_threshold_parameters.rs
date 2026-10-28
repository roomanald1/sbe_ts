fn order_selection_thresholds(risk_appetite: f64) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Order selection",
            "Risk appetite for aggressive order priority",
            AGGRESSIVE_ORDER_PRIORITY_APPETITE,
            "%",
        ),
        parameter(
            "Order selection",
            "Range probability for post-only limit",
            (ORDER_POST_ONLY_RANGE
                + ORDER_POST_ONLY_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
                .clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "Aggressor cost for post-only limit",
            ORDER_POST_ONLY_AGGRESSOR_COST_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for GTD",
            (ORDER_GTD_RANGE + ORDER_GTD_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
                .clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for GTD lower bound",
            ORDER_GTD_MIN_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Range probability for limit",
            (ORDER_LIMIT_RANGE + ORDER_LIMIT_RANGE_RISK_ADJUSTMENT * risk_appetite / 100.0)
                .clamp(0.0, 1.0),
            "probability",
        ),
        parameter(
            "Order selection",
            "VWAP gap for limit",
            ORDER_GTD_MAX_MISPRICING_BPS,
            "bps",
        ),
        parameter(
            "Order selection",
            "Volatility for stop-limit",
            ORDER_STOP_LIMIT_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Trend for stop-limit",
            ORDER_STOP_LIMIT_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Trend for stop-market",
            ORDER_STOP_MARKET_TREND_UNITS,
            "volatility units",
        ),
        parameter(
            "Order selection",
            "Strength for stop-market",
            ORDER_STOP_MARKET_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Pressure for market order",
            ORDER_MARKET_PRESSURE,
            "pressure",
        ),
        parameter(
            "Order selection",
            "Strength for market order",
            ORDER_MARKET_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Momentum for IOC",
            ORDER_IOC_MOMENTUM_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for IOC",
            ORDER_IOC_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Strength for VWAP",
            ORDER_VWAP_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
        parameter(
            "Order selection",
            "Range probability for TWAP",
            ORDER_TWAP_RANGE,
            "probability",
        ),
        parameter(
            "Order selection",
            "Volatility for trailing stop",
            ORDER_TRAILING_VOLATILITY_PERCENT,
            "%",
        ),
        parameter(
            "Order selection",
            "Strength for FOK",
            ORDER_FOK_STRENGTH * risk_floor_scale(risk_appetite),
            "strength",
        ),
    ]
}

fn market_context_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Market context",
            "Range regime probability",
            CONTEXT_RANGE_MIN_PROBABILITY,
            "probability",
        ),
        parameter(
            "Market context",
            "Trend regime volatility units",
            CONTEXT_TREND_MIN_VOLATILITY_UNITS,
            "volatility units",
        ),
        parameter(
            "Market context",
            "Volatile regime threshold",
            CONTEXT_VOLATILE_MIN_PERCENT,
            "%",
        ),
    ]
}

fn learning_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Learning",
            "Minimum execution learning samples",
            MIN_EXECUTION_LEARNING_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Learning",
            "Minimum profitable order-type hit rate",
            0.55,
            "rate",
        ),
        parameter(
            "Learning",
            "Number of parameter profiles",
            PARAMETER_PROFILE_COUNT as f64,
            "profiles",
        ),
    ]
}

fn risk_and_hedging_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Risk and hedging",
            "Portfolio shock threshold",
            PORTFOLIO_SHOCK_PERCENT * 100.0,
            "%",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation samples",
            MIN_HEDGE_CORRELATION_SAMPLES as f64,
            "samples",
        ),
        parameter(
            "Risk and hedging",
            "Minimum hedge correlation",
            MIN_HEDGE_CORRELATION,
            "correlation",
        ),
    ]
}

fn risk_and_re_entry_thresholds(max_entries_per_auto_position: u32) -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Risk and re-entry",
            "Entries per position",
            max_entries_per_auto_position as f64,
            "entries",
        ),
        parameter(
            "Risk and re-entry",
            "Deep-book impact ceiling",
            DEEP_BOOK_BAND_BPS,
            "bps",
        ),
        parameter(
            "Risk and re-entry",
            "Maximum depth participation",
            DEEP_BOOK_MAX_PARTICIPATION * 100.0,
            "%",
        ),
        parameter(
            "Risk and re-entry",
            "Minimum deep-book order",
            MIN_DEEP_BOOK_ORDER_NOTIONAL,
            "USD",
        ),
    ]
}

fn exit_policy_thresholds() -> Vec<DecisionParameter> {
    vec![
        parameter(
            "Exit policy",
            "Safe target minimum reward/risk ratio",
            MIN_EXIT_REWARD_RISK_RATIO,
            "ratio",
        ),
        parameter(
            "Exit policy",
            "Minimum net take-profit at maximum risk appetite",
            SMALL_NET_TAKE_PROFIT_PERCENT,
            "% after fees",
        ),
        parameter(
            "Exit policy",
            "Stop-loss minimum",
            MIN_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Stop-loss maximum",
            MAX_STOP_LOSS_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Take-profit minimum",
            MIN_TAKE_PROFIT_PERCENT,
            "%",
        ),
        parameter(
            "Exit policy",
            "Take-profit maximum",
            MAX_TAKE_PROFIT_PERCENT,
            "%",
        ),
    ]
}

fn parameter(section: &str, name: &str, value: f64, unit: &str) -> DecisionParameter {
    DecisionParameter {
        section: section.to_owned(),
        name: name.to_owned(),
        value,
        unit: unit.to_owned(),
    }
}
