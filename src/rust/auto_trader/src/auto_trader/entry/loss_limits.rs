async fn enforce_daily_realized_loss_limit(
    state: &AutoTraderState,
) -> Result<Option<CycleResult>, (StatusCode, String)> {
    let mut settings = state.settings.lock().await;
    reset_daily_budget(&mut settings);
    if settings.daily_realized_pnl > -settings.daily_realized_loss_limit {
        return Ok(None);
    }
    if !settings.daily_loss_limit_reduced {
        settings.daily_realized_loss_limit = (settings.daily_realized_loss_limit * 0.9)
            .max(MIN_DAILY_REALIZED_LOSS_LIMIT.min(settings.daily_realized_loss_limit));
        settings.daily_loss_limit_reduced = true;
        let _ = request_settings_persist(state);
    }
    Ok(Some(skipped(&format!(
        "Daily realized-loss limit reached at USD {:.2}; learned daily cap is now USD {:.2}",
        settings.daily_realized_pnl, settings.daily_realized_loss_limit
    ))))
}

fn unvalidated_profitable_close_probability_5m(
    market: &serde_json::Value,
    position: &TraderPosition,
) -> Option<f64> {
    let closing_long = position.quantity > 0.0;
    let levels = if closing_long {
        market.get("bids")?.as_array()?
    } else {
        market.get("asks")?.as_array()?
    };
    let mut remaining = position.quantity.abs();
    let mut notional = 0.0;
    for level in levels {
        let price = level.get("price")?.as_f64()?;
        let size = level.get("size")?.as_f64()?;
        if !price.is_finite() || price <= 0.0 || !size.is_finite() || size < 0.0 {
            return None;
        }
        let quantity = remaining.min(size);
        notional += quantity * price;
        remaining -= quantity;
        if remaining <= 1e-12 {
            break;
        }
    }
    if remaining > 1e-12 || position.quantity.abs() <= 1e-12 {
        return None;
    }
    let close_price = notional / position.quantity.abs();
    if !close_price.is_finite() || close_price <= 0.0 {
        return None;
    }
    let samples = market.get("price_samples")?.as_array()?;
    if samples.len() < 31 {
        return None;
    }
    let parsed = samples
        .iter()
        .map(|sample| {
            let pair = sample.as_array()?;
            Some((pair.first()?.as_u64()?, pair.get(1)?.as_f64()?))
        })
        .collect::<Option<Vec<_>>>()?;
    let returns = parsed
        .windows(2)
        .filter_map(|pair| {
            let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
            (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0)
                .then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
        })
        .collect::<Vec<_>>();
    if returns.len() < 30 {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns
        .iter()
        .map(|value| (value - mean).powi(2))
        .sum::<f64>()
        / (returns.len() - 1) as f64;
    let projected_mean = close_price.ln() + mean * 30.0;
    let projected_sd = (variance * 30.0).sqrt();
    if !projected_sd.is_finite() || projected_sd <= 0.0 {
        return None;
    }
    let fee_rate = configured_taker_fee_rate();
    let breakeven_price = if closing_long {
        (position.average_entry_price + position.entry_fees / position.quantity.abs())
            / (1.0 - fee_rate)
    } else {
        (position.average_entry_price - position.entry_fees / position.quantity.abs())
            / (1.0 + fee_rate)
    };
    if !breakeven_price.is_finite() || breakeven_price <= 0.0 {
        return None;
    }
    let below = normal_cdf((breakeven_price.ln() - projected_mean) / projected_sd);
    Some(if closing_long { 1.0 - below } else { below })
}

#[derive(Clone, Debug, PartialEq)]
struct ValidatedCloseForecast {
    status: &'static str,
    eligible: bool,
    forecast_id: Option<String>,
    model_id: Option<String>,
    validation_id: Option<String>,
    as_of_ms: Option<u64>,
    age_ms: Option<u64>,
    max_age_ms: Option<u64>,
    validation_effective_samples: Option<f64>,
    calibration_error: Option<f64>,
    probability_lower_bound: Option<f64>,
    net_return_lower_bound_percent: Option<f64>,
}

fn validated_close_forecast_5m(
    market: &serde_json::Value,
    symbol: &str,
    strategy: &str,
    profile_id: Option<&str>,
    direction: &str,
    market_regime: Option<&str>,
    now_ms: u64,
) -> ValidatedCloseForecast {
    let rejected = |status| ValidatedCloseForecast {
        status,
        eligible: false,
        forecast_id: None,
        model_id: None,
        validation_id: None,
        as_of_ms: None,
        age_ms: None,
        max_age_ms: None,
        validation_effective_samples: None,
        calibration_error: None,
        probability_lower_bound: None,
        net_return_lower_bound_percent: None,
    };
    let Some(forecast) = market.get("profit_close_forecast_v1") else {
        return rejected("missing_forecast");
    };
    if forecast["schema_version"].as_u64() != Some(1) {
        return rejected("unsupported_schema");
    }
    let (Some(profile_id), Some(market_regime)) = (profile_id, market_regime) else {
        return rejected("missing_decision_scope");
    };
    if forecast["symbol"].as_str() != Some(symbol)
        || forecast["strategy"].as_str() != Some(strategy)
        || forecast["profile_id"].as_str() != Some(profile_id)
        || forecast["direction"].as_str() != Some(direction)
        || forecast["market_regime"].as_str() != Some(market_regime)
    {
        return rejected("scope_mismatch");
    }
    if forecast["horizon_seconds"].as_u64() != Some(300) {
        return rejected("wrong_horizon");
    }
    let Some(forecast_id) = forecast["forecast_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    else {
        return rejected("missing_forecast_identity");
    };
    let Some(model_id) = forecast["model_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    else {
        return rejected("missing_model_identity");
    };
    let Some(validation_id) = forecast["validation_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
    else {
        return rejected("missing_validation_identity");
    };
    if forecast["decision_eligible"] != true
        || forecast["evidence_owner"].as_str() != Some("learning")
        || forecast["calibration_status"].as_str() != Some("validated")
        || forecast["out_of_sample_validation"] != true
        || forecast["includes_execution_costs"] != true
    {
        return rejected("forecast_not_validated_for_decision");
    }
    let Some(validation_effective_samples) = forecast["validation_effective_samples"].as_f64()
    else {
        return rejected("missing_validation_coverage");
    };
    let Some(calibration_error) = forecast["calibration_error"].as_f64() else {
        return rejected("missing_calibration_quality");
    };
    if !validation_effective_samples.is_finite()
        || validation_effective_samples <= 0.0
        || !calibration_error.is_finite()
        || calibration_error < 0.0
    {
        return rejected("invalid_calibration_quality");
    }
    let Some(as_of_ms) = forecast["as_of_ms"].as_u64() else {
        return rejected("missing_forecast_timestamp");
    };
    let Some(market_updated_at_ms) = market["updated_at_ms"].as_u64() else {
        return rejected("missing_market_timestamp");
    };
    let Some(max_age_ms) = market["symbol_freshness"]["max_age_ms"].as_u64() else {
        return rejected("missing_market_freshness");
    };
    if max_age_ms == 0
        || market["symbol_freshness"]["contract_version"].as_u64() != Some(1)
        || market["symbol_freshness"]["state"].as_str() != Some("fresh")
        || market["symbol"].as_str() != Some(symbol)
        || market_updated_at_ms > now_ms
        || as_of_ms > market_updated_at_ms
        || now_ms.saturating_sub(as_of_ms) > max_age_ms
    {
        return rejected("stale_or_unaligned_forecast");
    }
    let Some(probability_lower_bound) = forecast["probability_lower_bound"].as_f64() else {
        return rejected("missing_probability_bound");
    };
    let Some(net_return_lower_bound_percent) = forecast["net_return_lower_bound_percent"].as_f64()
    else {
        return rejected("missing_net_return_bound");
    };
    if !probability_lower_bound.is_finite()
        || !(0.0..=1.0).contains(&probability_lower_bound)
        || !net_return_lower_bound_percent.is_finite()
        || net_return_lower_bound_percent <= 0.0
    {
        return rejected("nonpositive_or_invalid_forecast_bound");
    }
    ValidatedCloseForecast {
        status: "validated",
        eligible: true,
        forecast_id: Some(forecast_id.to_owned()),
        model_id: Some(model_id.to_owned()),
        validation_id: Some(validation_id.to_owned()),
        as_of_ms: Some(as_of_ms),
        age_ms: Some(now_ms.saturating_sub(as_of_ms)),
        max_age_ms: Some(max_age_ms),
        validation_effective_samples: Some(validation_effective_samples),
        calibration_error: Some(calibration_error),
        probability_lower_bound: Some(probability_lower_bound),
        net_return_lower_bound_percent: Some(net_return_lower_bound_percent),
    }
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0
        - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t)
            * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}

fn missing_fx_rate_error(
    scope: &str,
    pnl_by_quote: &BTreeMap<String, f64>,
    rates: &BTreeMap<String, f64>,
) -> String {
    let missing = pnl_by_quote
        .keys()
        .filter(|quote| {
            !rates
                .get(*quote)
                .is_some_and(|rate| rate.is_finite() && *rate > 0.0)
        })
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        format!(
            "{scope} is unavailable from the position ledger: USD conversion returned no total although cached rates cover quote currencies {}",
            pnl_by_quote.keys().cloned().collect::<Vec<_>>().join(", ")
        )
    } else {
        format!(
            "{scope} is unavailable from the position ledger: missing FX rates for quote currencies {}",
            missing.join(", ")
        )
    }
}

