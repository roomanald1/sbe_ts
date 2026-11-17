use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

static TRADES: AtomicU64 = AtomicU64::new(0);
static BOOKS: AtomicU64 = AtomicU64::new(0);
static TICKERS: AtomicU64 = AtomicU64::new(0);
static QUEUES: OnceLock<Mutex<BTreeMap<String, (u64, u64)>>> = OnceLock::new();

fn queues() -> &'static Mutex<BTreeMap<String, (u64, u64)>> {
    QUEUES.get_or_init(|| Mutex::new(BTreeMap::new()))
}

pub fn record_queue_depth(queue: &str, depth: usize) {
    let mut queues = queues().lock().unwrap_or_else(|error| error.into_inner());
    queues.entry(queue.to_owned()).or_default().0 = depth as u64;
}

pub fn record_queue_lagged(queue: &str, dropped: u64) {
    let mut queues = queues().lock().unwrap_or_else(|error| error.into_inner());
    let metrics = queues.entry(queue.to_owned()).or_default();
    metrics.1 = metrics.1.saturating_add(dropped);
}

pub fn queue_snapshot() -> Vec<(String, u64, u64)> {
    queues()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .iter()
        .map(|(name, (depth, dropped))| (name.clone(), *depth, *dropped))
        .collect()
}

pub fn queue_capacity(queue: &str) -> Option<u64> {
    let capacity = match queue {
        "price_updates" => 4_096,
        "ticker_batches" => 256,
        "signal_events" => 65_536,
        "signal_feeds" => 64,
        _ if queue.starts_with("signal_prices/") => 64,
        _ if queue.starts_with("shared_trades/") => 512,
        _ if queue.starts_with("shared_books/") => 32,
        _ => return None,
    };
    Some(capacity)
}

pub fn record_trade() {
    TRADES.fetch_add(1, Ordering::Relaxed);
}
pub fn record_book() {
    BOOKS.fetch_add(1, Ordering::Relaxed);
}
pub fn record_ticker() {
    TICKERS.fetch_add(1, Ordering::Relaxed);
}

pub fn snapshot() -> (u64, u64, u64) {
    (
        TRADES.load(Ordering::Relaxed),
        BOOKS.load(Ordering::Relaxed),
        TICKERS.load(Ordering::Relaxed),
    )
}
