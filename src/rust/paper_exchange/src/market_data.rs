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
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Mutex;

const BOOK_DEPTH: usize = 7;
const BOOK_MESSAGE_LENGTH: usize = 272;
const TICKER_MESSAGE_LENGTH: usize = 64;
const MAX_MARKET_BOOK_SUBSCRIPTIONS: usize = 256;
const MAX_TICKER_HISTORY_SYMBOLS: usize = 128;
const MAX_MARK_EVIDENCE_PER_SYMBOL: usize =
    (TICKER_HISTORY_RETENTION.as_secs() / TICKER_HISTORY_SAMPLE_INTERVAL.as_secs()) as usize;
const MAX_MARK_EVIDENCE_RECORDS: usize = MAX_TICKER_HISTORY_SYMBOLS * MAX_MARK_EVIDENCE_PER_SYMBOL;
const MAX_MARK_EVIDENCE_PAGE_SIZE: u16 = 500;
const FEED_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const BOOK_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const TICKER_STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TICKER_HISTORY_SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const TICKER_HISTORY_RETENTION: Duration = Duration::from_secs(30 * 60);
const EXECUTABLE_PATH_SAMPLE_INTERVAL: Duration = Duration::from_secs(1);
const EXECUTABLE_PATH_RETENTION: Duration = Duration::from_secs(10 * 60);
const MAX_EXECUTABLE_PATH_SYMBOLS: usize = MAX_MARKET_BOOK_SUBSCRIPTIONS;
const MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL: usize = 600;
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cursor: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_record_id: Option<String>,
}

#[derive(Default)]
struct Histories {
    by_symbol: HashMap<String, Vec<(u64, f64)>>,
    executable_paths: HashMap<String, ExecutablePathHistory>,
}

#[derive(Default)]
struct ExecutablePathHistory {
    source_epoch: String,
    samples: VecDeque<ExecutableQuoteSample>,
}

#[derive(Clone, Copy)]
struct ExecutableQuoteSample {
    timestamp_ms: u64,
    bid: f64,
    ask: f64,
    source_cursor: u64,
}

#[derive(Serialize)]
struct ExecutableQuotePathResponse {
    schema_version: u32,
    symbol: String,
    source_system: &'static str,
    persistence_status: &'static str,
    captured_at_ms: u64,
    latest_sample_age_ms: Option<u64>,
    source_epoch: String,
    sample_interval_ms: u64,
    retention_ms: u64,
    coverage_start_ms: Option<u64>,
    coverage_end_ms: Option<u64>,
    maximum_observed_gap_ms: u64,
    truncated: bool,
    samples: Vec<ExecutableQuotePathPoint>,
}

#[derive(Serialize)]
struct ExecutableQuotePathPoint {
    timestamp_ms: u64,
    bid: f64,
    ask: f64,
    source_cursor: u64,
    source_record_id: String,
}

#[derive(Deserialize)]
struct ExecutableQuotePathQuery {
    symbol: String,
    since_ms: Option<u64>,
    limit: Option<usize>,
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
    source_epoch: Option<String>,
    source_cursor: AtomicU64,
    mark_evidence_path: PathBuf,
}

