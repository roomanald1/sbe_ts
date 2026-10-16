use anyhow::Result;
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{collections::{HashMap, HashSet}, env, pin::Pin, sync::{Arc, Mutex as StdMutex}};
use tokio::sync::{broadcast, mpsc};
use tokio::time::{Duration, sleep};

use crate::encoders::{SYMBOL_LENGTH, encode_symbol_price_bytes};
use crate::signals::{SignalEngine, SignalStore};
use crate::okx::connectivity::{
    RollingVolatility, get_all_spot_instruments, get_instrument_data, subscribe,
    subscribe_book_once, subscribe_trades_once,
};

const PRICE_UPDATE_CAPACITY: usize = 4096;
const BOOK_UPDATE_CAPACITY: usize = 32;
const TRADE_UPDATE_CAPACITY: usize = 32;

pub async fn serve() -> Result<()> {
    let instrument_data = get_instrument_data()
        .await?
        .into_iter()
        .map(Arc::<str>::from)
        .collect::<Vec<_>>();
    let all_spot_instruments = match get_all_spot_instruments().await {
        Ok(symbols) => symbols,
        Err(error) => {
            eprintln!(
                "Unable to load full spot instrument lookup: {error:#}; using top ticker symbols"
            );
            instrument_data.iter().map(ToString::to_string).collect()
        }
    };
    let instrument_lookup = Arc::new(
        all_spot_instruments
            .into_iter()
            .map(Arc::<str>::from)
            .collect::<HashSet<_>>(),
    );
    let (price_updates, _) = broadcast::channel::<Bytes>(PRICE_UPDATE_CAPACITY);
    let sub_sender = price_updates.clone();
    let sub = subscribe(&instrument_data, sub_sender);
    let signal_store = SignalStore::from_environment()?;
    let signal_feeds = Arc::new(StdMutex::new(HashMap::<String, broadcast::Sender<Bytes>>::new()));
    for instrument in instrument_data.iter().take(10) {
        start_signal_feed(instrument, &signal_feeds, &signal_store, &price_updates);
    }
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
                signal_feeds: Arc::clone(&signal_feeds),
                signal_store: signal_store.clone(),
            }))
        }))
        .serve();

    tokio::try_join!(sub, r_socket)?;
    Ok(())
}

struct ServerResponder {
    price_updates: tokio::sync::broadcast::Sender<Bytes>,
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    signal_feeds: Arc<StdMutex<HashMap<String, broadcast::Sender<Bytes>>>>,
    signal_store: SignalStore,
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
        if let Some(instrument) = requested_signal_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async { Err(anyhow::anyhow!("invalid signal instrument request")) }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let missing = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move { Err(anyhow::anyhow!("instrument is unavailable: {missing}")) }));
            };
            let instrument = instrument.to_string();
            let sender = start_signal_feed(&instrument, &self.signal_feeds, &self.signal_store, &self.price_updates);
            let stream = futures_util::stream::unfold(sender.subscribe(), |mut receiver| async move {
                loop {
                    match receiver.recv().await {
                        Ok(data) => break Some((Ok(Payload::new(Some(data), None)), receiver)),
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => break None,
                    }
                }
            }).boxed();
            return log_stream_end(stream, format!("RSocket client disconnected from signal stream for {instrument}"));
        }

        if let Some(instrument) = requested_trade_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid OKX trades instrument request"))
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
            println!("Subscribing to OKX trades for {instrument}");
            return log_stream_end(
                trade_request_stream(instrument.to_string()),
                format!(
                    "RSocket UI disconnected or cancelled the OKX trades stream for {instrument}"
                ),
            );
        }

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
                format!(
                    "RSocket UI disconnected or cancelled the OKX book stream for {instrument}"
                ),
            );
        }

        let synthetic = is_synthetic_request(&payload);
        println!(
            "RSocket request stream source: {}",
            if synthetic { "synthetic" } else { "OKX" }
        );
        let stream = if synthetic {
            futures_util::stream::unfold(
                (
                    0_u64,
                    0_usize,
                    (0..100)
                        .map(|_| RollingVolatility::default())
                        .collect::<Vec<_>>(),
                ),
                |(round, symbol_index, mut volatility_by_symbol)| async move {
                    if symbol_index == 0 {
                        sleep(Duration::from_millis(10)).await;
                    }

                    let (symbol, price) = synthetic_price_update(round, symbol_index);
                    let volatility = volatility_by_symbol[symbol_index]
                        .update(round.saturating_mul(1_000), price);
                    let result = Ok(Payload::builder()
                        .set_data(encode_symbol_price_bytes(
                            &symbol,
                            price,
                            volatility.0,
                            volatility.1,
                        ))
                        .build());
                    let next_state = if symbol_index == 99 {
                        (round.wrapping_add(1), 0, volatility_by_symbol)
                    } else {
                        (round, symbol_index + 1, volatility_by_symbol)
                    };
                    Some((result, next_state))
                },
            )
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

        log_stream_end(
            stream,
            "RSocket UI disconnected or cancelled the market price stream".to_owned(),
        )
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
        |(mut stream, end_log)| async move { stream.next().await.map(|item| (item, (stream, end_log))) },
    ))
}

