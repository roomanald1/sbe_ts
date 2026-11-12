use crate::{Signal, Strategy, StrategyParameterKey, parameter_specs};
use std::collections::BTreeMap;

/// Return the strategy/symbol/candle key used to deduplicate a daily strategy signal.
pub fn daily_strategy_signal_key(signal: &Signal) -> Option<(String, String, u64)> {
    let strategy = signal.strategy.as_deref()?;
    let decision = signal.strategy_decision.as_ref()?;
    let candle_or_anchor = if strategy == "risk-adjusted-monthly-momentum" {
        decision.get("monthly_anchor_close_time_ms")
    } else {
        decision.get("bar_close_timestamp")
    }?
    .as_u64()?;
    Some((strategy.to_owned(), signal.symbol.clone(), candle_or_anchor))
}

/// Validate a parameter probe carried by a daily candidate against the strategy catalog.
/// Ordinary candidates without the probe marker are accepted.
pub fn valid_scout_parameter_probe(signal: &Signal, strategy: Strategy) -> bool {
    let Some(decision) = signal.strategy_decision.as_ref() else {
        return false;
    };
    if decision
        .get("scout_parameter_probe")
        .and_then(serde_json::Value::as_bool)
        != Some(true)
    {
        return true;
    }
    let Some(key) = decision
        .get("scout_parameter_key")
        .and_then(serde_json::Value::as_str)
    else {
        return false;
    };
    let Some(value) = decision
        .get("scout_parameter_value")
        .and_then(serde_json::Value::as_f64)
    else {
        return false;
    };
    let expected_profile_symbol = if matches!(
        strategy,
        Strategy::RiskAdjustedTrendPortfolio | Strategy::RiskAdjustedMonthlyMomentum
    ) {
        "BTC-USDT"
    } else {
        signal.symbol.as_str()
    };
    if decision
        .get("scout_parameter_profile_symbol")
        .and_then(serde_json::Value::as_str)
        != Some(expected_profile_symbol)
    {
        return false;
    }
    if key.starts_with("combo:") {
        let Some(combination) = parse_scout_combination(key, strategy) else {
            return false;
        };
        return value == 0.0
            && combination.iter().all(|(parameter, expected)| {
                decision
                    .get("strategy_parameters")
                    .and_then(|parameters| parameters.get(parameter))
                    .and_then(serde_json::Value::as_f64)
                    .is_some_and(|actual| (actual - expected).abs() <= 1e-7)
            });
    }
    let Some(spec) = parameter_specs(strategy)
        .into_iter()
        .find(|spec| spec.key.as_str() == key)
    else {
        return false;
    };
    if spec.key == StrategyParameterKey::LearningRate
        || !value.is_finite()
        || value < spec.min
        || value > spec.max
    {
        return false;
    }
    let position = (value - spec.min) / spec.step;
    (position - position.round()).abs() <= 1e-6
        && decision
            .get("strategy_parameters")
            .and_then(|parameters| parameters.get(key))
            .and_then(serde_json::Value::as_f64)
            == Some(value)
}

