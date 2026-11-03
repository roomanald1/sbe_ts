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

fn profitable_close_probability_5m(
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

    fn position(quantity: f64, entry: f64) -> TraderPosition {
        TraderPosition { quantity, average_entry_price: entry, ..TraderPosition::default() }
    }

    fn samples(increasing: bool) -> Vec<serde_json::Value> {
        (0..31).map(|index| {
            let movement = index as f64 * 0.05 + (index % 2) as f64 * 0.01;
            let price = if increasing { 100.0 + movement } else { 100.0 - movement };
            serde_json::json!([index * 10_000, price])
        }).collect()
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
        let long = profitable_close_probability_5m(&long_market, &position(1.0, 100.0)).unwrap();
        assert!(long > 0.5 && long <= 1.0);

        let short_market = serde_json::json!({
            "bids": [], "asks": [{"price":90.0,"size":2.0}], "price_samples": samples(false)
        });
        let short = profitable_close_probability_5m(&short_market, &position(-1.0, 100.0)).unwrap();
        assert!(short > 0.5 && short <= 1.0);
    }

    #[test]
    fn probability_fails_closed_for_missing_depth_invalid_levels_short_history_or_zero_variance() {
        let good = samples(true);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[],"price_samples":good}), &position(1.0, 100.0)), None);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[{"price":100.0,"size":0.5}],"price_samples":samples(true)}), &position(1.0, 100.0)), None);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":[[0,100.0]]}), &position(1.0, 100.0)), None);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[{"price":-1.0,"size":2.0}],"price_samples":samples(true)}), &position(1.0, 100.0)), None);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":(0..31).map(|i| serde_json::json!([i*10_000,100.0])).collect::<Vec<_>>()}), &position(1.0, 100.0)), None);
        assert_eq!(profitable_close_probability_5m(&serde_json::json!({"bids":[{"price":100.0,"size":2.0}],"price_samples":samples(true)}), &position(0.0, 100.0)), None);
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
