use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const BUCKETS_US: [u64; 10] = [
    50, 100, 250, 500, 1_000, 2_000, 5_000, 10_000, 50_000, 100_000,
];

struct Histogram {
    buckets: [AtomicU64; 11],
    count: AtomicU64,
    total_micros: AtomicU64,
}

impl Histogram {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; 11],
            count: AtomicU64::new(0),
            total_micros: AtomicU64::new(0),
        }
    }

    fn record(&self, duration: Duration) {
        let micros = duration.as_micros().min(u64::MAX as u128) as u64;
        let bucket = BUCKETS_US
            .iter()
            .position(|limit| micros <= *limit)
            .unwrap_or(BUCKETS_US.len());
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
        self.total_micros.fetch_add(micros, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
    }

    fn percentile(&self, percentile: u64, count: u64) -> Option<u64> {
        if count == 0 {
            return None;
        }
        let target = count.saturating_mul(percentile).div_ceil(100).max(1);
        let mut seen = 0;
        for (index, bucket) in self.buckets.iter().enumerate() {
            seen += bucket.load(Ordering::Relaxed);
            if seen >= target {
                return Some(*BUCKETS_US.get(index).unwrap_or(&u64::MAX));
            }
        }
        Some(u64::MAX)
    }

    fn snapshot(&self) -> Value {
        let count = self.count.load(Ordering::Relaxed);
        let average_micros = (count > 0).then(|| self.total_micros.load(Ordering::Relaxed) / count);
        json!({
            "count": count,
            "p50_us": self.percentile(50, count),
            "p95_us": self.percentile(95, count),
            "average_us": average_micros,
        })
    }
}

static EVENT_QUEUE_WAIT: Histogram = Histogram::new();
static ENGINE_LOCK_WAIT: Histogram = Histogram::new();
static SNAPSHOT_COMPUTE: Histogram = Histogram::new();
static PRICE_EVENT_PROCESSING: Histogram = Histogram::new();
static TRADE_EVENT_PROCESSING: Histogram = Histogram::new();
static BOOK_EVENT_PROCESSING: Histogram = Histogram::new();
static PRICE_ENGINE_WORK: Histogram = Histogram::new();
static TRADE_ENGINE_WORK: Histogram = Histogram::new();
static BOOK_ENGINE_WORK: Histogram = Histogram::new();
static STRATEGY_UPDATE_ENCODING: Histogram = Histogram::new();
static UI_SNAPSHOT_PUSH: Histogram = Histogram::new();
static STRATEGY_UPDATE_RAW_BYTES: AtomicU64 = AtomicU64::new(0);
static STRATEGY_UPDATE_COMPRESSED_BYTES: AtomicU64 = AtomicU64::new(0);
static DROPPED_RESEARCH_RECORDS: AtomicU64 = AtomicU64::new(0);
static SIGNAL_FEED_CONNECTED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
static SIGNAL_FEED_LAST_EVENT_AT_MS: AtomicU64 = AtomicU64::new(0);
static SIGNAL_FEED_LAST_PRICE_AT_MS: AtomicU64 = AtomicU64::new(0);
static SIGNAL_FEED_PRICE_EVENTS: AtomicU64 = AtomicU64::new(0);
static SIGNAL_FEED_TRADE_EVENTS: AtomicU64 = AtomicU64::new(0);
static SIGNAL_FEED_BOOK_EVENTS: AtomicU64 = AtomicU64::new(0);

pub fn set_signal_feed_connected(connected: bool) {
    SIGNAL_FEED_CONNECTED.store(connected, Ordering::Relaxed);
}

