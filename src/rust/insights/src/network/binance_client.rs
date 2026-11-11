use anyhow::{Context, Result};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request<'a> {
    DailyCandles { symbols: Vec<&'a str> },
}

pub async fn daily_candles(symbols: &str) -> Result<BTreeMap<String, Vec<crate::InsightDailyBar>>> {
    let url = std::env::var("BINANCE_FEED_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10002".to_owned());
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(url.as_str()))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .context("Binance Feed RSocket connection failed")?;
    let request = serde_json::to_vec(&Request::DailyCandles {
        symbols: symbols
            .split(',')
            .filter(|symbol| !symbol.is_empty())
            .collect(),
    })?;
    let response = client
        .request_response(Payload::builder().set_data(request).build())
        .await
        .context("Binance Feed daily candle request failed")?
        .ok_or_else(|| anyhow::anyhow!("Binance Feed returned an empty daily candle response"))?;
    let bytes = response
        .data()
        .ok_or_else(|| anyhow::anyhow!("Binance Feed returned no daily candle data"))?;
    serde_json::from_slice(bytes.as_ref()).context("invalid Binance Feed daily candle response")
}
