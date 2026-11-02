use std::sync::atomic::{AtomicU64, Ordering};

static TRADES: AtomicU64 = AtomicU64::new(0);
static BOOKS: AtomicU64 = AtomicU64::new(0);
static TICKERS: AtomicU64 = AtomicU64::new(0);

pub fn record_trade() { TRADES.fetch_add(1, Ordering::Relaxed); }
pub fn record_book() { BOOKS.fetch_add(1, Ordering::Relaxed); }
pub fn record_ticker() { TICKERS.fetch_add(1, Ordering::Relaxed); }

pub fn snapshot() -> (u64, u64, u64) {
    (TRADES.load(Ordering::Relaxed), BOOKS.load(Ordering::Relaxed), TICKERS.load(Ordering::Relaxed))
}
