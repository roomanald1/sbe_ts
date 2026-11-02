use serde_json::{Value, json};
use std::{
    fs,
    hint::black_box,
    sync::{Arc, Barrier},
    thread,
    time::Instant,
};

const ROWS: usize = 100_000;
const READERS: usize = 4;
const WRITERS: usize = 1;

fn main() {
    let rows = (0..ROWS).map(|i| json!({"id": i, "symbol": format!("S{}", i % 200), "qty": i as f64 * 0.001, "price": 50_000.0 + (i % 1000) as f64})).collect::<Vec<_>>();
    let payload = serde_json::to_vec(&json!({"trades": rows})).expect("serialize fixture");
    let path = std::env::temp_dir().join(format!("ledger-bench-{}.json", std::process::id()));
    fs::write(&path, &payload).expect("write initial ledger");
    let path = Arc::new(path);
    let barrier = Arc::new(Barrier::new(READERS + WRITERS));
    let started = Instant::now();
    let mut workers = Vec::new();
    for _ in 0..READERS {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            let (mut bytes, mut invalid) = (0usize, 0usize);
            for _ in 0..20 {
                let data = fs::read(path.as_path()).expect("read ledger");
                bytes += data.len();
                if let Ok(value) = serde_json::from_slice::<Value>(&data) {
                    black_box(value);
                } else {
                    invalid += 1;
                }
            }
            (bytes, invalid)
        }));
    }
    for _ in 0..WRITERS {
        let path = Arc::clone(&path);
        let barrier = Arc::clone(&barrier);
        let payload = payload.clone();
        workers.push(thread::spawn(move || {
            barrier.wait();
            let mut bytes = 0usize;
            for _ in 0..20 {
                let temp = path.with_extension("msgpack.tmp");
                fs::write(&temp, &payload).expect("write temporary ledger");
                fs::rename(&temp, path.as_path()).expect("atomically replace ledger");
                bytes += payload.len();
            }
            (bytes, 0)
        }));
    }
    let (mut bytes, mut invalid_reads) = (0usize, 0usize);
    for worker in workers {
        match worker.join().expect("worker") {
            (worker_bytes, invalid) => {
                bytes += worker_bytes;
                invalid_reads += invalid;
            }
        }
    }
    let elapsed = started.elapsed();
    let final_rows =
        serde_json::from_slice::<Value>(&fs::read(path.as_path()).expect("read final ledger"))
            .expect("valid final JSON");
    fs::remove_file(path.as_path()).expect("remove benchmark fixture");
    println!(
        "ledger concurrent filesystem workload: rows={ROWS} readers={READERS} writers={WRITERS} transferred_bytes={bytes} invalid_reader_snapshots={invalid_reads} final_document_valid={} elapsed_ms={:.2}",
        final_rows.is_object(),
        elapsed.as_secs_f64() * 1000.0
    );
    println!(
        "Note: atomic rename mirrors the production snapshot replacement; this still does not exercise the service ledger lock or durability policy."
    );
}
