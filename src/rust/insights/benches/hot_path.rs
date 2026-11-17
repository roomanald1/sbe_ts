#[path = "common/mod.rs"]
mod support;

use std::hint::black_box;

use serde_json::{Value, json};

const SYMBOL_COUNT: usize = 100;
const BOOK_LEVEL_COUNT: usize = 7;
const EVENTS_PER_SNAPSHOT: usize = 14;

#[derive(Clone, Copy)]
struct CompactBook {
    timestamp: u64,
    best_bid: f64,
    best_ask: f64,
    bids: [(f64, f64); BOOK_LEVEL_COUNT],
    asks: [(f64, f64); BOOK_LEVEL_COUNT],
}

struct BookPathState {
    symbols: Vec<String>,
    json_cache: Vec<Value>,
    compact_cache: Vec<CompactBook>,
    updates_since_snapshot: Vec<usize>,
}

fn setup_book_path() -> BookPathState {
    let bids = std::array::from_fn(|level| (99.99 - level as f64 * 0.01, 1.0 + level as f64));
    let asks = std::array::from_fn(|level| (100.01 + level as f64 * 0.01, 1.5 + level as f64));
    BookPathState {
        symbols: (0..SYMBOL_COUNT)
            .map(|index| format!("SYMBOL-{index}"))
            .collect(),
        json_cache: vec![Value::Null; SYMBOL_COUNT],
        compact_cache: vec![
            CompactBook {
                timestamp: 0,
                best_bid: bids[0].0,
                best_ask: asks[0].0,
                bids,
                asks,
            };
            SYMBOL_COUNT
        ],
        updates_since_snapshot: vec![0; SYMBOL_COUNT],
    }
}

fn market_json(book: &CompactBook) -> Value {
    if !book.best_bid.is_finite()
        || !book.best_ask.is_finite()
        || book.best_bid <= 0.0
        || book.best_ask < book.best_bid
    {
        return Value::Null;
    }
    let bids = book
        .bids
        .iter()
        .filter(|(price, size)| {
            price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
        })
        .map(|(price, size)| json!({"price": price, "size": size}))
        .collect::<Vec<_>>();
    let asks = book
        .asks
        .iter()
        .filter(|(price, size)| {
            price.is_finite() && *price > 0.0 && size.is_finite() && *size > 0.0
        })
        .map(|(price, size)| json!({"price": price, "size": size}))
        .collect::<Vec<_>>();
    json!({
        "bids": bids,
        "asks": asks,
        "updated_at_ms": book.timestamp,
    })
}

fn old_json_per_book_event(state: &mut BookPathState, iterations: usize) {
    let mut book = state.compact_cache[0];
    for update in 0..iterations {
        let symbol_index = update % SYMBOL_COUNT;
        book.timestamp = update as u64;
        let value = market_json(&book);
        state.json_cache[symbol_index] = value.clone();
        let symbol = &state.symbols[symbol_index];
        black_box(json!({ (symbol.clone()): value }));
    }
}

fn compact_cache_and_snapshot_json(state: &mut BookPathState, iterations: usize) {
    let source = state.compact_cache[0];
    for update in 0..iterations {
        let symbol_index = update % SYMBOL_COUNT;
        let mut book = source;
        book.timestamp = update as u64;
        state.compact_cache[symbol_index] = book;
        let updates = &mut state.updates_since_snapshot[symbol_index];
        *updates += 1;
        if *updates == EVENTS_PER_SNAPSHOT {
            *updates = 0;
            black_box(market_json(&state.compact_cache[symbol_index]));
        }
    }
}

fn main() {
    const ITERATIONS: usize = 14_000;
    support::measure(
        "legacy nested JSON construction on every book update",
        ITERATIONS,
        setup_book_path,
        old_json_per_book_event,
    );
    support::measure(
        "compact book cache with JSON every 14 updates",
        ITERATIONS,
        setup_book_path,
        compact_cache_and_snapshot_json,
    );
}
