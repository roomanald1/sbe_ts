use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use bytes::Bytes;
use rsocket_rust::prelude::Payload;
use sbe_schema::{SBE_BLOCK_LENGTH, SymbolPriceEncoder, WriteBuf, message_header_codec};
use tokio::sync::broadcast;

const SYMBOL_LENGTH: usize = 32;
const ITERATIONS: u64 = 5_000_000;

fn encode(symbol: &[u8; SYMBOL_LENGTH], price: f64) -> Vec<u8> {
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + SBE_BLOCK_LENGTH as usize];

    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(*symbol);
    encoder.price(price);
    drop(encoder.header(0));

    buffer
}

fn encode_into(symbol: &[u8; SYMBOL_LENGTH], price: f64, buffer: &mut [u8]) {
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(buffer), header_length);
    encoder.symbol(*symbol);
    encoder.price(price);
    drop(encoder.header(0));
}

fn report(name: &str, elapsed: Duration, iterations: u64) {
    let nanos_per_update = elapsed.as_nanos() as f64 / iterations as f64;
    let updates_per_second = iterations as f64 / elapsed.as_secs_f64();
    println!(
        "{name}: {nanos_per_update:.1} ns/update, \
         {updates_per_second:.0} updates/s ({iterations} updates)"
    );
}

fn benchmark_fanout(clients: usize, iterations: u64) {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..8].copy_from_slice(b"BTC-USDT");

    let started = Instant::now();
    for index in 0..iterations {
        let price = black_box(42_000.25 + (index % 100) as f64);
        for _ in 0..clients {
            let data = encode(black_box(&symbol), price);
            black_box(Payload::builder().set_data(data).build());
        }
    }
    report(
        &format!("{clients} clients: encode and allocate per client"),
        started.elapsed(),
        iterations,
    );

    let started = Instant::now();
    for index in 0..iterations {
        let price = black_box(42_000.25 + (index % 100) as f64);
        let encoded = Bytes::from(encode(black_box(&symbol), price));
        for _ in 0..clients {
            black_box(Payload::new(Some(encoded.clone()), None));
        }
    }
    report(
        &format!("{clients} clients: encode once and share immutable Bytes"),
        started.elapsed(),
        iterations,
    );
}

fn benchmark_broadcast_fanout(clients: usize, iterations: u64) {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..8].copy_from_slice(b"BTC-USDT");
    let (sender, _) = broadcast::channel::<Bytes>(4096);
    let mut receivers = (0..clients).map(|_| sender.subscribe()).collect::<Vec<_>>();

    let started = Instant::now();
    for index in 0..iterations {
        let price = black_box(42_000.25 + (index % 100) as f64);
        let encoded = Bytes::from(encode(black_box(&symbol), price));
        black_box(sender.send(encoded).unwrap());
        for receiver in &mut receivers {
            let data = receiver.try_recv().unwrap();
            black_box(Payload::new(Some(data), None));
        }
    }
    let elapsed = started.elapsed();
    let events_per_second = iterations as f64 / elapsed.as_secs_f64();
    let deliveries_per_second = events_per_second * clients as f64;
    println!(
        "Tokio broadcast to {clients} subscribers: \
         {:.1} ns/event, {:.0} events/s, {:.0} payload deliveries/s \
         ({iterations} events)",
        elapsed.as_nanos() as f64 / iterations as f64,
        events_per_second,
        deliveries_per_second,
    );
}

fn synthetic_price_update(round: u64, symbol_index: usize) -> ([u8; SYMBOL_LENGTH], f64) {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..3].copy_from_slice(b"SYM");
    symbol[3] = b'0' + (symbol_index / 100) as u8;
    symbol[4] = b'0' + ((symbol_index / 10) % 10) as u8;
    symbol[5] = b'0' + (symbol_index % 10) as u8;
    let base_price = 25.0 + symbol_index as f64 * 4.75;
    let phase = round as f64 * 0.17 + symbol_index as f64 * 1.91;
    let movement = phase.sin() * 0.004 + (phase * 0.37).cos() * 0.001;
    (symbol, base_price * (1.0 + movement))
}

fn main() {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..8].copy_from_slice(b"BTC-USDT");
    let mut reused = vec![0; message_header_codec::ENCODED_LENGTH + SBE_BLOCK_LENGTH as usize];

    for _ in 0..100_000 {
        let data = encode(black_box(&symbol), black_box(42_000.25));
        black_box(Payload::builder().set_data(data).build());
        encode_into(black_box(&symbol), black_box(42_000.25), &mut reused);
        black_box(&reused);
    }

    let started = Instant::now();
    for index in 0..ITERATIONS {
        let price = black_box(42_000.25 + (index % 100) as f64);
        let data = encode(black_box(&symbol), price);
        black_box(Payload::builder().set_data(data).build());
    }
    report(
        "allocate + encode + build payload",
        started.elapsed(),
        ITERATIONS,
    );

    let started = Instant::now();
    for index in 0..ITERATIONS {
        let price = black_box(42_000.25 + (index % 100) as f64);
        encode_into(black_box(&symbol), price, &mut reused);
        black_box(&reused);
    }
    report(
        "encode into reused scratch buffer (not safe for in-flight payloads)",
        started.elapsed(),
        ITERATIONS,
    );

    let synthetic_iterations = ITERATIONS / 10;
    let started = Instant::now();
    for index in 0..synthetic_iterations {
        black_box(synthetic_price_update(index / 100, (index % 100) as usize));
    }
    report(
        "synthetic symbol + price generation",
        started.elapsed(),
        synthetic_iterations,
    );

    let started = Instant::now();
    for index in 0..synthetic_iterations {
        let (symbol, price) = synthetic_price_update(index / 100, (index % 100) as usize);
        let data = encode(black_box(&symbol), black_box(price));
        black_box(Payload::builder().set_data(data).build());
    }
    report(
        "synthetic generation + encode + build payload",
        started.elapsed(),
        synthetic_iterations,
    );

    for clients in [1, 4, 16] {
        benchmark_fanout(clients, ITERATIONS / 10);
    }

    for clients in [200, 400] {
        benchmark_broadcast_fanout(clients, 25_000);
    }
}
