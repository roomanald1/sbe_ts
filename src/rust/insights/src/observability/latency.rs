use serde_json::{Value, json};
use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

const BUCKETS_US: [u64; 10] = [
    50, 100, 250, 500, 1_000, 2_000, 5_000, 10_000, 50_000, 100_000,
];

struct Histogram {
    buckets: [AtomicU64; 11],
    count: AtomicU64,
}

impl Histogram {
    const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; 11],
            count: AtomicU64::new(0),
        }
    }

    fn record(&self, duration: Duration) {
        let micros = duration.as_micros().min(u64::MAX as u128) as u64;
        let bucket = BUCKETS_US
            .iter()
            .position(|limit| micros <= *limit)
            .unwrap_or(BUCKETS_US.len());
        self.buckets[bucket].fetch_add(1, Ordering::Relaxed);
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
        json!({
            "count": count,
            "p50_us": self.percentile(50, count),
            "p95_us": self.percentile(95, count),
        })
    }
}

static EVENT_QUEUE_WAIT: Histogram = Histogram::new();
static ENGINE_LOCK_WAIT: Histogram = Histogram::new();
static SNAPSHOT_COMPUTE: Histogram = Histogram::new();
static AUTO_TRADER_DELIVERY: Histogram = Histogram::new();
static DROPPED_RESEARCH_RECORDS: AtomicU64 = AtomicU64::new(0);

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

pub fn record_auto_trader_delivery(duration: Duration) {
    AUTO_TRADER_DELIVERY.record(duration);
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
        "auto_trader_delivery": AUTO_TRADER_DELIVERY.snapshot(),
        "dropped_research_records": DROPPED_RESEARCH_RECORDS.load(Ordering::Relaxed),
    })
}
