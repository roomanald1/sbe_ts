use std::{hint::black_box, time::Instant};

use execution::vwap::{NUM_BINS, OrderManager, VwapConfig, VwapScheduler};

const ITERATIONS: usize = 1_000_000;
const MANAGERS: usize = ITERATIONS / NUM_BINS;
const TICKS_PER_BIN: usize = 10;
const FULL_RUNS: usize = ITERATIONS / (NUM_BINS * TICKS_PER_BIN);

fn config() -> VwapConfig {
    VwapConfig {
        parent_qty: 10_000.0,
        historical_vwap_curve: [1.0 / NUM_BINS as f64; NUM_BINS],
        max_participation_rate: 0.1,
    }
}

fn manager(expected_bin_volume: f64) -> OrderManager {
    OrderManager::new(config(), [expected_bin_volume; NUM_BINS], 0.001, 300.0)
        .expect("valid benchmark VWAP configuration")
}

fn managers(expected_bin_volume: f64) -> Vec<OrderManager> {
    (0..MANAGERS)
        .map(|_| manager(expected_bin_volume))
        .collect()
}

fn report(name: &str, started: Instant, iterations: usize) {
    println!(
        "{name}: {:.1} ns/iteration",
        started.elapsed().as_nanos() as f64 / iterations as f64
    );
}

fn construction_benchmarks() {
    let config = config();
    let expected_volumes = [1_000.0; NUM_BINS];

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(VwapScheduler::new(black_box(config)).expect("valid benchmark config"));
    }
    report("VWAP scheduler construction and validation", started, ITERATIONS);

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(
            OrderManager::new(
                black_box(config),
                black_box(expected_volumes),
                black_box(0.001),
                black_box(300.0),
            )
            .expect("valid benchmark config"),
        );
    }
    report("VWAP order manager construction and validation", started, ITERATIONS);
}

fn scheduler_child_order_benchmark() {
    let scheduler = VwapScheduler::new(config()).expect("valid benchmark VWAP configuration");
    let started = Instant::now();
    for index in 0..ITERATIONS {
        black_box(scheduler.calculate_child_order(
            black_box(500.0 + (index % 100) as f64),
            black_box(1_000.0),
            black_box((index % 101) as f64 / 100.0),
        ));
    }
    report("VWAP scheduler child order calculation", started, ITERATIONS);
}

fn no_action_trade_benchmark() {
    let mut managers = managers(1_000.0);
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            black_box(manager.on_market_trade(black_box(0.25), black_box(100.0), black_box(0.0)));
            manager.on_bin_transition();
        }
    }
    report("VWAP market trade with no order action", started, MANAGERS * NUM_BINS);
}

fn new_order_benchmark() {
    let mut managers = managers(1_000.0);
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            black_box(manager.on_market_trade(
                black_box(1_000.0),
                black_box(100.0),
                black_box(300.0),
            ));
            manager.on_bin_transition();
        }
    }
    report("VWAP market trade creating a new order", started, MANAGERS * NUM_BINS);
}

fn modify_order_benchmark() {
    let mut managers = managers(10_000.0);
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            let _ = manager.on_market_trade(black_box(1_000.0), black_box(100.0), black_box(30.0));
            black_box(manager.on_market_trade(
                black_box(1_000.0),
                black_box(100.0),
                black_box(60.0),
            ));
            manager.on_bin_transition();
        }
    }
    report("VWAP market trade modifying an order", started, MANAGERS * NUM_BINS);
}

fn cancel_order_benchmark() {
    let mut managers = managers(1_000.0);
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            let _ = manager.on_market_trade(black_box(1_000.0), black_box(100.0), black_box(300.0));
            black_box(manager.on_market_trade(
                black_box(1.0),
                black_box(100.0),
                black_box(0.0),
            ));
            manager.on_bin_transition();
        }
    }
    report("VWAP market trade cancelling an order", started, MANAGERS * NUM_BINS);
}

fn fill_processing_benchmark() {
    let mut managers = managers(1_000.0);
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            let _ = manager.on_market_trade(black_box(1_000.0), black_box(100.0), black_box(300.0));
            manager.on_execution_fill(black_box(100.01), black_box(0.01));
            black_box(manager.benchmark());
            manager.on_bin_transition();
        }
    }
    report("VWAP order manager execution fill", started, MANAGERS * NUM_BINS);
}

fn full_run_with_fills_benchmark() {
    let mut managers = (0..FULL_RUNS).map(|_| manager(1_000.0)).collect::<Vec<_>>();
    let started = Instant::now();
    for manager in &mut managers {
        for _ in 0..NUM_BINS {
            for tick in 0..TICKS_PER_BIN {
                let elapsed_secs = (tick + 1) as f64 * 300.0 / TICKS_PER_BIN as f64;
                if let Some(action) = manager.on_market_trade(
                    black_box(100.0),
                    black_box(100.0),
                    black_box(elapsed_secs),
                ) {
                    black_box(action);
                }
                manager.on_execution_fill(black_box(100.01), black_box(1.0));
            }
            manager.on_bin_transition();
        }
        black_box(manager.state());
        black_box(manager.benchmark());
    }
    report(
        "VWAP full 78-bin execution run with fills",
        started,
        FULL_RUNS * NUM_BINS * TICKS_PER_BIN,
    );
}

fn main() {
    construction_benchmarks();
    scheduler_child_order_benchmark();
    no_action_trade_benchmark();
    new_order_benchmark();
    modify_order_benchmark();
    cancel_order_benchmark();
    fill_processing_benchmark();
    full_run_with_fills_benchmark();
}
