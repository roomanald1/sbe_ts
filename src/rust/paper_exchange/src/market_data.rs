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
    collections::{HashMap, HashSet},
    sync::{
        Arc, RwLock,
        atomic::{AtomicU8, Ordering},
    },
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
const MAX_HELD_SYMBOLS_PER_REQUEST: usize = 64;
const MAX_HELD_SYMBOLS_QUERY_BYTES: usize = 2_048;

const SUBSCRIPTION_PENDING: u8 = 0;
const SUBSCRIPTION_ACTIVE: u8 = 1;
const SUBSCRIPTION_UNAVAILABLE: u8 = 2;

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

struct BookSubscriptionState(AtomicU8);

impl BookSubscriptionState {
    fn new() -> Self {
        Self(AtomicU8::new(SUBSCRIPTION_PENDING))
    }

    fn status(&self) -> &'static str {
        match self.0.load(Ordering::Acquire) {
            SUBSCRIPTION_ACTIVE => "active",
            SUBSCRIPTION_UNAVAILABLE => "unavailable",
            _ => "pending",
        }
    }
}

pub struct MarketData {
    books: RwLock<HashMap<String, Arc<BookSnapshot>>>,
    histories: Mutex<Histories>,
    subscriptions: Mutex<HashMap<String, Arc<BookSubscriptionState>>>,
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
        let subscription = Arc::new(BookSubscriptionState::new());
        subscriptions.insert(symbol.clone(), Arc::clone(&subscription));
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
                                    if book_symbol(&bytes) == Some(symbol.as_str())
                                        && let Some(book) = decode_book(&bytes)
                                    {
                                        if let Some((timestamp, mid)) = book_history_sample(&book) {
                                            data.record_history_sample(&symbol, timestamp, mid)
                                                .await;
                                        }
                                        data.books
                                            .write()
                                            .unwrap_or_else(|p| p.into_inner())
                                            .insert(symbol.clone(), Arc::new(book));
                                        subscription
                                            .0
                                            .store(SUBSCRIPTION_ACTIVE, Ordering::Release);
                                    }
                                }
                                Ok(Some(Err(error))) => {
                                    if error.to_string().contains(
                                        "instrument is not available in the current OKX subscription",
                                    ) {
                                        subscription
                                            .0
                                            .store(SUBSCRIPTION_UNAVAILABLE, Ordering::Release);
                                    }
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
                        if subscription.status() != "unavailable" {
                            subscription
                                .0
                                .store(SUBSCRIPTION_PENDING, Ordering::Release);
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
        for message in bytes.chunks_exact(TICKER_MESSAGE_LENGTH) {
            let symbol = String::from_utf8_lossy(&message[8..40])
                .trim_end_matches([' ', '\0'])
                .to_owned();
            let price = f64::from_le_bytes(message[40..48].try_into().unwrap_or_default());
            if !valid_symbol(&symbol) || !price.is_finite() || price <= 0.0 {
                continue;
            }
            self.record_history_sample(&symbol, timestamp, price).await;
        }
    }

    async fn record_history_sample(&self, symbol: &str, timestamp: u64, price: f64) {
        let mut histories = self.histories.lock().await;
        record_history_sample(&mut histories, symbol, timestamp, price);
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
    symbol: String,
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    worst_ask: Option<f64>,
    updated_at_ms: u64,
    symbol_freshness: SymbolMarkFreshness,
}

#[derive(Serialize)]
struct SymbolMarkFreshness {
    contract_version: u16,
    scope: &'static str,
    state: &'static str,
    max_age_ms: u64,
    timestamp_field: &'static str,
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
            now.saturating_sub(book.updated_at_ms) <= MARKET_MARK_FRESHNESS.as_millis() as u64
        })
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("No fresh market mark for {symbol}"),
        ))?;
    let bid = book.bids.first().map(|level| level.price);
    let ask = book.asks.first().map(|level| level.price);
    Ok(Json(MarketMark {
        symbol,
        bid,
        ask,
        mid: bid.zip(ask).map(|(bid, ask)| (bid + ask) / 2.0),
        worst_ask: book.asks.last().map(|level| level.price),
        updated_at_ms: book.updated_at_ms,
        symbol_freshness: SymbolMarkFreshness {
            contract_version: 1,
            scope: "symbol",
            state: "fresh",
            max_age_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
            timestamp_field: "updated_at_ms",
        },
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
            now.saturating_sub(book.updated_at_ms) <= MARKET_MARK_FRESHNESS.as_millis() as u64
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
    history_freshness: HashMap<String, SymbolHistoryFreshness>,
    held_symbol_subscriptions: Vec<HeldSymbolSubscription>,
    timing_contract: MarketHistoryTimingContract,
}

#[derive(Serialize)]
struct MarketMarkSnapshot {
    mid: Option<f64>,
    updated_at_ms: u64,
}

#[derive(Serialize)]
struct SymbolHistoryFreshness {
    latest_observed_at_ms: Option<u64>,
    observed_age_ms: Option<u64>,
    state: &'static str,
}

#[derive(Serialize)]
struct HeldSymbolSubscription {
    requested_symbol: String,
    symbol: Option<String>,
    subscription: Option<String>,
    status: &'static str,
    error: Option<String>,
    fresh_mark: Option<MarketMarkSnapshot>,
    history_freshness: SymbolHistoryFreshness,
}

#[derive(Deserialize, Default)]
struct MarketHistoriesQuery {
    held_symbols: Option<String>,
}

#[derive(Serialize)]
struct MarketHistoryTimingContract {
    version: u16,
    sample_interval_ms: u64,
    retention_ms: u64,
    ticker_stream_idle_timeout_ms: u64,
    market_mark_freshness_ms: u64,
    history_freshness_max_age_ms: u64,
    history_freshness_basis: &'static str,
}

impl MarketHistoryTimingContract {
    fn current() -> Self {
        Self {
            version: 3,
            sample_interval_ms: TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64,
            retention_ms: TICKER_HISTORY_RETENTION.as_millis() as u64,
            ticker_stream_idle_timeout_ms: TICKER_STREAM_IDLE_TIMEOUT.as_millis() as u64,
            market_mark_freshness_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
            history_freshness_max_age_ms: history_freshness_max_age_ms(),
            history_freshness_basis: "per-symbol active book subscription and current mark; history sample age bounded by sample spacing plus mark freshness",
        }
    }
}

async fn get_market_histories(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<MarketHistoriesQuery>,
) -> Result<Json<MarketHistories>, (StatusCode, String)> {
    let requested_symbols = parse_held_symbols(query.held_symbols.as_deref())
        .map_err(|error| (StatusCode::BAD_REQUEST, error))?;
    let mut held_symbol_subscriptions = Vec::with_capacity(requested_symbols.len());
    for requested_symbol in requested_symbols {
        let symbol = requested_symbol.trim().to_ascii_uppercase();
        if !valid_symbol(&symbol) {
            held_symbol_subscriptions.push(HeldSymbolSubscription {
                requested_symbol,
                symbol: None,
                subscription: None,
                status: "invalid_symbol",
                error: Some("A valid market symbol is required".to_owned()),
                fresh_mark: None,
                history_freshness: history_freshness(
                    &[],
                    now_ms().unwrap_or_default(),
                    false,
                    false,
                ),
            });
            continue;
        }

        if let Err(error) = data.ensure_book_subscription(&symbol).await {
            held_symbol_subscriptions.push(HeldSymbolSubscription {
                requested_symbol,
                symbol: Some(symbol),
                subscription: None,
                status: "unavailable",
                error: Some(error.to_string()),
                fresh_mark: None,
                history_freshness: history_freshness(
                    &[],
                    now_ms().unwrap_or_default(),
                    false,
                    false,
                ),
            });
            continue;
        }

        let subscription = data.subscriptions.lock().await.get(&symbol).cloned();
        let subscription_status = subscription.as_ref().map(|state| state.status());
        let history_samples = data.history(&symbol).await;
        let now = now_ms().unwrap_or_default();
        let book = data.book(&symbol);
        let fresh_mark = (subscription_status == Some("active"))
            .then(|| book.as_ref())
            .flatten()
            .filter(|snapshot| market_mark_is_fresh(snapshot, now))
            .map(|book| market_mark_snapshot(book));
        let symbol_history_freshness = history_freshness(
            &history_samples,
            now,
            subscription_status == Some("active"),
            fresh_mark.is_some(),
        );
        let (status, error) = match subscription_status {
            Some("unavailable") => (
                "unavailable",
                Some("FeedHandlers does not provide this symbol".to_owned()),
            ),
            Some("active") => ("subscribed", None),
            _ => ("subscription_pending", None),
        };
        held_symbol_subscriptions.push(HeldSymbolSubscription {
            requested_symbol,
            symbol: Some(symbol),
            subscription: subscription_status.map(str::to_owned),
            status,
            error,
            fresh_mark,
            history_freshness: symbol_history_freshness,
        });
    }

    let histories = data.histories().await;
    let now = now_ms().unwrap_or_default();
    let active_book_subscriptions = data
        .subscriptions
        .lock()
        .await
        .iter()
        .filter_map(|(symbol, state)| (state.status() == "active").then(|| symbol.clone()))
        .collect::<HashSet<_>>();
    let book_snapshots = data
        .books
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .map(|(symbol, book)| (symbol.clone(), Arc::clone(book)))
        .collect::<HashMap<_, _>>();
    let history_freshness = histories
        .iter()
        .map(|(symbol, samples)| {
            let active = active_book_subscriptions.contains(symbol);
            let book_is_fresh = book_snapshots
                .get(symbol)
                .is_some_and(|book| market_mark_is_fresh(book, now));
            (
                symbol.clone(),
                history_freshness(samples, now, active, book_is_fresh),
            )
        })
        .collect();
    let market_marks = data
        .books
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .filter_map(|(symbol, book)| {
            (active_book_subscriptions.contains(symbol) && market_mark_is_fresh(book, now))
                .then(|| (symbol.clone(), market_mark_snapshot(book)))
        })
        .collect();
    Ok(Json(MarketHistories {
        histories,
        market_marks,
        history_freshness,
        held_symbol_subscriptions,
        timing_contract: MarketHistoryTimingContract::current(),
    }))
}

fn market_mark_snapshot(book: &BookSnapshot) -> MarketMarkSnapshot {
    MarketMarkSnapshot {
        mid: market_mid(book),
        updated_at_ms: book.updated_at_ms,
    }
}

fn parse_held_symbols(input: Option<&str>) -> Result<Vec<String>, String> {
    let Some(input) = input else {
        return Ok(Vec::new());
    };
    if input.len() > MAX_HELD_SYMBOLS_QUERY_BYTES {
        return Err(format!(
            "held_symbols exceeds the {MAX_HELD_SYMBOLS_QUERY_BYTES}-byte limit"
        ));
    }
    if input.trim().is_empty() {
        return Ok(Vec::new());
    }
    let symbols = input.split(',').map(str::to_owned).collect::<Vec<_>>();
    if symbols.len() > MAX_HELD_SYMBOLS_PER_REQUEST {
        return Err(format!(
            "held_symbols exceeds the {MAX_HELD_SYMBOLS_PER_REQUEST}-symbol limit"
        ));
    }
    Ok(symbols)
}

fn record_history_sample(histories: &mut Histories, symbol: &str, timestamp: u64, price: f64) {
    if !valid_symbol(symbol) || !price.is_finite() || price <= 0.0 {
        return;
    }
    if !histories.by_symbol.contains_key(symbol)
        && histories.by_symbol.len() >= MAX_TICKER_HISTORY_SYMBOLS
    {
        return;
    }
    let samples = histories.by_symbol.entry(symbol.to_owned()).or_default();
    if samples.last().is_none_or(|(last, _)| {
        timestamp.saturating_sub(*last) >= TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64
    }) {
        samples.push((timestamp, price));
        samples.retain(|(time, _)| {
            timestamp.saturating_sub(*time) <= TICKER_HISTORY_RETENTION.as_millis() as u64
        });
    }
}

fn history_freshness_max_age_ms() -> u64 {
    TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64 + MARKET_MARK_FRESHNESS.as_millis() as u64
}

fn history_freshness(
    samples: &[(u64, f64)],
    now: u64,
    active_subscription: bool,
    fresh_book_mark: bool,
) -> SymbolHistoryFreshness {
    let latest_observed_at_ms = samples.last().map(|(timestamp, _)| *timestamp);
    let observed_age_ms = latest_observed_at_ms.map(|timestamp| now.saturating_sub(timestamp));
    let state = if !active_subscription || latest_observed_at_ms.is_none() {
        "unavailable"
    } else if !fresh_book_mark
        || observed_age_ms.is_some_and(|age| age > history_freshness_max_age_ms())
    {
        "stale"
    } else {
        "fresh"
    };
    SymbolHistoryFreshness {
        latest_observed_at_ms,
        observed_age_ms,
        state,
    }
}

fn market_mid(book: &BookSnapshot) -> Option<f64> {
    book.bids
        .first()
        .zip(book.asks.first())
        .map(|(bid, ask)| (bid.price + ask.price) / 2.0)
        .filter(|mid| mid.is_finite() && *mid > 0.0)
}

fn market_mark_is_fresh(book: &BookSnapshot, now: u64) -> bool {
    market_mid(book).is_some()
        && now.saturating_sub(book.updated_at_ms) <= MARKET_MARK_FRESHNESS.as_millis() as u64
}

fn book_history_sample(book: &BookSnapshot) -> Option<(u64, f64)> {
    market_mid(book).map(|mid| (book.updated_at_ms, mid))
}

fn book_symbol(bytes: &[u8]) -> Option<&str> {
    let symbol = std::str::from_utf8(bytes.get(8..40)?).ok()?.trim();
    valid_symbol(symbol).then_some(symbol)
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
            history_freshness: HashMap::from([(
                "BTC-USDT".to_owned(),
                history_freshness(&[(1_000, 42.5)], 1_100, true, true),
            )]),
            held_symbol_subscriptions: vec![],
            timing_contract: MarketHistoryTimingContract::current(),
        };

        let serialized = serde_json::to_value(response).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({
                "histories": { "BTC-USDT": [[1000, 42.5]] },
                "market_marks": { "BTC-USDT": { "mid": 42.5, "updated_at_ms": 1100 } },
                "history_freshness": {
                    "BTC-USDT": {
                        "latest_observed_at_ms": 1000,
                        "observed_age_ms": 100,
                        "state": "fresh"
                    }
                },
                "held_symbol_subscriptions": [],
                "timing_contract": {
                    "version": 3,
                    "sample_interval_ms": 10_000,
                    "retention_ms": 1_800_000,
                    "ticker_stream_idle_timeout_ms": 60_000,
                    "market_mark_freshness_ms": 5_000,
                    "history_freshness_max_age_ms": 15_000,
                    "history_freshness_basis": "per-symbol active book subscription and current mark; history sample age bounded by sample spacing plus mark freshness"
                }
            })
        );
    }

    #[test]
    fn market_mark_serializes_a_per_symbol_freshness_guarantee() {
        let response = MarketMark {
            symbol: "BTC-USDT".into(),
            bid: Some(99.0),
            ask: Some(101.0),
            mid: Some(100.0),
            worst_ask: Some(101.0),
            updated_at_ms: 10_000,
            symbol_freshness: SymbolMarkFreshness {
                contract_version: 1,
                scope: "symbol",
                state: "fresh",
                max_age_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
                timestamp_field: "updated_at_ms",
            },
        };
        let value = serde_json::to_value(response).unwrap();
        assert_eq!(value["symbol"], "BTC-USDT");
        assert_eq!(value["updated_at_ms"], 10_000);
        assert_eq!(value["symbol_freshness"]["contract_version"], 1);
        assert_eq!(value["symbol_freshness"]["scope"], "symbol");
        assert_eq!(value["symbol_freshness"]["state"], "fresh");
        assert_eq!(
            value["symbol_freshness"]["max_age_ms"],
            MARKET_MARK_FRESHNESS.as_millis() as u64
        );
        assert_eq!(
            value["symbol_freshness"]["timestamp_field"],
            "updated_at_ms"
        );
    }

    #[tokio::test]
    async fn refuses_book_subscriptions_over_the_cap() {
        let data = Arc::new(empty_data());
        data.subscriptions
            .lock()
            .await
            .extend((0..MAX_MARKET_BOOK_SUBSCRIPTIONS).map(|index| {
                (
                    format!("S{index}-USDT"),
                    Arc::new(BookSubscriptionState::new()),
                )
            }));
        assert!(data.ensure_book_subscription("BTC-USDT").await.is_err());
    }

    #[test]
    fn held_symbol_request_is_bounded_and_validates_each_symbol_separately() {
        assert_eq!(parse_held_symbols(None).unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_held_symbols(Some("BTC-USDT,not-valid,ETH-USDT")).unwrap(),
            vec!["BTC-USDT", "not-valid", "ETH-USDT"]
        );
        assert!(parse_held_symbols(Some(&"x".repeat(MAX_HELD_SYMBOLS_QUERY_BYTES + 1))).is_err());
        let too_many = (0..=MAX_HELD_SYMBOLS_PER_REQUEST)
            .map(|index| format!("S{index}-USDT"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_held_symbols(Some(&too_many)).is_err());
    }

    #[test]
    fn per_symbol_history_marks_one_symbol_stale_while_another_continues() {
        let observed_at = 1_000_000;
        let old_symbol = history_freshness(&[(observed_at, 1.0)], observed_at + 16_000, true, true);
        let continuing_symbol = history_freshness(
            &[(observed_at + 14_000, 2.0)],
            observed_at + 15_000,
            true,
            true,
        );
        assert_eq!(old_symbol.observed_age_ms, Some(16_000));
        assert_eq!(continuing_symbol.observed_age_ms, Some(1_000));
        assert_eq!(old_symbol.state, "stale");
        assert_eq!(continuing_symbol.state, "fresh");
        assert_eq!(
            MarketHistoryTimingContract::current().history_freshness_max_age_ms,
            15_000,
        );
    }

    #[tokio::test]
    async fn book_midpoints_are_added_to_history_at_the_existing_sample_spacing() {
        let mut histories = Histories::default();
        let first_book = BookSnapshot {
            bids: vec![BookLevel {
                price: 99.0,
                size: 1.0,
            }],
            asks: vec![BookLevel {
                price: 101.0,
                size: 1.0,
            }],
            updated_at_ms: 10_000,
        };
        let too_soon_book = BookSnapshot {
            updated_at_ms: 19_999,
            ..first_book.clone()
        };
        let next_sample_book = BookSnapshot {
            updated_at_ms: 20_000,
            ..first_book.clone()
        };
        for book in [&first_book, &too_soon_book, &next_sample_book] {
            let (timestamp, mid) = book_history_sample(book).unwrap();
            record_history_sample(&mut histories, "BTC-USDT", timestamp, mid);
        }
        assert_eq!(
            histories.by_symbol.get("BTC-USDT").unwrap(),
            &vec![(10_000, 100.0), (20_000, 100.0)]
        );
    }

    #[test]
    fn history_freshness_requires_active_subscription_and_fresh_book_mark() {
        let timestamp = 10_000;
        let no_subscription = history_freshness(&[(timestamp, 100.0)], 11_000, false, true);
        let stale_mark = history_freshness(&[(timestamp, 100.0)], 11_000, true, false);
        let no_history = history_freshness(&[], 11_000, true, true);
        assert_eq!(no_subscription.state, "unavailable");
        assert_eq!(stale_mark.state, "stale");
        assert_eq!(no_history.state, "unavailable");
    }

    #[tokio::test]
    async fn held_symbols_are_subscribed_before_fresh_marks_are_included_and_failures_are_explicit()
    {
        let data = Arc::new(empty_data());
        data.subscriptions.lock().await.insert(
            "BTC-USDT".to_owned(),
            Arc::new(BookSubscriptionState(AtomicU8::new(SUBSCRIPTION_ACTIVE))),
        );
        data.subscriptions.lock().await.insert(
            "MISSING-USDT".to_owned(),
            Arc::new(BookSubscriptionState(AtomicU8::new(
                SUBSCRIPTION_UNAVAILABLE,
            ))),
        );
        data.books.write().unwrap().insert(
            "BTC-USDT".to_owned(),
            Arc::new(BookSnapshot {
                bids: vec![BookLevel {
                    price: 99.0,
                    size: 1.0,
                }],
                asks: vec![BookLevel {
                    price: 101.0,
                    size: 1.0,
                }],
                updated_at_ms: now_ms().unwrap(),
            }),
        );

        let Json(response) = get_market_histories(
            State(Arc::clone(&data)),
            Query(MarketHistoriesQuery {
                held_symbols: Some("BTC-USDT,MISSING-USDT,bad".to_owned()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.held_symbol_subscriptions[0].status, "subscribed");
        assert!(response.held_symbol_subscriptions[0].fresh_mark.is_some());
        assert_eq!(response.held_symbol_subscriptions[1].status, "unavailable");
        assert!(response.held_symbol_subscriptions[1].error.is_some());
        assert!(response.held_symbol_subscriptions[1].fresh_mark.is_none());
        assert_eq!(
            response.held_symbol_subscriptions[2].status,
            "invalid_symbol"
        );
        assert!(response.held_symbol_subscriptions[2].fresh_mark.is_none());
    }

    #[tokio::test]
    async fn requested_held_symbol_starts_a_book_subscription_before_returning() {
        let data = Arc::new(empty_data());
        data.ensure_book_subscription("BTC-USDT").await.unwrap();
        let subscriptions = data.subscriptions.lock().await;
        assert_eq!(subscriptions.get("BTC-USDT").unwrap().status(), "pending");
        assert!(data.book("BTC-USDT").is_none());
    }
}
