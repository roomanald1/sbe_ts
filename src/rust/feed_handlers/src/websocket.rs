use anyhow::Result;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use futures_util::{Stream, StreamExt};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{
    collections::{HashMap, HashSet},
    env,
    pin::Pin,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};
use tokio::sync::{broadcast, mpsc, watch};
use tokio::time::{Duration, sleep};

use crate::encoders::{SYMBOL_LENGTH, encode_symbol_price_bytes};
use crate::okx::connectivity::{
    RollingVolatility, get_all_spot_instruments, get_instrument_data, subscribe,
    subscribe_book_once, subscribe_trades_once, run_usd_rate_stream,
};

const PRICE_UPDATE_CAPACITY: usize = 4096;
const TICKER_BATCH_INTERVAL: Duration = Duration::from_millis(10);
const SYMBOL_PRICE_MESSAGE_LENGTH: usize = 64;
const MAX_TICKER_BATCH_MESSAGES: usize = 256;
const TICKER_BATCH_CAPACITY: usize = SYMBOL_PRICE_MESSAGE_LENGTH * MAX_TICKER_BATCH_MESSAGES;
const BOOK_UPDATE_CAPACITY: usize = 32;
const TRADE_UPDATE_CAPACITY: usize = 32;
// Generate signals across the full top-volume spot universe so Auto Trader can
// find fresh candidates beyond the first 30 instruments.
const SIGNAL_INSTRUMENT_LIMIT: usize = 100;
static TRADE_FEEDS: OnceLock<StdMutex<HashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static BOOK_FEEDS: OnceLock<StdMutex<HashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static SIGNAL_FEEDS: OnceLock<Arc<DashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static SIGNAL_PRICE_FEEDS: OnceLock<Arc<DashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static NEWS_SNAPSHOT: OnceLock<watch::Sender<Bytes>> = OnceLock::new();