fn requested_signal_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    std::str::from_utf8(data.as_ref()).ok()?.strip_prefix("signals:")
}

fn start_signal_feed(
    instrument: &str,
    feeds: &Arc<StdMutex<HashMap<String, broadcast::Sender<Bytes>>>>,
    store: &SignalStore,
    price_updates: &broadcast::Sender<Bytes>,
) -> broadcast::Sender<Bytes> {
    let mut feeds = feeds.lock().expect("signal feed map poisoned");
    feeds.entry(instrument.to_owned()).or_insert_with(|| {
        let (sender, _) = broadcast::channel(64);
        tokio::spawn(run_signal_feed(instrument.to_owned(), sender.clone(), store.clone(), price_updates.clone()));
        sender
    }).clone()
}

async fn run_signal_feed(
    instrument: String,
    output: broadcast::Sender<Bytes>,
    store: SignalStore,
    price_updates: broadcast::Sender<Bytes>,
) {
    let mut prices = price_updates.subscribe();
    let mut trades = trade_request_stream(instrument.clone());
    let mut books = book_request_stream(instrument.clone());
    let mut engine = SignalEngine::default();
    let mut timer = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            incoming = prices.recv() => match incoming {
                Ok(bytes) if get_symbol(&bytes) == instrument => {
                    if let (Some(price), Some(volatility)) = (read_f64(&bytes, 40), read_f64(&bytes, 48)) {
                        for outcome in engine.on_price(now_ms(), price, volatility) {
                            if let Err(error) = store.append_outcome(&outcome).await { eprintln!("Unable to persist signal outcome: {error:#}"); }
                        }
                    }
                }
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {},
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = trades.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if get_symbol(bytes) == instrument {
                        if let (Some(timestamp), Some(price), Some(size)) = (read_u64(bytes, 72), read_f64(bytes, 80), read_f64(bytes, 88)) {
                            engine.on_trade(timestamp, price, size, bytes.get(96) == Some(&1));
                        }
                    }
                }
            },
            incoming = books.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if get_symbol(bytes) == instrument {
                        let timestamp = read_u64(bytes, 40).unwrap_or_else(now_ms);
                        let best_bid = read_f64(bytes, 48).unwrap_or_default();
                        let best_ask = read_f64(bytes, 160).unwrap_or_default();
                        let bid_sizes = (0..7).filter_map(|index| read_f64(bytes, 56 + index * 16)).collect::<Vec<_>>();
                        let ask_sizes = (0..7).filter_map(|index| read_f64(bytes, 168 + index * 16)).collect::<Vec<_>>();
                        engine.on_book(timestamp, best_bid, best_ask, &bid_sizes, &ask_sizes);
                    }
                }
            },
            _ = timer.tick() => {
                if let Some(snapshot) = engine.snapshot(&instrument, now_ms()) {
                    engine.track(&snapshot);
                    match serde_json::to_vec(&snapshot) {
                        Ok(json) => {
                            let _ = output.send(Bytes::from(json));
                            if let Err(error) = store.append(&snapshot).await { eprintln!("Unable to persist signal snapshot: {error:#}"); }
                        }
                        Err(error) => eprintln!("Unable to encode signal snapshot: {error}"),
                    }
                }
            }
        }
    }
}

fn get_symbol(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.get(8..40).unwrap_or_default()).trim_end_matches([' ', '\0']).to_owned()
}
fn read_f64(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    value.is_finite().then_some(value)
}
fn read_u64(bytes: &[u8], offset: usize) -> Option<u64> { Some(u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?)) }
fn now_ms() -> u64 { std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64 }

fn requested_trade_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    let request = std::str::from_utf8(data.as_ref()).ok()?;
    request.strip_prefix("trades:")
}

fn trade_request_stream(
    instrument: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
    let (sender, receiver) = mpsc::channel(TRADE_UPDATE_CAPACITY);
    tokio::spawn(async move {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            if sender.is_closed() {
                break;
            }
            match subscribe_trades_once(&instrument, sender.clone()).await {
                Ok(()) => break,
                Err(error) => {
                    eprintln!(
                        "OKX trades stream for {instrument} failed: {error:#}; retrying in {}s",
                        retry_delay.as_secs()
                    );
                    tokio::select! { _ = sender.closed() => break, _ = sleep(retry_delay) => {} }
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
                println!(
                    "UI unsubscribed from OKX book stream for {instrument} before connection setup"
                );
                break;
            }

            match subscribe_book_once(&instrument, sender.clone()).await {
                Ok(()) => break,
                Err(error) => {
                    eprintln!(
                        "OKX book stream for {instrument} failed: {error:#}; retrying in {}s",
                        retry_delay.as_secs()
                    );
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
        assert_eq!(
            encode_symbol_price_bytes(&symbol, first_price, f64::NAN, f64::NAN).len(),
            56
        );
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