#[cfg(test)]
mod loss_limit_probability_tests {
    use super::*;

    fn validated_forecast_market() -> serde_json::Value {
        serde_json::json!({
            "symbol": "BTC-USDT",
            "updated_at_ms": 1_000,
            "symbol_freshness": {
                "contract_version": 1,
                "state": "fresh",
                "max_age_ms": 5_000
            },
            "profit_close_forecast_v1": {
                "schema_version": 1,
                "forecast_id": "forecast-1",
                "model_id": "normal-close-v1",
                "validation_id": "validation-1",
                "symbol": "BTC-USDT",
                "strategy": "rsi2-oversold-rebound",
                "profile_id": "profile-1",
                "direction": "long",
                "market_regime": "trend",
                "horizon_seconds": 300,
                "as_of_ms": 1_000,
                "decision_eligible": true,
                "evidence_owner": "learning",
                "calibration_status": "validated",
                "out_of_sample_validation": true,
                "includes_execution_costs": true,
                "validation_effective_samples": 24.0,
                "calibration_error": 0.04,
                "probability_lower_bound": 0.82,
                "net_return_lower_bound_percent": 0.12
            }
        })
    }

    fn validate_forecast(market: &serde_json::Value, now_ms: u64) -> ValidatedCloseForecast {
        validated_close_forecast_5m(
            market,
            "BTC-USDT",
            "rsi2-oversold-rebound",
            Some("profile-1"),
            "long",
            Some("trend"),
            now_ms,
        )
    }

