#[derive(Clone, Serialize, Deserialize)]

struct UsdRatesResponse {
    rates: BTreeMap<String, f64>,
    updated_at: u64,
    #[serde(default)]
    updated_at_by_currency: BTreeMap<String, u64>,
}

fn latest_stream_usd_rates() -> Option<UsdRatesResponse> {
    let snapshot = USD_RATE_STREAM_CACHE
        .get_or_init(|| StdRwLock::new(None))
        .read()
        .ok()?
        .clone()?;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_millis() as u64;
    let max_age_ms = USD_RATE_STREAM_MAX_AGE.as_millis() as u64;
    let mut rates = BTreeMap::from([("USD".to_owned(), 1.0)]);
    let mut updated_at_by_currency = BTreeMap::from([("USD".to_owned(), 0)]);
    let mut updated_at = 0;
    for (currency, rate) in snapshot.rates {
        if currency == "USD" {
            continue;
        }
        let timestamp = snapshot
            .updated_at_by_currency
            .get(&currency)
            .copied()
            .unwrap_or(snapshot.updated_at);
        if !rate.is_finite()
            || rate <= 0.0
            || timestamp == 0
            || now_ms.saturating_sub(timestamp) > max_age_ms
        {
            continue;
        }
        rates.insert(currency.clone(), rate);
        updated_at_by_currency.insert(currency, timestamp);
        updated_at = updated_at.max(timestamp);
    }
    (rates.len() > 1).then_some(UsdRatesResponse {
        rates,
        updated_at,
        updated_at_by_currency,
    })
}

fn merge_usd_rate_snapshots(
    fresh: &UsdRatesResponse,
    fallback: &UsdRatesResponse,
) -> UsdRatesResponse {
    let mut merged = fallback.clone();
    merged.rates.extend(fresh.rates.clone());
    merged
        .updated_at_by_currency
        .extend(fresh.updated_at_by_currency.clone());
    merged.updated_at = merged.updated_at.max(fresh.updated_at);
    merged.rates.insert("USD".to_owned(), 1.0);
    merged
}

fn cached_usd_rates_with_stream(
    stream: Option<&UsdRatesResponse>,
    cached: &UsdRatesResponse,
) -> UsdRatesResponse {
    stream.map_or_else(
        || cached.clone(),
        |fresh| merge_usd_rate_snapshots(fresh, cached),
    )
}

fn refresh_order_usd_rate_snapshot() {
    let cache = USD_RATE_CACHE.get_or_init(|| Mutex::new(None));
    let Ok(cached_guard) = cache.try_lock() else {
        return;
    };
    let cached = cached_guard
        .as_ref()
        .map(|(_, rates)| rates.clone());
    let stream = latest_stream_usd_rates();
    let merged = match (stream, cached) {
        (Some(stream), Some(cached)) => Some(merge_usd_rate_snapshots(&stream, &cached)),
        (Some(stream), None) => Some(stream),
        (None, Some(cached)) => Some(cached),
        (None, None) => None,
    };
    USD_RATE_ORDER_SNAPSHOT
        .get_or_init(ArcSwapOption::empty)
        .store(merged.map(Arc::new));
}

async fn get_usd_rates() -> Result<Json<UsdRatesResponse>, (StatusCode, String)> {
    let stream_rates = latest_stream_usd_rates();
    let cache = USD_RATE_CACHE.get_or_init(|| Mutex::new(None));
    let cached_rates = {
        let cached = cache.lock().await;
        cached
            .as_ref()
            .map(|(created, rates)| (created.elapsed(), rates.clone()))
    };
    if let Some((age, rates)) = cached_rates {
        if age >= USD_RATE_CACHE_TTL && !USD_RATE_REFRESHING.swap(true, Ordering::AcqRel) {
            // Keep the order path independent of remote OKX latency. A background
            // refresh updates the last usable snapshot for the next request.
            tokio::spawn(async move {
                if let Ok(rates) = fetch_usd_rates().await {
                    let cache = USD_RATE_CACHE.get_or_init(|| Mutex::new(None));
                    *cache.lock().await = Some((Instant::now(), rates));
                    refresh_order_usd_rate_snapshot();
                }
                USD_RATE_REFRESHING.store(false, Ordering::Release);
            });
        }
        refresh_order_usd_rate_snapshot();
        return Ok(Json(cached_usd_rates_with_stream(
            stream_rates.as_ref(),
            &rates,
        )));
    }
    if let Some(stream) = stream_rates {
        // A live stream can be fresh for only a subset of quote currencies.
        // Seed the full REST cache once, then let fresh streamed rates override it.
        match fetch_usd_rates().await {
            Ok(fallback) => {
                *cache.lock().await = Some((Instant::now(), fallback.clone()));
                refresh_order_usd_rate_snapshot();
                return Ok(Json(merge_usd_rate_snapshots(&stream, &fallback)));
            }
            Err(_) => return Ok(Json(stream)),
        }
    }
    let rates = fetch_usd_rates().await?;
    *cache.lock().await = Some((Instant::now(), rates.clone()));
    refresh_order_usd_rate_snapshot();
    Ok(Json(rates))
}

// Order submission must never wait for an external rate request. The background
// refresh loop owns cache population; requests may use the last cached snapshot.
async fn cached_usd_rates_for_order() -> Result<Arc<UsdRatesResponse>, (StatusCode, String)> {
    USD_RATE_ORDER_SNAPSHOT
        .get_or_init(ArcSwapOption::empty)
        .load_full()
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "USD conversion rates are not warmed yet".to_owned(),
        ))
}

