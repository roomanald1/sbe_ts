use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::Deserialize;
use std::pin::Pin;

#[derive(Deserialize)]
struct InstrumentsResponse {
    symbols: Vec<String>,
}

pub async fn spot_symbols() -> Result<Vec<String>> {
    let base =
        std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
    let response = reqwest::Client::new()
        .get(format!("{base}/instruments"))
        .send()
        .await
        .context("FeedHandlers instrument request failed")?
        .error_for_status()
        .context("FeedHandlers instrument request returned an error")?
        .json::<InstrumentsResponse>()
        .await
        .context("Invalid FeedHandlers instrument response")?;
    let symbols = response.symbols;
    if symbols.is_empty() {
        bail!("FeedHandlers returned no spot symbols");
    }
    Ok(symbols)
}

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

pub async fn stream(request: String) -> Result<ByteStream> {
    let url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(url.as_str()))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .context("FeedHandlers RSocket connection failed")?;
    let source = client.request_stream(Payload::builder().set_data_utf8(&request).build());
    Ok(Box::pin(source.map(move |item| {
        // Keep the RSocket connection alive for as long as its request stream is polled.
        let _client = &client;
        item.map_err(anyhow::Error::from).and_then(|payload| {
            payload
                .data()
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("FeedHandlers returned an empty payload"))
        })
    })))
}
