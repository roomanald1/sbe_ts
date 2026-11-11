use anyhow::{Context, Result, bail};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::Deserialize;
use std::pin::Pin;

const INSTRUMENTS_REQUEST: &str = "instruments:list";

#[derive(Deserialize)]
struct InstrumentsResponse {
    symbols: Vec<String>,
}

pub async fn spot_symbols() -> Result<Vec<String>> {
    let url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(url.as_str()))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .context("FeedHandlers RSocket connection failed")?;
    let response = client
        .request_response(instrument_request_payload())
        .await
        .context("FeedHandlers instrument request failed")?
        .ok_or_else(|| anyhow::anyhow!("FeedHandlers returned an empty instrument response"))?;
    let data = response
        .data()
        .ok_or_else(|| anyhow::anyhow!("FeedHandlers returned no instrument data"))?;
    let symbols = decode_spot_symbols(data.as_ref())?;
    if symbols.is_empty() {
        bail!("FeedHandlers returned no spot symbols");
    }
    Ok(symbols)
}

fn decode_spot_symbols(data: &[u8]) -> Result<Vec<String>> {
    let response: InstrumentsResponse =
        serde_json::from_slice(data).context("Invalid FeedHandlers instrument response")?;
    if response.symbols.is_empty() {
        bail!("FeedHandlers returned no spot symbols");
    }
    Ok(response.symbols)
}

fn instrument_request_payload() -> Payload {
    Payload::builder()
        .set_data_utf8(INSTRUMENTS_REQUEST)
        .build()
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

#[cfg(test)]
mod tests {
    use super::{decode_spot_symbols, instrument_request_payload};

    #[test]
    fn uses_feed_handlers_instrument_list_route() {
        assert_eq!(
            instrument_request_payload().data().unwrap().as_ref(),
            b"instruments:list"
        );
    }

    #[test]
    fn decodes_symbols_from_feed_handlers_response() {
        assert_eq!(
            decode_spot_symbols(br#"{"symbols":["BTC-USDT","ETH-USDT"],"instruments":[]}"#)
                .unwrap(),
            ["BTC-USDT", "ETH-USDT"]
        );
    }

    #[test]
    fn rejects_malformed_instrument_response() {
        assert!(decode_spot_symbols(b"not-json").is_err());
    }

    #[test]
    fn rejects_empty_instrument_response() {
        assert!(decode_spot_symbols(br#"{"symbols":[],"instruments":[]}"#).is_err());
    }
}
