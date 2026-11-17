#[path = "common/mod.rs"]
mod support;

use std::hint::black_box;

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

fn pending_scout_engine(tracked_scouts: usize) -> SignalEngine {
    let mut engine = SignalEngine::default();
    for index in 0..tracked_scouts {
        assert!(engine.track_scout(
            format!("pending-{index}"),
            "BTC-USDT".to_owned(),
            "benchmark".to_owned(),
            0,
            "bullish".to_owned(),
            100.0,
            0.5,
            1.0,
            0.5,
            0.25,
            0.1,
            None,
            None,
            None,
            None,
        ));
    }
    engine
}

fn main() {
    support::measure(
        "live insight snapshot",
        100_000,
        seeded_engine,
        |engine, count| {
            for _ in 0..count {
                black_box(engine.snapshot_with_reasons("BTC-USDT", black_box(360_000), false));
            }
        },
    );
    support::measure(
        "live snapshot including explanations",
        100_000,
        seeded_engine,
        |engine, count| {
            for _ in 0..count {
                black_box(engine.snapshot_with_reasons("BTC-USDT", black_box(360_000), true));
            }
        },
    );
    support::measure(
        "research feature extraction",
        20_000,
        seeded_engine,
        |engine, count| {
            for _ in 0..count {
                black_box(engine.research_features(
                    "BTC-USDT",
                    black_box(359_000),
                    (Some(0.2), Some(0.5)),
                    (Some(-0.1), Some(0.3)),
                ));
            }
        },
    );
    support::measure(
        "price return context",
        100_000,
        seeded_engine,
        |engine, count| {
            for _ in 0..count {
                black_box(engine.return_context(black_box(359_000)));
            }
        },
    );
    support::measure(
        "market input diagnostics",
        20_000,
        seeded_engine,
        |engine, count| {
            for _ in 0..count {
                black_box(engine.input_diagnostic(black_box(359_000)));
            }
        },
    );
    support::measure("book snapshot", 100_000, seeded_engine, |engine, count| {
        for _ in 0..count {
            black_box(engine.book_snapshot("BTC-USDT", black_box(359_000)));
        }
    });
    for tracked_scouts in [0, 20, 40, 80, 160] {
        let pending_outcomes = tracked_scouts * 5;
        support::measure(
            &format!("price update with {pending_outcomes} pending scout outcomes"),
            5_000,
            move || pending_scout_engine(tracked_scouts),
            |engine, count| {
                for timestamp in 1..=count as u64 {
                    black_box(engine.on_price(timestamp, 100.0, 0.1));
                }
            },
        );
    }
}
