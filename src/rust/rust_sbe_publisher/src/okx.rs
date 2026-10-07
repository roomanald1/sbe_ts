use anyhow::Result;
use futures_util::{SinkExt, StreamExt};
use reqwest::header::USER_AGENT;
use serde_json::{Value, json};
use tokio::sync::broadcast::Sender;
use tokio_tungstenite::{connect_async, tungstenite::Utf8Bytes};

use crate::websocket::SYMBOL_LENGTH;

pub async fn get_instrument_data() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/market/tickers?instType=SPOT";
    println!("{}", request_url);

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

pub async fn subscribe(
    instrument_data: &Vec<String>,
    price_updates: Sender<(String, f64)>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX WebSocket: {error}"))?;

    println!("OKX WebSocket handshake has been successfully completed");
    let (mut write, read) = incoming_ws.split();
    let ws = {
        let price_updates = price_updates.clone();
        read.for_each(move |message| {
            let price_updates = price_updates.clone();
            async move {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        eprintln!("OKX WebSocket receive error: {error}");
                        return;
                    }
                };
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        eprintln!("Ignoring non-text OKX WebSocket message: {error}");
                        return;
                    }
                };
                let json = match serde_json::from_str::<Value>(text) {
                    Ok(json) => json,
                    Err(error) => {
                        eprintln!("Ignoring invalid OKX JSON message: {error}");
                        return;
                    }
                };

                if json.get("event").and_then(Value::as_str) == Some("error") {
                    eprintln!("OKX subscription error: {json}");
                    return;
                }

                if let Some(items) = json.get("data").and_then(Value::as_array) {
                    for item in items {
                        let Some(instrument) = item.get("instId").and_then(Value::as_str) else {
                            continue;
                        };
                        let Some(price) = item
                            .get("last")
                            .and_then(Value::as_str)
                            .and_then(|price| price.parse::<f64>().ok())
                        else {
                            continue;
                        };
                        if !price.is_finite() || price <= 0.0 {
                            continue;
                        }
                        let _ = price_updates.send((instrument.to_owned(), price));
                    }
                }
            }
        })
    };

    for (batch_index, instruments) in instrument_data.chunks(240).enumerate() {
        let args = instruments
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument }))
            .collect::<Vec<_>>();
        let request = json!({
            "op": "subscribe",
            "id": (batch_index + 1).to_string(),
            "args": args
        });
        println!("Subscribing to {}", request);
        let request = Utf8Bytes::from(request.to_string());

        write
            .send(tokio_tungstenite::tungstenite::Message::Text(request))
            .await?;
    }

    Ok(ws.await)
}

#[cfg(test)]
mod tests {
    use super::select_top_instruments;
    use serde_json::json;

    #[test]
    fn selects_top_spot_instruments_by_24_hour_quote_volume() {
        let tickers = vec![
            json!({"instId": "LOW-USDT", "volCcy24h": "10"}),
            json!({"instId": "HIGH-USDT", "volCcy24h": "1000"}),
            json!({"instId": "MID-USDT", "volCcy24h": "100"}),
        ];

        assert_eq!(
            select_top_instruments(&tickers).unwrap(),
            ["HIGH-USDT", "MID-USDT", "LOW-USDT"]
        );
    }

    #[test]
    fn selects_no_more_than_the_top_100_instruments() {
        let tickers = (0..110)
            .map(|index| {
                json!({
                    "instId": format!("COIN{index:03}-USDT"),
                    "volCcy24h": index.to_string()
                })
            })
            .collect::<Vec<_>>();

        let selected = select_top_instruments(&tickers).unwrap();
        assert_eq!(selected.len(), 100);
        assert_eq!(selected[0], "COIN109-USDT");
        assert_eq!(selected[99], "COIN010-USDT");
    }
}