pub async fn serve() -> Result<()> {
    let instrument_data = get_instrument_data()
        .await?
        .into_iter()
        .map(Arc::<str>::from)
        .collect::<Vec<_>>();
    let all_spot_instruments = match get_all_spot_instruments().await {
        Ok(symbols) => symbols,
        Err(error) => {
            crate::service_log!("ERROR", 
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
    let (ticker_batches, _) = broadcast::channel::<Bytes>(256);
    let initial_usd_rates = Bytes::from_static(br#"{"rates":{"USD":1.0},"updated_at":0,"updated_at_by_currency":{"USD":0}}"#);
    let (usd_rate_snapshot, _) = watch::channel(initial_usd_rates);
    tokio::spawn(run_usd_rate_stream(usd_rate_snapshot.clone()));
    let (news_snapshot, _) = watch::channel(Bytes::from_static(br#"{"updated_at_ms":0,"source":"CoinDesk RSS","stale":true,"error":null,"articles":[]}"#));
    let _ = NEWS_SNAPSHOT.set(news_snapshot.clone());
    let signal_feeds = SIGNAL_FEEDS.get_or_init(|| Arc::new(DashMap::new())).clone();
    let signal_price_feeds = SIGNAL_PRICE_FEEDS.get_or_init(|| Arc::new(DashMap::new())).clone();
    let (signal_events, _) = broadcast::channel::<Bytes>(65_536);
    let mut ticker_router = price_updates.subscribe();
    let routed_prices = Arc::clone(&signal_price_feeds);
    tokio::spawn(async move {
        loop {
            match ticker_router.recv().await {
                Ok(bytes) => {
                    if let Some(symbol) = symbol_from_sbe(&bytes) {
                        let output = routed_prices.get(symbol).map(|entry| entry.value().clone());
                        if let Some(output) = output { let _ = output.send(bytes); }
                    }
                }
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    crate::service_log!("WARN", "Signal price router lagged; skipped {skipped} ticker updates");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });
    let mut ticker_batch_input = price_updates.subscribe();
    let ticker_batch_output = ticker_batches.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(TICKER_BATCH_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut batch = Vec::with_capacity(TICKER_BATCH_CAPACITY);
        loop {
            tokio::select! {
                received = ticker_batch_input.recv() => match received {
                    Ok(message) => {
                        batch.extend_from_slice(&message);
                        if batch.len() >= TICKER_BATCH_CAPACITY {
                            let _ = ticker_batch_output.send(Bytes::from(std::mem::replace(
                                &mut batch,
                                Vec::with_capacity(TICKER_BATCH_CAPACITY),
                            )));
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                },
                _ = interval.tick() => {
                    if !batch.is_empty() {
                        let _ = ticker_batch_output.send(Bytes::from(std::mem::replace(
                            &mut batch,
                            Vec::with_capacity(TICKER_BATCH_CAPACITY),
                        )));
                    }
                }
            }
        }
    });
    for instrument in instrument_data.iter().take(SIGNAL_INSTRUMENT_LIMIT) {
        start_signal_feed(instrument, &signal_feeds, &signal_price_feeds, &signal_events);
    }
    let port = env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{port}");

    crate::service_log!("INFO", "RSocket WebSocket server on ws://{addr}");

    let r_socket = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(addr))
        .acceptor(Box::new(move |setup, _socket| {
            crate::service_log!("INFO", "socket establish: setup={:?}", setup);
            Ok(Box::new(ServerResponder {
                price_updates: ticker_batches.clone(),
                instrument_lookup: Arc::clone(&instrument_lookup),
                signal_feeds: Arc::clone(&signal_feeds),
                signal_price_feeds: Arc::clone(&signal_price_feeds),
                signal_events: signal_events.clone(),
                usd_rate_snapshot: usd_rate_snapshot.clone(),
                news_snapshot: news_snapshot.clone(),
            }))
        }))
        .serve();

    tokio::try_join!(sub, r_socket)?;
    Ok(())
}

struct ServerResponder {
    price_updates: tokio::sync::broadcast::Sender<Bytes>,
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    signal_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_price_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_events: broadcast::Sender<Bytes>,
    usd_rate_snapshot: watch::Sender<Bytes>,
    news_snapshot: watch::Sender<Bytes>,
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
        if payload
            .data()
            .and_then(|data| std::str::from_utf8(data.as_ref()).ok())
            == Some("tickers:*")
        {
            let stream = futures_util::stream::unfold(
                self.price_updates.subscribe(),
                |mut updates| async move {
                    loop {
                        match updates.recv().await {
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), updates)),
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                crate::service_log!("WARN", "RSocket ticker subscriber lagged; skipped {skipped} updates");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                },
            )
            .boxed();
            return log_stream_end(
                stream,
                "FeedHandlers ticker API client disconnected".to_owned(),
            );
        }

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("news:*") {
            let stream = futures_util::stream::unfold(
                (self.news_snapshot.subscribe(), true),
                |(mut snapshot, initial)| async move {
                    if !initial && snapshot.changed().await.is_err() { return None; }
                    let data = snapshot.borrow_and_update().clone();
                    Some((Ok(Payload::new(Some(data), None)), (snapshot, false)))
                },
            ).boxed();
            return log_stream_end(stream, "Backend news snapshot stream disconnected".to_owned());
        }

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("usd-rates:*") {
            let stream = futures_util::stream::unfold(
                (self.usd_rate_snapshot.subscribe(), true),
                |(mut snapshot, initial)| async move {
                    if !initial && snapshot.changed().await.is_err() { return None; }
                    let data = snapshot.borrow_and_update().clone();
                    Some((Ok(Payload::new(Some(data), None)), (snapshot, false)))
                },
            ).boxed();
            return log_stream_end(stream, "FeedHandlers USD rate stream disconnected".to_owned());
        }

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("signal-events:*") {
            let stream = futures_util::stream::unfold(
                self.signal_events.subscribe(),
                |mut events| async move {
                    loop {
                        match events.recv().await {
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), events)),
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                crate::service_log!("WARN", "Insights signal-event subscriber lagged; skipped {skipped} events");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                },
            ).boxed();
            return log_stream_end(stream, "Insights signal-event stream disconnected".to_owned());
        }

        if let Some(instrument) = requested_signal_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid signal instrument request"))
                }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let missing = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move {
                    Err(anyhow::anyhow!("instrument is unavailable: {missing}"))
                }));
            };
            let instrument = instrument.to_string();
            let sender = start_signal_feed(&instrument, &self.signal_feeds, &self.signal_price_feeds, &self.signal_events);
            let stream =
                futures_util::stream::unfold(sender.subscribe(), |mut receiver| async move {
                    loop {
                        match receiver.recv().await {
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), receiver)),
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                })
                .boxed();
            return log_stream_end(
                stream,
                format!("RSocket client disconnected from signal stream for {instrument}"),
            );
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
            crate::service_log!("INFO", "Subscribing to OKX trades for {instrument}");
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
            crate::service_log!("INFO", "Subscribing to OKX books for {instrument}");
            return log_stream_end(
                book_request_stream(instrument.to_string()),
                format!(
                    "RSocket UI disconnected or cancelled the OKX book stream for {instrument}"
                ),
            );
        }

        let synthetic = is_synthetic_request(&payload);
        crate::service_log!("INFO", 
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
                            crate::service_log!("ERROR", 
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
        crate::service_log!("INFO", "{}", self.0);
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
    std::str::from_utf8(data.as_ref())
        .ok()?
        .strip_prefix("signals:")
}

pub fn publish_news_snapshot(snapshot: Bytes) -> bool {
    let Some(sender) = NEWS_SNAPSHOT.get() else { return false; };
    sender.send_replace(snapshot);
    true
}

pub fn publish_signal_snapshot(symbol: &str, snapshot: Bytes) -> bool {
    let Some(feeds) = SIGNAL_FEEDS.get() else { return false; };
    let sender = feeds.get(symbol).map(|entry| entry.value().clone());
    if let Some(sender) = sender {
        let _ = sender.send(snapshot);
        true
    } else {
        false
    }
}

fn start_signal_feed(
    instrument: &str,
    feeds: &Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    price_feeds: &Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    events: &broadcast::Sender<Bytes>,
) -> broadcast::Sender<Bytes> {
    let price_sender = price_feeds
        .entry(instrument.to_owned())
        .or_insert_with(|| broadcast::channel(64).0)
        .value()
        .clone();
    let price_receiver = price_sender.subscribe();

    // Reserve the symbol entry first, then start its task after releasing the
    // DashMap shard guard. The task may itself access shared feed maps.
    let (sender, should_start) = match feeds.entry(instrument.to_owned()) {
        dashmap::mapref::entry::Entry::Occupied(entry) => (entry.get().clone(), false),
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            let (sender, _) = broadcast::channel(64);
            entry.insert(sender.clone());
            (sender, true)
        }
    };
    if should_start {
        tokio::spawn(run_signal_feed(instrument.to_owned(), price_receiver, events.clone()));
    }
    sender
}


async fn run_signal_feed(
    instrument: String,
    mut prices: broadcast::Receiver<Bytes>,
    events: broadcast::Sender<Bytes>,
) {
    let mut trades = trade_request_stream(instrument.clone());
    let mut books = book_request_stream(instrument.clone());
    loop {
        tokio::select! {
            incoming = prices.recv() => match incoming {
                Ok(bytes) if has_symbol(&bytes, &instrument) => {
                    if bytes.len() >= 64 && read_f64(&bytes, 40).is_some() && read_f64(&bytes, 48).is_some() {
                        let mut message = BytesMut::with_capacity(bytes.len() + 8);
                        message.extend_from_slice(&bytes);
                        message.extend_from_slice(&now_ms().to_le_bytes());
                        let _ = events.send(message.freeze());
                    }
                }
                Ok(_) => {},
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    crate::service_log!("WARN", "Signal price feed for {instrument} lagged; skipped {skipped} ticker updates");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = trades.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if has_symbol(bytes, &instrument) {
                        if bytes.len() >= 97 {
                            let _ = events.send(Bytes::clone(payload.data().expect("checked payload data")));
                        }
                    }
                }
            },
            incoming = books.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if has_symbol(bytes, &instrument) && bytes.len() >= 272 {
                        let _ = events.send(Bytes::clone(payload.data().expect("checked payload data")));
                    }
                }
            },
        }
    }
}

