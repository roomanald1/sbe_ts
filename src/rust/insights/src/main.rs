mod daily_summary;
mod feed_client;
mod logging;
use axum::{Json, Router, http::StatusCode, routing::get};
use dashmap::DashMap;
use futures_util::StreamExt;
use insights::signals::{
    ExternalMarketFeatures, NewsAttribution, NewsHeadlineAttribution, SignalEngine,
    SignalEvaluation, SignalSnapshot, SignalStore, StoredSignalRecord,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{sync::{watch, Mutex, Notify}, time::{Duration, Instant}};

#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalFeedEvent {
    Price {
        timestamp: u64,
        price: f64,
        volatility_1m: f64,
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
    #[serde(flatten)]
    event: SignalFeedEvent,
}
struct SignalRuntime {
    store: SignalStore,
    engines: Mutex<HashMap<String, SignalEngine>>,
    signal_push_cache: Arc<DashMap<String, serde_json::Value>>,
    signal_push_notify: Arc<Notify>,
    ui_snapshot_cache: Arc<DashMap<String, serde_json::Value>>,
    ui_snapshot_notify: Arc<Notify>,
    news: watch::Receiver<Arc<NewsFeedSnapshot>>,
    news_learning: Mutex<NewsLearning>,
    snapshot_diagnostic_at: DashMap<String, u64>,
}

static POSITION_MANAGEMENT_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

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

fn signal_diagnostic_due(runtime: &SignalRuntime, symbol: &str, timestamp: u64) -> bool {
    let mut last = runtime.snapshot_diagnostic_at.entry(symbol.to_owned()).or_insert(0);
    if timestamp >= *last && timestamp.saturating_sub(*last) < 60_000 {
        return false;
    }
    *last = timestamp;
    true
}


include!("news_classification.rs");

include!("external_market_feed.rs");
include!("dated_carry_signals.rs");
include!("daily_strategy_signals.rs");

include!("signal_feed_workers.rs");

include!("signal_ingestion.rs");


include!("service_startup.rs");
