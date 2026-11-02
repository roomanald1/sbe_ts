use std::{hint::black_box, time::Instant};

use serde_json::{Value, json};
#[path = "support/memory.rs"]
mod memory;

const SMALL_ROWS: usize = 100;
const LARGE_ROWS: usize = 100_000;
const SMALL_ITERATIONS: usize = 250;
const LARGE_ITERATIONS: usize = 2;

fn ledger_fixture(rows: usize) -> Value {
    let trades = (0..rows)
        .map(|index| {
            json!({
                "id": format!("fill-{index}"),
                "symbol": format!("ASSET{}-USDT", index % 20),
                "side": if index % 2 == 0 { "buy" } else { "sell" },
                "quantity": 0.01 + (index % 10) as f64 * 0.001,
                "execution_price": 50_000.0 + (index % 500) as f64,
                "timestamp": 1_700_000_000_000_u64 + index as u64,
                "source": if index % 4 == 0 { "auto" } else { "manual" },
                "parent_order_id": format!("order-{}", index / 4),
                "fee": -0.5,
                "fee_currency": "USDT",
                "fee_rate": 0.001,
                "fee_liquidity": if index % 2 == 0 { "taker" } else { "maker" }
            })
        })
        .collect::<Vec<_>>();
    json!({
        "trades": trades,
        "initial_cash": 100_000.0,
        "topups": [],
        "max_leverage": null,
        "orders": [],
        "barrier_orders": [],
        "scheduled_runs": []
    })
}

fn report(name: &str, started: Instant, iterations: usize, bytes_per_iteration: usize) {
    let elapsed = started.elapsed();
    let ns_per_iteration = elapsed.as_nanos() as f64 / iterations as f64;
    let mib_per_second =
        (bytes_per_iteration * iterations) as f64 / elapsed.as_secs_f64() / (1024.0 * 1024.0);
    println!("{name}: {ns_per_iteration:.1} ns/iteration, {mib_per_second:.1} MiB/s");
}

fn benchmark_size(rows: usize, iterations: usize) {
    let memory_before = memory::snapshot();
    let ledger = ledger_fixture(rows);
    let messagepack = rmp_serde::to_vec_named(&ledger).expect("encode benchmark fixture");
    let json = serde_json::to_vec(&ledger).expect("encode JSON benchmark fixture");

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(
            rmp_serde::from_slice::<Value>(black_box(&messagepack)).expect("decode MessagePack"),
        );
    }
    report(
        &format!("{rows} fills: MessagePack decode"),
        started,
        iterations,
        messagepack.len(),
    );

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(serde_json::from_slice::<Value>(black_box(&json)).expect("decode JSON"));
    }
    report(
        &format!("{rows} fills: JSON decode"),
        started,
        iterations,
        json.len(),
    );

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(rmp_serde::to_vec_named(black_box(&ledger)).expect("encode MessagePack"));
    }
    report(
        &format!("{rows} fills: MessagePack encode"),
        started,
        iterations,
        messagepack.len(),
    );

    let started = Instant::now();
    for _ in 0..iterations {
        black_box(serde_json::to_vec(black_box(&ledger)).expect("encode JSON"));
    }
    report(
        &format!("{rows} fills: JSON encode"),
        started,
        iterations,
        json.len(),
    );
    memory::report(
        &format!("{rows}-fill ledger serialization memory"),
        memory_before,
    );
}

fn main() {
    benchmark_size(SMALL_ROWS, SMALL_ITERATIONS);
    benchmark_size(LARGE_ROWS, LARGE_ITERATIONS);
}
