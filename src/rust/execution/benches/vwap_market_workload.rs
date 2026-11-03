use execution::vwap::{NUM_BINS, OrderManager, VwapConfig};
use std::{hint::black_box, time::Instant};

const SYMBOLS: usize = 64;
const EVENTS_PER_SYMBOL: usize = 20_000;

fn main() {
    let curve = [1.0 / NUM_BINS as f64; NUM_BINS];
    let mut managers = (0..SYMBOLS)
        .map(|_| {
            OrderManager::new(
                VwapConfig {
                    parent_qty: 500.0,
                    historical_vwap_curve: curve,
                    max_participation_rate: 0.1,
                },
                [1000.0; NUM_BINS],
                0.001,
                300.0,
            )
            .expect("valid VWAP fixture")
        })
        .collect::<Vec<_>>();
    let started = Instant::now();
    let mut fills = 0usize;
    let events = SYMBOLS * EVENTS_PER_SYMBOL;
    for event in 0..events {
        let manager = &mut managers[event % SYMBOLS];
        let phase = (event / SYMBOLS) % 200;
        let (volume, elapsed, fill_qty) = match phase {
            0..=119 => (0.3, 2.0, 0.0),
            120..=179 => (8.0, 30.0, 0.0),
            _ => (2.0, 150.0, 0.25),
        };
        let action = manager.on_market_trade(black_box(volume), 100.0, elapsed);
        black_box(action);
        if fill_qty > 0.0 {
            manager.on_execution_fill(black_box(100.01), black_box(fill_qty));
            fills += 1;
        }
        if event % (SYMBOLS * 10) == 0 {
            for manager in &mut managers {
                manager.on_bin_transition();
            }
        }
    }
    let elapsed = started.elapsed();
    let executed: f64 = managers
        .iter()
        .map(|manager| manager.state().executed_qty)
        .sum();
    println!(
        "VWAP multi-symbol workload: symbols={SYMBOLS} events={events} partial_fill_events={fills} executed_qty={executed:.2}, {:.1} ns/event, {:.0} events/s",
        elapsed.as_nanos() as f64 / events as f64,
        events as f64 / elapsed.as_secs_f64()
    );
}
