pub async fn get_instrument_data() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/market/tickers?instType=SPOT";
    crate::service_log!("INFO", "{}", request_url);

    let client = reqwest::Client::new();
    fetch_instrument_data(&client, request_url).await
}

async fn fetch_instrument_data(client: &reqwest::Client, request_url: &str) -> Result<Vec<String>> {
    let response = client
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;

    let json: Value = serde_json::from_str(&response.text().await?)?;

    let tickers = response_data(&json, "OKX spot tickers response is missing data")?;
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
    let client = reqwest::Client::new();
    fetch_spot_instrument_precisions(
        &client,
        "https://www.okx.com/api/v5/public/instruments?instType=SPOT",
    )
    .await
}

async fn fetch_spot_instrument_precisions(
    client: &reqwest::Client,
    request_url: &str,
) -> Result<Vec<SpotInstrumentPrecision>> {
    let response = client
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = response_data(&json, "OKX spot instruments response is missing data")?;
    parse_spot_instrument_precisions(instruments)
}

fn parse_spot_instrument_precisions(instruments: &[Value]) -> Result<Vec<SpotInstrumentPrecision>> {
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
    let client = reqwest::Client::new();
    fetch_all_spot_instruments(&client, request_url).await
}

async fn fetch_all_spot_instruments(
    client: &reqwest::Client,
    request_url: &str,
) -> Result<Vec<String>> {
    let response = client
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = response_data(&json, "OKX spot instruments response is missing data")?;
    parse_live_spot_instruments(instruments)
}

fn response_data<'a>(response: &'a Value, error_message: &str) -> Result<&'a [Value]> {
    response
        .get("data")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(|| anyhow::anyhow!("{error_message}"))
}

fn parse_live_spot_instruments(instruments: &[Value]) -> Result<Vec<String>> {
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

#[cfg(test)]
mod instrument_catalog_tests {
    use super::{
        fetch_all_spot_instruments, fetch_instrument_data, fetch_spot_instrument_precisions,
        parse_live_spot_instruments, parse_spot_instrument_precisions, response_data,
        select_top_instruments,
    };
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn mock_http_response(body: &'static str, status: &'static str) -> (String, tokio::task::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0_u8; 4096];
            let _ = stream.read(&mut request).await.unwrap();
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });
        (format!("http://{address}"), task)
    }

    #[test]
    fn parses_only_live_ascii_instruments_and_sorts_deduplicated_symbols() {
        let rows = vec![
            json!({"instId":"ZED-USDT","state":"live"}),
            json!({"instId":"ABC-USDT","state":"live"}),
            json!({"instId":"ZED-USDT","state":"live"}),
            json!({"instId":"OLD-USDT","state":"suspend"}),
            json!({"instId":"币-USDT","state":"live"}),
        ];
        assert_eq!(parse_live_spot_instruments(&rows).unwrap(), ["ABC-USDT", "ZED-USDT"]);
    }

    #[test]
    fn parses_sorted_precisions_for_live_symbols_only() {
        let rows = vec![
            json!({"instId":"ZED-USDT","state":"live","tickSz":"0.1","lotSz":"1"}),
            json!({"instId":"OLD-USDT","state":"suspend","tickSz":"0.1","lotSz":"1"}),
            json!({"instId":"ABC-USDT","state":"live","tickSz":"0.01","lotSz":"0.1"}),
            json!({"instId":"BAD-USDT","state":"live","tickSz":"0.1"}),
        ];
        let parsed = parse_spot_instrument_precisions(&rows).unwrap();
        assert_eq!(parsed.iter().map(|row| row.symbol.as_str()).collect::<Vec<_>>(), ["ABC-USDT", "ZED-USDT"]);
        assert_eq!(parsed[0].tick_size, "0.01");
        assert_eq!(parsed[0].lot_size, "0.1");
    }

    #[test]
    fn rejects_empty_valid_instrument_sets() {
        assert!(parse_live_spot_instruments(&[json!({"instId":"A","state":"suspend"})]).is_err());
        assert!(parse_spot_instrument_precisions(&[]).is_err());
        assert!(select_top_instruments(&[json!({"instId":"BTC-USDT","volCcy24h":"NaN"})]).is_err());
    }

    #[test]
    fn extracts_data_arrays_and_reports_missing_or_non_array_data() {
        let response = json!({"data":[{"instId":"BTC-USDT"}]});
        assert_eq!(response_data(&response, "missing").unwrap().len(), 1);
        assert!(response_data(&json!({}), "missing data").unwrap_err().to_string().contains("missing data"));
        assert!(response_data(&json!({"data":null}), "not an array").is_err());
    }

    #[tokio::test]
    async fn fetches_catalog_responses_from_http_and_validates_status_and_payloads() {
        let client = reqwest::Client::new();
        let (url, server) = mock_http_response(
            r#"{"data":[{"instId":"BTC-USDT","volCcy24h":"10"},{"instId":"ETH-USDT","volCcy24h":"20"}]}"#,
            "200 OK",
        )
        .await;
        assert_eq!(fetch_instrument_data(&client, &url).await.unwrap(), ["ETH-USDT", "BTC-USDT"]);
        server.await.unwrap();

        let (url, server) = mock_http_response(
            r#"{"data":[{"instId":"BTC-USDT","state":"live","tickSz":"0.1","lotSz":"1"}]}"#,
            "200 OK",
        )
        .await;
        let precision = fetch_spot_instrument_precisions(&client, &url).await.unwrap();
        assert_eq!(precision[0].symbol, "BTC-USDT");
        server.await.unwrap();

        let (url, server) = mock_http_response(
            r#"{"data":[{"instId":"BTC-USDT","state":"live"},{"instId":"ETH-USDT","state":"live"}]}"#,
            "200 OK",
        )
        .await;
        assert_eq!(fetch_all_spot_instruments(&client, &url).await.unwrap(), ["BTC-USDT", "ETH-USDT"]);
        server.await.unwrap();

        let (url, server) = mock_http_response("{}", "200 OK").await;
        assert!(fetch_instrument_data(&client, &url).await.is_err());
        server.await.unwrap();

        let (url, server) = mock_http_response("{}", "500 Internal Server Error").await;
        assert!(fetch_all_spot_instruments(&client, &url).await.is_err());
        server.await.unwrap();
    }
}