    #[test]
    fn close_forecast_requires_versioned_calibrated_costed_point_in_time_scope() {
        let market = validated_forecast_market();
        let valid = validate_forecast(&market, 1_001);
        assert!(valid.eligible);
        assert_eq!(valid.status, "validated");
        assert_eq!(valid.forecast_id.as_deref(), Some("forecast-1"));
        assert_eq!(valid.model_id.as_deref(), Some("normal-close-v1"));
        assert_eq!(valid.validation_id.as_deref(), Some("validation-1"));
        assert_eq!(valid.age_ms, Some(1));
        assert_eq!(valid.max_age_ms, Some(5_000));
        assert_eq!(valid.validation_effective_samples, Some(24.0));
        assert_eq!(valid.calibration_error, Some(0.04));
        assert_eq!(valid.probability_lower_bound, Some(0.82));
        assert_eq!(valid.net_return_lower_bound_percent, Some(0.12));

        assert_eq!(
            validate_forecast(&serde_json::json!({"symbol":"BTC-USDT"}), 1_001).status,
            "missing_forecast"
        );

        let mut stale = market.clone();
        stale["profit_close_forecast_v1"]["as_of_ms"] = serde_json::json!(0);
        assert_eq!(
            validate_forecast(&stale, 6_001).status,
            "stale_or_unaligned_forecast"
        );

        let mut wrong_scope = market.clone();
        wrong_scope["profit_close_forecast_v1"]["profile_id"] = serde_json::json!("other");
        assert_eq!(
            validate_forecast(&wrong_scope, 1_001).status,
            "scope_mismatch"
        );

        assert_eq!(
            validated_close_forecast_5m(
                &market,
                "BTC-USDT",
                "rsi2-oversold-rebound",
                None,
                "long",
                Some("trend"),
                1_001,
            )
            .status,
            "missing_decision_scope"
        );

        let mut wrong_strategy = market.clone();
        wrong_strategy["profit_close_forecast_v1"]["strategy"] =
            serde_json::json!("other-strategy");
        assert_eq!(
            validate_forecast(&wrong_strategy, 1_001).status,
            "scope_mismatch"
        );

        let mut wrong_horizon = market.clone();
        wrong_horizon["profit_close_forecast_v1"]["horizon_seconds"] = serde_json::json!(60);
        assert_eq!(
            validate_forecast(&wrong_horizon, 1_001).status,
            "wrong_horizon"
        );

        let mut wrong_direction = market.clone();
        wrong_direction["profit_close_forecast_v1"]["direction"] = serde_json::json!("short");
        assert_eq!(
            validate_forecast(&wrong_direction, 1_001).status,
            "scope_mismatch"
        );

        let mut wrong_regime = market.clone();
        wrong_regime["profit_close_forecast_v1"]["market_regime"] = serde_json::json!("range");
        assert_eq!(
            validate_forecast(&wrong_regime, 1_001).status,
            "scope_mismatch"
        );

        let mut future_forecast = market.clone();
        future_forecast["profit_close_forecast_v1"]["as_of_ms"] = serde_json::json!(1_002);
        assert_eq!(
            validate_forecast(&future_forecast, 1_001).status,
            "stale_or_unaligned_forecast"
        );

        let mut zero_age_window = market.clone();
        zero_age_window["symbol_freshness"]["max_age_ms"] = serde_json::json!(0);
        assert_eq!(
            validate_forecast(&zero_age_window, 1_001).status,
            "stale_or_unaligned_forecast"
        );

        let mut uncalibrated = market.clone();
        uncalibrated["profit_close_forecast_v1"]["calibration_status"] =
            serde_json::json!("provisional");
        assert_eq!(
            validate_forecast(&uncalibrated, 1_001).status,
            "forecast_not_validated_for_decision"
        );

        let mut costs_unknown = market.clone();
        costs_unknown["profit_close_forecast_v1"]["includes_execution_costs"] =
            serde_json::json!(false);
        assert_eq!(
            validate_forecast(&costs_unknown, 1_001).status,
            "forecast_not_validated_for_decision"
        );

        let mut nonpositive_net_bound = market;
        nonpositive_net_bound["profit_close_forecast_v1"]["net_return_lower_bound_percent"] =
            serde_json::json!(0.0);
        assert_eq!(
            validate_forecast(&nonpositive_net_bound, 1_001).status,
            "nonpositive_or_invalid_forecast_bound"
        );
    }

