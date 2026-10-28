pub async fn get_instrument_data() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/market/tickers?instType=SPOT";
    crate::service_log!("INFO", "{}", request_url);

    let client = reqwest::Client::new();
    let response = client
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;

    let json: Value = serde_json::from_str(&response.text().await?)?;

    let tickers = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot tickers response is missing data"))?;
    select_top_instruments(tickers)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SpotInstrumentPrecision {
    pub symbol: String,
    #[serde(rename = "tickSz")]
    pub tick_size: String,
    #[serde(rename = "lotSz")]
    pub lot_size: String,
}

pub async fn get_spot_instrument_precisions() -> Result<Vec<SpotInstrumentPrecision>> {
    let response = reqwest::Client::new()
        .get("https://www.okx.com/api/v5/public/instruments?instType=SPOT")
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot instruments response is missing data"))?;
    let mut result = instruments
        .iter()
        .filter_map(|instrument| {
            let symbol = instrument.get("instId")?.as_str()?;
            let is_live = instrument.get("state").and_then(Value::as_str) == Some("live");
            let tick_size = instrument.get("tickSz")?.as_str()?;
            let lot_size = instrument.get("lotSz")?.as_str()?;
            (is_live && symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH).then(|| {
                SpotInstrumentPrecision {
                    symbol: symbol.to_owned(),
                    tick_size: tick_size.to_owned(),
                    lot_size: lot_size.to_owned(),
                }
            })
        })
        .collect::<Vec<_>>();
    result.sort_unstable_by(|a, b| a.symbol.cmp(&b.symbol));
    anyhow::ensure!(
        !result.is_empty(),
        "OKX returned no live spot instrument precisions"
    );
    Ok(result)
}

pub async fn get_all_spot_instruments() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/public/instruments?instType=SPOT";
    let response = reqwest::Client::new()
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot instruments response is missing data"))?;
    let mut symbols = instruments
        .iter()
        .filter_map(|instrument| {
            let symbol = instrument.get("instId")?.as_str()?;
            let is_live = instrument.get("state").and_then(Value::as_str) == Some("live");
            (is_live && symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH)
                .then(|| symbol.to_owned())
        })
        .collect::<Vec<_>>();
    symbols.sort_unstable();
    symbols.dedup();
    anyhow::ensure!(!symbols.is_empty(), "OKX returned no live spot instruments");
    Ok(symbols)
}

fn select_top_instruments(tickers: &[Value]) -> Result<Vec<String>> {
    let mut instruments = tickers
        .iter()
        .filter_map(|ticker| {
            let instrument = ticker.get("instId")?.as_str()?;
            let volume = ticker.get("volCcy24h")?.as_str()?.parse::<f64>().ok()?;

            (instrument.is_ascii()
                && instrument.len() <= SYMBOL_LENGTH
                && volume.is_finite()
                && volume >= 0.0)
                .then(|| (instrument.to_owned(), volume))
        })
        .collect::<Vec<_>>();
    instruments.sort_by(|(left_id, left_volume), (right_id, right_volume)| {
        right_volume
            .total_cmp(left_volume)
            .then_with(|| left_id.cmp(right_id))
    });
    instruments.truncate(100);

    anyhow::ensure!(
        !instruments.is_empty(),
        "OKX returned no valid spot tickers"
    );

    Ok(instruments
        .into_iter()
        .map(|(instrument, _)| instrument)
        .collect())
}


