use anyhow::{Context, Result};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use bytes::Bytes;
use futures_util::StreamExt;
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const BOOK_DEPTH: usize = 7;
const BOOK_MESSAGE_LENGTH: usize = 272;
const TICKER_MESSAGE_LENGTH: usize = 64;
const MAX_MARKET_BOOK_SUBSCRIPTIONS: usize = 256;
const MAX_TICKER_HISTORY_SYMBOLS: usize = 128;
const FEED_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const BOOK_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const TICKER_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TICKER_HISTORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const TICKER_HISTORY_RETENTION: Duration = Duration::from_secs(30 * 60);
const MARKET_MARK_FRESHNESS: Duration = Duration::from_secs(5);

#[derive(Clone, Serialize, Deserialize)]
pub struct BookLevel {
    pub price: f64,
    pub size: f64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BookSnapshot {
    pub bids: Vec<BookLevel>,
    pub asks: Vec<BookLevel>,
    pub updated_at_ms: u64,
}

#[derive(Default)]
struct Histories {
    by_symbol: HashMap<String, Vec<(u64, f64)>>,
}

pub struct MarketData {
    books: RwLock<HashMap<String, Arc<BookSnapshot>>>,
    histories: Mutex<Histories>,
    subscriptions: Mutex<HashMap<String, ()>>,
    feed_client: Mutex<Option<Arc<Client>>>,
}

impl MarketData {
    pub fn new() -> Arc<Self> {
        let data = Arc::new(Self {
            books: RwLock::new(HashMap::new()),
            histories: Mutex::new(Histories::default()),
            subscriptions: Mutex::new(HashMap::new()),
            feed_client: Mutex::new(None),
        });
        Self::start_ticker_history(Arc::clone(&data));
        data
    }

    async fn client(&self) -> Result<Arc<Client>> {
        let mut slot = self.feed_client.lock().await;
        if let Some(client) = slot.as_ref() {
            return Ok(Arc::clone(client));
        }
        let url = std::env::var("FEED_HANDLERS_RSOCKET_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:10001".to_owned());
        let client = tokio::time::timeout(
            FEED_CONNECT_TIMEOUT,
            RSocketFactory::connect()
                .transport(WebsocketClientTransport::from(url.as_str()))
                .acceptor(Box::new(|| Box::new(EchoRSocket)))
                .start(),
        )
        .await
        .context("FeedHandlers RSocket connection timed out")?
        .context("FeedHandlers RSocket connection failed")?;
        let client = Arc::new(client);
        *slot = Some(Arc::clone(&client));
        Ok(client)
    }

