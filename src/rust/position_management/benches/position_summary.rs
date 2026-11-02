use std::{collections::BTreeMap, hint::black_box, time::Instant};
#[path = "support/memory.rs"]
mod memory;

#[derive(Clone)]
struct Fill {
    symbol: String,
    source: &'static str,
    signed_quantity: f64,
    price: f64,
    timestamp: u64,
}

#[derive(Clone, Default)]
struct Position {
    quantity: f64,
    average_entry_price: f64,
    last_trade_timestamp: u64,
}

fn fixture(count: usize) -> Vec<Fill> {
    (0..count)
        .map(|index| Fill {
            symbol: format!("ASSET{}-USDT", index % 100),
            source: if index % 4 == 0 { "auto" } else { "manual" },
            signed_quantity: if index % 2 == 0 { 0.01 } else { -0.01 },
            price: 50_000.0 + (index % 500) as f64,
            timestamp: index as u64,
        })
        .collect()
}

fn replay(fills: &[Fill], source: Option<&str>) -> BTreeMap<String, Position> {
    let mut positions = BTreeMap::<String, Position>::new();
    for fill in fills {
        if source.is_some_and(|source| source != fill.source) {
            continue;
        }
        let position = positions.entry(fill.symbol.clone()).or_default();
        let old_quantity = position.quantity;
        let next_quantity = old_quantity + fill.signed_quantity;
        if old_quantity == 0.0 || old_quantity.signum() == fill.signed_quantity.signum() {
            position.average_entry_price = (old_quantity.abs() * position.average_entry_price
                + fill.signed_quantity.abs() * fill.price)
                / next_quantity.abs();
        } else if next_quantity.abs() < 1e-12 {
            position.average_entry_price = 0.0;
        } else if next_quantity.signum() != old_quantity.signum() {
            position.average_entry_price = fill.price;
        }
        position.quantity = next_quantity;
        position.last_trade_timestamp = fill.timestamp;
    }
    positions
}

fn report(name: &str, started: Instant, iterations: usize) {
    println!(
        "{name}: {:.1} ns/iteration",
        started.elapsed().as_nanos() as f64 / iterations as f64
    );
}

fn main() {
    const FILL_COUNT: usize = 100_000;
    const REPLAY_ITERATIONS: usize = 20;
    const READ_ITERATIONS: usize = 1_000_000;

    let fills = fixture(FILL_COUNT);
    let memory_before = memory::snapshot();
    let started = Instant::now();
    for _ in 0..REPLAY_ITERATIONS {
        black_box(replay(black_box(&fills), None));
    }
    report(
        "100k-fill position aggregation proxy",
        started,
        REPLAY_ITERATIONS,
    );
    memory::report("position aggregation memory", memory_before);

    let indexed = replay(&fills, None);
    let auto_index = replay(&fills, Some("auto"));
    black_box(auto_index);
    let started = Instant::now();
    for index in 0..READ_ITERATIONS {
        let symbol = format!("ASSET{}-USDT", index % 100);
        black_box(indexed.get(black_box(&symbol)));
    }
    report("indexed position lookup", started, READ_ITERATIONS);

    let started = Instant::now();
    for _ in 0..READ_ITERATIONS {
        black_box(indexed.values().count());
    }
    report("position count scan", started, READ_ITERATIONS);
}