fn symbol_from_sbe(bytes: &[u8]) -> Option<&str> {
    let field = bytes.get(8..40)?;
    let end = field.iter().position(|byte| *byte == b' ' || *byte == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).ok()
}

fn has_symbol(bytes: &[u8], symbol: &str) -> bool {
    let Some(field) = bytes.get(8..40) else {
        return false;
    };
    let end = field
        .iter()
        .position(|byte| *byte == b' ' || *byte == 0)
        .unwrap_or(field.len());
    &field[..end] == symbol.as_bytes()
}

fn read_f64(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    value.is_finite().then_some(value)
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn requested_trade_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    let request = std::str::from_utf8(data.as_ref()).ok()?;
    request.strip_prefix("trades:")
}

fn trade_request_stream(
    instrument: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
    let feeds = TRADE_FEEDS.get_or_init(|| StdMutex::new(HashMap::new()));
    let sender = {
        let mut feeds = feeds.lock().expect("trades feed map poisoned");
        feeds
            .entry(instrument.clone())
            .or_insert_with(|| {
                let (output, _) = broadcast::channel(TRADE_UPDATE_CAPACITY);
                let task_output = output.clone();
                tokio::spawn(async move {
                    let mut delay = Duration::from_secs(1);
                    loop {
                        let (tx, mut rx) = mpsc::channel(TRADE_UPDATE_CAPACITY);
                        let sender = task_output.clone();
                        let forward = tokio::spawn(async move {
                            while let Some(Ok(bytes)) = rx.recv().await {
                                let _ = sender.send(bytes);
                            }
                        });
                        let attempt_started = std::time::Instant::now();
                        match subscribe_trades_once(&instrument, tx).await {
                            Ok(()) => break,
                            Err(error) => {
                                if attempt_started.elapsed() >= Duration::from_secs(60) {
                                    delay = Duration::from_secs(1);
                                }
                                crate::service_log!("ERROR", 
                                    "Shared trades subscription for {instrument} failed: {error:#}; retrying in {}s",
                                    delay.as_secs()
                                );
                                forward.abort();
                                sleep(delay).await;
                                delay = (delay * 2).min(Duration::from_secs(30));
                            }
                        }
                    }
                });
                output
            })
            .clone()
    };
    Box::pin(futures_util::stream::unfold(
        sender.subscribe(),
        |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(bytes) => break Some((Ok(Payload::new(Some(bytes), None)), rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break None,
                }
            }
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
    let feeds = BOOK_FEEDS.get_or_init(|| StdMutex::new(HashMap::new()));
    let sender = {
        let mut feeds = feeds.lock().expect("book feed map poisoned");
        feeds
            .entry(instrument.clone())
            .or_insert_with(|| {
                let (output, _) = broadcast::channel(BOOK_UPDATE_CAPACITY);
                let task_output = output.clone();
                tokio::spawn(async move {
                    let mut delay = Duration::from_secs(1);
                    loop {
                        let (tx, mut rx) = mpsc::channel(BOOK_UPDATE_CAPACITY);
                        let sender = task_output.clone();
                        let forward = tokio::spawn(async move {
                            while let Some(Ok(bytes)) = rx.recv().await {
                                let _ = sender.send(bytes);
                            }
                        });
                        let attempt_started = std::time::Instant::now();
                        match subscribe_book_once(&instrument, tx).await {
                            Ok(()) => break,
                            Err(error) => {
                                if attempt_started.elapsed() >= Duration::from_secs(60) {
                                    delay = Duration::from_secs(1);
                                }
                                crate::service_log!("ERROR", 
                                    "Shared book subscription for {instrument} failed: {error:#}; retrying in {}s",
                                    delay.as_secs()
                                );
                                forward.abort();
                                sleep(delay).await;
                                delay = (delay * 2).min(Duration::from_secs(30));
                            }
                        }
                    }
                });
                output
            })
            .clone()
    };
    Box::pin(futures_util::stream::unfold(
        sender.subscribe(),
        |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(bytes) => break Some((Ok(Payload::new(Some(bytes), None)), rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break None,
                }
            }
        },
    ))
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
