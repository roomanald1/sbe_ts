use serde_json::{Map, Value};
use std::collections::HashMap;

async fn portfolio_hedge_candidates() -> anyhow::Result<Value> {
    static CACHE: std::sync::OnceLock<Mutex<Option<(Instant, Value)>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Some(value) = cache.lock().await.as_ref()
        .filter(|(updated, _)| updated.elapsed() < Duration::from_secs(10))
        .map(|(_, value)| value.clone())
    {
        return Ok(value);
    }

    let ledger = crate::service_rpc::position_get("/positions").await?;
    let positions = ledger
        .get("positions")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let held_symbols = positions
        .iter()
        .filter(|position| {
            position["quantity"]
                .as_f64()
                .is_some_and(|quantity| quantity.is_finite() && quantity.abs() > 1e-12)
        })
        .filter_map(|position| position["symbol"].as_str())
        .filter(|symbol| valid_market_history_symbol(symbol))
        .collect::<Vec<_>>();
    let invalid_held_symbols = positions
        .iter()
        .filter(|position| {
            position["quantity"]
                .as_f64()
                .is_some_and(|quantity| quantity.is_finite() && quantity.abs() > 1e-12)
        })
        .filter_map(|position| position["symbol"].as_str())
        .filter(|symbol| !valid_market_history_symbol(symbol))
        .collect::<Vec<_>>();
    let market = market_histories_for_held_symbols(&held_symbols).await?;
    let market = with_invalid_held_symbol_failures(market, &invalid_held_symbols);
    let now_ms = current_time_ms();
    let timing_contract = market.get("timing_contract");
    let history_timing_contract_version = timing_contract
        .and_then(|contract| contract["version"].as_u64())
        .filter(|version| *version == 3);
    // Sampling cadence and the global stream-idle timeout are not symbol-level
    // guarantees. Version 3 publishes each symbol's active-book freshness.
    let per_symbol_freshness_limit_ms = timing_contract
        .and_then(|contract| contract["history_freshness_max_age_ms"].as_u64())
        .filter(|value| *value > 0)
        .unwrap_or(0);
    let market_mark_freshness_limit_ms = timing_contract
        .and_then(|contract| contract["market_mark_freshness_ms"].as_u64())
        .filter(|value| *value > 0)
        .unwrap_or(0);
    let histories = market.get("histories").and_then(Value::as_object).cloned().unwrap_or_default();
    let market_marks = market
        .get("market_marks")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let history_freshness = market
        .get("history_freshness")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let (open, unknown_position_records) = open_position_records(positions);
    let symbols = open.iter().filter_map(|position| position["symbol"].as_str()).collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(open.len());
    let mut correlations = Vec::new();
    for position in &open {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let samples = history_for(&histories, symbol);
        let history_latest_price = samples.last().map(|(_, price)| *price);
        let latest_sample_at_ms = latest_history_timestamp(&samples);
        let (market_mark_price, market_mark_updated_at_ms, market_mark_fresh) =
            fresh_market_mark(&market_marks, symbol, now_ms, market_mark_freshness_limit_ms);
        let quote = symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
        let mut candidates = Vec::new();
        for (candidate_symbol, candidate_history) in &histories {
            if candidate_symbol == symbol
                || candidate_symbol.split_once('-').map(|(_, value)| value) != Some(quote)
            {
                continue;
            }
            let candidate_samples = parse_history(candidate_history);
            let (correlation, paired_samples) = return_correlation(&samples, &candidate_samples);
            let latest_candidate_sample_at_ms = latest_history_timestamp(&candidate_samples);
            let (candidate_mark_price, candidate_mark_updated_at_ms, candidate_mark_fresh) =
                fresh_market_mark(
                    &market_marks,
                    candidate_symbol,
                    now_ms,
                    market_mark_freshness_limit_ms,
                );
            let latest_pair_sample_at_ms = latest_sample_at_ms
                .zip(latest_candidate_sample_at_ms)
                .map(|(left, right)| left.min(right));
            let fresh = history_contract_is_fresh(
                &history_freshness,
                symbol,
                candidate_symbol,
                history_timing_contract_version,
            );
            candidates.push(serde_json::json!({
                "symbol": candidate_symbol,
                "correlation": correlation,
                "paired_samples": paired_samples,
                "sampled_at_ms": latest_pair_sample_at_ms,
                "per_symbol_freshness_limit_ms": per_symbol_freshness_limit_ms,
                "fresh": fresh,
                "market_mark_price": candidate_mark_price,
                "market_mark_updated_at_ms": candidate_mark_updated_at_ms,
                "market_mark_fresh": candidate_mark_fresh,
                "history_latest_price": candidate_samples.last().map(|(_, price)| *price),
                "latest_price": candidate_mark_price,
            }));
        }
        candidates.sort_by(|left, right| {
            match (left["correlation"].as_f64(), right["correlation"].as_f64()) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => right["paired_samples"].as_u64().cmp(&left["paired_samples"].as_u64()),
            }.then_with(|| left["symbol"].as_str().cmp(&right["symbol"].as_str()))
        });
        candidates.truncate(10);
        rows.push(serde_json::json!({
            "symbol": symbol,
            "quantity": position["quantity"],
            "average_entry_price": position["average_entry_price"],
            "latest_price": market_mark_price,
            "history_latest_price": history_latest_price,
            "history_sampled_at_ms": latest_sample_at_ms,
            "per_symbol_freshness_limit_ms": per_symbol_freshness_limit_ms,
            "market_mark_price": market_mark_price,
            "market_mark_updated_at_ms": market_mark_updated_at_ms,
            "market_mark_freshness_limit_ms": market_mark_freshness_limit_ms,
            "market_mark_fresh": market_mark_fresh,
            "candidates": candidates,
        }));
    }
    for i in 0..symbols.len() {
        for j in (i + 1)..symbols.len() {
            let a = symbols[i];
            let b = symbols[j];
            let left = history_for(&histories, a);
            let right = history_for(&histories, b);
            let (correlation, paired_samples) = return_correlation(&left, &right);
            let left_latest = latest_history_timestamp(&left);
            let right_latest = latest_history_timestamp(&right);
            let sampled_at_ms = left_latest.zip(right_latest).map(|(x, y)| x.min(y));
            let fresh = history_contract_is_fresh(
                &history_freshness,
                a,
                b,
                history_timing_contract_version,
            );
            correlations.push(serde_json::json!({
                "symbol_a": a,
                "symbol_b": b,
                "correlation": correlation,
                "paired_samples": paired_samples,
                "sampled_at_ms": sampled_at_ms,
                "per_symbol_freshness_limit_ms": per_symbol_freshness_limit_ms,
                "fresh": fresh,
            }));
        }
    }
    let mut result = Map::new();
    result.insert("positions".into(), serde_json::json!(rows));
    result.insert("pair_correlations".into(), serde_json::json!(correlations));
    result.insert("history_timing_contract_version".into(), serde_json::json!(history_timing_contract_version));
    result.insert("per_symbol_freshness_limit_ms".into(), serde_json::json!(per_symbol_freshness_limit_ms));
    result.insert("market_mark_freshness_limit_ms".into(), serde_json::json!(market_mark_freshness_limit_ms));
    result.insert("unknown_position_records".into(), serde_json::json!(unknown_position_records));
    let result = Value::Object(result);
    *cache.lock().await = Some((Instant::now(), result.clone()));
    Ok(result)
}

