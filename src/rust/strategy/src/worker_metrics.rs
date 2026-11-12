use axum::{Router, extract::State, routing::get};
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

const LATENCY_BUCKETS_SECONDS: [f64; 8] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0];

#[derive(Debug)]
pub(crate) struct WorkerMetrics {
    role: &'static str,
    strategy: String,
    requests: AtomicU64,
    errors: AtomicU64,
    timeouts: AtomicU64,
    payload_rejects: AtomicU64,
    saturation_rejects: AtomicU64,
    in_flight: AtomicUsize,
    latency_count: AtomicU64,
    latency_sum_ns: AtomicU64,
    latency_buckets: [AtomicU64; 8],
}

impl WorkerMetrics {
    pub(crate) fn new(role: &'static str, strategy: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            role,
            strategy: strategy.into(),
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            payload_rejects: AtomicU64::new(0),
            saturation_rejects: AtomicU64::new(0),
            in_flight: AtomicUsize::new(0),
            latency_count: AtomicU64::new(0),
            latency_sum_ns: AtomicU64::new(0),
            latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        })
    }

    pub(crate) fn begin(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn finish(&self, elapsed_ns: u64, error: Option<&str>) {
        self.in_flight.fetch_sub(1, Ordering::Relaxed);
        self.latency_count.fetch_add(1, Ordering::Relaxed);
        self.latency_sum_ns.fetch_add(elapsed_ns, Ordering::Relaxed);
        let elapsed = elapsed_ns as f64 / 1_000_000_000.0;
        for (index, bound) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            if elapsed <= *bound {
                self.latency_buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
        if let Some(reason) = error {
            self.errors.fetch_add(1, Ordering::Relaxed);
            if reason == "timeout" {
                self.timeouts.fetch_add(1, Ordering::Relaxed);
            } else if reason == "payload" {
                self.payload_rejects.fetch_add(1, Ordering::Relaxed);
            } else if reason == "saturated" {
                self.saturation_rejects.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn classify_error(message: &str) -> &'static str {
        let message = message.to_ascii_lowercase();
        if message.contains("payload") || message.contains("exceed") {
            "payload"
        } else if message.contains("in-flight") || message.contains("capacity is busy") {
            "saturated"
        } else if message.contains("timeout") {
            "timeout"
        } else {
            "error"
        }
    }

    pub(crate) fn render(&self) -> String {
        let labels = format!("role=\"{}\",strategy=\"{}\"", self.role, self.strategy);
        let mut output = String::from(
            "# HELP app_strategy_worker_requests_total Worker request-response attempts.\n# TYPE app_strategy_worker_requests_total counter\n",
        );
        metric(
            &mut output,
            "app_strategy_worker_requests_total",
            &labels,
            self.requests.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_errors_total Worker request and evaluation errors.\n# TYPE app_strategy_worker_errors_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_errors_total",
            &labels,
            self.errors.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_timeouts_total Worker request timeouts.\n# TYPE app_strategy_worker_timeouts_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_timeouts_total",
            &labels,
            self.timeouts.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_payload_rejects_total Requests rejected for exceeding the payload limit.\n# TYPE app_strategy_worker_payload_rejects_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_payload_rejects_total",
            &labels,
            self.payload_rejects.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_saturation_rejects_total Requests rejected because worker capacity is full.\n# TYPE app_strategy_worker_saturation_rejects_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_saturation_rejects_total",
            &labels,
            self.saturation_rejects.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_in_flight Current worker requests in flight.\n# TYPE app_strategy_worker_in_flight gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_in_flight",
            &labels,
            self.in_flight.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_request_duration_seconds Request processing latency.\n# TYPE app_strategy_worker_request_duration_seconds histogram\n");
        for (index, bound) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            let cumulative = self.latency_buckets[index].load(Ordering::Relaxed);
            output.push_str(&format!("app_strategy_worker_request_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {cumulative}\n"));
        }
        output.push_str(&format!(
            "app_strategy_worker_request_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}\n",
            self.latency_count.load(Ordering::Relaxed)
        ));
        output.push_str(&format!(
            "app_strategy_worker_request_duration_seconds_sum{{{labels}}} {:.9}\n",
            self.latency_sum_ns.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        output.push_str(&format!(
            "app_strategy_worker_request_duration_seconds_count{{{labels}}} {}\n",
            self.latency_count.load(Ordering::Relaxed)
        ));
        if let Some((rss, cpu_seconds)) = process_usage() {
            output.push_str("# HELP app_strategy_worker_process_resident_memory_bytes Worker process resident memory in bytes.\n# TYPE app_strategy_worker_process_resident_memory_bytes gauge\n");
            metric(
                &mut output,
                "app_strategy_worker_process_resident_memory_bytes",
                &labels,
                rss,
            );
            output.push_str("# HELP app_strategy_worker_process_cpu_seconds_total Worker process cumulative CPU time.\n# TYPE app_strategy_worker_process_cpu_seconds_total counter\n");
            output.push_str(&format!(
                "app_strategy_worker_process_cpu_seconds_total{{{labels}}} {cpu_seconds:.6}\n"
            ));
        }
        output
    }
}

fn metric(output: &mut String, name: &str, labels: &str, value: impl std::fmt::Display) {
    output.push_str(&format!("{name}{{{labels}}} {value}\n"));
}

fn process_usage() -> Option<(u64, f64)> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let rss_kib = status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })?;
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields = stat
        .rsplit_once(") ")?
        .1
        .split_whitespace()
        .collect::<Vec<_>>();
    let user_ticks = fields.get(11)?.parse::<u64>().ok()?;
    let system_ticks = fields.get(12)?.parse::<u64>().ok()?;
    let ticks_per_second = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if ticks_per_second <= 0 {
        return None;
    }
    Some((
        rss_kib * 1024,
        (user_ticks + system_ticks) as f64 / ticks_per_second as f64,
    ))
}

async fn health() -> &'static str {
    "ok\n"
}
async fn metrics_handler(State(metrics): State<Arc<WorkerMetrics>>) -> String {
    metrics.render()
}

pub(crate) async fn serve_metrics(addr: String, metrics: Arc<WorkerMetrics>) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    let app = Router::new()
        .route("/health", get(health))
        .route("/metrics", get(metrics_handler))
        .with_state(metrics);
    axum::serve(listener, app).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exports_bounded_role_strategy_metrics_and_histogram() {
        let metrics = WorkerMetrics::new("live", "live-momentum");
        metrics.begin();
        metrics.finish(8_000_000, Some("payload"));
        metrics.begin();
        metrics.finish(2_000_000, None);
        metrics.begin();
        metrics.finish(600_000_000, None);
        let text = metrics.render();
        assert!(text.contains("role=\"live\",strategy=\"live-momentum\""));
        assert!(text.contains("app_strategy_worker_payload_rejects_total"));
        assert!(text.contains("app_strategy_worker_request_duration_seconds_bucket"));
        assert!(text.contains("le=\"0.005\"} 1\n"));
        assert!(text.contains("le=\"0.01\"} 2\n"));
        assert!(text.contains("le=\"0.5\"} 2\n"));
        assert!(text.contains("le=\"1\"} 3\n"));
        assert!(
            text.contains(
                "app_strategy_worker_in_flight{role=\"live\",strategy=\"live-momentum\"} 0"
            )
        );
        assert!(!text.contains("symbol="));
    }
}