pub fn record_signal_feed_event(kind: &str) {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    SIGNAL_FEED_LAST_EVENT_AT_MS.store(now_ms, Ordering::Relaxed);
    match kind {
        "price" => {
            SIGNAL_FEED_LAST_PRICE_AT_MS.store(now_ms, Ordering::Relaxed);
            SIGNAL_FEED_PRICE_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
        "trade" => {
            SIGNAL_FEED_TRADE_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
        "book" => {
            SIGNAL_FEED_BOOK_EVENTS.fetch_add(1, Ordering::Relaxed);
        }
        _ => {}
    }
}

pub fn signal_feed_status() -> Value {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let last_event_at_ms = SIGNAL_FEED_LAST_EVENT_AT_MS.load(Ordering::Relaxed);
    let last_price_at_ms = SIGNAL_FEED_LAST_PRICE_AT_MS.load(Ordering::Relaxed);
    json!({
        "connected": SIGNAL_FEED_CONNECTED.load(Ordering::Relaxed),
        "last_event_at_ms": (last_event_at_ms > 0).then_some(last_event_at_ms),
        "last_event_age_ms": (last_event_at_ms > 0).then(|| now_ms.saturating_sub(last_event_at_ms)),
        "last_price_at_ms": (last_price_at_ms > 0).then_some(last_price_at_ms),
        "last_price_age_ms": (last_price_at_ms > 0).then(|| now_ms.saturating_sub(last_price_at_ms)),
        "price_events": SIGNAL_FEED_PRICE_EVENTS.load(Ordering::Relaxed),
        "trade_events": SIGNAL_FEED_TRADE_EVENTS.load(Ordering::Relaxed),
        "book_events": SIGNAL_FEED_BOOK_EVENTS.load(Ordering::Relaxed),
    })
}

pub fn record_event_queue_wait(duration: Duration) {
    EVENT_QUEUE_WAIT.record(duration);
}

#[allow(dead_code)]
pub fn record_engine_lock_wait(duration: Duration) {
    ENGINE_LOCK_WAIT.record(duration);
}

pub fn record_snapshot_compute(duration: Duration) {
    SNAPSHOT_COMPUTE.record(duration);
}

pub fn record_event_processing(kind: &str, duration: Duration) {
    match kind {
        "price" => PRICE_EVENT_PROCESSING.record(duration),
        "trade" => TRADE_EVENT_PROCESSING.record(duration),
        "book" => BOOK_EVENT_PROCESSING.record(duration),
        _ => {}
    }
}

pub fn record_engine_work(kind: &str, duration: Duration) {
    match kind {
        "price" => PRICE_ENGINE_WORK.record(duration),
        "trade" => TRADE_ENGINE_WORK.record(duration),
        "book" => BOOK_ENGINE_WORK.record(duration),
        _ => {}
    }
}

pub fn record_strategy_update_encoding(
    duration: Duration,
    raw_bytes: usize,
    compressed_bytes: usize,
) {
    STRATEGY_UPDATE_ENCODING.record(duration);
    STRATEGY_UPDATE_RAW_BYTES.fetch_add(raw_bytes as u64, Ordering::Relaxed);
    STRATEGY_UPDATE_COMPRESSED_BYTES.fetch_add(compressed_bytes as u64, Ordering::Relaxed);
}

pub fn record_ui_snapshot_push(duration: Duration) {
    UI_SNAPSHOT_PUSH.record(duration);
}

#[allow(dead_code)]
pub fn record_dropped_research_records(count: usize) {
    DROPPED_RESEARCH_RECORDS.fetch_add(count as u64, Ordering::Relaxed);
}

pub fn snapshot() -> Value {
    json!({
        "event_queue_wait": EVENT_QUEUE_WAIT.snapshot(),
        "engine_lock_wait": ENGINE_LOCK_WAIT.snapshot(),
        "snapshot_compute": SNAPSHOT_COMPUTE.snapshot(),
        "event_processing": {
            "price": PRICE_EVENT_PROCESSING.snapshot(),
            "trade": TRADE_EVENT_PROCESSING.snapshot(),
            "book": BOOK_EVENT_PROCESSING.snapshot(),
        },
        "engine_work": {
            "price": PRICE_ENGINE_WORK.snapshot(),
            "trade": TRADE_ENGINE_WORK.snapshot(),
            "book": BOOK_ENGINE_WORK.snapshot(),
        },
        "strategy_update_encoding": STRATEGY_UPDATE_ENCODING.snapshot(),
        "strategy_update_raw_bytes": STRATEGY_UPDATE_RAW_BYTES.load(Ordering::Relaxed),
        "strategy_update_compressed_bytes": STRATEGY_UPDATE_COMPRESSED_BYTES.load(Ordering::Relaxed),
        "ui_snapshot_push": UI_SNAPSHOT_PUSH.snapshot(),
        "dropped_research_records": DROPPED_RESEARCH_RECORDS.load(Ordering::Relaxed),
    })
}
