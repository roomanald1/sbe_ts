use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use dashmap::DashMap;

static TRADES: AtomicU64 = AtomicU64::new(0);
static BOOKS: AtomicU64 = AtomicU64::new(0);
static TICKERS: AtomicU64 = AtomicU64::new(0);
static QUEUES: OnceLock<Mutex<BTreeMap<String, (u64, u64)>>> = OnceLock::new();
static SYMBOL_PROVENANCE: OnceLock<DashMap<String, Arc<SymbolProvenance>>> = OnceLock::new();
static SYMBOL_PROVENANCE_OVERFLOW: AtomicU64 = AtomicU64::new(0);
static PROCESS_STARTED_AT_MS: OnceLock<u64> = OnceLock::new();
const MAX_SYMBOL_PROVENANCE_ROWS: usize = 256;

pub fn market_event_timestamp(bytes: &[u8]) -> Option<(&'static str, u64)> {
    let template_id = u16::from_le_bytes(bytes.get(2..4)?.try_into().ok()?);
    let (kind, offset) = match template_id {
        1 => ("price", 48),
        2 => ("book", 40),
        3 => ("trade", 72),
        _ => return None,
    };
    Some((
        kind,
        u64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?),
    ))
}

#[derive(Default)]
struct EventProvenance {
    source_count: AtomicU64,
    source_last_timestamp_ms: AtomicU64,
    source_max_timestamp_ms: AtomicU64,
    source_last_received_at_ms: AtomicU64,
    source_regressions: AtomicU64,
    source_equal_timestamps: AtomicU64,
    per_symbol_queue_count: AtomicU64,
    per_symbol_queue_last_timestamp_ms: AtomicU64,
    per_symbol_queue_last_at_ms: AtomicU64,
    per_symbol_queue_failures: AtomicU64,
    per_symbol_queue_timestamp_regressions: AtomicU64,
    aggregate_attempt_count: AtomicU64,
    aggregate_published_count: AtomicU64,
    aggregate_last_timestamp_ms: AtomicU64,
    aggregate_last_at_ms: AtomicU64,
    aggregate_failures: AtomicU64,
    aggregate_timestamp_regressions: AtomicU64,
}

#[derive(Default)]
struct SymbolProvenance {
    price: EventProvenance,
    book: EventProvenance,
    trade: EventProvenance,
}

fn metrics_started_at_ms() -> u64 {
    *PROCESS_STARTED_AT_MS.get_or_init(now_ms)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn symbol_provenance(symbol: &str) -> Option<Arc<SymbolProvenance>> {
    metrics_started_at_ms();
    let rows = SYMBOL_PROVENANCE.get_or_init(DashMap::new);
    if let Some(existing) = rows.get(symbol) {
        return Some(Arc::clone(existing.value()));
    }
    if rows.len() >= MAX_SYMBOL_PROVENANCE_ROWS {
        SYMBOL_PROVENANCE_OVERFLOW.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    Some(Arc::clone(
        rows.entry(symbol.to_owned())
            .or_insert_with(|| Arc::new(SymbolProvenance::default()))
            .value(),
    ))
}

fn event_provenance<'a>(row: &'a SymbolProvenance, kind: &str) -> Option<&'a EventProvenance> {
    match kind {
        "price" => Some(&row.price),
        "book" => Some(&row.book),
        "trade" => Some(&row.trade),
        _ => None,
    }
}

pub fn record_symbol_source_event(symbol: &str, kind: &str, timestamp_ms: u64) {
    let Some(row) = symbol_provenance(symbol) else {
        return;
    };
    let Some(event) = event_provenance(&row, kind) else {
        return;
    };
    event.source_count.fetch_add(1, Ordering::Relaxed);
    event
        .source_last_timestamp_ms
        .swap(timestamp_ms, Ordering::Relaxed);
    let previous_max = event
        .source_max_timestamp_ms
        .fetch_max(timestamp_ms, Ordering::Relaxed);
    if previous_max > 0 && timestamp_ms < previous_max {
        event.source_regressions.fetch_add(1, Ordering::Relaxed);
    } else if previous_max > 0 && timestamp_ms == previous_max {
        event
            .source_equal_timestamps
            .fetch_add(1, Ordering::Relaxed);
    }
    event
        .source_last_received_at_ms
        .store(now_ms(), Ordering::Relaxed);
}