    fn position(quantity: f64, entry: f64) -> TraderPosition {
        TraderPosition {
            quantity,
            average_entry_price: entry,
            ..TraderPosition::default()
        }
    }

    fn samples(increasing: bool) -> Vec<serde_json::Value> {
        (0..31)
            .map(|index| {
                let movement = index as f64 * 0.05 + (index % 2) as f64 * 0.01;
                let price = if increasing {
                    100.0 + movement
                } else {
                    100.0 - movement
                };
                serde_json::json!([index * 10_000, price])
            })
            .collect()
    }

    #[test]
    fn normal_cdf_is_symmetric_and_has_expected_center_and_tails() {
        assert!((normal_cdf(0.0) - 0.5).abs() < 1e-6);
        assert!((normal_cdf(1.0) + normal_cdf(-1.0) - 1.0).abs() < 1e-6);
        assert!(normal_cdf(-5.0) < 0.001);
        assert!(normal_cdf(5.0) > 0.999);
    }

    #[test]
    fn profitable_close_probability_uses_bid_for_longs_ask_for_shorts_and_price_history() {
        let long_market = serde_json::json!({
            "bids": [{"price":110.0,"size":2.0}], "asks": [], "price_samples": samples(true)
        });
        let long = unvalidated_profitable_close_probability_5m(&long_market, &position(1.0, 100.0))
            .unwrap();
        assert!(long > 0.5 && long <= 1.0);

        let short_market = serde_json::json!({
            "bids": [], "asks": [{"price":90.0,"size":2.0}], "price_samples": samples(false)
        });
        let short =
            unvalidated_profitable_close_probability_5m(&short_market, &position(-1.0, 100.0))
                .unwrap();
        assert!(short > 0.5 && short <= 1.0);
    }

    #[test]
    fn probability_fails_closed_for_missing_depth_invalid_levels_short_history_or_zero_variance() {
        let good = samples(true);
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[],"price_samples":good}),
                &position(1.0, 100.0)
            ),
            None
        );
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[{"price":100.0,"size":0.5}],"price_samples":samples(true)}),
                &position(1.0, 100.0)
            ),
            None
        );
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":[[0,100.0]]}),
                &position(1.0, 100.0)
            ),
            None
        );
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[{"price":-1.0,"size":2.0}],"price_samples":samples(true)}),
                &position(1.0, 100.0)
            ),
            None
        );
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":(0..31).map(|i| serde_json::json!([i*10_000,100.0])).collect::<Vec<_>>()}),
                &position(1.0, 100.0)
            ),
            None
        );
        assert_eq!(
            unvalidated_profitable_close_probability_5m(
                &serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":samples(true)}),
                &position(0.0, 100.0)
            ),
            None
        );
    }

    #[test]
    fn missing_fx_error_distinguishes_uncached_and_cached_quote_rates() {
        let pnl = BTreeMap::from([("USDT".to_owned(), 2.0), ("EUR".to_owned(), -1.0)]);
        let missing = missing_fx_rate_error("Realized PnL", &pnl, &BTreeMap::new());
        assert!(missing.contains("missing FX rates"));
        assert!(missing.contains("EUR, USDT"));
        let cached = BTreeMap::from([("EUR".to_owned(), 1.1), ("USDT".to_owned(), 1.0)]);
        let unavailable = missing_fx_rate_error("Realized PnL", &pnl, &cached);
        assert!(unavailable.contains("cached rates cover quote currencies EUR, USDT"));
    }
}
