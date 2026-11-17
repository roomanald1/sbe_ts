#[path = "common/mod.rs"]
mod support;

use std::hint::black_box;

use insights::signals::SignalEngine;

const SEEDED_PRICE_SAMPLES: usize = 12_000;
const ACTIVE_SCOUTS: usize = 200;

struct FeedEventState {
    engine: SignalEngine,
    timestamp: u64,
    price: f64,
    bid_levels: [(f64, f64); 7],
    ask_levels: [(f64, f64); 7],
    bid_sizes: [f64; 7],
    ask_sizes: [f64; 7],
}

fn seeded_feed_event_state() -> FeedEventState {
    let bid_levels = std::array::from_fn(|level| (99.99 - level as f64 * 0.01, 1.0 + level as f64));
    let ask_levels =
        std::array::from_fn(|level| (100.01 + level as f64 * 0.01, 1.5 + level as f64));
    let bid_sizes = bid_levels.map(|(_, size)| size);
    let ask_sizes = ask_levels.map(|(_, size)| size);
    let mut engine = SignalEngine::default();
    for index in 0..SEEDED_PRICE_SAMPLES {
        let timestamp = index as u64 * 250;
        let price = 100.0 + (index as f64 * 0.01).sin() * 0.1;
        engine.on_book(
            timestamp,
            bid_levels[0].0,
            ask_levels[0].0,
            &bid_sizes,
            &ask_sizes,
            &bid_levels,
            &ask_levels,
        );
        engine.on_price(timestamp, price, 0.01);
        if index % 2 == 0 {
            engine.on_trade(timestamp, price, 0.25, index % 4 == 0);
        }
    }
    for index in 0..ACTIVE_SCOUTS {
        let timestamp = (SEEDED_PRICE_SAMPLES as u64 * 250).saturating_sub(60_000);
        assert!(engine.track_scout(
            format!("active-scout-{index}"),
            "BTC-USDT".into(),
            "live-momentum".into(),
            timestamp,
            "bullish".into(),
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
    FeedEventState {
        engine,
        timestamp: SEEDED_PRICE_SAMPLES as u64 * 250,
        price: 100.0,
        bid_levels,
        ask_levels,
        bid_sizes,
        ask_sizes,
    }
}

fn run_prices(state: &mut FeedEventState, iterations: usize) {
    for index in 0..iterations {
        state.timestamp += 250;
        state.price += (index as f64 * 0.01).sin() * 0.0001;
        black_box(state.engine.on_price(state.timestamp, state.price, 0.01));
    }
}

fn run_trades(state: &mut FeedEventState, iterations: usize) {
    for index in 0..iterations {
        state.timestamp += 250;
        black_box(
            state
                .engine
                .on_trade(state.timestamp, state.price, 0.25, index % 2 == 0),
        );
    }
}

fn run_books(state: &mut FeedEventState, iterations: usize) {
    for _ in 0..iterations {
        state.timestamp += 250;
        black_box(state.engine.on_book(
            state.timestamp,
            state.bid_levels[0].0,
            state.ask_levels[0].0,
            &state.bid_sizes,
            &state.ask_sizes,
            &state.bid_levels,
            &state.ask_levels,
        ));
    }
}

fn run_snapshots(state: &mut FeedEventState, iterations: usize) {
    for _ in 0..iterations {
        state.timestamp += 2_000;
        black_box(
            state
                .engine
                .snapshot_with_reasons("BTC-USDT", state.timestamp, false),
        );
    }
}

fn main() {
    support::measure(
        "SignalEngine::on_price (1,000 pending scout outcomes)",
        10_000,
        seeded_feed_event_state,
        run_prices,
    );
    support::measure(
        "SignalEngine::on_trade (bounded 60-second tape)",
        10_000,
        seeded_feed_event_state,
        run_trades,
    );
    support::measure(
        "SignalEngine::on_book (7 levels, 60-second history)",
        10_000,
        seeded_feed_event_state,
        run_books,
    );
    support::measure(
        "SignalEngine::snapshot_with_reasons (12,000 price samples)",
        1_000,
        seeded_feed_event_state,
        run_snapshots,
    );
}