    async fn invalidate_client(&self, failed: &Arc<Client>) {
        let mut slot = self.feed_client.lock().await;
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, failed))
        {
            *slot = None;
        }
    }

    pub async fn ensure_book_subscription(self: &Arc<Self>, symbol: &str) -> Result<()> {
        let symbol = symbol.to_ascii_uppercase();
        let mut subscriptions = self.subscriptions.lock().await;
        if subscriptions.contains_key(&symbol) {
            return Ok(());
        }
        anyhow::ensure!(
            subscriptions.len() < MAX_MARKET_BOOK_SUBSCRIPTIONS,
            "Paper Exchange market-book subscription limit reached"
        );
        subscriptions.insert(symbol.clone(), ());
        drop(subscriptions);
        let data = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                match data.client().await {
                    Ok(client) => {
                        let source = client.request_stream(
                            Payload::builder()
                                .set_data_utf8(&format!("book:{symbol}"))
                                .build(),
                        );
                        let stream_client = Arc::clone(&client);
                        let mut stream = source.map(move |item| {
                            let _client = &stream_client;
                            item.map_err(anyhow::Error::from).and_then(|payload| {
                                payload.data().cloned().ok_or_else(|| {
                                    anyhow::anyhow!("FeedHandlers returned an empty book payload")
                                })
                            })
                        });
                        loop {
                            match tokio::time::timeout(BOOK_STREAM_IDLE_TIMEOUT, stream.next())
                                .await
                            {
                                Ok(Some(Ok(bytes))) => {
                                    if let Some(book) = decode_book(&bytes) {
                                        data.books
                                            .write()
                                            .unwrap_or_else(|p| p.into_inner())
                                            .insert(symbol.clone(), Arc::new(book));
                                    }
                                }
                                Ok(Some(Err(error))) => {
                                    eprintln!(
                                        "Paper Exchange book stream failed for {symbol}: {error:#}"
                                    );
                                    break;
                                }
                                Ok(None) => break,
                                Err(_) => {
                                    eprintln!(
                                        "Paper Exchange book stream idle for {symbol}; reconnecting"
                                    );
                                    break;
                                }
                            }
                        }
                        data.invalidate_client(&client).await;
                    }
                    Err(error) => eprintln!("Paper Exchange book connection failed: {error:#}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
        Ok(())
    }

    fn start_ticker_history(data: Arc<Self>) {
        tokio::spawn(async move {
            loop {
                match data.client().await {
                    Ok(client) => {
                        let source = client
                            .request_stream(Payload::builder().set_data_utf8("tickers:*").build());
                        let stream_client = Arc::clone(&client);
                        let mut stream = source.map(move |item| {
                            let _client = &stream_client;
                            item.map_err(anyhow::Error::from).and_then(|payload| {
                                payload.data().cloned().ok_or_else(|| {
                                    anyhow::anyhow!("FeedHandlers returned an empty ticker payload")
                                })
                            })
                        });
                        loop {
                            match tokio::time::timeout(TICKER_STREAM_IDLE_TIMEOUT, stream.next())
                                .await
                            {
                                Ok(Some(Ok(bytes))) => data.record_ticker_batch(&bytes).await,
                                Ok(Some(Err(error))) => {
                                    eprintln!("Paper Exchange ticker stream failed: {error:#}");
                                    break;
                                }
                                Ok(None) => break,
                                Err(_) => {
                                    eprintln!("Paper Exchange ticker stream idle; reconnecting");
                                    break;
                                }
                            }
                        }
                        data.invalidate_client(&client).await;
                    }
                    Err(error) => eprintln!("Paper Exchange ticker connection failed: {error:#}"),
                }
                tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            }
        });
    }

    async fn record_ticker_batch(&self, bytes: &Bytes) {
        let Ok(timestamp) = now_ms() else {
            return;
        };
        let mut histories = self.histories.lock().await;
        for message in bytes.chunks_exact(TICKER_MESSAGE_LENGTH) {
            let symbol = String::from_utf8_lossy(&message[8..40])
                .trim_end_matches([' ', '\0'])
                .to_owned();
            let price = f64::from_le_bytes(message[40..48].try_into().unwrap_or_default());
            if !valid_symbol(&symbol) || !price.is_finite() || price <= 0.0 {
                continue;
            }
            if !histories.by_symbol.contains_key(&symbol)
                && histories.by_symbol.len() >= MAX_TICKER_HISTORY_SYMBOLS
            {
                continue;
            }
            let samples = histories.by_symbol.entry(symbol).or_default();
            if samples
                .last()
                .is_none_or(|(last, _)| {
                    timestamp.saturating_sub(*last)
                        >= TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64
                })
            {
                samples.push((timestamp, price));
                samples.retain(|(time, _)| {
                    timestamp.saturating_sub(*time) <= TICKER_HISTORY_RETENTION.as_millis() as u64
                });
            }
        }
    }

    pub fn book(&self, symbol: &str) -> Option<Arc<BookSnapshot>> {
        self.books
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .get(symbol)
            .cloned()
    }

    pub async fn history(&self, symbol: &str) -> Vec<(u64, f64)> {
        self.histories
            .lock()
            .await
            .by_symbol
            .get(symbol)
            .cloned()
            .unwrap_or_default()
    }

    async fn histories(&self) -> HashMap<String, Vec<(u64, f64)>> {
        self.histories.lock().await.by_symbol.clone()
    }
}

#[derive(Deserialize)]
struct SymbolQuery {
    symbol: String,
}

#[derive(Serialize)]
struct MarketMark {
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    worst_ask: Option<f64>,
    updated_at_ms: u64,
}

async fn get_market_mark(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<MarketMark>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    data.ensure_book_subscription(&symbol)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let now = now_ms().map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let book = data
        .book(&symbol)
        .filter(|book| {
            now.saturating_sub(book.updated_at_ms)
                <= MARKET_MARK_FRESHNESS.as_millis() as u64
        })
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("No fresh market mark for {symbol}"),
        ))?;
    let bid = book.bids.first().map(|level| level.price);
    let ask = book.asks.first().map(|level| level.price);
    Ok(Json(MarketMark {
        bid,
        ask,
        mid: bid.zip(ask).map(|(bid, ask)| (bid + ask) / 2.0),
        worst_ask: book.asks.last().map(|level| level.price),
        updated_at_ms: book.updated_at_ms,
    }))
}

