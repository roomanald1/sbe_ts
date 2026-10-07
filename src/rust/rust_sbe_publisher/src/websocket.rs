use anyhow::Result;
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use sbe_schema::{SBE_BLOCK_LENGTH, SymbolPriceEncoder, WriteBuf, message_header_codec};
use std::{env, pin::Pin};
use tokio::sync::broadcast::{self};
use tokio::time::{Duration, sleep};

use crate::okx::{get_instrument_data, subscribe};

pub const SYMBOL_LENGTH: usize = 32;
const PRICE_UPDATE_CAPACITY: usize = 4096;

pub async fn serve() -> Result<()> {
    let instrument_data = get_instrument_data().await?;
    let (price_updates, _) = broadcast::channel::<(String, f64)>(PRICE_UPDATE_CAPACITY);
    let sub_sender = price_updates.clone();
    let sub = subscribe(&instrument_data, sub_sender);

    let port = env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{port}");

    println!("RSocket WebSocket server on ws://{addr}");

    let r_socket = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(addr))
        .acceptor(Box::new(move |setup, _socket| {
            println!("socket establish: setup={:?}", setup);
            Ok(Box::new(ServerResponder {
                price_updates: price_updates.clone(),
            }))
        }))
        .serve();

    let (_ws_result, r_socket_result) = tokio::join!(sub, r_socket);
    r_socket_result?;
    Ok(())
}

fn encode_symbol_price(symbol: &str, price: f64) -> Result<Vec<u8>> {
    anyhow::ensure!(
        symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH,
        "instrument ID does not fit the SBE symbol field: {symbol}"
    );

    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + SBE_BLOCK_LENGTH as usize];

    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(&mut buffer), header_length);
    let mut symbol_bytes = [b' '; SYMBOL_LENGTH];
    symbol_bytes[..symbol.len()].copy_from_slice(symbol.as_bytes());
    encoder.symbol(symbol_bytes);
    encoder.price(price);
    drop(encoder.header(0));

    Ok(buffer)
}

struct ServerResponder {
    price_updates: broadcast::Sender<(String, f64)>,
}

impl RSocket for ServerResponder {
    fn request_stream(
        &self,
        payload: Payload,
    ) -> Pin<
        Box<
            dyn Stream<Item = Result<rsocket_rust::prelude::Payload, anyhow::Error>>
                + Send
                + 'static,
        >,
    > {
        let synthetic = is_synthetic_request(&payload);
        println!(
            "RSocket request stream source: {}",
            if synthetic { "synthetic" } else { "OKX" }
        );
        let stream = if synthetic {
            futures_util::stream::unfold((0_u64, 0_usize), |(round, symbol_index)| async move {
                if symbol_index == 0 {
                    sleep(Duration::from_millis(10)).await;
                }

                let (symbol, price) = synthetic_price_update(round, symbol_index);
                let result = encode_symbol_price(&symbol, price)
                    .map(|data| Payload::builder().set_data(data).build());
                let next_state = if symbol_index == 99 {
                    (round.wrapping_add(1), 0)
                } else {
                    (round, symbol_index + 1)
                };
                Some((result, next_state))
            })
            .boxed()
        } else {
            futures_util::stream::unfold(self.price_updates.subscribe(), |mut updates| async move {
                match updates.recv().await {
                    Ok((instrument, price)) => {
                        let result = encode_symbol_price(&instrument, price)
                            .map(|data| Payload::builder().set_data(data).build());
                        Some((result, updates))
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => Some((
                        Err(anyhow::anyhow!(
                            "RSocket price stream fell behind and skipped {skipped} updates"
                        )),
                        updates,
                    )),
                    Err(broadcast::error::RecvError::Closed) => None,
                }
            })
            .boxed()
        };

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

fn synthetic_price_update(round: u64, symbol_index: usize) -> (String, f64) {
    let symbol = format!("SYM{symbol_index:03}");
    let base_price = 25.0 + symbol_index as f64 * 4.75;
    let phase = round as f64 * 0.17 + symbol_index as f64 * 1.91;
    let movement = phase.sin() * 0.004 + (phase * 0.37).cos() * 0.001;
    (symbol, base_price * (1.0 + movement))
}

fn is_synthetic_request(payload: &Payload) -> bool {
    payload
        .data()
        .is_some_and(|data| data.as_ref() == b"synthetic")
}

#[cfg(test)]
mod tests {
    use super::{encode_symbol_price, is_synthetic_request, synthetic_price_update};
    use rsocket_rust::prelude::Payload;

    #[test]
    fn encodes_full_instrument_id_and_price_in_sbe_payload() {
        let payload = encode_symbol_price("1INCH-USDT", 1.25).unwrap();
        let mut expected_symbol = [b' '; 32];
        expected_symbol[..10].copy_from_slice(b"1INCH-USDT");
        assert_eq!(&payload[8..40], expected_symbol);
        assert_eq!(
            f64::from_le_bytes(payload[40..48].try_into().unwrap()),
            1.25
        );
    }

    #[test]
    fn rejects_instrument_ids_that_exceed_the_sbe_field() {
        assert!(encode_symbol_price("123456789012345678901234567890123", 1.0).is_err());
    }

    #[test]
    fn synthetic_updates_produce_fast_feed_symbols_and_vary_by_round() {
        let (symbol, first_price) = synthetic_price_update(0, 0);
        let (_, next_price) = synthetic_price_update(1, 0);

        assert_eq!(symbol, "SYM000");
        assert_ne!(first_price, next_price);
        assert!(first_price.is_finite() && first_price > 0.0);
    }

    #[test]
    fn selects_synthetic_stream_only_for_synthetic_request_payload() {
        let synthetic = Payload::builder().set_data(b"synthetic".to_vec()).build();
        let okx = Payload::builder().set_data(b"okx".to_vec()).build();

        assert!(is_synthetic_request(&synthetic));
        assert!(!is_synthetic_request(&okx));
        assert!(!is_synthetic_request(&Payload::new(None, None)));
    }
}
