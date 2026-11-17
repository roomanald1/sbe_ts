#[path = "../src/scouts/registration_replay.rs"]
mod registration_replay;

use std::{hint::black_box, time::Instant};

use registration_replay::RegistrationReplayQueue;
use serde_json::Value;

const ACTIVE_REGISTRATIONS: usize = 10_000;
const REPLAY_INTERVALS: usize = 60;
const FIRST_PASS_FAILED: usize = ACTIVE_REGISTRATIONS / 20;
const REPEATS: usize = 5;

fn registrations() -> Vec<Value> {
    (0..ACTIVE_REGISTRATIONS)
        .map(|index| {
            serde_json::json!({
                "decision_id": format!("decision-{index}"),
                "symbol": "BTC-USDT",
                "timestamp": 1_800_000_000_000_u64,
                "direction": "bullish",
                "entry_price": 100_000.0,
                "strategy": "live-momentum",
                "exit_scenario": "strategy",
                "stop_loss_percent": 0.5,
                "take_profit_percent": 1.0,
                "profit_protection_activation_percent": 0.5,
                "profit_protection_giveback_percent": 0.25,
                "profit_protection_minimum_lock_percent": 0.1,
            })
        })
        .collect()
}

fn encode_request(registration: &Value) {
    black_box(serde_json::to_vec(registration).expect("registration encodes"));
}

fn old_periodic_replay(active: &[Value]) -> usize {
    let mut request_count = 0;
    for _ in 0..REPLAY_INTERVALS {
        for registration in active {
            encode_request(registration);
            request_count += 1;
        }
    }
    request_count
}

fn retry_only_replay(active: Vec<Value>) -> usize {
    let mut queue = RegistrationReplayQueue::new(active);
    let mut request_count = 0;
    let mut first_pass = true;
    let mut intervals = 0;

    while !queue.is_empty() && intervals < REPLAY_INTERVALS {
        let mut failed = Vec::new();
        for registration in queue.take_pending() {
            encode_request(&registration);
            request_count += 1;
            if first_pass && request_count > ACTIVE_REGISTRATIONS - FIRST_PASS_FAILED {
                failed.push(registration);
            }
        }
        queue.retry_failed(failed);
        assert_eq!(
            queue.pending_len(),
            if first_pass { FIRST_PASS_FAILED } else { 0 }
        );
        first_pass = false;
        intervals += 1;
    }

    assert!(queue.is_empty(), "failed registrations recover on retry");
    request_count
}

fn measure<T>(setup: impl Fn() -> T, workload: impl Fn(T) -> usize) -> (usize, u128) {
    let mut elapsed = Vec::with_capacity(REPEATS);
    let mut request_count = 0;
    for _ in 0..REPEATS {
        let input = setup();
        let started = Instant::now();
        request_count = workload(input);
        elapsed.push(started.elapsed().as_micros());
    }
    elapsed.sort_unstable();
    (request_count, elapsed[REPEATS / 2])
}

fn main() {
    let active = registrations();
    let (old_requests, old_micros) =
        measure(|| active.clone(), |active| old_periodic_replay(&active));
    let (fixed_requests, fixed_micros) = measure(|| active.clone(), retry_only_replay);
    let reduction = 100.0 * (1.0 - fixed_requests as f64 / old_requests as f64);

    println!("synthetic active registrations: {ACTIVE_REGISTRATIONS}");
    println!("replay intervals: {REPLAY_INTERVALS}");
    println!("first-pass failures recovered: {FIRST_PASS_FAILED}");
    println!("timing repetitions: {REPEATS} (median; dataset cloning excluded)");
    println!("old periodic replay: {old_requests} encoded requests, {old_micros} us");
    println!("fixed retry-only replay: {fixed_requests} encoded requests, {fixed_micros} us");
    println!("encoded request reduction: {reduction:.2}%");

    assert_eq!(old_requests, ACTIVE_REGISTRATIONS * REPLAY_INTERVALS);
    assert_eq!(fixed_requests, ACTIVE_REGISTRATIONS + FIRST_PASS_FAILED);
}