async fn get_market_book(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<BookSnapshot>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    data.ensure_book_subscription(&symbol)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let now = now_ms().map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    data.book(&symbol)
        .filter(|book| {
            now.saturating_sub(book.updated_at_ms)
                <= MARKET_MARK_FRESHNESS.as_millis() as u64
        })
        .map(|book| Json((*book).clone()))
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("No fresh book for {symbol}"),
        ))
}

#[derive(Serialize)]
struct MarketHistory {
    samples: Vec<(u64, f64)>,
}

async fn get_market_history(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<MarketHistory>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    Ok(Json(MarketHistory {
        samples: data.history(&symbol).await,
    }))
}

#[derive(Serialize)]
struct MarketInputs {
    markets: HashMap<String, MarketInput>,
}

#[derive(Serialize)]
struct MarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}

async fn get_insights_inputs(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<SymbolQuery>,
) -> Result<Json<MarketInputs>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    data.ensure_book_subscription(&symbol)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let book = data.book(&symbol);
    Ok(Json(MarketInputs {
        markets: [(
            symbol.clone(),
            MarketInput {
                bids: book
                    .as_ref()
                    .map(|snapshot| snapshot.bids.clone())
                    .unwrap_or_default(),
                asks: book
                    .as_ref()
                    .map(|snapshot| snapshot.asks.clone())
                    .unwrap_or_default(),
                price_samples: data.history(&symbol).await,
            },
        )]
        .into_iter()
        .collect(),
    }))
}

#[derive(Serialize)]
struct MarketHistories {
    histories: HashMap<String, Vec<(u64, f64)>>,
    market_marks: HashMap<String, MarketMarkSnapshot>,
    timing_contract: MarketHistoryTimingContract,
}

#[derive(Serialize)]
struct MarketMarkSnapshot {
    mid: Option<f64>,
    updated_at_ms: u64,
}

#[derive(Serialize)]
struct MarketHistoryTimingContract {
    version: u16,
    sample_interval_ms: u64,
    retention_ms: u64,
    ticker_stream_idle_timeout_ms: u64,
    market_mark_freshness_ms: u64,
}

impl MarketHistoryTimingContract {
    fn current() -> Self {
        Self {
            version: 1,
            sample_interval_ms: TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64,
            retention_ms: TICKER_HISTORY_RETENTION.as_millis() as u64,
            ticker_stream_idle_timeout_ms: TICKER_STREAM_IDLE_TIMEOUT.as_millis() as u64,
            market_mark_freshness_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
        }
    }
}

async fn get_market_histories(State(data): State<Arc<MarketData>>) -> Json<MarketHistories> {
    let histories = data.histories().await;
    let market_marks = data
        .books
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|(symbol, book)| {
            let bid = book.bids.first().map(|level| level.price);
            let ask = book.asks.first().map(|level| level.price);
            (
                symbol.clone(),
                MarketMarkSnapshot {
                    mid: bid.zip(ask).map(|(bid, ask)| (bid + ask) / 2.0),
                    updated_at_ms: book.updated_at_ms,
                },
            )
        })
        .collect();
    Json(MarketHistories {
        histories,
        market_marks,
        timing_contract: MarketHistoryTimingContract::current(),
    })
}

pub fn router(data: Arc<MarketData>) -> Router {
    Router::new()
        .route("/internal/market-book", get(get_market_book))
        .route("/internal/market-mark", get(get_market_mark))
        .route("/internal/market-history", get(get_market_history))
        .route("/internal/insights-inputs", get(get_insights_inputs))
        .route("/internal/market-histories", get(get_market_histories))
        .with_state(data)
}

fn decode_book(bytes: &Bytes) -> Option<BookSnapshot> {
    if bytes.len() < BOOK_MESSAGE_LENGTH {
        return None;
    }
    let symbol = std::str::from_utf8(&bytes[8..40]).ok()?.trim();
    if !valid_symbol(symbol) {
        return None;
    }
    let mut bids = Vec::with_capacity(BOOK_DEPTH);
    let mut asks = Vec::with_capacity(BOOK_DEPTH);
    for index in 0..BOOK_DEPTH {
        let bid = read_level(bytes, 48 + index * 16)?;
        let ask = read_level(bytes, 160 + index * 16)?;
        if bid.size > 0.0 {
            bids.push(bid);
        }
        if ask.size > 0.0 {
            asks.push(ask);
        }
    }
    Some(BookSnapshot {
        bids,
        asks,
        updated_at_ms: now_ms().ok()?,
    })
}

