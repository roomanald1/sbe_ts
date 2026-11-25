#[derive(Clone)]
struct RiskHolding {
    symbol: String,
    signed_usd: f64,
}

#[derive(Default, serde::Deserialize)]
struct HedgePairCorrelationData {
    symbol_a: String,
    symbol_b: String,
    correlation: Option<f64>,
    #[serde(default)]
    sampled_at_ms: Option<u64>,
    #[serde(default)]
    per_symbol_freshness_limit_ms: u64,
    #[serde(default)]
    fresh: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct PortfolioVarianceEstimate {
    variance: f64,
    unknown_pair_count: usize,
    used_worst_case_bound: bool,
}

fn calculate_risk_holdings(
    positions: &[serde_json::Value],
    rates: &serde_json::Value,
    now_ms: u64,
) -> (Vec<RiskHolding>, usize, usize) {
    let mut holdings = Vec::new();
    let mut fallback_marks = 0;
    for position in positions {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let Some(quantity) = position["quantity"].as_f64().filter(|value| value.is_finite()) else {
            continue;
        };
        let live_price = position["market_mark_price"]
            .as_f64()
            .filter(|_| position["market_mark_fresh"].as_bool() == Some(true))
            .filter(|_| {
                history_is_fresh(
                    position["market_mark_updated_at_ms"].as_u64(),
                    now_ms,
                    position["market_mark_freshness_limit_ms"]
                        .as_u64()
                        .unwrap_or(0),
                )
            })
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
                let signed_usd = quantity * price * rate;
                if signed_usd.is_finite() {
                    if live_price.is_none() {
                        fallback_marks += 1;
                    }
                    holdings.push(RiskHolding {
                        symbol: symbol.to_owned(),
                        signed_usd,
                    });
                }
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
    unknown_position_records: usize,
    unknown_correlation_pairs: usize,
    used_worst_case_correlation: bool,
) {
    let mut settings = state.settings.lock().await;
    let mark_coverage = match (fallback_marks, unknown_marks) {
        (fallback, unknown) if unknown > 0 => format!(
            "; {fallback} use entry-price marks and {unknown} are unmarked, so hedging is paused"
        ),
        (fallback, _) if fallback > 0 => {
            format!("; {fallback} use entry-price marks, so hedging is paused")
        }
        _ => String::new(),
    };
    let invalid_positions = if unknown_position_records > 0 {
        format!("; {unknown_position_records} position records have invalid exposure")
    } else {
        String::new()
    };
    let correlation_coverage = if unknown_correlation_pairs > 0 {
        format!(
            "; {unknown_correlation_pairs} pair correlations are missing or stale; worst-case signed-exposure bound is active"
        )
    } else if used_worst_case_correlation {
        "; pairwise correlation matrix failed validation; worst-case signed-exposure bound is active"
            .to_owned()
    } else {
        String::new()
    };
    settings.last_risk_note = format!(
        "risk covers {position_count} open positions{mark_coverage}{invalid_positions}{correlation_coverage}"
    );
    settings.risk_limited_sizing = fallback_marks > 0
        || unknown_marks > 0
        || unknown_position_records > 0
        || unknown_correlation_pairs > 0
        || used_worst_case_correlation;
}

fn calculate_portfolio_variance(
    holdings: &[RiskHolding],
    pair_correlations: &[HedgePairCorrelationData],
    now_ms: u64,
) -> PortfolioVarianceEstimate {
    let mut correlation_matrix = vec![vec![0.0; holdings.len()]; holdings.len()];
    for (index, row) in correlation_matrix.iter_mut().enumerate() {
        row[index] = 1.0;
    }
    let mut unknown_pair_count = 0;
    for i in 0..holdings.len() {
        for j in (i + 1)..holdings.len() {
            let correlation = pair_correlations
                .iter()
                .find(|pair| {
                    (pair.symbol_a == holdings[i].symbol && pair.symbol_b == holdings[j].symbol)
                        || (pair.symbol_b == holdings[i].symbol
                            && pair.symbol_a == holdings[j].symbol)
                })
                .filter(|pair| {
                    pair.fresh
                        && pair.per_symbol_freshness_limit_ms > 0
                        && pair.sampled_at_ms.is_some_and(|timestamp| {
                            timestamp <= now_ms
                                && now_ms.saturating_sub(timestamp)
                                    <= pair.per_symbol_freshness_limit_ms
                        })
                })
                .and_then(|pair| pair.correlation)
                .filter(|value| value.is_finite())
                .map(|value| value.clamp(-1.0, 1.0));
            if let Some(correlation) = correlation {
                correlation_matrix[i][j] = correlation;
                correlation_matrix[j][i] = correlation;
            } else {
                unknown_pair_count += 1;
            }
        }
    }

    // Without a complete, fresh, positive-semidefinite correlation matrix,
    // the maximum possible variance for signed USD exposures is the square
    // of their total absolute exposure. This never silently treats unknown
    // covariance as zero and remains finite under overflow.
    let worst_case_variance = || {
        let absolute_exposure = holdings
            .iter()
            .try_fold(0.0_f64, |sum, holding| {
                let next = sum + holding.signed_usd.abs();
                next.is_finite().then_some(next)
            })
            .unwrap_or(f64::MAX.sqrt());
        let variance = absolute_exposure * absolute_exposure;
        if variance.is_finite() {
            variance
        } else {
            f64::MAX
        }
    };
    if unknown_pair_count > 0 || !correlation_matrix_is_positive_semidefinite(&correlation_matrix) {
        return PortfolioVarianceEstimate {
            variance: worst_case_variance(),
            unknown_pair_count,
            used_worst_case_bound: true,
        };
    }

    let mut variance = 0.0_f64;
    for i in 0..holdings.len() {
        for j in 0..holdings.len() {
            variance += holdings[i].signed_usd
                * correlation_matrix[i][j]
                * holdings[j].signed_usd;
        }
    }
    if !variance.is_finite() || variance < 0.0 {
        PortfolioVarianceEstimate {
            variance: worst_case_variance(),
            unknown_pair_count,
            used_worst_case_bound: true,
        }
    } else {
        PortfolioVarianceEstimate {
            variance,
            unknown_pair_count,
            used_worst_case_bound: false,
        }
    }
}

fn correlation_matrix_is_positive_semidefinite(matrix: &[Vec<f64>]) -> bool {
    let size = matrix.len();
    if matrix.iter().any(|row| row.len() != size) {
        return false;
    }
    let mut lower = vec![vec![0.0_f64; size]; size];
    for i in 0..size {
        for j in 0..=i {
            let residual = matrix[i][j]
                - (0..j).map(|k| lower[i][k] * lower[j][k]).sum::<f64>();
            if !residual.is_finite() {
                return false;
            }
            if i == j {
                if residual < -1e-10 {
                    return false;
                }
                lower[i][j] = residual.max(0.0).sqrt();
            } else if lower[j][j] > 1e-10 {
                lower[i][j] = residual / lower[j][j];
            } else if residual.abs() > 1e-10 {
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod portfolio_hedge_risk_tests {
    use super::*;

    fn holdings() -> Vec<RiskHolding> {
        vec![
            RiskHolding {
                symbol: "BTC-USDT".into(),
                signed_usd: 10.0,
            },
            RiskHolding {
                symbol: "ETH-USDT".into(),
                signed_usd: -5.0,
            },
        ]
    }

    fn pair(correlation: f64, sampled_at_ms: u64) -> HedgePairCorrelationData {
        HedgePairCorrelationData {
            symbol_a: "BTC-USDT".into(),
            symbol_b: "ETH-USDT".into(),
            correlation: Some(correlation),
            sampled_at_ms: Some(sampled_at_ms),
            per_symbol_freshness_limit_ms: 100,
            fresh: true,
        }
    }

    fn hedge_candidate(
        now_ms: u64,
        sampled_at_ms: u64,
        freshness_limit_ms: u64,
        fresh: bool,
        paired_samples: usize,
    ) -> Option<(String, String, f64, f64, f64, f64)> {
        let holdings = vec![RiskHolding {
            symbol: "BTC-USDT".into(),
            signed_usd: 10_000.0,
        }];
        let positions = vec![serde_json::json!({
            "symbol": "BTC-USDT",
            "market_mark_price": 100.0,
            "market_mark_updated_at_ms": now_ms,
            "market_mark_freshness_limit_ms": 5_000,
            "market_mark_fresh": true,
            "average_entry_price": 100.0,
            "candidates": [{
                "symbol": "ETH-USDT",
                "correlation": 0.9,
                "paired_samples": paired_samples,
                "market_mark_price": 100.0,
                "market_mark_updated_at_ms": now_ms,
                "market_mark_freshness_limit_ms": 5_000,
                "market_mark_fresh": true,
                "sampled_at_ms": sampled_at_ms,
                "per_symbol_freshness_limit_ms": freshness_limit_ms,
                "fresh": fresh,
            }]
        })];
        let mut settings = Settings::default();
        // Skip the direct same-symbol hedge so this fixture exercises only the
        // correlated-proxy eligibility path.
        settings.last_hedge_at.insert("BTC-USDT".into(), now_ms);
        select_portfolio_hedge_candidate(
            &holdings,
            &positions,
            &serde_json::json!({ "USDT": 1.0 }),
            &settings,
            now_ms,
            10_000.0,
        )
    }

    #[test]
    fn missing_or_stale_pair_uses_worst_case_signed_exposure_bound() {
        let holdings = holdings();
        let missing = calculate_portfolio_variance(&holdings, &[], 1_000);
        assert_eq!(missing.unknown_pair_count, 1);
        assert!(missing.used_worst_case_bound);
        assert_eq!(missing.variance, (10.0_f64.abs() + 5.0).powi(2));

        let stale = calculate_portfolio_variance(&holdings, &[pair(0.9, 899)], 1_000);
        assert_eq!(stale.unknown_pair_count, 1);
        assert!(stale.used_worst_case_bound);
        assert_eq!(stale.variance, missing.variance);

        let same_direction = vec![
            RiskHolding { symbol: "BTC-USDT".into(), signed_usd: 10.0 },
            RiskHolding { symbol: "ETH-USDT".into(), signed_usd: 5.0 },
        ];
        let same_direction_bound = calculate_portfolio_variance(&same_direction, &[], 1_000);
        assert_eq!(same_direction_bound.variance, 225.0);
    }

    #[test]
    fn fresh_complete_pair_uses_signed_covariance_at_freshness_boundary() {
        let estimate = calculate_portfolio_variance(&holdings(), &[pair(0.9, 900)], 1_000);
        assert_eq!(estimate.unknown_pair_count, 0);
        assert!(!estimate.used_worst_case_bound);
        // Opposite signed positions and positive rho reduce net shock variance.
        assert!((estimate.variance - 35.0).abs() < 1e-12);
    }

    #[test]
    fn non_positive_semidefinite_pairwise_correlations_use_worst_case_bound() {
        let holdings = vec![
            RiskHolding { symbol: "A-USD".into(), signed_usd: 10.0 },
            RiskHolding { symbol: "B-USD".into(), signed_usd: 10.0 },
            RiskHolding { symbol: "C-USD".into(), signed_usd: 10.0 },
        ];
        let pairs = [("A-USD", "B-USD"), ("A-USD", "C-USD"), ("B-USD", "C-USD")]
            .into_iter()
            .map(|(symbol_a, symbol_b)| HedgePairCorrelationData {
                symbol_a: symbol_a.into(),
                symbol_b: symbol_b.into(),
                correlation: Some(-1.0),
                sampled_at_ms: Some(1_000),
                per_symbol_freshness_limit_ms: 100,
                fresh: true,
            })
            .collect::<Vec<_>>();
        let estimate = calculate_portfolio_variance(&holdings, &pairs, 1_000);
        assert_eq!(estimate.unknown_pair_count, 0);
        assert!(estimate.used_worst_case_bound);
        assert_eq!(estimate.variance, 900.0);
    }

    #[test]
    fn invalid_usd_exposure_overflow_is_counted_as_unmarked() {
        let positions = vec![serde_json::json!({
            "symbol": "BTC-USDT",
            "quantity": 1e308,
            "market_mark_price": 2.0,
            "market_mark_updated_at_ms": 1_000,
            "market_mark_freshness_limit_ms": 5_000,
            "market_mark_fresh": true,
            "average_entry_price": 2.0,
        })];
        let rates = serde_json::json!({ "USDT": 1.0 });
        let (holdings, fallback_marks, unknown_marks) =
            calculate_risk_holdings(&positions, &rates, 1_000);
        assert!(holdings.is_empty());
        assert_eq!(fallback_marks, 0);
        assert_eq!(unknown_marks, 1);
    }

    #[test]
    fn hedge_proxy_requires_fresh_complete_observations_and_sample_floor() {
        assert!(hedge_candidate(400_000, 399_900, 100, true, 30).is_some());
        assert!(hedge_candidate(400_000, 399_899, 100, false, 30).is_none());
        assert!(hedge_candidate(400_000, 399_900, 0, false, 30).is_none());
        assert!(hedge_candidate(400_000, 399_900, 100, true, 29).is_none());
    }

    #[test]
    fn missing_or_nonfinite_quantity_is_unknown_not_zero_exposure() {
        for position in [
            serde_json::json!({
                "symbol": "BTC-USDT",
                "latest_price": 100.0,
                "average_entry_price": 100.0,
            }),
            serde_json::json!({
                "symbol": "BTC-USDT",
                "quantity": "not-a-number",
                "latest_price": 100.0,
                "average_entry_price": 100.0,
            }),
        ] {
            let (holdings, fallback_marks, unknown_marks) = calculate_risk_holdings(
                &[position],
                &serde_json::json!({ "USDT": 1.0 }),
                1_000,
            );
            assert!(holdings.is_empty());
            assert_eq!(fallback_marks, 0);
            assert_eq!(unknown_marks, 1);
        }
    }

    #[test]
    fn unverified_history_mark_uses_entry_fallback_and_is_not_treated_as_fresh() {
        let positions = vec![serde_json::json!({
            "symbol": "BTC-USDT",
            "quantity": 2.0,
            "market_mark_price": 120.0,
            "average_entry_price": 100.0,
            "market_mark_updated_at_ms": 1_000,
            "market_mark_freshness_limit_ms": 0,
            "market_mark_fresh": false,
        })];
        let rates = serde_json::json!({ "USDT": 1.0 });
        let (holdings, fallback_marks, unknown_marks) =
            calculate_risk_holdings(&positions, &rates, 1_000);
        assert_eq!(holdings.len(), 1);
        assert_eq!(holdings[0].signed_usd, 200.0);
        assert_eq!(fallback_marks, 1);
        assert_eq!(unknown_marks, 0);
    }

    #[test]
    fn fresh_market_mark_drives_signed_risk_exposure() {
        let positions = vec![serde_json::json!({
            "symbol": "BTC-USDT",
            "quantity": 2.0,
            "market_mark_price": 120.0,
            "average_entry_price": 100.0,
            "market_mark_updated_at_ms": 995,
            "market_mark_freshness_limit_ms": 5,
            "market_mark_fresh": true,
        })];
        let rates = serde_json::json!({ "USDT": 1.0 });
        let (holdings, fallback_marks, unknown_marks) =
            calculate_risk_holdings(&positions, &rates, 1_000);
        assert_eq!(holdings[0].signed_usd, 240.0);
        assert_eq!(fallback_marks, 0);
        assert_eq!(unknown_marks, 0);
    }
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
                    let freshness_limit = row["market_mark_freshness_limit_ms"]
                        .as_u64()
                        .unwrap_or(0);
                    (row["market_mark_fresh"].as_bool() == Some(true)
                        && history_is_fresh(
                            row["market_mark_updated_at_ms"].as_u64(),
                            now_ms,
                            freshness_limit,
                        ))
                    .then(|| row["market_mark_price"].as_f64())
                    .flatten()
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
            let sample_timestamp = candidate["sampled_at_ms"].as_u64();
            let freshness_limit = candidate["per_symbol_freshness_limit_ms"]
                .as_u64()
                .unwrap_or(0);
            if candidate["fresh"].as_bool() != Some(true)
                || !history_is_fresh(sample_timestamp, now_ms, freshness_limit)
            {
                continue;
            }
            let mark_freshness_limit = candidate["market_mark_freshness_limit_ms"]
                .as_u64()
                .unwrap_or(0);
            if candidate["market_mark_fresh"].as_bool() != Some(true)
                || !history_is_fresh(
                    candidate["market_mark_updated_at_ms"].as_u64(),
                    now_ms,
                    mark_freshness_limit,
                )
            {
                continue;
            }
            let hedge_price = candidate["market_mark_price"].as_f64().unwrap_or(0.0);
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
                        .filter(|row| {
                            let sample_timestamp = row["sampled_at_ms"].as_u64();
                            let freshness_limit = row["per_symbol_freshness_limit_ms"]
                                .as_u64()
                                .unwrap_or(0);
                            row["fresh"].as_bool() == Some(true)
                                && history_is_fresh(sample_timestamp, now_ms, freshness_limit)
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

