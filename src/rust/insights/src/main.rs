#[path = "network/binance_client.rs"]
mod binance_client;
#[path = "network/feed_client.rs"]
mod feed_client;
#[path = "observability/latency.rs"]
mod latency;
#[path = "observability/logging.rs"]
mod logging;
#[path = "feeds/rolling_volatility.rs"]
mod rolling_volatility;
use anyhow::Result;
use axum::{Json, Router, http::StatusCode, routing::get};
use bytes::Bytes;
use dashmap::DashMap;
use futures_util::StreamExt;
use insights::{
    daily_summary,
    signals::{
        ExternalMarketFeatures, LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS, NewsAttribution,
        NewsHeadlineAttribution, SignalEngine, SignalEvaluation, SignalSnapshot, SignalStore,
        StoredSignalRecord,
    },
};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    pin::Pin,
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, Semaphore, broadcast, watch},
    time::{Duration, Instant},
};

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalFeedEvent {
    Price {
        timestamp: u64,
        price: f64,
    },
    Trade {
        timestamp: u64,
        price: f64,
        size: f64,
        is_buy: bool,
    },
    Book {
        timestamp: u64,
        best_bid: f64,
        best_ask: f64,
        bid_sizes: [f64; 7],
        ask_sizes: [f64; 7],
        #[serde(default)]
        bid_levels: [(f64, f64); 7],
        #[serde(default)]
        ask_levels: [(f64, f64); 7],
    },
}
#[derive(Deserialize)]
struct SignalEventRequest {
    symbol: String,
    #[serde(skip)]
    queued_at: Option<Instant>,
    #[serde(skip)]
    raw_payload: Option<Bytes>,
    #[serde(flatten)]
    event: SignalFeedEvent,
}
struct SignalRuntime {
    store: SignalStore,
    engines: DashMap<String, Arc<Mutex<SignalEngine>>>,
    research_record_sender: tokio::sync::mpsc::Sender<Vec<StoredSignalRecord>>,
    signal_push_cache: Arc<DashMap<String, serde_json::Value>>,
    signal_push_notify: Arc<Notify>,
    market_cache: Arc<DashMap<String, (u64, serde_json::Value)>>,
    market_push_at: DashMap<String, u64>,
    ui_snapshot_cache: Arc<DashMap<String, serde_json::Value>>,
    ui_snapshot_notify: Arc<Notify>,
    news: watch::Receiver<Arc<NewsFeedSnapshot>>,
    news_learning: Mutex<NewsLearning>,
    snapshot_diagnostic_at: DashMap<String, u64>,
    latest_prices: DashMap<String, (u64, f64)>,
    rolling_volatility: DashMap<String, rolling_volatility::RollingVolatility>,
    enriched_market_events: broadcast::Sender<Bytes>,
}

static SCOUT_LEARNING_DELIVERY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

#[derive(Default)]
struct NewsLearning {
    samples: usize,
    correct: usize,
    weight: f64,
    pending: HashMap<(String, u64), f64>,
}

#[derive(Clone, Serialize)]
struct NewsArticle {
    title: String,
    article_text: String,
    url: String,
    published_at: String,
    received_at_ms: u64,
    matched_asset: String,
    direction: String,
    score: f64,
    signal_reason: String,
    sentiment_model: String,
    sentiment_confidence: f64,
    matched_keywords: Vec<String>,
}

#[derive(Clone, Default, Serialize)]
struct NewsFeedSnapshot {
    updated_at_ms: u64,
    source: String,
    stale: bool,
    error: Option<String>,
    articles: Vec<NewsArticle>,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn research_only() -> bool {
    static RESEARCH_ONLY: OnceLock<bool> = OnceLock::new();
    *RESEARCH_ONLY.get_or_init(|| {
        std::env::var("INSIGHTS_RESEARCH_ONLY")
            .is_ok_and(|value| matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes"))
    })
}

fn signal_diagnostic_due(runtime: &SignalRuntime, symbol: &str, timestamp: u64) -> bool {
    let mut last = runtime
        .snapshot_diagnostic_at
        .entry(symbol.to_owned())
        .or_insert(0);
    if timestamp >= *last && timestamp.saturating_sub(*last) < 60_000 {
        return false;
    }
    *last = timestamp;
    true
}

include!("feeds/news_classification.rs");

include!("feeds/external_market_feed.rs");
include!("feeds/dated_carry_signals.rs");
include!("feeds/daily_strategy_signals.rs");
include!("workers/signal_feed_workers.rs");

include!("api/signal_ingestion.rs");
include!("api/market_event_rsocket.rs");

include!("workers/service_startup.rs");

// TODO figure out if this project really is worth its own service. News classification has very little if any impact on trades.
// Daily Signals should probably just be inside live-signals crate
