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
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
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
    active_symbols: DashMap<String, AtomicU64>,
    symbol_admission_gate: Mutex<()>,
    max_active_symbols: usize,
    symbol_idle_ttl_ms: u64,
    engines: DashMap<String, Arc<Mutex<SignalEngine>>>,
    research_record_sender: tokio::sync::mpsc::Sender<Vec<StoredSignalRecord>>,
    signal_push_cache: Arc<DashMap<String, serde_json::Value>>,
    scout_push_cache: Arc<DashMap<String, serde_json::Value>>,
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

// FeedHandlers currently selects the top 100 spot instruments. Leave a 2.5x
// margin for configured/temporary subscriptions while bounding per-symbol
// rolling state and its associated delivery caches.
const DEFAULT_MAX_ACTIVE_SYMBOLS: usize = 256;
const DEFAULT_SYMBOL_IDLE_TTL_MS: u64 = 60_000;
const NEWS_PENDING_MAX_COUNT: usize = 8_192;
const NEWS_PENDING_MAX_AGE_MS: u64 = 60_000;

fn configured_positive_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

fn configured_positive_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
}

#[cfg(test)]
fn admit_symbol(
    active: &mut HashMap<String, u64>,
    symbol: &str,
    now: u64,
    cap: usize,
    idle_ttl_ms: u64,
) -> Result<Vec<String>, ()> {
    if let Some(last_seen) = active.get_mut(symbol) {
        *last_seen = now;
        return Ok(Vec::new());
    }
    let stale = active
        .iter()
        .filter(|(_, last_seen)| now.saturating_sub(**last_seen) >= idle_ttl_ms)
        .map(|(symbol, _)| symbol.clone())
        .collect::<Vec<_>>();
    if active.len().saturating_sub(stale.len()) >= cap {
        return Err(());
    }
    for old_symbol in &stale {
        active.remove(old_symbol);
    }
    active.insert(symbol.to_owned(), now);
    Ok(stale)
}

static SCOUT_LEARNING_DELIVERY_LIMIT: OnceLock<Semaphore> = OnceLock::new();