impl MarketData {
    pub fn new() -> Arc<Self> {
        let data = Arc::new(Self {
            books: RwLock::new(HashMap::new()),
            histories: Mutex::new(Histories::default()),
            subscriptions: Mutex::new(HashMap::new()),
            feed_client: Mutex::new(None),
            source_epoch: now_ms().ok().map(|timestamp| {
                format!(
                    "paper-exchange-market-data:v1:{timestamp}:{}",
                    std::process::id()
                )
            }),
            source_cursor: AtomicU64::new(0),
            mark_evidence_path: std::env::var_os("PAPER_EXCHANGE_EXECUTION_LEDGER_FILE")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::current_dir()
                        .unwrap_or_else(|_| PathBuf::from("."))
                        .join("paper_exchange_matches.sqlite")
                }),
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
                                        && let Some(mut book) = decode_book(&bytes)
                                    {
                                        let source_cursor = data
                                            .source_cursor
                                            .fetch_add(1, Ordering::AcqRel)
                                            .saturating_add(1);
                                        attach_book_source_identity(
                                            &mut book,
                                            &symbol,
                                            data.source_epoch.as_deref(),
                                            source_cursor,
                                        );
                                        data.record_executable_path_sample(&symbol, &book).await;
                                        let mut source_evidence_persisted = false;
                                        if let Some(sample) = book_history_sample(&book) {
                                            if data
                                                .record_history_sample(
                                                    &symbol,
                                                    sample.timestamp_ms,
                                                    sample.mid,
                                                )
                                                .await
                                            {
                                                if let Some(evidence) =
                                                    sample.into_evidence(&symbol, &book)
                                                {
                                                    if let Err(error) = persist_mark_evidence(
                                                        data.mark_evidence_path.clone(),
                                                        evidence,
                                                    )
                                                    .await
                                                    {
                                                        eprintln!(
                                                            "Paper Exchange market mark evidence write failed for {symbol}: {error:#}"
                                                        );
                                                    } else {
                                                        source_evidence_persisted = true;
                                                    }
                                                }
                                            }
                                        }
                                        if !source_evidence_persisted {
                                            clear_book_source_identity(&mut book);
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

    async fn record_history_sample(&self, symbol: &str, timestamp: u64, price: f64) -> bool {
        let mut histories = self.histories.lock().await;
        record_history_sample(&mut histories, symbol, timestamp, price)
    }

    async fn record_executable_path_sample(&self, symbol: &str, book: &BookSnapshot) -> bool {
        let mut histories = self.histories.lock().await;
        record_executable_path_sample(&mut histories, symbol, book)
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

    async fn executable_path(
        &self,
        symbol: &str,
        since_ms: Option<u64>,
        limit: usize,
    ) -> Option<ExecutableQuotePathResponse> {
        let histories = self.histories.lock().await;
        let path = histories.executable_paths.get(symbol)?;
        let captured_at_ms = now_ms().ok()?;
        let cutoff = captured_at_ms.saturating_sub(EXECUTABLE_PATH_RETENTION.as_millis() as u64);
        let retained_start = path
            .samples
            .iter()
            .find(|sample| sample.timestamp_ms >= cutoff)
            .map(|sample| sample.timestamp_ms);
        let retained_end = path
            .samples
            .iter()
            .rev()
            .find(|sample| sample.timestamp_ms >= cutoff)
            .map(|sample| sample.timestamp_ms);
        let retained = path
            .samples
            .iter()
            .copied()
            .filter(|sample| sample.timestamp_ms >= cutoff)
            .collect::<Vec<_>>();
        let mut samples = retained
            .into_iter()
            .filter(|sample| since_ms.is_none_or(|since| sample.timestamp_ms >= since))
            .collect::<Vec<_>>();
        let mut truncated =
            since_ms.is_some_and(|since| retained_start.is_some_and(|start| since < start));
        if samples.len() > limit {
            samples.drain(..samples.len() - limit);
            truncated = true;
        }
        let maximum_observed_gap_ms = samples
            .windows(2)
            .map(|pair| pair[1].timestamp_ms.saturating_sub(pair[0].timestamp_ms))
            .max()
            .unwrap_or_default();
        Some(ExecutableQuotePathResponse {
            schema_version: 1,
            symbol: symbol.to_owned(),
            source_system: "feed_handlers.book_stream",
            persistence_status: "volatile_process_epoch",
            captured_at_ms,
            latest_sample_age_ms: retained_end
                .map(|timestamp| captured_at_ms.saturating_sub(timestamp)),
            source_epoch: path.source_epoch.clone(),
            sample_interval_ms: EXECUTABLE_PATH_SAMPLE_INTERVAL.as_millis() as u64,
            retention_ms: EXECUTABLE_PATH_RETENTION.as_millis() as u64,
            coverage_start_ms: samples.first().map(|sample| sample.timestamp_ms),
            coverage_end_ms: samples.last().map(|sample| sample.timestamp_ms),
            maximum_observed_gap_ms,
            truncated,
            samples: samples
                .into_iter()
                .map(|sample| ExecutableQuotePathPoint {
                    timestamp_ms: sample.timestamp_ms,
                    bid: sample.bid,
                    ask: sample.ask,
                    source_cursor: sample.source_cursor,
                    source_record_id: format!(
                        "paper-book:v1:{}:{symbol}:{}",
                        path.source_epoch, sample.source_cursor
                    ),
                })
                .collect(),
        })
    }

    async fn histories(&self) -> HashMap<String, Vec<(u64, f64)>> {
        self.histories.lock().await.by_symbol.clone()
    }
}

#[derive(Deserialize)]
struct SymbolQuery {
    symbol: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct MarketMarkEvidence {
    evidence_cursor: i64,
    symbol: String,
    quote_timestamp_ms: u64,
    bid: f64,
    ask: f64,
    source_epoch: String,
    source_cursor: u64,
    source_record_id: String,
}

#[derive(Deserialize)]
struct MarketMarkEvidenceQuery {
    symbol: String,
    source_epoch: Option<String>,
    source_cursor: Option<u64>,
    source_record_id: Option<String>,
    after_cursor: Option<i64>,
    limit: Option<u16>,
}

#[derive(Serialize)]
struct MarketMarkEvidencePage {
    evidence: Vec<MarketMarkEvidence>,
    next_cursor: Option<i64>,
    has_more: bool,
    contract_version: u16,
    retention_ms: u64,
    sample_interval_ms: u64,
    freshness_max_age_ms: u64,
}

#[derive(Serialize)]
struct MarketMark {
    symbol: String,
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    worst_ask: Option<f64>,
    updated_at_ms: u64,
    source_system: &'static str,
    source_epoch: Option<String>,
    source_cursor: Option<u64>,
    source_record_id: Option<String>,
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
        source_system: "paper_exchange.market_data_cache",
        source_epoch: book.source_epoch.clone(),
        source_cursor: book.source_cursor,
        source_record_id: book.source_record_id.clone(),
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

async fn get_market_quote_path(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<ExecutableQuotePathQuery>,
) -> Result<Json<ExecutableQuotePathResponse>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    let limit = query
        .limit
        .unwrap_or(MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL)
        .clamp(1, MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL);
    data.executable_path(&symbol, query.since_ms, limit)
        .await
        .map(Json)
        .ok_or((
            StatusCode::NOT_FOUND,
            format!("No retained executable quote path for {symbol}"),
        ))
}

async fn get_market_mark_evidence(
    State(data): State<Arc<MarketData>>,
    Query(query): Query<MarketMarkEvidenceQuery>,
) -> Result<Json<MarketMarkEvidencePage>, (StatusCode, String)> {
    let symbol = query.symbol.to_ascii_uppercase();
    if !valid_symbol(&symbol) {
        return Err((
            StatusCode::BAD_REQUEST,
            "A valid market symbol is required".into(),
        ));
    }
    let identity_parts = [
        query.source_epoch.is_some(),
        query.source_cursor.is_some(),
        query.source_record_id.is_some(),
    ];
    if identity_parts.iter().any(|present| *present) {
        if !identity_parts.iter().all(|present| *present) || query.after_cursor.is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Provide a complete source identity or a page cursor".into(),
            ));
        }
        if query
            .source_cursor
            .is_some_and(|cursor| cursor > i64::MAX as u64)
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "source_cursor is out of range".into(),
            ));
        }
        let retention_cutoff = mark_evidence_retention_cutoff()
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
        let evidence = lookup_mark_evidence(
            data.mark_evidence_path.clone(),
            symbol,
            query.source_epoch.unwrap_or_default(),
            query.source_cursor.unwrap_or_default(),
            query.source_record_id.unwrap_or_default(),
            retention_cutoff,
        )
        .await
        .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?
        .ok_or((
            StatusCode::NOT_FOUND,
            "Market mark source identity was not found".into(),
        ))?;
        return Ok(Json(MarketMarkEvidencePage {
            evidence: vec![evidence],
            next_cursor: None,
            has_more: false,
            contract_version: 1,
            retention_ms: TICKER_HISTORY_RETENTION.as_millis() as u64,
            sample_interval_ms: TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64,
            freshness_max_age_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
        }));
    }

    let limit = query.limit.unwrap_or(100);
    if limit == 0
        || limit > MAX_MARK_EVIDENCE_PAGE_SIZE
        || query.after_cursor.is_some_and(|cursor| cursor < 0)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("limit must be between 1 and {MAX_MARK_EVIDENCE_PAGE_SIZE}"),
        ));
    }
    let (evidence, has_more) = page_mark_evidence(
        data.mark_evidence_path.clone(),
        symbol,
        query.after_cursor.unwrap_or(0),
        limit,
        mark_evidence_retention_cutoff()
            .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?,
    )
    .await
    .map_err(|error| (StatusCode::SERVICE_UNAVAILABLE, error.to_string()))?;
    let next_cursor = if has_more {
        evidence.last().map(|mark| mark.evidence_cursor)
    } else {
        None
    };
    Ok(Json(MarketMarkEvidencePage {
        evidence,
        next_cursor,
        has_more,
        contract_version: 1,
        retention_ms: TICKER_HISTORY_RETENTION.as_millis() as u64,
        sample_interval_ms: TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64,
        freshness_max_age_ms: MARKET_MARK_FRESHNESS.as_millis() as u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_cursor: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source_record_id: Option<String>,
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
        source_epoch: book.source_epoch.clone(),
        source_cursor: book.source_cursor,
        source_record_id: book.source_record_id.clone(),
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

fn record_history_sample(
    histories: &mut Histories,
    symbol: &str,
    timestamp: u64,
    price: f64,
) -> bool {
    if !valid_symbol(symbol) || !price.is_finite() || price <= 0.0 {
        return false;
    }
    if !histories.by_symbol.contains_key(symbol)
        && histories.by_symbol.len() >= MAX_TICKER_HISTORY_SYMBOLS
    {
        return false;
    }
    let samples = histories.by_symbol.entry(symbol.to_owned()).or_default();
    if samples.last().is_none_or(|(last, _)| {
        timestamp.saturating_sub(*last) >= TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64
    }) {
        samples.push((timestamp, price));
        samples.retain(|(time, _)| {
            timestamp.saturating_sub(*time) <= TICKER_HISTORY_RETENTION.as_millis() as u64
        });
        true
    } else {
        false
    }
}

fn record_executable_path_sample(
    histories: &mut Histories,
    symbol: &str,
    book: &BookSnapshot,
) -> bool {
    if !valid_symbol(symbol) || book.updated_at_ms == 0 {
        return false;
    }
    let Some(source_epoch) = book
        .source_epoch
        .as_deref()
        .filter(|epoch| !epoch.trim().is_empty())
    else {
        return false;
    };
    let Some(source_cursor) = book.source_cursor.filter(|cursor| *cursor > 0) else {
        return false;
    };
    let Some((bid, ask)) = book
        .bids
        .first()
        .zip(book.asks.first())
        .map(|(bid, ask)| (bid.price, ask.price))
    else {
        return false;
    };
    if !bid.is_finite() || bid <= 0.0 || !ask.is_finite() || ask < bid {
        return false;
    }
    if !histories.executable_paths.contains_key(symbol)
        && histories.executable_paths.len() >= MAX_EXECUTABLE_PATH_SYMBOLS
    {
        return false;
    }
    let path = histories
        .executable_paths
        .entry(symbol.to_owned())
        .or_default();
    if path.source_epoch != source_epoch {
        path.source_epoch = source_epoch.to_owned();
        path.samples.clear();
    }
    if path.samples.back().is_some_and(|last| {
        book.updated_at_ms <= last.timestamp_ms
            || book.updated_at_ms.saturating_sub(last.timestamp_ms)
                < EXECUTABLE_PATH_SAMPLE_INTERVAL.as_millis() as u64
    }) {
        return false;
    }
    path.samples.push_back(ExecutableQuoteSample {
        timestamp_ms: book.updated_at_ms,
        bid,
        ask,
        source_cursor,
    });
    let cutoff = book
        .updated_at_ms
        .saturating_sub(EXECUTABLE_PATH_RETENTION.as_millis() as u64);
    while path
        .samples
        .front()
        .is_some_and(|sample| sample.timestamp_ms < cutoff)
    {
        path.samples.pop_front();
    }
    while path.samples.len() > MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL {
        path.samples.pop_front();
    }
    true
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

struct BookHistorySample {
    timestamp_ms: u64,
    mid: f64,
}

impl BookHistorySample {
    fn into_evidence(self, symbol: &str, book: &BookSnapshot) -> Option<MarketMarkEvidence> {
        let evidence = MarketMarkEvidence {
            evidence_cursor: 0,
            symbol: symbol.to_owned(),
            quote_timestamp_ms: self.timestamp_ms,
            bid: book.bids.first()?.price,
            ask: book.asks.first()?.price,
            source_epoch: book.source_epoch.clone()?,
            source_cursor: book.source_cursor?,
            source_record_id: book.source_record_id.clone()?,
        };
        let expected_mid = (evidence.bid + evidence.ask) / 2.0;
        (evidence.quote_timestamp_ms == book.updated_at_ms
            && expected_mid.is_finite()
            && (expected_mid - self.mid).abs() <= f64::EPSILON * self.mid.abs().max(1.0))
        .then_some(evidence)
    }
}

fn book_history_sample(book: &BookSnapshot) -> Option<BookHistorySample> {
    market_mid(book).map(|mid| BookHistorySample {
        timestamp_ms: book.updated_at_ms,
        mid,
    })
}

fn mark_evidence_db(path: &PathBuf) -> anyhow::Result<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS executable_market_mark_evidence (
             evidence_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
             symbol TEXT NOT NULL,
             quote_timestamp_ms INTEGER NOT NULL,
             bid REAL NOT NULL,
             ask REAL NOT NULL,
             source_epoch TEXT NOT NULL,
             source_cursor INTEGER NOT NULL,
             source_record_id TEXT NOT NULL,
             UNIQUE(source_epoch, source_cursor, source_record_id)
         );
         CREATE INDEX IF NOT EXISTS executable_market_mark_symbol_page
             ON executable_market_mark_evidence(symbol, evidence_cursor);
         CREATE INDEX IF NOT EXISTS executable_market_mark_timestamp
             ON executable_market_mark_evidence(quote_timestamp_ms);",
    )?;
    Ok(connection)
}

fn mark_evidence_retention_cutoff() -> anyhow::Result<i64> {
    let cutoff = now_ms()?.saturating_sub(TICKER_HISTORY_RETENTION.as_millis() as u64);
    Ok(cutoff.min(i64::MAX as u64) as i64)
}

async fn persist_mark_evidence(path: PathBuf, evidence: MarketMarkEvidence) -> anyhow::Result<()> {
    tokio::task::spawn_blocking(move || persist_mark_evidence_sync(&path, &evidence)).await??;
    Ok(())
}

fn persist_mark_evidence_sync(path: &PathBuf, evidence: &MarketMarkEvidence) -> anyhow::Result<()> {
    anyhow::ensure!(
        valid_symbol(&evidence.symbol),
        "invalid mark evidence symbol"
    );
    anyhow::ensure!(
        evidence.quote_timestamp_ms > 0
            && evidence.quote_timestamp_ms <= i64::MAX as u64
            && evidence.bid.is_finite()
            && evidence.bid > 0.0
            && evidence.ask.is_finite()
            && evidence.ask >= evidence.bid
            && !evidence.source_epoch.is_empty()
            && evidence.source_cursor > 0
            && evidence.source_cursor <= i64::MAX as u64
            && !evidence.source_record_id.is_empty(),
        "incomplete or invalid source mark evidence"
    );
    anyhow::ensure!(
        evidence.source_record_id
            == format!(
                "paper-book:v1:{}:{}:{}",
                evidence.source_epoch, evidence.symbol, evidence.source_cursor
            ),
        "source record ID does not match its symbol, epoch, and cursor"
    );
    let mut connection = mark_evidence_db(path)?;
    let transaction = connection.transaction()?;
    transaction.execute(
        "INSERT OR IGNORE INTO executable_market_mark_evidence
         (symbol, quote_timestamp_ms, bid, ask, source_epoch, source_cursor, source_record_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            evidence.symbol,
            evidence.quote_timestamp_ms as i64,
            evidence.bid,
            evidence.ask,
            evidence.source_epoch,
            evidence.source_cursor as i64,
            evidence.source_record_id,
        ],
    )?;
    let stored: (String, i64, f64, f64, String) = transaction.query_row(
        "SELECT symbol, quote_timestamp_ms, bid, ask, source_record_id
         FROM executable_market_mark_evidence
         WHERE source_epoch = ?1 AND source_cursor = ?2 AND source_record_id = ?3",
        params![
            evidence.source_epoch,
            evidence.source_cursor as i64,
            evidence.source_record_id
        ],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        },
    )?;
    anyhow::ensure!(
        stored.0 == evidence.symbol
            && stored.1 == evidence.quote_timestamp_ms as i64
            && stored.2 == evidence.bid
            && stored.3 == evidence.ask
            && stored.4 == evidence.source_record_id,
        "source identity was reused for different market mark evidence"
    );
    let cutoff = mark_evidence_retention_cutoff()? as u64;
    transaction.execute(
        "DELETE FROM executable_market_mark_evidence WHERE quote_timestamp_ms < ?1",
        [cutoff as i64],
    )?;
    transaction.execute(
        "DELETE FROM executable_market_mark_evidence WHERE evidence_cursor IN (
             SELECT evidence_cursor FROM executable_market_mark_evidence
             WHERE symbol = ?1 ORDER BY evidence_cursor DESC LIMIT -1 OFFSET ?2
         )",
        params![evidence.symbol, MAX_MARK_EVIDENCE_PER_SYMBOL as i64],
    )?;
    transaction.execute(
        "DELETE FROM executable_market_mark_evidence WHERE evidence_cursor IN (
             SELECT evidence_cursor FROM executable_market_mark_evidence
             ORDER BY evidence_cursor DESC LIMIT -1 OFFSET ?1
         )",
        [MAX_MARK_EVIDENCE_RECORDS as i64],
    )?;
    transaction.commit()?;
    Ok(())
}

