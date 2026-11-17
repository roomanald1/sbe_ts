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

    let (ledger, market) = tokio::try_join!(
        crate::service_rpc::position_get("/positions"),
        crate::service_rpc::execution_get("/internal/market-histories"),
    )?;
    let histories = market.get("histories").and_then(Value::as_object).cloned().unwrap_or_default();
    let positions = ledger.get("positions").and_then(Value::as_array).cloned().unwrap_or_default();
    let open = positions.into_iter().filter(|position| {
        position["quantity"].as_f64().is_some_and(|quantity| quantity.abs() > 1e-12)
    }).collect::<Vec<_>>();
    let symbols = open.iter().filter_map(|position| position["symbol"].as_str()).collect::<Vec<_>>();
    let mut rows = Vec::with_capacity(open.len());
    let mut correlations = Vec::new();
    for position in &open {
        let symbol = position["symbol"].as_str().unwrap_or_default();
        let samples = history_for(&histories, symbol);
        let latest_price = samples.last().map(|(_, price)| *price);
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
            candidates.push(serde_json::json!({
                "symbol": candidate_symbol,
                "correlation": correlation,
                "paired_samples": paired_samples,
                "latest_price": candidate_samples.last().map(|(_, price)| *price),
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
            "latest_price": latest_price,
            "candidates": candidates,
        }));
    }
    for i in 0..symbols.len() {
        for j in (i + 1)..symbols.len() {
            let a = symbols[i];
            let b = symbols[j];
            let (correlation, _) = return_correlation(&history_for(&histories, a), &history_for(&histories, b));
            correlations.push(serde_json::json!({"symbol_a":a,"symbol_b":b,"correlation":correlation}));
        }
    }
    let mut result = Map::new();
    result.insert("positions".into(), serde_json::json!(rows));
    result.insert("pair_correlations".into(), serde_json::json!(correlations));
    let result = Value::Object(result);
    *cache.lock().await = Some((Instant::now(), result.clone()));
    Ok(result)
}

async fn get_portfolio_hedge_candidates(
) -> Result<axum::Json<Value>, (StatusCode, String)> {
    portfolio_hedge_candidates().await.map(axum::Json)
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))
}

fn history_for(histories: &Map<String, Value>, symbol: &str) -> Vec<(u64, f64)> {
    histories.get(symbol).map(parse_history).unwrap_or_default()
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
    use super::return_correlation;

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

