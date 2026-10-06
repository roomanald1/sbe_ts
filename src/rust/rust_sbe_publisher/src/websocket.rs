use anyhow::Result;
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use sbe_schema::{SBE_BLOCK_LENGTH, SymbolPriceEncoder, WriteBuf, message_header_codec};
use std::{env, pin::Pin};
use tokio::time::{Duration, sleep};

pub async fn serve() -> Result<()> {
    let port = env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{port}");

    println!("RSocket WebSocket server on ws://{addr}");

    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(addr))
        .acceptor(Box::new(|setup, _socket| {
            println!("socket establish: setup={:?}", setup);
            Ok(Box::new(ServerResponder))
        }))
        .serve()
        .await?;

    Ok(())
}

fn encode_symbol_price(symbol: [u8; 8], price: f64) -> Vec<u8> {
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + SBE_BLOCK_LENGTH as usize];

    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(symbol);
    encoder.price(price);
    drop(encoder.header(0));

    buffer
}

struct ServerResponder;

impl RSocket for ServerResponder {
    fn request_stream(
        &self,
        _payload: Payload,
    ) -> Pin<
        Box<
            dyn Stream<Item = Result<rsocket_rust::prelude::Payload, anyhow::Error>>
                + Send
                + 'static,
        >,
    > {
        // Emit one update for all symbols every 10 ms.
        let stream =
            futures_util::stream::unfold((0_u64, 0_usize), |(round, symbol_index)| async move {
                if symbol_index == 0 {
                    sleep(Duration::from_millis(10)).await;
                }

                let symbol_text = format!("SYM{symbol_index:03}");
                let mut symbol = [b' '; 8];
                symbol[..symbol_text.len()].copy_from_slice(symbol_text.as_bytes());
                let base_price = 25.0 + symbol_index as f64 * 4.75;
                let phase = round as f64 * 0.17 + symbol_index as f64 * 1.91;
                let movement = phase.sin() * 0.004 + (phase * 0.37).cos() * 0.001;
                let price = base_price * (1.0 + movement);
                let payload = Payload::builder()
                    .set_data(encode_symbol_price(symbol, price))
                    .build();

                let next_state = if symbol_index == 99 {
                    (round.wrapping_add(1), 0)
                } else {
                    (round, symbol_index + 1)
                };

                Some((Ok(payload), next_state))
            });

        Box::pin(stream)
    }

    #[doc = " Metadata-Push interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Fire and Forget interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Request-Response interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<Option<Payload>>>
                + ::core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Request-Channel interaction model of RSocket."]
    fn request_channel(&self, reqs: Flux<Result<Payload>>) -> Flux<Result<Payload>> {
        todo!()
    }
}
