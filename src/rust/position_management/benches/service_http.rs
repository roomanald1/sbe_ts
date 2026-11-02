use std::{env, time::Instant};

const REQUESTS: usize = 2_000;
const CONCURRENCY: usize = 32;

#[tokio::main]
async fn main() {
    let Some(base) = env::var("BENCH_SERVICE_URL").ok() else {
        println!("service HTTP benchmark skipped: set BENCH_SERVICE_URL to an already-running service URL (for example http://127.0.0.1:3002)");
        return;
    };
    let url = format!("{}/health", base.trim_end_matches('/'));
    let client = reqwest::Client::builder().pool_max_idle_per_host(CONCURRENCY).build().expect("HTTP client");
    let latencies = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::with_capacity(REQUESTS)));
    let started = Instant::now();
    let mut tasks = Vec::new();
    for worker in 0..CONCURRENCY {
        let client = client.clone(); let url = url.clone(); let latencies = latencies.clone();
        tasks.push(tokio::spawn(async move {
            let mut local = Vec::new();
            for _ in (worker..REQUESTS).step_by(CONCURRENCY) {
                let before = Instant::now();
                let response = client.get(&url).send().await.expect("service request");
                assert!(response.status().is_success(), "health endpoint returned {}", response.status());
                local.push(before.elapsed().as_nanos() as u64);
            }
            latencies.lock().await.extend(local);
        }));
    }
    for task in tasks { task.await.expect("request worker"); }
    let elapsed = started.elapsed();
    let mut samples = std::sync::Arc::try_unwrap(latencies).expect("workers released samples").into_inner();
    samples.sort_unstable();
    let percentile = |p: usize| samples[((samples.len() - 1) * p) / 100] as f64 / 1e6;
    println!("service HTTP health workload: url={url} requests={} concurrency={CONCURRENCY} throughput={:.0} req/s p50={:.3} ms p95={:.3} ms p99={:.3} ms", samples.len(), samples.len() as f64 / elapsed.as_secs_f64(), percentile(50), percentile(95), percentile(99));
}