fn valid_market_history_symbol(symbol: &str) -> bool {
    symbol.len() <= 32
        && symbol.split_once('-').is_some_and(|(base, quote)| {
            !base.is_empty()
                && !quote.is_empty()
                && base.bytes().all(|byte| byte.is_ascii_alphanumeric())
                && quote.bytes().all(|byte| byte.is_ascii_alphanumeric())
        })
}

fn with_invalid_held_symbol_failures(mut market: Value, symbols: &[&str]) -> Value {
    for symbol in symbols {
        market["history_freshness"][*symbol] = serde_json::json!({
            "latest_observed_at_ms": null,
            "observed_age_ms": null,
            "state": "unavailable"
        });
        if let Some(subscriptions) = market["held_symbol_subscriptions"].as_array_mut() {
            subscriptions.push(serde_json::json!({
                "requested_symbol": symbol,
                "symbol": null,
                "subscription": null,
                "status": "invalid_symbol",
                "error": "Held position symbol is not a valid bounded market symbol",
                "fresh_mark": null,
                "history_freshness": {
                    "latest_observed_at_ms": null,
                    "observed_age_ms": null,
                    "state": "unavailable"
                }
            }));
        }
    }
    market
}

async fn market_histories_for_held_symbols(held_symbols: &[&str]) -> anyhow::Result<Value> {
    const HELD_SYMBOLS_PER_REQUEST: usize = 32;
    let mut combined = serde_json::json!({
        "histories": {},
        "market_marks": {},
        "history_freshness": {},
        "held_symbol_subscriptions": [],
    });
    let chunks: Vec<&[&str]> = if held_symbols.is_empty() {
        vec![&[]]
    } else {
        held_symbols.chunks(HELD_SYMBOLS_PER_REQUEST).collect()
    };
    for chunk in chunks {
        let path = if chunk.is_empty() {
            "/internal/market-histories".to_owned()
        } else {
            format!(
                "/internal/market-histories?held_symbols={}",
                chunk.join(",")
            )
        };
        let response = crate::service_rpc::execution_get(&path).await?;
        for field in ["histories", "market_marks", "history_freshness"] {
            if let (Some(target), Some(source)) = (
                combined[field].as_object_mut(),
                response.get(field).and_then(Value::as_object),
            ) {
                target.extend(source.iter().map(|(key, value)| (key.clone(), value.clone())));
            }
        }
        if let (Some(target), Some(source)) = (
            combined["held_symbol_subscriptions"].as_array_mut(),
            response
                .get("held_symbol_subscriptions")
                .and_then(Value::as_array),
        ) {
            target.extend(source.iter().cloned());
        }
        combined["timing_contract"] = response
            .get("timing_contract")
            .cloned()
            .unwrap_or(Value::Null);
    }
    Ok(combined)
}

