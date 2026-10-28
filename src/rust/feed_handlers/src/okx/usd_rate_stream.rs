pub async fn run_usd_rate_stream(output: watch::Sender<Bytes>) {
    loop {
        match bootstrap_usd_rates().await {
            Ok((instruments, mut snapshot)) => {
                if let Ok(bytes) = serde_json::to_vec(&snapshot) {
                    output.send_replace(Bytes::from(bytes));
                }
                let (update_tx, mut update_rx) = mpsc::channel(1024);
                let subscription = subscribe_usd_index_tickers(&instruments, update_tx);
                tokio::pin!(subscription);
                loop {
                    tokio::select! {
                        result = &mut subscription => {
                            if let Err(error) = result {
                                crate::service_log!("ERROR", "OKX USD index ticker stream disconnected: {error:#}");
                            } else {
                                crate::service_log!("WARN", "OKX USD index ticker stream ended");
                            }
                            break;
                        }
                        update = update_rx.recv() => {
                            let Some(update) = update else { break; };
                            snapshot.rates.insert(update.currency.clone(), update.rate);
                            snapshot.updated_at_by_currency.insert(update.currency, update.timestamp);
                            snapshot.updated_at = snapshot.updated_at.max(update.timestamp);
                            if let Ok(bytes) = serde_json::to_vec(&snapshot) {
                                output.send_replace(Bytes::from(bytes));
                            }
                        }
                    }
                }
            }
            Err(error) => crate::service_log!("ERROR", "Could not bootstrap OKX USD index rates: {error:#}"),
        }
        sleep(Duration::from_secs(1)).await;
    }
}

async fn bootstrap_usd_rates() -> Result<(Vec<String>, UsdRateSnapshot)> {
    let response = reqwest::Client::new()
        .get("https://www.okx.com/api/v5/market/index-tickers?quoteCcy=USD")
        .send().await?.error_for_status()?.json::<IndexTickerRestResponse>().await?;
    let mut instruments = Vec::new();
    let mut rates = BTreeMap::from([("USD".to_owned(), 1.0)]);
    let mut updated_at_by_currency = BTreeMap::from([("USD".to_owned(), 0)]);
    let mut updated_at = 0;
    for item in response.data {
        let Some((currency, quote)) = item.instrument.split_once('-') else { continue; };
        let Ok(rate) = item.index_price.parse::<f64>() else { continue; };
        let Ok(timestamp) = item.ts.parse::<u64>() else { continue; };
        if quote != "USD" || !rate.is_finite() || rate <= 0.0 { continue; }
        let currency = currency.to_owned();
        instruments.push(item.instrument);
        rates.insert(currency.clone(), rate);
        updated_at_by_currency.insert(currency, timestamp);
        updated_at = updated_at.max(timestamp);
    }
    if instruments.is_empty() { anyhow::bail!("OKX returned no usable USD index rates"); }
    instruments.sort();
    instruments.dedup();
    Ok((instruments, UsdRateSnapshot { rates, updated_at, updated_at_by_currency }))
}

async fn subscribe_usd_index_tickers(instruments: &[String], updates: mpsc::Sender<UsdIndexUpdate>) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public").await?;
    let (mut write, mut read) = incoming_ws.split();
    for (batch_index, batch) in instruments.chunks(240).enumerate() {
        let args = batch.iter().map(|instrument| json!({ "channel": "index-tickers", "instId": instrument })).collect::<Vec<_>>();
        let request = json!({ "op": "subscribe", "id": (batch_index + 1).to_string(), "args": args });
        write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from(request.to_string()))).await?;
        if batch_index + 1 < instruments.chunks(240).len() { sleep(Duration::from_millis(500)).await; }
    }
    loop {
        let Some(message) = read.next().await else { anyhow::bail!("OKX USD index ticker stream ended") };
        let message = message?;
        if !message.is_text() { continue; }
        let update = match serde_json::from_str::<IndexTickerWsMessage>(message.to_text()?) {
            Ok(update) => update,
            Err(error) => { crate::service_log!("ERROR", "Ignoring invalid OKX USD index ticker JSON: {error}"); continue; }
        };
        if update.event.as_deref() == Some("error") {
            anyhow::bail!("OKX USD index subscription error: {}", update.msg.unwrap_or_else(|| "unknown error".to_owned()));
        }
        for item in update.data.unwrap_or_default() {
            let (Some((currency, quote)), Ok(rate), Ok(timestamp)) = (
                item.instrument.split_once('-'), item.index_price.parse::<f64>(), item.ts.parse::<u64>()
            ) else { continue; };
            if quote != "USD" || !rate.is_finite() || rate <= 0.0 { continue; }
            updates.send(UsdIndexUpdate { currency: currency.to_owned(), rate, timestamp }).await
                .map_err(|_| anyhow::anyhow!("USD rate update receiver was dropped"))?;
        }
    }
}