#[derive(Default)]
struct NewsLearning {
    samples: usize,
    correct: usize,
    weight: f64,
    pending: HashMap<(String, u64), (f64, u64)>,
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

async fn admit_runtime_symbol(runtime: &SignalRuntime, symbol: &str) -> Result<(), ()> {
    let now = now_ms();
    if let Some(last_seen) = runtime.active_symbols.get(symbol) {
        last_seen.store(now, Ordering::Relaxed);
        return Ok(());
    }
    // Only first-seen symbols contend on this gate; existing feed symbols update
    // their activity timestamp directly without serializing unrelated symbols.
    let _gate = runtime.symbol_admission_gate.lock().await;
    if let Some(last_seen) = runtime.active_symbols.get(symbol) {
        last_seen.store(now, Ordering::Relaxed);
        return Ok(());
    }
    let stale = runtime
        .active_symbols
        .iter()
        .filter(|entry| {
            now.saturating_sub(entry.value().load(Ordering::Relaxed)) >= runtime.symbol_idle_ttl_ms
        })
        .map(|entry| entry.key().clone())
        .collect::<Vec<_>>();
    let stale = stale
        .into_iter()
        .filter(|old_symbol| {
            runtime
                .active_symbols
                .remove_if(old_symbol, |_, last_seen| {
                    let engine_is_busy = runtime
                        .engines
                        .get(old_symbol)
                        .is_some_and(|engine| engine.try_lock().is_err());
                    !engine_is_busy
                        && now.saturating_sub(last_seen.load(Ordering::Relaxed))
                            >= runtime.symbol_idle_ttl_ms
                })
                .is_some()
        })
        .collect::<Vec<_>>();
    for stale_symbol in stale {
        runtime.engines.remove(&stale_symbol);
        runtime.signal_push_cache.remove(&stale_symbol);
        runtime.scout_push_cache.remove(&stale_symbol);
        runtime.market_cache.remove(&stale_symbol);
        runtime.market_push_at.remove(&stale_symbol);
        runtime.ui_snapshot_cache.remove(&stale_symbol);
        runtime.snapshot_diagnostic_at.remove(&stale_symbol);
        runtime.latest_prices.remove(&stale_symbol);
        runtime.rolling_volatility.remove(&stale_symbol);
        let mut learning = runtime.news_learning.lock().await;
        learning
            .pending
            .retain(|(pending_symbol, _), _| pending_symbol != &stale_symbol);
    }
    if runtime.active_symbols.len() >= runtime.max_active_symbols {
        return Err(());
    }
    runtime
        .active_symbols
        .insert(symbol.to_owned(), AtomicU64::new(now));
    Ok(())
}

fn prune_news_pending(learning: &mut NewsLearning, now_ms: u64) {
    learning.pending.retain(|_, (_, inserted_at_ms)| {
        now_ms.saturating_sub(*inserted_at_ms) <= NEWS_PENDING_MAX_AGE_MS
    });
    while learning.pending.len() > NEWS_PENDING_MAX_COUNT {
        let Some(oldest) = learning
            .pending
            .iter()
            .min_by_key(|(_, (_, inserted_at_ms))| *inserted_at_ms)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        learning.pending.remove(&oldest);
    }
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

#[cfg(test)]
mod active_symbol_tests {
    use super::*;

    #[test]
    fn default_symbol_and_news_state_caps_cover_feed_universe_with_bounded_headroom() {
        assert_eq!(DEFAULT_MAX_ACTIVE_SYMBOLS, 256);
        assert_eq!(NEWS_PENDING_MAX_COUNT, 8_192);
    }

    #[test]
    fn rejects_new_symbol_at_capacity_without_evicting_active_symbols() {
        let mut active = HashMap::from([("BTC-USDT".to_owned(), 95), ("ETH-USDT".to_owned(), 99)]);
        assert!(admit_symbol(&mut active, "SOL-USDT", 100, 2, 10).is_err());
        assert_eq!(active.len(), 2);
        assert_eq!(active.get("BTC-USDT"), Some(&95));
        assert_eq!(active.get("ETH-USDT"), Some(&99));
    }

    #[test]
    fn evicts_only_symbols_idle_for_the_configured_ttl() {
        let mut active = HashMap::from([("BTC-USDT".to_owned(), 80), ("ETH-USDT".to_owned(), 95)]);
        let evicted = admit_symbol(&mut active, "SOL-USDT", 100, 2, 10).unwrap();
        assert_eq!(evicted, vec!["BTC-USDT".to_owned()]);
        assert!(!active.contains_key("BTC-USDT"));
        assert_eq!(active.get("ETH-USDT"), Some(&95));
        assert_eq!(active.get("SOL-USDT"), Some(&100));
        assert_eq!(active.len(), 2);
    }

    #[test]
    fn refreshes_existing_symbol_and_news_pending_is_age_bounded() {
        let mut active = HashMap::from([("BTC-USDT".to_owned(), 1)]);
        assert!(
            admit_symbol(&mut active, "BTC-USDT", 100, 1, 10)
                .unwrap()
                .is_empty()
        );
        assert_eq!(active.get("BTC-USDT"), Some(&100));

        let mut learning = NewsLearning::default();
        learning.pending.insert(("BTC-USDT".into(), 1), (0.5, 1));
        learning
            .pending
            .insert(("ETH-USDT".into(), 99_000), (0.5, 99_000));
        prune_news_pending(&mut learning, 100_000);
        assert_eq!(learning.pending.len(), 1);
        assert!(learning.pending.contains_key(&("ETH-USDT".into(), 99_000)));
    }

    #[test]
    fn news_pending_count_is_bounded() {
        let mut learning = NewsLearning::default();
        for timestamp in 0..=NEWS_PENDING_MAX_COUNT as u64 {
            learning
                .pending
                .insert(("BTC-USDT".into(), timestamp), (0.5, 100_000));
        }
        prune_news_pending(&mut learning, 100_000);
        assert_eq!(learning.pending.len(), NEWS_PENDING_MAX_COUNT);
    }
}
