use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use std::pin::Pin;

pub type ByteStream = Pin<Box<dyn Stream<Item = Result<Bytes>> + Send>>;

pub async fn signal_events() -> Result<ByteStream> {
    let url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(url.as_str()))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .context("FeedHandlers RSocket connection failed")?;
    let source = client.request_stream(Payload::builder().set_data_utf8("signal-events:*").build());
    Ok(Box::pin(source.map(move |item| {
        let _client = &client;
        item.map_err(anyhow::Error::from).and_then(|payload| {
            payload.data().cloned().ok_or_else(|| anyhow::anyhow!("FeedHandlers returned an empty signal event"))
        })
    })))
}