pub fn parse_scout_combination(key: &str, strategy: Strategy) -> Option<BTreeMap<String, f64>> {
    let body = key.strip_prefix("combo:")?.split_once("|ctx:")?.0;
    let context = key.strip_prefix("combo:")?.rsplit_once("|ctx:")?.1;
    if context.len() != 16 || !context.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let specs = parameter_specs(strategy);
    let mut values = BTreeMap::new();
    for item in body.split(',') {
        let (name, raw) = item.split_once('=')?;
        let value = raw.parse::<f64>().ok()?;
        let spec = specs.iter().find(|spec| spec.key.as_str() == name)?;
        if spec.key == StrategyParameterKey::LearningRate
            || !value.is_finite()
            || value < spec.min
            || value > spec.max
        {
            return None;
        }
        let step = (value - spec.min) / spec.step;
        if (step - step.round()).abs() > 1e-6 || values.insert(name.to_owned(), value).is_some() {
            return None;
        }
    }
    (values.len() >= 2).then_some(values)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Signal, SignalComponents};

    fn probe(
        strategy_name: &str,
        symbol: &str,
        key: &str,
        value: f64,
        parameters: serde_json::Value,
    ) -> Signal {
        let mut candidate = signal(strategy_name, serde_json::json!({}));
        candidate.symbol = symbol.into();
        candidate.strategy_decision = Some(serde_json::json!({
            "scout_parameter_probe": true,
            "scout_parameter_key": key,
            "scout_parameter_value": value,
            "scout_parameter_profile_symbol": if matches!(strategy_name, "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") { "BTC-USDT" } else { symbol },
            "strategy_parameters": parameters,
        }));
        candidate
    }

    fn signal(strategy: &str, decision: serde_json::Value) -> Signal {
        Signal {
            symbol: "ETH-USDT".into(),
            direction: "bullish".into(),
            strength: 1.0,
            timestamp: 10,
            stale: false,
            score: 1.0,
            ranging_probability: None,
            components: SignalComponents::default(),
            strategy_candidate: true,
            strategy: Some(strategy.into()),
            strategy_decision: Some(decision),
            market_regime: None,
            news_attribution: None,
            entry_gate: None,
        }
    }

    #[test]
    fn uses_month_anchor_for_monthly_momentum() {
        let candidate = signal(
            "risk-adjusted-monthly-momentum",
            serde_json::json!({"monthly_anchor_close_time_ms": 123}),
        );
        assert_eq!(
            daily_strategy_signal_key(&candidate),
            Some((
                "risk-adjusted-monthly-momentum".into(),
                "ETH-USDT".into(),
                123
            ))
        );
    }

    #[test]
    fn uses_bar_close_for_other_daily_strategies() {
        let candidate = signal(
            "donchian-20-btc200",
            serde_json::json!({"bar_close_timestamp": 456}),
        );
        assert_eq!(
            daily_strategy_signal_key(&candidate),
            Some(("donchian-20-btc200".into(), "ETH-USDT".into(), 456))
        );
    }

    #[test]
    fn missing_strategy_or_close_timestamp_has_no_key() {
        let mut candidate = signal("donchian-20-btc200", serde_json::json!({}));
        assert_eq!(daily_strategy_signal_key(&candidate), None);
        candidate.strategy = None;
        assert_eq!(daily_strategy_signal_key(&candidate), None);
    }

    #[test]
    fn accepts_catalogued_scalar_probe() {
        let candidate = probe(
            "donchian-55-btc200",
            "ETH-USDT",
            "breakout_lookback_days",
            55.0,
            serde_json::json!({"breakout_lookback_days": 55.0}),
        );
        assert!(valid_scout_parameter_probe(
            &candidate,
            Strategy::Donchian55Btc200
        ));
    }

    #[test]
    fn accepts_bounded_combo_probe_and_parses_values() {
        let key = "combo:breakout_lookback_days=55.00000000,btc_trend_days=200.00000000|ctx:0123456789abcdef";
        let candidate = probe(
            "donchian-55-btc200",
            "ETH-USDT",
            key,
            0.0,
            serde_json::json!({"breakout_lookback_days": 55.0, "btc_trend_days": 200.0}),
        );
        assert_eq!(
            parse_scout_combination(key, Strategy::Donchian55Btc200)
                .unwrap()
                .len(),
            2
        );
        assert!(valid_scout_parameter_probe(
            &candidate,
            Strategy::Donchian55Btc200
        ));
    }

    #[test]
    fn rejects_unknown_or_out_of_bounds_probe_parameters() {
        let unknown = probe(
            "donchian-55-btc200",
            "ETH-USDT",
            "not_a_strategy_parameter",
            55.0,
            serde_json::json!({"not_a_strategy_parameter": 55.0}),
        );
        assert!(!valid_scout_parameter_probe(
            &unknown,
            Strategy::Donchian55Btc200
        ));
        let out_of_bounds = probe(
            "donchian-55-btc200",
            "ETH-USDT",
            "breakout_lookback_days",
            9.0,
            serde_json::json!({"breakout_lookback_days": 9.0}),
        );
        assert!(!valid_scout_parameter_probe(
            &out_of_bounds,
            Strategy::Donchian55Btc200
        ));
        assert!(parse_scout_combination(
            "combo:breakout_lookback_days=9.00000000,btc_trend_days=200.00000000|ctx:0123456789abcdef",
            Strategy::Donchian55Btc200,
        ).is_none());
    }

    #[test]
    fn requires_correct_strategy_profile_symbol() {
        let mut candidate = probe(
            "risk-adjusted-monthly-momentum",
            "ETH-USDT",
            "monthly_momentum_lookback_days",
            90.0,
            serde_json::json!({"monthly_momentum_lookback_days": 90.0}),
        );
        assert!(valid_scout_parameter_probe(
            &candidate,
            Strategy::RiskAdjustedMonthlyMomentum
        ));
        candidate.strategy_decision.as_mut().unwrap()["scout_parameter_profile_symbol"] =
            serde_json::json!("ETH-USDT");
        assert!(!valid_scout_parameter_probe(
            &candidate,
            Strategy::RiskAdjustedMonthlyMomentum
        ));
    }
}
