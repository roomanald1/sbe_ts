async fn poll_news_feed(news: watch::Sender<Arc<NewsFeedSnapshot>>) {
    let mut snapshot = NewsFeedSnapshot { source: "CoinDesk RSS".to_owned(), stale: true, ..NewsFeedSnapshot::default() };
    let client = reqwest::Client::builder()
        .user_agent("sbe-ts-news-ticker/1.0 (RSS reader)")
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap_or_default();
    loop {
        let result = async {
            let response = client
                .get("https://www.coindesk.com/arc/outboundfeeds/rss/?outputType=xml")
                .send()
                .await?
                .error_for_status()?;
            response.text().await
        }
        .await;
        let refreshed = match result {
            Ok(xml) => {
                let timestamp = now_ms();
                let mut articles = parse_news_rss(&xml, timestamp);
                classify_headlines_with_model(&mut articles).await;
                Some((timestamp, articles, None))
            }
            Err(error) => Some((now_ms(), Vec::new(), Some(format!("Could not refresh CoinDesk RSS: {error}")))),
        };
        if let Some((timestamp, articles, error)) = refreshed {
            if let Some(error) = error {
                snapshot.error = Some(error);
                snapshot.stale = true;
            } else if articles.is_empty() {
                snapshot.error = Some("RSS feed returned no parseable articles".to_owned());
                snapshot.stale = true;
            } else {
                snapshot.updated_at_ms = timestamp;
                snapshot.source = "CoinDesk RSS".to_owned();
                snapshot.stale = false;
                snapshot.error = None;
                snapshot.articles = articles;
            }
        }
        let forwarded = serde_json::to_vec(&snapshot).unwrap_or_default();
        news.send_replace(Arc::new(snapshot.clone()));
        let feed_handlers_url = std::env::var("FEED_HANDLERS_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
        if !forwarded.is_empty() {
            if let Err(error) = client
                .post(format!("{feed_handlers_url}/internal/news/snapshot"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(forwarded)
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                crate::service_log!("WARN", "Could not forward RSS snapshot to FeedHandlers: {error}");
            }
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

fn json_number(value: &serde_json::Value, field: &str) -> Option<f64> {
    value
        .get(field)?
        .as_str()?
        .parse::<f64>()
        .ok()
        .filter(|number| number.is_finite())
}

async fn okx_first(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> Option<serde_json::Value> {
    let query = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let response = client
        .get(format!("https://www.okx.com{path}?{query}"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    if response.get("code")?.as_str()? != "0" {
        return None;
    }
    response.get("data")?.as_array()?.first().cloned()
}

async fn okx_data(
    client: &reqwest::Client,
    path: &str,
    query: &[(&str, &str)],
) -> Option<Vec<serde_json::Value>> {
    let query = query
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let response = client
        .get(format!("https://www.okx.com{path}?{query}"))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<serde_json::Value>()
        .await
        .ok()?;
    if response.get("code")?.as_str()? != "0" {
        return None;
    }
    response.get("data")?.as_array().cloned()
}

fn option_volatility_summary(
    rows: &[serde_json::Value],
    now: u64,
) -> (Option<f64>, Option<f64>, Option<u64>) {
    let days_now = 86_400_000.0;
    let options = rows
        .iter()
        .filter_map(|row| {
            let expiry = row.get("expTime")?.as_str()?.parse::<u64>().ok()?;
            let days = expiry.saturating_sub(now) as f64 / days_now;
            if !(7.0..=60.0).contains(&days) {
                return None;
            }
            Some((
                row.get("optType")?.as_str()?.to_owned(),
                json_number(row, "delta")?,
                json_number(row, "markVol")?,
                expiry,
                days,
            ))
        })
        .collect::<Vec<_>>();
    let select = |kind: &str, target_delta: f64| {
        options
            .iter()
            .filter(|(option_type, _, _, _, _)| option_type == kind)
            .min_by(|a, b| {
                let score_a = (a.4 - 30.0).abs() + (a.1.abs() - target_delta).abs() * 30.0;
                let score_b = (b.4 - 30.0).abs() + (b.1.abs() - target_delta).abs() * 30.0;
                score_a.total_cmp(&score_b)
            })
    };
    let call_atm = select("C", 0.5);
    let put_atm = select("P", 0.5);
    let iv = call_atm
        .zip(put_atm)
        .map(|(call, put)| (call.2 + put.2) / 2.0)
        .or_else(|| call_atm.map(|row| row.2))
        .or_else(|| put_atm.map(|row| row.2));
    let call_25d = select("C", 0.25);
    let put_25d = select("P", 0.25);
    let skew = put_25d.zip(call_25d).map(|(put, call)| put.2 - call.2);
    let expiry = call_atm
        .map(|row| row.3)
        .or_else(|| put_atm.map(|row| row.3));
    (iv, skew, expiry)
}

async fn poll_external_market_features(store: SignalStore) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let mut previous_open_interest = HashMap::<String, f64>::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for underlying in ["BTC", "ETH"] {
            let swap = format!("{underlying}-USDT-SWAP");
            let spot_index = format!("{underlying}-USDT");
            let option_underlying = format!("{underlying}-USD");
            let funding_q = [("instId", swap.as_str())];
            let oi_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let mark_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let index_q = [("instId", spot_index.as_str())];
            let options_q = [("uly", option_underlying.as_str())];
            let (funding, oi, mark, index, options) = tokio::join!(
                okx_first(&client, "/api/v5/public/funding-rate", &funding_q),
                okx_first(&client, "/api/v5/public/open-interest", &oi_q),
                okx_first(&client, "/api/v5/public/mark-price", &mark_q),
                okx_first(&client, "/api/v5/market/index-tickers", &index_q),
                okx_data(&client, "/api/v5/public/opt-summary", &options_q),
            );
            let timestamp = now_ms();
            let open_interest_contracts = oi.as_ref().and_then(|row| json_number(row, "oi"));
            let open_interest_change_percent = open_interest_contracts.and_then(|current| {
                let change = previous_open_interest
                    .insert(underlying.to_owned(), current)
                    .map(|previous| (current / previous - 1.0) * 100.0);
                change.filter(|value| value.is_finite())
            });
            let mark_price = mark.as_ref().and_then(|row| json_number(row, "markPx"));
            let index_price = index.as_ref().and_then(|row| json_number(row, "idxPx"));
            let option_summary = options
                .as_deref()
                .map(|rows| option_volatility_summary(rows, timestamp))
                .unwrap_or((None, None, None));
            let features = ExternalMarketFeatures {
                underlying: underlying.to_owned(),
                timestamp,
                swap_instrument: swap,
                funding_rate: funding
                    .as_ref()
                    .and_then(|row| json_number(row, "fundingRate")),
                next_funding_rate: funding
                    .as_ref()
                    .and_then(|row| json_number(row, "nextFundingRate")),
                open_interest_contracts,
                open_interest_currency: oi.as_ref().and_then(|row| json_number(row, "oiCcy")),
                open_interest_change_percent,
                mark_price,
                index_price,
                perp_basis_bps: mark_price
                    .zip(index_price)
                    .filter(|(_, index)| *index > 0.0)
                    .map(|(mark, index)| (mark / index - 1.0) * 10_000.0),
                options_atm_iv: option_summary.0,
                options_25d_skew: option_summary.1,
                options_expiry_timestamp: option_summary.2,
            };
            let has_data = features.funding_rate.is_some()
                || features.open_interest_contracts.is_some()
                || features.mark_price.is_some()
                || features.options_atm_iv.is_some()
                || features.options_25d_skew.is_some();
            if has_data {
                if let Err(error) = store.append_external_market_features(&features).await {
                    crate::service_log!(
                        "ERROR",
                        "Failed to store {underlying} derivatives/options context: {error}"
                    );
                }
            }
        }
    }
}