fn history_contract_is_fresh(
    freshness: &Map<String, Value>,
    left: &str,
    right: &str,
    contract_version: Option<u64>,
) -> bool {
    contract_version == Some(3)
        && [left, right].into_iter().all(|symbol| {
            freshness
                .get(symbol)
                .is_some_and(|status| status["state"].as_str() == Some("fresh"))
        })
}

async fn get_portfolio_hedge_candidates(
) -> Result<axum::Json<Value>, (StatusCode, String)> {
    portfolio_hedge_candidates().await.map(axum::Json)
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

fn history_for(histories: &Map<String, Value>, symbol: &str) -> Vec<(u64, f64)> {
    histories.get(symbol).map(parse_history).unwrap_or_default()
}

fn open_position_records(positions: Vec<Value>) -> (Vec<Value>, usize) {
    let mut unknown_position_records = 0usize;
    let open = positions
        .into_iter()
        .filter_map(|position| match position["quantity"].as_f64() {
            Some(quantity) if quantity.is_finite() && quantity.abs() > 1e-12 => Some(position),
            Some(quantity) if quantity.is_finite() => None,
            _ => {
                unknown_position_records += 1;
                None
            }
        })
        .collect();
    (open, unknown_position_records)
}

fn latest_history_timestamp(samples: &[(u64, f64)]) -> Option<u64> {
    samples
        .iter()
        .filter(|(_, price)| price.is_finite() && *price > 0.0)
        .map(|(timestamp, _)| *timestamp)
        .max()
}

fn history_is_fresh(timestamp_ms: Option<u64>, now_ms: u64, stale_after_ms: u64) -> bool {
    timestamp_ms.is_some_and(|timestamp| {
        stale_after_ms > 0
            && timestamp <= now_ms
            && now_ms.saturating_sub(timestamp) <= stale_after_ms
    })
}

fn fresh_market_mark(
    market_marks: &Map<String, Value>,
    symbol: &str,
    now_ms: u64,
    freshness_limit_ms: u64,
) -> (Option<f64>, Option<u64>, bool) {
    let mark = market_marks.get(symbol);
    let mid = mark
        .and_then(|value| value["mid"].as_f64())
        .filter(|value| value.is_finite() && *value > 0.0);
    let updated_at_ms = mark.and_then(|value| value["updated_at_ms"].as_u64());
    let fresh = freshness_limit_ms > 0
        && mid.is_some()
        && updated_at_ms.is_some_and(|timestamp| {
            timestamp <= now_ms && now_ms.saturating_sub(timestamp) <= freshness_limit_ms
        });
    (fresh.then_some(mid).flatten(), updated_at_ms, fresh)
}

fn parse_history(value: &Value) -> Vec<(u64, f64)> {
    value.as_array().into_iter().flatten().filter_map(|sample| {
        Some((sample.get(0)?.as_u64()?, sample.get(1)?.as_f64()?))
    }).collect()
}

fn return_correlation(a: &[(u64, f64)], b: &[(u64, f64)]) -> (Option<f64>, usize) {
    const BUCKET_MS: u64 = 10_000;
    let bucket_prices = |samples: &[(u64, f64)]| {
        samples.iter().filter(|(_, price)| price.is_finite() && *price > 0.0)
            .map(|(timestamp, price)| (timestamp / BUCKET_MS, *price))
            .collect::<HashMap<_, _>>()
    };
    let left = bucket_prices(a);
    let right = bucket_prices(b);
    let mut buckets = left.keys().copied().filter(|bucket| right.contains_key(bucket)).collect::<Vec<_>>();
    buckets.sort_unstable();
    let returns = buckets.windows(2).map(|window| {
        let prior = window[0]; let current = window[1];
        ((left[&current] / left[&prior]).ln(), (right[&current] / right[&prior]).ln())
    }).filter(|(x, y)| x.is_finite() && y.is_finite()).collect::<Vec<_>>();
    let count = returns.len();
    if count < 10 { return (None, count); }
    let mean_a = returns.iter().map(|(x, _)| x).sum::<f64>() / count as f64;
    let mean_b = returns.iter().map(|(_, y)| y).sum::<f64>() / count as f64;
    let covariance = returns.iter().map(|(x, y)| (x - mean_a) * (y - mean_b)).sum::<f64>();
    let variance_a = returns.iter().map(|(x, _)| (x - mean_a).powi(2)).sum::<f64>();
    let variance_b = returns.iter().map(|(_, y)| (y - mean_b).powi(2)).sum::<f64>();
    let correlation = if variance_a > 0.0 && variance_b > 0.0 {
        Some((covariance / (variance_a * variance_b).sqrt()).clamp(-1.0, 1.0))
    } else { None };
    (correlation.filter(|value| value.is_finite()), count)
}

#[cfg(test)]
mod portfolio_hedge_candidates_tests {
    use super::{
        fresh_market_mark, history_contract_is_fresh, history_is_fresh, open_position_records,
        return_correlation, valid_market_history_symbol, with_invalid_held_symbol_failures,
    };

    #[test]
    fn ticker_idle_timeout_is_not_an_implicit_per_symbol_freshness_bound() {
        assert!(!history_is_fresh(Some(1_000), 1_000, 0));
        assert!(!history_is_fresh(Some(899), 1_000, 100));
        assert!(history_is_fresh(Some(900), 1_000, 100));
        assert!(!history_is_fresh(Some(1_001), 1_000, 100));
    }

    #[test]
    fn proxy_freshness_requires_v3_per_symbol_source_status_for_both_symbols() {
        let freshness = serde_json::json!({
            "BTC-USDT": { "state": "fresh" },
            "ETH-USDT": { "state": "fresh" },
            "SOL-USDT": { "state": "stale" },
        });
        let freshness = freshness.as_object().unwrap();
        assert!(history_contract_is_fresh(
            freshness,
            "BTC-USDT",
            "ETH-USDT",
            Some(3)
        ));
        assert!(!history_contract_is_fresh(
            freshness,
            "BTC-USDT",
            "SOL-USDT",
            Some(3)
        ));
        assert!(!history_contract_is_fresh(
            freshness,
            "BTC-USDT",
            "MISSING-USDT",
            Some(3)
        ));
        assert!(!history_contract_is_fresh(
            freshness,
            "BTC-USDT",
            "ETH-USDT",
            Some(1)
        ));
    }

    #[test]
    fn invalid_held_symbol_is_reported_unavailable_without_a_query() {
        assert!(valid_market_history_symbol("BTC-USDT"));
        assert!(!valid_market_history_symbol("BTC/USDT"));
        assert!(!valid_market_history_symbol("-USDT"));
        assert!(!valid_market_history_symbol(&format!("{}-USDT", "A".repeat(28))));
        let market = with_invalid_held_symbol_failures(
            serde_json::json!({"history_freshness": {}, "held_symbol_subscriptions": []}),
            &["BAD/PAIR"],
        );
        assert_eq!(market["history_freshness"]["BAD/PAIR"]["state"], "unavailable");
        assert_eq!(
            market["held_symbol_subscriptions"][0]["status"],
            "invalid_symbol"
        );
    }

    #[test]
    fn invalid_position_quantity_is_explicit_unknown_not_dropped_as_zero() {
        let (open, unknown) = open_position_records(vec![
            serde_json::json!({ "symbol": "BTC-USDT", "quantity": 2.0 }),
            serde_json::json!({ "symbol": "ETH-USDT" }),
            serde_json::json!({ "symbol": "SOL-USDT", "quantity": "invalid" }),
            serde_json::json!({ "symbol": "XRP-USDT", "quantity": 0.0 }),
        ]);
        assert_eq!(open.len(), 1);
        assert_eq!(unknown, 2);
    }

    #[test]
    fn market_mark_requires_per_symbol_update_within_source_bound() {
        let marks = [(
            "BTC-USDT".to_owned(),
            serde_json::json!({ "mid": 100.0, "updated_at_ms": 995 }),
        )]
        .into_iter()
        .collect::<serde_json::Map<_, _>>();
        assert_eq!(
            fresh_market_mark(&marks, "BTC-USDT", 1_000, 5),
            (Some(100.0), Some(995), true)
        );
        assert_eq!(
            fresh_market_mark(&marks, "BTC-USDT", 1_001, 5),
            (None, Some(995), false)
        );
        assert_eq!(
            fresh_market_mark(&marks, "BTC-USDT", 1_000, 0),
            (None, Some(995), false)
        );
    }

    #[test]
    fn correlation_requires_history_and_detects_aligned_returns() {
        let left = (0..12).map(|i| (i * 10_000, 100.0 + i as f64)).collect::<Vec<_>>();
        let right = (0..12).map(|i| (i * 10_000, 200.0 + 2.0 * i as f64)).collect::<Vec<_>>();
        let (correlation, samples) = return_correlation(&left, &right);
        assert_eq!(samples, 11);
        assert!((correlation.unwrap() - 1.0).abs() < 1e-10);
        assert_eq!(return_correlation(&left[..3], &right[..3]), (None, 2));
    }
}

