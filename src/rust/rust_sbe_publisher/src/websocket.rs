use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{collections::HashSet, env, pin::Pin, sync::Arc};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, sleep};

use crate::encoders::{SYMBOL_LENGTH, encode_symbol_price_bytes};
use crate::okx::connectivity::{get_instrument_data, subscribe, subscribe_book_once};

const PRICE_UPDATE_CAPACITY: usize = 4096;
const BOOK_UPDATE_CAPACITY: usize = 32;

pub async fn serve() -> Result<()> {
    let instrument_data = get_instrument_data()
        .await?
        .into_iter()
        .map(Arc::<str>::from)
        .collect::<Vec<_>>();
    let instrument_lookup = Arc::new(instrument_data.iter().cloned().collect::<HashSet<_>>());
    let (price_updates, _) = broadcast::channel::<Bytes>(PRICE_UPDATE_CAPACITY);
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
                instrument_lookup: Arc::clone(&instrument_lookup),
            }))
        }))
        .serve();

    tokio::try_join!(sub, r_socket)?;
    Ok(())
}

struct ServerResponder {
    price_updates: tokio::sync::broadcast::Sender<Bytes>,
    instrument_lookup: Arc<HashSet<Arc<str>>>,
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
        if let Some(instrument) = requested_book_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid OKX book instrument request"))
                }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let instrument = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move {
                    Err(anyhow::anyhow!(
                        "instrument is not available in the current OKX subscription: {instrument}"
                    ))
                }));
            };
            println!("Subscribing to OKX books for {instrument}");
            return log_stream_end(
                book_request_stream(instrument.to_string()),
                format!("RSocket UI disconnected or cancelled the OKX book stream for {instrument}"),
            );
        }

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
                let result = Ok(Payload::builder()
                    .set_data(encode_symbol_price_bytes(&symbol, price))
                    .build());
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
                loop {
                    match updates.recv().await {
                        Ok(data) => {
                            break Some((Ok(Payload::new(Some(data), None)), updates));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            eprintln!(
                                "RSocket price stream recovered after skipping {skipped} updates"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => break None,
                    }
                }
            })
            .boxed()
        };

        log_stream_end(stream, "RSocket UI disconnected or cancelled the market price stream".to_owned())
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

struct StreamEndLog(String);

impl Drop for StreamEndLog {
    fn drop(&mut self) {
        println!("{}", self.0);
    }
}

fn log_stream_end<S>(
    stream: S,
    message: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>>
where
    S: Stream<Item = Result<Payload, anyhow::Error>> + Send + Unpin + 'static,
{
    Box::pin(futures_util::stream::unfold(
        (stream, StreamEndLog(message)),
        |(mut stream, end_log)| async move {
            stream
                .next()
                .await
                .map(|item| (item, (stream, end_log)))
        },
    ))
}

fn requested_book_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    let request = std::str::from_utf8(data.as_ref()).ok()?;
    request.strip_prefix("book:")
}

fn book_request_stream(
    instrument: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
    let (sender, receiver) = mpsc::channel(BOOK_UPDATE_CAPACITY);
    tokio::spawn(async move {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            if sender.is_closed() {
                println!("UI unsubscribed from OKX book stream for {instrument} before connection setup");
                break;
            }

            match subscribe_book_once(&instrument, sender.clone()).await {
                Ok(()) => break,
                Err(error) => {
                    eprintln!("OKX book stream for {instrument} failed: {error:#}; retrying in {}s", retry_delay.as_secs());
                    tokio::select! {
                        _ = sender.closed() => break,
                        _ = sleep(retry_delay) => {}
                    }
                    retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
                }
            }
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|item| {
            let item = item.map(|bytes| Payload::new(Some(bytes), None));
            (item, receiver)
        })
    })
    .boxed();
    Box::pin(stream)
}

fn synthetic_price_update(round: u64, symbol_index: usize) -> ([u8; SYMBOL_LENGTH], f64) {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..3].copy_from_slice(b"SYM");
    symbol[3] = b'0' + (symbol_index / 100) as u8;
    symbol[4] = b'0' + ((symbol_index / 10) % 10) as u8;
    symbol[5] = b'0' + (symbol_index % 10) as u8;
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
    use super::{is_synthetic_request, requested_book_instrument, synthetic_price_update};
    use crate::encoders::encode_symbol_price_bytes;
    use rsocket_rust::prelude::Payload;

    #[test]
    fn synthetic_updates_produce_fast_feed_symbols_and_vary_by_round() {
        let (symbol, first_price) = synthetic_price_update(0, 0);
        let (_, next_price) = synthetic_price_update(1, 0);

        assert_eq!(&symbol[..6], b"SYM000");
        assert!(symbol[6..].iter().all(|byte| *byte == b' '));
        assert_ne!(first_price, next_price);
        assert!(first_price.is_finite() && first_price > 0.0);
        assert_eq!(encode_symbol_price_bytes(&symbol, first_price).len(), 48);
    }

    #[test]
    fn selects_synthetic_stream_only_for_synthetic_request_payload() {
        let synthetic = Payload::builder().set_data(b"synthetic".to_vec()).build();
        let okx = Payload::builder().set_data(b"okx".to_vec()).build();
        let book = Payload::builder()
            .set_data(b"book:BTC-USDT".to_vec())
            .build();

        assert!(is_synthetic_request(&synthetic));
        assert!(!is_synthetic_request(&okx));
        assert!(!is_synthetic_request(&Payload::new(None, None)));
        assert_eq!(requested_book_instrument(&book), Some("BTC-USDT"));
    }
}