async fn lookup_mark_evidence(
    path: PathBuf,
    symbol: String,
    source_epoch: String,
    source_cursor: u64,
    source_record_id: String,
    retention_cutoff_ms: i64,
) -> anyhow::Result<Option<MarketMarkEvidence>> {
    tokio::task::spawn_blocking(move || {
        let connection = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.query_row(
            "SELECT evidence_cursor, symbol, quote_timestamp_ms, bid, ask, source_epoch, source_cursor, source_record_id
             FROM executable_market_mark_evidence
             WHERE symbol = ?1 AND source_epoch = ?2 AND source_cursor = ?3 AND source_record_id = ?4
               AND quote_timestamp_ms >= ?5",
            params![symbol, source_epoch, source_cursor as i64, source_record_id, retention_cutoff_ms],
            |row| Ok(MarketMarkEvidence {
                evidence_cursor: row.get(0)?, symbol: row.get(1)?, quote_timestamp_ms: row.get::<_, i64>(2)? as u64,
                bid: row.get(3)?, ask: row.get(4)?, source_epoch: row.get(5)?, source_cursor: row.get::<_, i64>(6)? as u64,
                source_record_id: row.get(7)?,
            }),
        ).optional().map_err(Into::into)
    }).await?
}

async fn page_mark_evidence(
    path: PathBuf,
    symbol: String,
    after_cursor: i64,
    limit: u16,
    retention_cutoff_ms: i64,
) -> anyhow::Result<(Vec<MarketMarkEvidence>, bool)> {
    tokio::task::spawn_blocking(move || {
        let connection = Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        let mut statement = connection.prepare(
            "SELECT evidence_cursor, symbol, quote_timestamp_ms, bid, ask, source_epoch, source_cursor, source_record_id
             FROM executable_market_mark_evidence WHERE symbol = ?1 AND evidence_cursor > ?2
               AND quote_timestamp_ms >= ?3
             ORDER BY evidence_cursor LIMIT ?4",
        )?;
        let rows = statement.query_map(params![symbol, after_cursor, retention_cutoff_ms, i64::from(limit) + 1], |row| Ok(MarketMarkEvidence {
            evidence_cursor: row.get(0)?, symbol: row.get(1)?, quote_timestamp_ms: row.get::<_, i64>(2)? as u64,
            bid: row.get(3)?, ask: row.get(4)?, source_epoch: row.get(5)?, source_cursor: row.get::<_, i64>(6)? as u64,
            source_record_id: row.get(7)?,
        }))?;
        let mut evidence = rows.collect::<rusqlite::Result<Vec<_>>>()?;
        let has_more = evidence.len() > usize::from(limit);
        evidence.truncate(usize::from(limit));
        Ok((evidence, has_more))
    }).await?
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
        .route("/internal/market-quote-path", get(get_market_quote_path))
        .route(
            "/internal/market-mark-evidence",
            get(get_market_mark_evidence),
        )
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
    let updated_at_ms = u64::from_le_bytes(bytes.get(40..48)?.try_into().ok()?);
    if updated_at_ms == 0 {
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
        updated_at_ms,
        source_epoch: None,
        source_cursor: None,
        source_record_id: None,
    })
}

