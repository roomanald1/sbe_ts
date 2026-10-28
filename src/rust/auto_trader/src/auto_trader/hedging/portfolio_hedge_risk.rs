#[derive(Clone)]
struct RiskHolding {
    symbol: String,
    signed_usd: f64,
}

#[derive(serde::Deserialize)]
struct HedgePositionData {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    latest_price: Option<f64>,
    candidates: Vec<HedgeCandidateData>,
}

#[derive(serde::Deserialize)]
struct HedgeCandidateData {
    symbol: String,
    correlation: Option<f64>,
    paired_samples: usize,
    latest_price: Option<f64>,
}

#[derive(Default, serde::Deserialize)]
struct HedgePairCorrelationData {
    symbol_a: String,
    symbol_b: String,
    correlation: Option<f64>,
}

#[derive(serde::Deserialize)]
struct HedgeCandidatesData {
    positions: Vec<HedgePositionData>,
    pair_correlations: Vec<HedgePairCorrelationData>,
}

fn calculate_risk_holdings(
    positions: &[serde_json::Value],
    rates: &serde_json::Value,
) -> (Vec<RiskHolding>, usize, usize) {
    let mut holdings = Vec::new();
    let mut fallback_marks = 0;
    for position in positions {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let quantity = position["quantity"].as_f64().unwrap_or(0.0);
        let live_price = position["latest_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let entry_price = position["average_entry_price"]
            .as_f64()
            .filter(|price| price.is_finite() && *price > 0.0);
        let quote = symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or_default();
        let rate = rates[quote].as_f64().unwrap_or(0.0);
        if quantity.is_finite() && rate.is_finite() && rate > 0.0 {
            if let Some(price) = live_price.or(entry_price) {
                if live_price.is_none() {
                    fallback_marks += 1;
                }
                holdings.push(RiskHolding {
                    symbol: symbol.to_owned(),
                    signed_usd: quantity * price * rate,
                });
            }
        }
    }
    let unknown_marks = positions.len().saturating_sub(holdings.len());
    (holdings, fallback_marks, unknown_marks)
}

async fn update_risk_coverage_note(
    state: &AutoTraderState,
    position_count: usize,
    fallback_marks: usize,
    unknown_marks: usize,
) {
    let mut settings = state.settings.lock().await;
    settings.last_risk_note = match (fallback_marks, unknown_marks) {
        (fallback, unknown) if unknown > 0 => format!(
            "risk covers {position_count} open positions; {fallback} use entry-price marks and {unknown} are unmarked, so hedging is paused"
        ),
        (fallback, _) if fallback > 0 => {
            format!("risk covers {position_count} open positions; {fallback} use entry-price marks")
        }
        _ => format!("risk covers {position_count} open positions"),
    };
    settings.risk_limited_sizing = unknown_marks > 0;
}

fn calculate_portfolio_variance(
    holdings: &[RiskHolding],
    pair_correlations: &[HedgePairCorrelationData],
) -> f64 {
    let mut portfolio_variance = holdings
        .iter()
        .map(|holding| holding.signed_usd.powi(2))
        .sum::<f64>();
    for i in 0..holdings.len() {
        for j in (i + 1)..holdings.len() {
            if let Some(correlation) = pair_correlations
                .iter()
                .find(|pair| {
                    (pair.symbol_a == holdings[i].symbol && pair.symbol_b == holdings[j].symbol)
                        || (pair.symbol_b == holdings[i].symbol
                            && pair.symbol_a == holdings[j].symbol)
                })
                .and_then(|pair| pair.correlation)
                .filter(|value| value.is_finite())
            {
                portfolio_variance += 2.0
                    * holdings[i].signed_usd
                    * holdings[j].signed_usd
                    * correlation.clamp(-1.0, 1.0);
            }
        }
    }
    portfolio_variance
}

fn select_portfolio_hedge_candidate(
    holdings: &[RiskHolding],
    positions: &[serde_json::Value],
    rates: &serde_json::Value,
    settings: &Settings,
    now_ms: u64,
    portfolio_shock_risk: f64,
) -> Option<(String, String, f64, f64, f64, f64)> {
    let mut best: Option<(String, String, f64, f64, f64, f64)> = None;
    // Reduce the largest direct holding first, before considering a correlated proxy.
    if let Some(holding) = holdings
        .iter()
        .max_by(|a, b| a.signed_usd.abs().total_cmp(&b.signed_usd.abs()))
    {
        if holding.signed_usd.abs() > 0.0 {
            let row = positions
                .iter()
                .find(|row| row["symbol"].as_str() == Some(&holding.symbol));
            let price = row
                .and_then(|row| {
                    row["latest_price"]
                        .as_f64()
                        .or_else(|| row["average_entry_price"].as_f64())
                })
                .unwrap_or(0.0);
            let quote = holding
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            let last = settings
                .last_hedge_at
                .get(&holding.symbol)
                .copied()
                .unwrap_or(0);
            if price > 0.0 && quote_rate > 0.0 && now_ms.saturating_sub(last) >= 300_000 {
                best = Some((
                    holding.symbol.clone(),
                    if holding.signed_usd > 0.0 {
                        "sell"
                    } else {
                        "buy"
                    }
                    .into(),
                    holding.signed_usd.abs().min(settings.max_order_notional),
                    price,
                    quote_rate,
                    holding.signed_usd.abs(),
                ));
            }
        }
    }
    for position in positions {
        if best.is_some() {
            break;
        }
        let candidate_rows = position["candidates"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        for candidate in candidate_rows {
            let hedge_symbol = candidate["symbol"].as_str().unwrap_or_default();
            let hedge_price = candidate["latest_price"].as_f64().unwrap_or(0.0);
            let quote = hedge_symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or_default();
            let quote_rate = rates[quote].as_f64().unwrap_or(0.0);
            if hedge_price <= 0.0 || quote_rate <= 0.0 {
                continue;
            }
            let mut covariance = 0.0_f64;
            let mut usable_correlations = 0;
            let mut strongest = 0.0_f64;
            let mut enough_samples = false;
            let mut complete_sample = true;
            for holding in holdings {
                let relation = if holding.symbol == hedge_symbol {
                    Some((1.0, usize::MAX))
                } else {
                    positions
                        .iter()
                        .find(|row| row["symbol"].as_str() == Some(&holding.symbol))
                        .and_then(|row| row["candidates"].as_array())
                        .and_then(|rows| {
                            rows.iter()
                                .find(|row| row["symbol"].as_str() == Some(hedge_symbol))
                        })
                        .and_then(|row| {
                            row["correlation"]
                                .as_f64()
                                .zip(row["paired_samples"].as_u64())
                                .map(|(corr, n)| (corr, n as usize))
                        })
                };
                if let Some((correlation, samples)) = relation.filter(|(corr, _)| corr.is_finite())
                {
                    if samples < MIN_HEDGE_CORRELATION_SAMPLES {
                        complete_sample = false;
                    }
                    let correlation = correlation.clamp(-1.0, 1.0);
                    covariance += holding.signed_usd * correlation;
                    usable_correlations += 1;
                    if correlation.abs() > strongest {
                        strongest = correlation.abs();
                        enough_samples = samples >= MIN_HEDGE_CORRELATION_SAMPLES;
                    }
                }
            }
            if usable_correlations != holdings.len()
                || !complete_sample
                || !enough_samples
                || strongest < MIN_HEDGE_CORRELATION
                || !covariance.is_finite()
                || covariance.abs() < settings.max_order_notional * 0.1
            {
                continue;
            }
            let last = settings
                .last_hedge_at
                .get(hedge_symbol)
                .copied()
                .unwrap_or(0);
            if now_ms.saturating_sub(last) < 300_000 {
                continue;
            }
            let hedge_notional = covariance.abs().min(settings.max_order_notional);
            let score = covariance.abs().min(portfolio_shock_risk);
            if best.as_ref().is_none_or(|current| score > current.5) {
                let side = if covariance > 0.0 { "sell" } else { "buy" };
                best = Some((
                    hedge_symbol.to_owned(),
                    side.to_owned(),
                    hedge_notional,
                    hedge_price,
                    quote_rate,
                    score,
                ));
            }
        }
    }
    best
}

