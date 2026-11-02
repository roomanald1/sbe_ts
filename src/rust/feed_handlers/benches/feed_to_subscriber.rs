use std::{hint::black_box, time::Instant};
use tokio::sync::broadcast;
#[path = "support/memory.rs"]
mod memory;

const EVENTS: usize = 50_000;

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    for (subscribers, capacity, yield_every) in [(1, 100_000, 0), (16, 100_000, 0), (16, 64, 20)] {
        let memory_before = memory::snapshot();
        let (tx, _) = broadcast::channel::<u64>(capacity);
        let tasks = (0..subscribers)
            .map(|_| {
                let mut rx = tx.subscribe();
                tokio::spawn(async move {
                    let (mut received, mut missed) = (0usize, 0u64);
                    loop {
                        match rx.recv().await {
                            Ok(value) => {
                                black_box(value);
                                received += 1;
                                if yield_every > 0 && received % yield_every == 0 {
                                    tokio::task::yield_now().await;
                                }
                            }
                            Err(broadcast::error::RecvError::Lagged(count)) => missed += count,
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    (received, missed)
                })
            })
            .collect::<Vec<_>>();
        let started = Instant::now();
        for event in 0..EVENTS {
            let _ = tx.send(event as u64);
            if event % 1024 == 0 {
                tokio::task::yield_now().await;
            }
        }
        drop(tx);
        let (mut received, mut missed) = (0usize, 0u64);
        for task in tasks {
            let (count, lag) = task.await.expect("subscriber task");
            received += count;
            missed += lag;
        }
        let elapsed = started.elapsed();
        println!(
            "broadcast subscribers={subscribers} capacity={capacity} yield_every={yield_every}: {:.1} us/event, {:.0} deliveries/s, received={received}, reported_lag={missed}",
            elapsed.as_secs_f64() * 1e6 / EVENTS as f64,
            received as f64 / elapsed.as_secs_f64()
        );
        memory::report("broadcast workload memory", memory_before);
    }
}
