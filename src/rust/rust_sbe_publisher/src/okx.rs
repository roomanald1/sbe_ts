use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::HashSet, sync::Arc};
use tokio::sync::broadcast::Sender;
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Utf8Bytes};

use crate::websocket::{SYMBOL_LENGTH, encode_symbol_price};

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const STABLE_CONNECTION_DURATION: Duration = Duration::from_secs(60);

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

#[derive(Deserialize)]
struct TickerMessage<'a> {
    #[serde(borrow)]
    data: Option<Vec<Ticker<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
}

#[derive(Deserialize)]
struct Ticker<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
    #[serde(borrow)]
    last: &'a str,
}

pub async fn subscribe(instrument_data: &[Arc<str>], price_updates: Sender<Bytes>) -> Result<()> {
    let instrument_lookup = Arc::new(instrument_data.iter().cloned().collect::<HashSet<_>>());
    let mut retry_delay = INITIAL_RECONNECT_DELAY;

    loop {
        let attempt_started = Instant::now();
        let result = subscribe_once(
            instrument_data,
            Arc::clone(&instrument_lookup),
            price_updates.clone(),
        )
        .await;

        retry_delay = retry_delay_after_failure(retry_delay, attempt_started.elapsed());

        match result {
            Ok(()) => eprintln!(
                "OKX WebSocket stream ended; reconnecting in {}s",
                retry_delay.as_secs()
            ),
            Err(error) => eprintln!(
                "OKX WebSocket subscription failed: {error:#}; reconnecting in {}s",
                retry_delay.as_secs()
            ),
        }

        sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

async fn subscribe_once(
    instrument_data: &[Arc<str>],
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    price_updates: Sender<Bytes>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX WebSocket: {error}"))?;

    println!("OKX WebSocket handshake has been successfully completed");
    let (mut write, read) = incoming_ws.split();
    let ws = {
        let read = read.map(|message| message.map_err(anyhow::Error::from));
        read.try_for_each(move |message| {
            let instrument_lookup = instrument_lookup.clone();
            let price_updates = price_updates.clone();
            async move {
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        eprintln!("Ignoring non-text OKX WebSocket message: {error}");
                        return Ok(());
                    }
                };
                let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
                    Ok(update) => update,
                    Err(error) => {
                        eprintln!("Ignoring invalid OKX JSON message: {error}");
                        return Ok(());
                    }
                };

                if update.event == Some("error") {
                    anyhow::bail!(
                        "OKX subscription error: {}",
                        update.msg.unwrap_or("unknown subscription error")
                    );
                }

                if let Some(items) = update.data {
                    for item in items {
                        let Some(instrument) = instrument_lookup.get(item.instrument) else {
                            continue;
                        };
                        let Ok(price) = item.last.parse::<f64>() else {
                            continue;
                        };
                        if price.is_finite() && price > 0.0 {
                            let encoded = encode_symbol_price(instrument, price)?;
                            let _ = price_updates.send(encoded);
                        }
                    }
                }
                Ok(())
            }
        })
    };

    for (batch_index, instruments) in instrument_data.chunks(240).enumerate() {
        let args = instruments
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument.as_ref() }))
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

    ws.await?;
    anyhow::bail!("OKX WebSocket stream ended")
}

fn retry_delay_after_failure(current_delay: Duration, connection_duration: Duration) -> Duration {
    if connection_duration >= STABLE_CONNECTION_DURATION {
        INITIAL_RECONNECT_DELAY
    } else {
        current_delay
    }
}

#[cfg(test)]
mod tests {
    use super::{
        INITIAL_RECONNECT_DELAY, MAX_RECONNECT_DELAY, STABLE_CONNECTION_DURATION, TickerMessage,
        retry_delay_after_failure, select_top_instruments,
    };
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn reconnect_delay_resets_after_a_stable_connection() {
        assert_eq!(
            retry_delay_after_failure(
                MAX_RECONNECT_DELAY,
                STABLE_CONNECTION_DURATION + Duration::from_secs(1)
            ),
            INITIAL_RECONNECT_DELAY
        );
    }

    #[test]
    fn reconnect_delay_is_retained_after_a_short_connection() {
        assert_eq!(
            retry_delay_after_failure(Duration::from_secs(8), Duration::from_secs(5)),
            Duration::from_secs(8)
        );
    }

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

    #[test]
    fn ticker_updates_borrow_instrument_and_price_from_json() {
        let text =
            r#"{"arg":{"channel":"tickers"},"data":[{"instId":"BTC-USDT","last":"84000.5"}]}"#;
        let message: TickerMessage<'_> = serde_json::from_str(text).unwrap();
        let ticker = &message.data.unwrap()[0];

        assert_eq!(ticker.instrument, "BTC-USDT");
        assert_eq!(ticker.last, "84000.5");
    }
}