async fn fetch_usd_rates() -> Result<UsdRatesResponse, (StatusCode, String)> {
    let response = HTTP_CLIENT
        .get_or_init(reqwest::Client::new)
        .get("https://www.okx.com/api/v5/market/index-tickers?quoteCcy=USD")
        .send()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("OKX USD index request failed: {error}"),
            )
        })?
        .error_for_status()
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("OKX USD index request failed: {error}"),
            )
        })?
        .text()
        .await
        .map_err(|error| {
            (
                StatusCode::BAD_GATEWAY,
                format!("Could not read OKX USD index response: {error}"),
            )
        })?;
    let response: serde_json::Value = serde_json::from_str(&response).map_err(|error| {
        (
            StatusCode::BAD_GATEWAY,
            format!("Invalid OKX USD index response: {error}"),
        )
    })?;
    let data = response
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            (
                StatusCode::BAD_GATEWAY,
                "OKX USD index response has no data".to_owned(),
            )
        })?;
    let mut rates = BTreeMap::from([("USD".to_owned(), 1.0)]);
    let mut updated_at_by_currency = BTreeMap::from([("USD".to_owned(), 0)]);
    let mut updated_at = 0;
    for item in data {
        let Some(instrument) = item.get("instId").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let Some((currency, quote)) = instrument.split_once('-') else {
            continue;
        };
        if quote != "USD" {
            continue;
        }
        let Some(rate) = item
            .get("idxPx")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<f64>().ok())
        else {
            continue;
        };
        if !rate.is_finite() || rate <= 0.0 {
            continue;
        }
        rates.insert(currency.to_owned(), rate);
        if let Some(timestamp) = item
            .get("ts")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| value.parse::<u64>().ok())
        {
            updated_at_by_currency.insert(currency.to_owned(), timestamp);
            updated_at = updated_at.max(timestamp);
        }
    }
    if rates.len() == 1 {
        return Err((
            StatusCode::BAD_GATEWAY,
            "OKX returned no usable USD index rates".to_owned(),
        ));
    }
    Ok(UsdRatesResponse {
        rates,
        updated_at,
        updated_at_by_currency,
    })
}

#[cfg(test)]
mod tests {
    use super::{ArcSwapOption, UsdRatesResponse, cached_usd_rates_with_stream};

    fn rates(
        values: &[(&str, f64)],
        updated_at: u64,
        currency_times: &[(&str, u64)],
    ) -> UsdRatesResponse {
        UsdRatesResponse {
            rates: values
                .iter()
                .map(|(currency, rate)| ((*currency).to_owned(), *rate))
                .collect(),
            updated_at,
            updated_at_by_currency: currency_times
                .iter()
                .map(|(currency, timestamp)| ((*currency).to_owned(), *timestamp))
                .collect(),
        }
    }

    #[test]
    #[ignore = "manual microbenchmark; run with cargo test --release usd_rate_order_lookup_benchmark -- --ignored --nocapture"]
    fn usd_rate_order_lookup_benchmark() {
        use std::{hint::black_box, sync::Arc, time::Instant};

        let stream = rates(&[("USD", 1.0), ("USDT", 0.999), ("BTC", 68_000.0)], 200, &[("USDT", 200), ("BTC", 200)]);
        let cached = rates(
            &[("USD", 1.0), ("USDT", 1.0), ("BTC", 67_900.0), ("EUR", 1.08), ("GBP", 1.27), ("JPY", 0.0067)],
            150,
            &[("USDT", 150), ("BTC", 150), ("EUR", 150), ("GBP", 150), ("JPY", 150)],
        );
        let merged = cached_usd_rates_with_stream(Some(&stream), &cached);
        let snapshot = ArcSwapOption::from(Some(Arc::new(merged)));
        const ITERATIONS: usize = 1_000_000;

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(cached_usd_rates_with_stream(Some(&stream), &cached));
        }
        let merge_ns = started.elapsed().as_nanos() as f64 / ITERATIONS as f64;

        let started = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(snapshot.load_full());
        }
        let snapshot_ns = started.elapsed().as_nanos() as f64 / ITERATIONS as f64;

        println!("USD rate order lookup: merge+clone={merge_ns:.1} ns/op, premerged snapshot={snapshot_ns:.1} ns/op, speedup={:.1}x", merge_ns / snapshot_ns);
    }

    #[test]
    fn partial_fresh_stream_keeps_cached_quote_rates_and_overrides_stale_values() {
        let fresh_stream = rates(&[("USD", 1.0), ("USDT", 0.999)], 200, &[("USDT", 200)]);
        let cached = rates(
            &[("USD", 1.0), ("USDT", 1.0), ("EUR", 1.08), ("TRY", 0.03)],
            150,
            &[("USDT", 140), ("EUR", 150), ("TRY", 150)],
        );

        let merged = cached_usd_rates_with_stream(Some(&fresh_stream), &cached);

        assert_eq!(merged.rates["USDT"], 0.999);
        assert_eq!(merged.rates["EUR"], 1.08);
        assert_eq!(merged.rates["TRY"], 0.03);
        assert_eq!(merged.rates["USD"], 1.0);
        assert_eq!(merged.updated_at_by_currency["USDT"], 200);
        assert_eq!(merged.updated_at_by_currency["EUR"], 150);
        assert_eq!(merged.updated_at, 200);
    }
}
