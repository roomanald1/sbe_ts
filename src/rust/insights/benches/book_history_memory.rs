use std::{hint::black_box, mem::size_of, time::Instant};

use insights::signals::SignalEngine;

fn rss_mib() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            let value = line.strip_prefix("VmRSS:")?.split_whitespace().next()?;
            value.parse::<u64>().ok().map(|kilobytes| kilobytes / 1024)
        })
}

// Mirrors the layout stored for each historical book event before sampling.
struct BookHistoryRow {
    _timestamp: u64,
    _bid: f64,
    _ask: f64,
    _bid_levels: [(f64, f64); 7],
    _ask_levels: [(f64, f64); 7],
}

fn main() {
    let symbols = std::env::var("BOOK_MEMORY_SYMBOLS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(100);
    let updates_per_second = std::env::var("BOOK_MEMORY_UPDATES_PER_SECOND")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(1_000);
    let window_seconds = std::env::var("BOOK_MEMORY_WINDOW_SECONDS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(60);
    let updates_per_symbol = updates_per_second * window_seconds;

    let bid_levels = [(99.99, 1.0); 7];
    let ask_levels = [(100.01, 1.0); 7];
    let sizes = [1.0; 7];
    let before = rss_mib();
    let started = Instant::now();
    let mut engines = (0..symbols)
        .map(|_| SignalEngine::default())
        .collect::<Vec<_>>();
    for engine in &mut engines {
        for update in 0..updates_per_symbol {
            engine.on_book(
                update as u64,
                99.99,
                100.01,
                &sizes,
                &sizes,
                &bid_levels,
                &ask_levels,
            );
        }
    }
    let after = rss_mib();
    let retained_rows = symbols * updates_per_symbol;
    let estimated_payload_mib = retained_rows * size_of::<BookHistoryRow>() / (1024 * 1024);
    println!(
        "book history reproduction: {symbols} symbols × {updates_per_second} updates/s × {window_seconds}s; {retained_rows} retained rows (~{estimated_payload_mib} MiB row payload); RSS {before:?} -> {after:?} MiB; elapsed {:.2}s",
        started.elapsed().as_secs_f64(),
    );
    black_box(engines);
}