fn read_level(bytes: &[u8], offset: usize) -> Option<BookLevel> {
    let price = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    let size = f64::from_le_bytes(bytes.get(offset + 8..offset + 16)?.try_into().ok()?);
    (price.is_finite()
        && price >= 0.0
        && size.is_finite()
        && size >= 0.0
        && (price > 0.0 || size == 0.0))
        .then_some(BookLevel { price, size })
}

fn valid_symbol(symbol: &str) -> bool {
    symbol.len() <= 32
        && symbol.is_ascii()
        && symbol.split_once('-').is_some_and(|(base, quote)| {
            !base.is_empty()
                && !quote.is_empty()
                && base.bytes().all(|b| b.is_ascii_alphanumeric())
                && quote.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn now_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_data() -> MarketData {
        MarketData {
            books: RwLock::new(HashMap::new()),
            histories: Mutex::new(Histories::default()),
            subscriptions: Mutex::new(HashMap::new()),
            feed_client: Mutex::new(None),
        }
    }

    #[test]
    fn decodes_a_book_snapshot_from_the_sbe_wire_layout() {
        let mut bytes = vec![0; BOOK_MESSAGE_LENGTH];
        bytes[8..40].fill(b' ');
        bytes[8..16].copy_from_slice(b"BTC-USDT");
        bytes[48..56].copy_from_slice(&100.0_f64.to_le_bytes());
        bytes[56..64].copy_from_slice(&2.0_f64.to_le_bytes());
        bytes[160..168].copy_from_slice(&101.0_f64.to_le_bytes());
        bytes[168..176].copy_from_slice(&3.0_f64.to_le_bytes());
        let snapshot = decode_book(&Bytes::from(bytes)).unwrap();
        assert_eq!(snapshot.bids[0].price, 100.0);
        assert_eq!(snapshot.asks[0].price, 101.0);
        assert!(snapshot.updated_at_ms > 0);
    }

    #[tokio::test]
    async fn ticker_history_is_bounded_by_symbol_count() {
        let data = empty_data();
        let mut batch = vec![0; (MAX_TICKER_HISTORY_SYMBOLS + 32) * TICKER_MESSAGE_LENGTH];
        for index in 0..(MAX_TICKER_HISTORY_SYMBOLS + 32) {
            let start = index * TICKER_MESSAGE_LENGTH;
            let symbol = format!("T{index}-USDT");
            batch[start + 8..start + 40].fill(b' ');
            batch[start + 8..start + 8 + symbol.len()].copy_from_slice(symbol.as_bytes());
            batch[start + 40..start + 48].copy_from_slice(&1.0_f64.to_le_bytes());
        }
        data.record_ticker_batch(&Bytes::from(batch)).await;
        assert_eq!(
            data.histories.lock().await.by_symbol.len(),
            MAX_TICKER_HISTORY_SYMBOLS
        );
    }

    #[test]
    fn market_histories_serializes_versioned_timing_contract() {
        let response = MarketHistories {
            histories: HashMap::from([("BTC-USDT".to_owned(), vec![(1_000, 42.5)])]),
            market_marks: HashMap::from([(
                "BTC-USDT".to_owned(),
                MarketMarkSnapshot {
                    mid: Some(42.5),
                    updated_at_ms: 1_100,
                },
            )]),
            timing_contract: MarketHistoryTimingContract::current(),
        };

        let serialized = serde_json::to_value(response).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({
                "histories": { "BTC-USDT": [[1000, 42.5]] },
                "market_marks": { "BTC-USDT": { "mid": 42.5, "updated_at_ms": 1100 } },
                "timing_contract": {
                    "version": 1,
                    "sample_interval_ms": 10_000,
                    "retention_ms": 1_800_000,
                    "ticker_stream_idle_timeout_ms": 60_000,
                    "market_mark_freshness_ms": 5_000
                }
            })
        );
    }

    #[tokio::test]
    async fn refuses_book_subscriptions_over_the_cap() {
        let data = Arc::new(empty_data());
        data.subscriptions
            .lock()
            .await
            .extend((0..MAX_MARKET_BOOK_SUBSCRIPTIONS).map(|index| (format!("S{index}-USDT"), ())));
        assert!(data.ensure_book_subscription("BTC-USDT").await.is_err());
    }
}