pub fn record_symbol_per_symbol_queue(
    symbol: &str,
    kind: &str,
    timestamp_ms: u64,
    delivered: bool,
) {
    let Some(row) = symbol_provenance(symbol) else {
        return;
    };
    let Some(event) = event_provenance(&row, kind) else {
        return;
    };
    event.per_symbol_queue_count.fetch_add(1, Ordering::Relaxed);
    let previous_timestamp = event
        .per_symbol_queue_last_timestamp_ms
        .swap(timestamp_ms, Ordering::Relaxed);
    if previous_timestamp > 0 && timestamp_ms < previous_timestamp {
        event
            .per_symbol_queue_timestamp_regressions
            .fetch_add(1, Ordering::Relaxed);
    }
    event
        .per_symbol_queue_last_at_ms
        .store(now_ms(), Ordering::Relaxed);
    if !delivered {
        event
            .per_symbol_queue_failures
            .fetch_add(1, Ordering::Relaxed);
    }
}

pub fn record_symbol_aggregate_event(symbol: &str, kind: &str, timestamp_ms: u64, delivered: bool) {
    let Some(row) = symbol_provenance(symbol) else {
        return;
    };
    let Some(event) = event_provenance(&row, kind) else {
        return;
    };
    event
        .aggregate_attempt_count
        .fetch_add(1, Ordering::Relaxed);
    let previous_timestamp = event
        .aggregate_last_timestamp_ms
        .swap(timestamp_ms, Ordering::Relaxed);
    if previous_timestamp > 0 && timestamp_ms < previous_timestamp {
        event
            .aggregate_timestamp_regressions
            .fetch_add(1, Ordering::Relaxed);
    }
    event
        .aggregate_last_at_ms
        .store(now_ms(), Ordering::Relaxed);
    if delivered {
        event
            .aggregate_published_count
            .fetch_add(1, Ordering::Relaxed);
    } else {
        event.aggregate_failures.fetch_add(1, Ordering::Relaxed);
    }
}

fn event_provenance_json(event: &EventProvenance) -> serde_json::Value {
    let last_source = event.source_last_timestamp_ms.load(Ordering::Relaxed);
    let last_source_received = event.source_last_received_at_ms.load(Ordering::Relaxed);
    let last_queue_timestamp = event
        .per_symbol_queue_last_timestamp_ms
        .load(Ordering::Relaxed);
    let last_queue_at = event.per_symbol_queue_last_at_ms.load(Ordering::Relaxed);
    let last_aggregate_timestamp = event.aggregate_last_timestamp_ms.load(Ordering::Relaxed);
    let last_aggregate_at = event.aggregate_last_at_ms.load(Ordering::Relaxed);
    serde_json::json!({
        "source_count": event.source_count.load(Ordering::Relaxed),
        "source_last_timestamp_ms": (last_source > 0).then_some(last_source),
        "source_max_timestamp_ms": (event.source_max_timestamp_ms.load(Ordering::Relaxed) > 0)
            .then(|| event.source_max_timestamp_ms.load(Ordering::Relaxed)),
        "source_last_received_at_ms": (last_source_received > 0).then_some(last_source_received),
        "source_timestamp_regressions": event.source_regressions.load(Ordering::Relaxed),
        "source_equal_timestamps": event.source_equal_timestamps.load(Ordering::Relaxed),
        "per_symbol_queue_count": event.per_symbol_queue_count.load(Ordering::Relaxed),
        "per_symbol_queue_last_timestamp_ms": (last_queue_timestamp > 0).then_some(last_queue_timestamp),
        "per_symbol_queue_last_at_ms": (last_queue_at > 0).then_some(last_queue_at),
        "per_symbol_queue_failures": event.per_symbol_queue_failures.load(Ordering::Relaxed),
        "per_symbol_queue_timestamp_regressions": event.per_symbol_queue_timestamp_regressions.load(Ordering::Relaxed),
        "aggregate_attempt_count": event.aggregate_attempt_count.load(Ordering::Relaxed),
        "aggregate_published_count": event.aggregate_published_count.load(Ordering::Relaxed),
        "aggregate_last_timestamp_ms": (last_aggregate_timestamp > 0).then_some(last_aggregate_timestamp),
        "aggregate_last_at_ms": (last_aggregate_at > 0).then_some(last_aggregate_at),
        "aggregate_failures": event.aggregate_failures.load(Ordering::Relaxed),
        "aggregate_timestamp_regressions": event.aggregate_timestamp_regressions.load(Ordering::Relaxed),
    })
}