fn attach_book_source_identity(
    book: &mut BookSnapshot,
    symbol: &str,
    source_epoch: Option<&str>,
    source_cursor: u64,
) {
    let Some(source_epoch) = source_epoch.filter(|epoch| !epoch.trim().is_empty()) else {
        book.source_epoch = None;
        book.source_cursor = None;
        book.source_record_id = None;
        return;
    };
    book.source_epoch = Some(source_epoch.to_owned());
    book.source_cursor = (source_cursor > 0).then_some(source_cursor);
    book.source_record_id = (source_cursor > 0)
        .then(|| format!("paper-book:v1:{source_epoch}:{symbol}:{source_cursor}"));
}

fn clear_book_source_identity(book: &mut BookSnapshot) {
    book.source_epoch = None;
    book.source_cursor = None;
    book.source_record_id = None;
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
            source_epoch: Some("paper-exchange-market-data:v1:test".to_owned()),
            source_cursor: AtomicU64::new(0),
            mark_evidence_path: test_database_path("empty"),
        }
    }

    fn test_database_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "paper-exchange-market-marks-{label}-{}-{}.sqlite",
            std::process::id(),
            now_ms().unwrap_or_default()
        ))
    }

    fn sample_evidence(epoch: &str, cursor: u64, timestamp: u64) -> MarketMarkEvidence {
        MarketMarkEvidence {
            evidence_cursor: 0,
            symbol: "BTC-USDT".to_owned(),
            quote_timestamp_ms: timestamp,
            bid: 99.0,
            ask: 101.0,
            source_epoch: epoch.to_owned(),
            source_cursor: cursor,
            source_record_id: format!("paper-book:v1:{epoch}:BTC-USDT:{cursor}"),
        }
    }

    fn remove_test_database(path: &PathBuf) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn decodes_a_book_snapshot_from_the_sbe_wire_layout() {
        let mut bytes = vec![0; BOOK_MESSAGE_LENGTH];
        bytes[8..40].fill(b' ');
        bytes[8..16].copy_from_slice(b"BTC-USDT");
        let source_updated_at_ms = 1_700_000_000_000_u64;
        bytes[40..48].copy_from_slice(&source_updated_at_ms.to_le_bytes());
        bytes[48..56].copy_from_slice(&100.0_f64.to_le_bytes());
        bytes[56..64].copy_from_slice(&2.0_f64.to_le_bytes());
        bytes[160..168].copy_from_slice(&101.0_f64.to_le_bytes());
        bytes[168..176].copy_from_slice(&3.0_f64.to_le_bytes());
        let snapshot = decode_book(&Bytes::from(bytes)).unwrap();
        assert_eq!(snapshot.bids[0].price, 100.0);
        assert_eq!(snapshot.asks[0].price, 101.0);
        assert_eq!(snapshot.updated_at_ms, source_updated_at_ms);
        assert!(
            !market_mark_is_fresh(
                &snapshot,
                source_updated_at_ms + MARKET_MARK_FRESHNESS.as_millis() as u64 + 1
            ),
            "old source quotes must not be refreshed by local decode time"
        );
        assert!(market_mark_is_fresh(
            &snapshot,
            source_updated_at_ms + MARKET_MARK_FRESHNESS.as_millis() as u64
        ));
        let mut sourced = snapshot;
        attach_book_source_identity(&mut sourced, "BTC-USDT", Some("book-epoch-1"), 7);
        assert_eq!(sourced.source_epoch.as_deref(), Some("book-epoch-1"));
        assert_eq!(sourced.source_cursor, Some(7));
        assert_eq!(
            sourced.source_record_id.as_deref(),
            Some("paper-book:v1:book-epoch-1:BTC-USDT:7")
        );
        attach_book_source_identity(&mut sourced, "BTC-USDT", None, 8);
        assert!(sourced.source_epoch.is_none());
        assert!(sourced.source_cursor.is_none());
        assert!(sourced.source_record_id.is_none());
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
                    source_epoch: None,
                    source_cursor: None,
                    source_record_id: None,
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
            source_system: "paper_exchange.market_data_cache",
            source_epoch: Some("market-epoch".into()),
            source_cursor: Some(3),
            source_record_id: Some("paper-book:v1:market-epoch:BTC-USDT:3".into()),
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
            source_epoch: None,
            source_cursor: None,
            source_record_id: None,
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
            let sample = book_history_sample(book).unwrap();
            record_history_sample(&mut histories, "BTC-USDT", sample.timestamp_ms, sample.mid);
        }
        assert_eq!(
            histories.by_symbol.get("BTC-USDT").unwrap(),
            &vec![(10_000, 100.0), (20_000, 100.0)]
        );
    }

    #[tokio::test]
    async fn executable_quote_buffer_is_one_second_sampled_and_retained_for_ten_minutes() {
        let make_book = |timestamp_ms: u64, source_cursor: u64| BookSnapshot {
            bids: vec![BookLevel {
                price: 99.0 + source_cursor as f64 / 1_000.0,
                size: 1.0,
            }],
            asks: vec![BookLevel {
                price: 101.0 + source_cursor as f64 / 1_000.0,
                size: 1.0,
            }],
            updated_at_ms: timestamp_ms,
            source_epoch: Some("book-epoch".into()),
            source_cursor: Some(source_cursor),
            source_record_id: Some(format!("paper-book:v1:book-epoch:BTC-USDT:{source_cursor}")),
        };
        let mut histories = Histories::default();
        assert!(record_executable_path_sample(
            &mut histories,
            "BTC-USDT",
            &make_book(10_000, 1)
        ));
        assert!(!record_executable_path_sample(
            &mut histories,
            "BTC-USDT",
            &make_book(10_999, 2)
        ));
        assert!(record_executable_path_sample(
            &mut histories,
            "BTC-USDT",
            &make_book(11_000, 3)
        ));
        assert!(record_executable_path_sample(
            &mut histories,
            "BTC-USDT",
            &make_book(611_001, 4)
        ));
        let path = histories.executable_paths.get("BTC-USDT").unwrap();
        assert_eq!(
            path.samples.len(),
            1,
            "observations older than ten minutes must roll out"
        );
        assert_eq!(path.samples[0].source_cursor, 4);
        assert_eq!(std::mem::size_of::<ExecutableQuoteSample>(), 32);
        assert!(
            MAX_EXECUTABLE_PATH_SYMBOLS * 1_024 * std::mem::size_of::<ExecutableQuoteSample>()
                <= 8 * 1_024 * 1_024,
            "worst-case power-of-two ring payload stays under 8 MiB"
        );

        let mut retained = Histories::default();
        for second in 0..(MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL + 100) {
            let ts = 1_000 + second as u64 * 1_000;
            assert!(record_executable_path_sample(
                &mut retained,
                "BTC-USDT",
                &make_book(ts, second as u64 + 1)
            ));
        }
        assert_eq!(
            retained.executable_paths["BTC-USDT"].samples.len(),
            MAX_EXECUTABLE_PATH_SAMPLES_PER_SYMBOL
        );
    }

    #[tokio::test]
    async fn executable_quote_path_endpoint_exposes_chronological_source_identity() {
        let data = empty_data();
        {
            let mut histories = data.histories.lock().await;
            let timestamp = now_ms().unwrap();
            let book = BookSnapshot {
                bids: vec![BookLevel {
                    price: 99.0,
                    size: 1.0,
                }],
                asks: vec![BookLevel {
                    price: 101.0,
                    size: 1.0,
                }],
                updated_at_ms: timestamp,
                source_epoch: Some("epoch".into()),
                source_cursor: Some(7),
                source_record_id: Some("paper-book:v1:epoch:BTC-USDT:7".into()),
            };
            assert!(record_executable_path_sample(
                &mut histories,
                "BTC-USDT",
                &book
            ));
        }
        let response = data
            .executable_path(
                "BTC-USDT",
                Some(now_ms().unwrap().saturating_sub(1_000)),
                10,
            )
            .await
            .unwrap();
        assert_eq!(response.source_system, "feed_handlers.book_stream");
        assert_eq!(response.source_epoch, "epoch");
        assert_eq!(response.maximum_observed_gap_ms, 0);
        assert_eq!(response.samples[0].source_cursor, 7);
        assert_eq!(
            response.samples[0].source_record_id,
            "paper-book:v1:epoch:BTC-USDT:7"
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
                source_epoch: None,
                source_cursor: None,
                source_record_id: None,
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

    #[tokio::test]
    async fn sampled_mark_evidence_survives_restart_and_preserves_executable_sides() {
        let path = test_database_path("restart-sides");
        let mut evidence = sample_evidence("market-epoch-1", 41, now_ms().unwrap());
        evidence.bid = 99.25;
        evidence.ask = 100.75;
        persist_mark_evidence_sync(&path, &evidence).unwrap();

        // A newly constructed market-data owner has no in-memory quote state,
        // but the exact sampled source identity remains independently queryable.
        let mut restarted = empty_data();
        restarted.mark_evidence_path = path.clone();
        assert!(restarted.book("BTC-USDT").is_none());
        let recovered = lookup_mark_evidence(
            restarted.mark_evidence_path.clone(),
            "BTC-USDT".into(),
            evidence.source_epoch.clone(),
            evidence.source_cursor,
            evidence.source_record_id.clone(),
            mark_evidence_retention_cutoff().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(recovered.quote_timestamp_ms, evidence.quote_timestamp_ms);
        assert_eq!(
            recovered.bid, 99.25,
            "long valuation must use the executable bid"
        );
        assert_eq!(
            recovered.ask, 100.75,
            "short valuation must use the executable ask"
        );
        assert!(recovered.evidence_cursor > 0);
        remove_test_database(&path);
    }

    #[test]
    fn sampled_mark_evidence_rejects_reuse_of_a_source_identity_for_changed_quote() {
        let path = test_database_path("mismatch");
        let evidence = sample_evidence("market-epoch-2", 7, now_ms().unwrap());
        persist_mark_evidence_sync(&path, &evidence).unwrap();
        let mut changed = evidence.clone();
        changed.bid = 98.0;
        assert!(persist_mark_evidence_sync(&path, &changed).is_err());
        let mut mismatched_identity = evidence.clone();
        mismatched_identity.source_record_id.push_str("-other");
        assert!(persist_mark_evidence_sync(&path, &mismatched_identity).is_err());
        remove_test_database(&path);
    }

    #[test]
    fn sampled_mark_evidence_is_retained_within_per_symbol_resource_bound() {
        let path = test_database_path("retention-bound");
        let start = now_ms().unwrap();
        for cursor in 1..=(MAX_MARK_EVIDENCE_PER_SYMBOL as u64 + 1) {
            let evidence = sample_evidence(
                "market-epoch-retention",
                cursor,
                start + (cursor - 1) * TICKER_HISTORY_SAMPLE_INTERVAL.as_millis() as u64,
            );
            persist_mark_evidence_sync(&path, &evidence).unwrap();
        }

        let connection = Connection::open(&path).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM executable_market_mark_evidence WHERE symbol = 'BTC-USDT'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count <= MAX_MARK_EVIDENCE_PER_SYMBOL as i64);
        let first_cursor = 1;
        let old = connection.query_row(
            "SELECT COUNT(*) FROM executable_market_mark_evidence WHERE source_epoch = ?1 AND source_cursor = ?2",
            params!["market-epoch-retention", first_cursor],
            |row| row.get::<_, i64>(0),
        ).unwrap();
        assert_eq!(old, 0, "the per-symbol cap must roll out the oldest sample");
        drop(connection);
        remove_test_database(&path);
    }

    #[test]
    fn sampled_mark_evidence_enforces_global_record_cap() {
        let path = test_database_path("global-bound");
        let connection = mark_evidence_db(&path).unwrap();
        let transaction = connection.unchecked_transaction().unwrap();
        let seed_timestamp = now_ms().unwrap() as i64;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO executable_market_mark_evidence
                 (symbol, quote_timestamp_ms, bid, ask, source_epoch, source_cursor, source_record_id)
                 VALUES (?1, ?2, 99.0, 101.0, 'seed-epoch', ?3, ?4)",
            ).unwrap();
            for cursor in 0..MAX_MARK_EVIDENCE_RECORDS as u64 {
                let symbol = format!("S{}-USDT", cursor / MAX_MARK_EVIDENCE_PER_SYMBOL as u64);
                insert
                    .execute(params![
                        symbol,
                        seed_timestamp,
                        cursor as i64 + 1,
                        format!("seed-{cursor}")
                    ])
                    .unwrap();
            }
        }
        transaction.commit().unwrap();
        drop(connection);

        persist_mark_evidence_sync(
            &path,
            &sample_evidence("global-cap-epoch", 1, now_ms().unwrap()),
        )
        .unwrap();
        let connection = Connection::open(&path).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM executable_market_mark_evidence",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(count <= MAX_MARK_EVIDENCE_RECORDS as i64);
        drop(connection);
        remove_test_database(&path);
    }

    #[test]
    fn sampled_mark_evidence_expires_rows_older_than_the_retention_window() {
        let path = test_database_path("age-retention");
        let now = now_ms().unwrap();
        let connection = mark_evidence_db(&path).unwrap();
        connection
            .execute(
                "INSERT INTO executable_market_mark_evidence
             (symbol, quote_timestamp_ms, bid, ask, source_epoch, source_cursor, source_record_id)
             VALUES (?1, ?2, 99.0, 101.0, ?3, 1, ?4)",
                params![
                    "BTC-USDT",
                    now.saturating_sub(TICKER_HISTORY_RETENTION.as_millis() as u64 + 1) as i64,
                    "age-retention-epoch",
                    "paper-book:v1:age-retention-epoch:BTC-USDT:1"
                ],
            )
            .unwrap();
        drop(connection);
        let recent = sample_evidence("age-retention-epoch", 2, now);
        persist_mark_evidence_sync(&path, &recent).unwrap();

        let connection = Connection::open(&path).unwrap();
        let old_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM executable_market_mark_evidence WHERE source_epoch = ?1 AND source_cursor = 1",
                params!["age-retention-epoch"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            old_count, 0,
            "rows beyond quote-time retention must be removed"
        );
        drop(connection);
        remove_test_database(&path);
    }

    #[tokio::test]
    async fn sampled_mark_evidence_pages_are_bounded_and_use_durable_cursors() {
        let path = test_database_path("paging");
        let timestamp = now_ms().unwrap();
        for cursor in 1..=3 {
            persist_mark_evidence_sync(
                &path,
                &sample_evidence("paging-epoch", cursor, timestamp + cursor),
            )
            .unwrap();
        }
        let cutoff = mark_evidence_retention_cutoff().unwrap();
        let (first, has_more) = page_mark_evidence(path.clone(), "BTC-USDT".into(), 0, 2, cutoff)
            .await
            .unwrap();
        assert!(has_more);
        assert_eq!(first.len(), 2);
        assert!(first[0].evidence_cursor < first[1].evidence_cursor);

        let (second, has_more) = page_mark_evidence(
            path.clone(),
            "BTC-USDT".into(),
            first.last().unwrap().evidence_cursor,
            2,
            cutoff,
        )
        .await
        .unwrap();
        assert!(!has_more);
        assert_eq!(second.len(), 1);
        assert!(second[0].evidence_cursor > first[1].evidence_cursor);
        remove_test_database(&path);
    }

    #[test]
    fn mark_evidence_requires_complete_source_identity_before_persistence() {
        let book = BookSnapshot {
            bids: vec![BookLevel {
                price: 99.0,
                size: 1.0,
            }],
            asks: vec![BookLevel {
                price: 101.0,
                size: 1.0,
            }],
            updated_at_ms: 1_700_000_000_000,
            source_epoch: None,
            source_cursor: Some(1),
            source_record_id: Some("record-1".into()),
        };
        let sample = book_history_sample(&book).unwrap();
        assert!(sample.into_evidence("BTC-USDT", &book).is_none());
    }

    #[tokio::test]
    async fn missing_mark_source_identity_fails_closed_as_not_found() {
        let path = test_database_path("missing-source");
        persist_mark_evidence_sync(
            &path,
            &sample_evidence("present-epoch", 9, now_ms().unwrap()),
        )
        .unwrap();
        let mut data = empty_data();
        data.mark_evidence_path = path.clone();
        let result = get_market_mark_evidence(
            State(Arc::new(data)),
            Query(MarketMarkEvidenceQuery {
                symbol: "BTC-USDT".into(),
                source_epoch: Some("absent-epoch".into()),
                source_cursor: Some(9),
                source_record_id: Some("paper-book:v1:absent-epoch:BTC-USDT:9".into()),
                after_cursor: None,
                limit: None,
            }),
        )
        .await;
        match result {
            Err((status, _)) => assert_eq!(status, StatusCode::NOT_FOUND),
            Ok(_) => panic!("missing market mark source identity must fail closed"),
        }
        remove_test_database(&path);
    }
}
