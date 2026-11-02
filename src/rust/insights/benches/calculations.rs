use std::{hint::black_box, time::Instant};

use insights::signals::SignalEngine;

fn seeded_engine() -> SignalEngine {
    let mut engine = SignalEngine::default();
    let bid_levels: Vec<_> = (0..7)
        .map(|level| (99.99 - level as f64 * 0.01, 1.0 + level as f64))
        .collect();
    let ask_levels: Vec<_> = (0..7)
        .map(|level| (100.01 + level as f64 * 0.01, 1.5 + level as f64))
        .collect();
    let bid_sizes: Vec<_> = bid_levels.iter().map(|(_, size)| *size).collect();
    let ask_sizes: Vec<_> = ask_levels.iter().map(|(_, size)| *size).collect();
    for index in 0..360 {
        let timestamp = index * 1_000;
        let price = 100.0 + (index as f64 * 0.07).sin();
        engine.on_price(timestamp, price, 0.01);
        engine.on_book(
            timestamp,
            99.99,
            100.01,
            &bid_sizes,
            &ask_sizes,
            &bid_levels,
            &ask_levels,
        );
        engine.on_trade(timestamp, price, 0.25, index % 2 == 0);
    }
    engine
}

fn report(name: &str, started: Instant, iterations: usize) {
    println!(
        "{name}: {:.1} ns/calculation",
        started.elapsed().as_nanos() as f64 / iterations as f64
    );
}

fn snapshot_calculation_benchmark() {
    const ITERATIONS: usize = 100_000;
    let mut engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.snapshot_with_reasons("BTC-USDT", black_box(360_000), false));
    }
    report("live insight snapshot", started, ITERATIONS);
}

fn snapshot_with_explanations_benchmark() {
    const ITERATIONS: usize = 100_000;
    let mut engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.snapshot_with_reasons("BTC-USDT", black_box(360_000), true));
    }
    report("live snapshot including explanations", started, ITERATIONS);
}

fn research_feature_calculation_benchmark() {
    const ITERATIONS: usize = 20_000;
    let engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.research_features(
            "BTC-USDT",
            black_box(359_000),
            (Some(0.2), Some(0.5)),
            (Some(-0.1), Some(0.3)),
        ));
    }
    report("research feature extraction", started, ITERATIONS);
}

fn price_return_calculation_benchmark() {
    const ITERATIONS: usize = 100_000;
    let engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.return_context(black_box(359_000)));
    }
    report("price return context", started, ITERATIONS);
}

fn input_diagnostic_calculation_benchmark() {
    const ITERATIONS: usize = 20_000;
    let engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.input_diagnostic(black_box(359_000)));
    }
    report("market input diagnostics", started, ITERATIONS);
}

fn book_snapshot_calculation_benchmark() {
    const ITERATIONS: usize = 100_000;
    let engine = seeded_engine();
    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(engine.book_snapshot("BTC-USDT", black_box(359_000)));
    }
    report("book snapshot", started, ITERATIONS);
}

fn main() {
    snapshot_calculation_benchmark();
    snapshot_with_explanations_benchmark();
    research_feature_calculation_benchmark();
    price_return_calculation_benchmark();
    input_diagnostic_calculation_benchmark();
    book_snapshot_calculation_benchmark();
}