pub fn symbol_provenance_snapshot() -> serde_json::Value {
    let rows = SYMBOL_PROVENANCE.get_or_init(DashMap::new);
    let mut symbols = rows
        .iter()
        .map(|entry| {
            let row = entry.value();
            serde_json::json!({
                "symbol": entry.key(),
                "price": event_provenance_json(&row.price),
                "book": event_provenance_json(&row.book),
                "trade": event_provenance_json(&row.trade),
            })
        })
        .collect::<Vec<_>>();
    symbols.sort_by(|left, right| left["symbol"].as_str().cmp(&right["symbol"].as_str()));
    serde_json::json!({
        "schema_version": 1,
        "process_started_at_ms": metrics_started_at_ms(),
        "symbol_limit": MAX_SYMBOL_PROVENANCE_ROWS,
        "symbols_retained": symbols.len(),
        "symbol_rows_dropped_at_limit": SYMBOL_PROVENANCE_OVERFLOW.load(Ordering::Relaxed),
        "symbols": symbols,
    })
}

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

#[cfg(test)]
mod symbol_provenance_tests {
    use super::*;

    #[test]
    fn per_symbol_provenance_distinguishes_source_regressions_queue_and_aggregate_delivery() {
        let symbol = format!("PROVENANCE-{}", std::process::id());
        record_symbol_source_event(&symbol, "book", 20_000);
        record_symbol_source_event(&symbol, "book", 19_000);
        record_symbol_source_event(&symbol, "book", 20_000);
        record_symbol_per_symbol_queue(&symbol, "book", 20_000, false);
        record_symbol_aggregate_event(&symbol, "book", 20_000, true);

        let snapshot = symbol_provenance_snapshot();
        let row = snapshot["symbols"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["symbol"] == symbol)
            .unwrap();
        assert_eq!(row["book"]["source_count"], 3);
        assert_eq!(row["book"]["source_timestamp_regressions"], 1);
        assert_eq!(row["book"]["source_equal_timestamps"], 1);
        assert_eq!(row["book"]["source_max_timestamp_ms"], 20_000);
        assert_eq!(row["book"]["per_symbol_queue_count"], 1);
        assert_eq!(row["book"]["per_symbol_queue_failures"], 1);
        assert_eq!(row["book"]["aggregate_attempt_count"], 1);
        assert_eq!(row["book"]["aggregate_published_count"], 1);
    }

    #[test]
    fn sbe_event_timestamp_decoder_uses_each_market_message_layout() {
        for (template_id, offset, kind) in [(1_u16, 48, "price"), (2, 40, "book"), (3, 72, "trade")]
        {
            let mut bytes = vec![0; offset + 8];
            bytes[2..4].copy_from_slice(&template_id.to_le_bytes());
            bytes[offset..offset + 8].copy_from_slice(&123_456_u64.to_le_bytes());
            assert_eq!(market_event_timestamp(&bytes), Some((kind, 123_456)));
        }
        assert_eq!(market_event_timestamp(&[0; 16]), None);
    }
}
