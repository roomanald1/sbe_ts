const SYMBOL_LENGTH: usize = 32;

#[path = "../src/encoders/book_depth.rs"]
mod book_depth;
#[path = "../src/encoders/trade_tick.rs"]
mod trade_tick;

mod okx_feed {
    use crate::{book_depth, trade_tick};
    use anyhow::Result;
    use serde::{Deserialize, Serialize};
    use std::collections::BTreeMap;

    include!("../src/okx/okx_message_types.rs");
    include!("../src/okx/book_update_helpers.rs");

    pub(crate) fn trade_message(raw: &str) -> Result<bytes::Bytes> {
        let message: TradeMessage<'_> = serde_json::from_str(raw)?;
        let Some(trade) = message.data.and_then(|mut trades| trades.pop()) else {
            anyhow::bail!("trade message had no trade");
        };
        let _ = (message.event, message.msg);
        trade_tick::encode_trade_tick(
            trade.instrument,
            trade.trade_id,
            trade.ts.parse()?,
            trade.px.parse()?,
            trade.sz.parse()?,
            trade.side,
        )
    }

    pub(crate) struct BookState {
        bids: Vec<(f64, f64)>,
        asks: Vec<(f64, f64)>,
    }

    impl Default for BookState {
        fn default() -> Self {
            Self {
                bids: Vec::with_capacity(16),
                asks: Vec::with_capacity(16),
            }
        }
    }

    pub(crate) fn book_message(state: &mut BookState, raw: &str) -> Result<bytes::Bytes> {
        let message: BookMessage<'_> = serde_json::from_str(raw)?;
        let (Some(argument), Some(mut updates)) = (message.arg, message.data) else {
            anyhow::bail!("book message had no update");
        };
        let update = updates
            .pop()
            .ok_or_else(|| anyhow::anyhow!("empty book update"))?;
        let _ = (message.event, message.msg);
        if message.action == Some("snapshot") {
            load_book_snapshot(&mut state.bids, &update.bids)?;
            load_book_snapshot(&mut state.asks, &update.asks)?;
        } else {
            apply_book_side(&mut state.bids, &update.bids, true)?;
            apply_book_side(&mut state.asks, &update.asks, false)?;
        }
        let best_bids = top_book_side(&state.bids);
        let best_asks = top_book_side(&state.asks);
        book_depth::encode_book_depth(
            argument.instrument,
            update.ts.parse()?,
            &best_bids,
            &best_asks,
        )
    }
}

use std::{hint::black_box, time::Instant};

const ITERATIONS: u64 = 1_000_000;
const BOOK_ITERATIONS: u64 = 10_000;
const TRADE: &str = r#"{"arg":{"channel":"trades","instId":"BTC-USDT"},"data":[{"instId":"BTC-USDT","tradeId":"123456789","px":"67321.50","sz":"0.125","side":"buy","ts":"1700000000000"}]}"#;

fn book_fixture(levels: usize) -> String {
    let asks = (0..levels)
        .map(|index| {
            format!(
                "[\"{:.2}\",\"1.25\",\"0\",\"1\"]",
                67_321.51 + index as f64 * 0.01
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let bids = (0..levels)
        .map(|index| {
            format!(
                "[\"{:.2}\",\"1.10\",\"0\",\"1\"]",
                67_321.49 - index as f64 * 0.01
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"arg":{{"channel":"books","instId":"BTC-USDT"}},"action":"snapshot","data":[{{"asks":[{asks}],"bids":[{bids}],"ts":"1700000000000"}}]}}"#
    )
}

fn incremental_book_fixture(start: usize, count: usize) -> String {
    let asks = (start..start + count)
        .map(|index| {
            format!(
                "[\"{:.2}\",\"1.50\",\"0\",\"1\"]",
                67_321.51 + index as f64 * 0.01
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let bids = (start..start + count)
        .map(|index| {
            format!(
                "[\"{:.2}\",\"1.25\",\"0\",\"1\"]",
                67_321.49 - index as f64 * 0.01
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        r#"{{"arg":{{"channel":"books","instId":"BTC-USDT"}},"action":"update","data":[{{"asks":[{asks}],"bids":[{bids}],"ts":"1700000000001"}}]}}"#
    )
}

fn benchmark_snapshot_depth(levels: usize, iterations: u64) {
    let snapshot = book_fixture(levels);
    let mut state = okx_feed::BookState::default();
    for _ in 0..2_000 {
        black_box(okx_feed::book_message(&mut state, black_box(&snapshot)).unwrap());
    }
    let started = Instant::now();
    for _ in 0..iterations {
        black_box(okx_feed::book_message(&mut state, black_box(&snapshot)).unwrap());
    }
    report(
        &format!("OKX {levels}-level snapshot decode + load + top-seven + encode"),
        started,
        iterations,
    );
}

fn report(label: &str, started: Instant, iterations: u64) {
    let elapsed = started.elapsed();
    println!(
        "{label}: {:.1} ns/message, {:.0} messages/s ({iterations} messages)",
        elapsed.as_nanos() as f64 / iterations as f64,
        iterations as f64 / elapsed.as_secs_f64(),
    );
}

fn main() {
    let mut trade_state = okx_feed::BookState::default();
    let book = book_fixture(400);
    for _ in 0..10_000 {
        black_box(okx_feed::trade_message(black_box(TRADE)).unwrap());
        black_box(okx_feed::book_message(&mut trade_state, black_box(&book)).unwrap());
    }

    let started = Instant::now();
    for _ in 0..ITERATIONS {
        black_box(okx_feed::trade_message(black_box(TRADE)).unwrap());
    }
    report(
        "OKX trade JSON decode + value parse + SBE encode",
        started,
        ITERATIONS,
    );

    benchmark_snapshot_depth(7, 250_000);
    benchmark_snapshot_depth(50, 100_000);
    benchmark_snapshot_depth(400, 25_000);

    let mut incremental_state = okx_feed::BookState::default();
    black_box(okx_feed::book_message(&mut incremental_state, black_box(&book)).unwrap());
    let incremental = incremental_book_fixture(200, 5);
    for _ in 0..2_000 {
        black_box(okx_feed::book_message(&mut incremental_state, black_box(&incremental)).unwrap());
    }
    let started = Instant::now();
    for _ in 0..250_000 {
        black_box(okx_feed::book_message(&mut incremental_state, black_box(&incremental)).unwrap());
    }
    report(
        "OKX 5-level incremental update on 400-level state + top-seven + encode",
        started,
        250_000,
    );
}
