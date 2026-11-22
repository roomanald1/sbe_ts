use axum::{Router, extract::State, routing::get};
use rusqlite::{Connection, OpenFlags};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::{
    path::PathBuf,
    time::{Duration, Instant},
};

const LATENCY_BUCKETS_SECONDS: [f64; 8] = [0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0];
const SCOUT_OUTCOME_TTL_MS: u64 = 7 * 86_400_000;

#[derive(Debug)]
pub(crate) struct WorkerMetrics {
    role: &'static str,
    strategy: String,
    scout_state_path: Option<PathBuf>,
    scout_counts_cache: Mutex<Option<(Instant, u64, u64, u64)>>,
    requests: AtomicU64,
    errors: AtomicU64,
    timeouts: AtomicU64,
    payload_rejects: AtomicU64,
    saturation_rejects: AtomicU64,
    in_flight: AtomicUsize,
    insights_connected: AtomicUsize,
    last_insight_timestamp_ms: AtomicU64,
    last_insight_received_at_ms: AtomicU64,
    insight_update_failures: AtomicU64,
    insight_update_timeouts: AtomicU64,
    decision_report_successes: AtomicU64,
    decision_report_failures: AtomicU64,
    last_decision_report_timestamp_ms: AtomicU64,
    profile_updates: AtomicU64,
    profile_update_failures: AtomicU64,
    last_profile_update_timestamp_ms: AtomicU64,
    profile_set_version: AtomicU64,
    latency_count: AtomicU64,
    latency_sum_ns: AtomicU64,
    latency_buckets: [AtomicU64; 8],
    insight_route_latency_count: AtomicU64,
    insight_route_latency_sum_ns: AtomicU64,
    insight_route_latency_buckets: [AtomicU64; 8],
}

impl WorkerMetrics {
    pub(crate) fn new(
        role: &'static str,
        strategy: impl Into<String>,
        scout_state_path: Option<PathBuf>,
    ) -> Arc<Self> {
        Arc::new(Self {
            role,
            strategy: strategy.into(),
            scout_state_path,
            scout_counts_cache: Mutex::new(None),
            requests: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            timeouts: AtomicU64::new(0),
            payload_rejects: AtomicU64::new(0),
            saturation_rejects: AtomicU64::new(0),
            in_flight: AtomicUsize::new(0),
            insights_connected: AtomicUsize::new(0),
            last_insight_timestamp_ms: AtomicU64::new(0),
            last_insight_received_at_ms: AtomicU64::new(0),
            insight_update_failures: AtomicU64::new(0),
            insight_update_timeouts: AtomicU64::new(0),
            decision_report_successes: AtomicU64::new(0),
            decision_report_failures: AtomicU64::new(0),
            last_decision_report_timestamp_ms: AtomicU64::new(0),
            profile_updates: AtomicU64::new(0),
            profile_update_failures: AtomicU64::new(0),
            last_profile_update_timestamp_ms: AtomicU64::new(0),
            profile_set_version: AtomicU64::new(0),
            latency_count: AtomicU64::new(0),
            latency_sum_ns: AtomicU64::new(0),
            latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
            insight_route_latency_count: AtomicU64::new(0),
            insight_route_latency_sum_ns: AtomicU64::new(0),
            insight_route_latency_buckets: std::array::from_fn(|_| AtomicU64::new(0)),
        })
    }

    pub(crate) fn begin(&self) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        self.in_flight.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn set_insights_connected(&self, connected: bool) {
        self.insights_connected
            .store(usize::from(connected), Ordering::Relaxed);
    }

    pub(crate) fn record_insight_update(&self, timestamp_ms: u64) {
        self.last_insight_timestamp_ms
            .store(timestamp_ms, Ordering::Relaxed);
        let received_at_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u64::MAX as u128) as u64;
        self.last_insight_received_at_ms
            .store(received_at_ms, Ordering::Relaxed);
    }

    pub(crate) fn record_insight_update_failure(&self, timed_out: bool) {
        self.insight_update_failures.fetch_add(1, Ordering::Relaxed);
        if timed_out {
            self.insight_update_timeouts.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_decision_report(&self, timestamp_ms: u64, succeeded: bool) {
        if succeeded {
            self.decision_report_successes
                .fetch_add(1, Ordering::Relaxed);
            self.last_decision_report_timestamp_ms
                .store(timestamp_ms, Ordering::Relaxed);
        } else {
            self.decision_report_failures
                .fetch_add(1, Ordering::Relaxed);
        }
    }

    pub(crate) fn record_profile_update(&self, timestamp_ms: u64) {
        self.profile_updates.fetch_add(1, Ordering::Relaxed);
        self.last_profile_update_timestamp_ms
            .store(timestamp_ms, Ordering::Relaxed);
    }

    pub(crate) fn record_profile_update_failure(&self) {
        self.profile_update_failures.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_insight_route_latency(&self, elapsed_ns: u64) {
        self.insight_route_latency_count
            .fetch_add(1, Ordering::Relaxed);
        self.insight_route_latency_sum_ns
            .fetch_add(elapsed_ns, Ordering::Relaxed);
        let elapsed = elapsed_ns as f64 / 1_000_000_000.0;
        for (index, bound) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            if elapsed <= *bound {
                self.insight_route_latency_buckets[index].fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    pub(crate) fn set_profile_set_version(&self, version: u64) {
        self.profile_set_version.store(version, Ordering::Relaxed);
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
        output.push_str("# HELP app_strategy_worker_insights_connected Whether the worker has an active Insights subscription.\n# TYPE app_strategy_worker_insights_connected gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_insights_connected",
            &labels,
            self.insights_connected.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_last_insight_timestamp_ms Source event timestamp of the most recent Insights update observed by the worker.\n# TYPE app_strategy_worker_last_insight_timestamp_ms gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_last_insight_timestamp_ms",
            &labels,
            self.last_insight_timestamp_ms.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_last_insight_received_at_ms Local timestamp when the most recent Insights update was received by the worker.\n# TYPE app_strategy_worker_last_insight_received_at_ms gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_last_insight_received_at_ms",
            &labels,
            self.last_insight_received_at_ms.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_insight_update_failures_total Insights updates that failed during evaluation or reporting.\n# TYPE app_strategy_worker_insight_update_failures_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_insight_update_failures_total",
            &labels,
            self.insight_update_failures.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_insight_update_timeouts_total Insights updates that timed out during evaluation or reporting.\n# TYPE app_strategy_worker_insight_update_timeouts_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_insight_update_timeouts_total",
            &labels,
            self.insight_update_timeouts.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_decision_reports_total Decision reports accepted by AutoTrader.\n# TYPE app_strategy_worker_decision_reports_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_decision_reports_total",
            &labels,
            self.decision_report_successes.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_decision_report_failures_total Decision reports that failed to reach AutoTrader.\n# TYPE app_strategy_worker_decision_report_failures_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_decision_report_failures_total",
            &labels,
            self.decision_report_failures.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_last_decision_report_timestamp_ms Timestamp of the most recent decision report accepted by AutoTrader.\n# TYPE app_strategy_worker_last_decision_report_timestamp_ms gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_last_decision_report_timestamp_ms",
            &labels,
            self.last_decision_report_timestamp_ms
                .load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_profile_updates_total Profile sets pushed by AutoTrader and accepted by the worker.\n# TYPE app_strategy_worker_profile_updates_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_profile_updates_total",
            &labels,
            self.profile_updates.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_profile_update_failures_total Invalid pushed profile sets rejected by the worker.\n# TYPE app_strategy_worker_profile_update_failures_total counter\n");
        metric(
            &mut output,
            "app_strategy_worker_profile_update_failures_total",
            &labels,
            self.profile_update_failures.load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_last_profile_update_timestamp_ms Timestamp of the most recent profile set pushed by AutoTrader and accepted by the worker.\n# TYPE app_strategy_worker_last_profile_update_timestamp_ms gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_last_profile_update_timestamp_ms",
            &labels,
            self.last_profile_update_timestamp_ms
                .load(Ordering::Relaxed),
        );
        output.push_str("# HELP app_strategy_worker_profile_set_version AutoTrader profile-set version currently installed by a live worker.\n# TYPE app_strategy_worker_profile_set_version gauge\n");
        metric(
            &mut output,
            "app_strategy_worker_profile_set_version",
            &labels,
            self.profile_set_version.load(Ordering::Relaxed),
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
        output.push_str("# HELP app_strategy_worker_insight_route_duration_seconds Time spent dispatching an Insights update through the local Scout handler.\n# TYPE app_strategy_worker_insight_route_duration_seconds histogram\n");
        for (index, bound) in LATENCY_BUCKETS_SECONDS.iter().enumerate() {
            let cumulative = self.insight_route_latency_buckets[index].load(Ordering::Relaxed);
            output.push_str(&format!("app_strategy_worker_insight_route_duration_seconds_bucket{{{labels},le=\"{bound}\"}} {cumulative}\n"));
        }
        output.push_str(&format!(
            "app_strategy_worker_insight_route_duration_seconds_bucket{{{labels},le=\"+Inf\"}} {}\n",
            self.insight_route_latency_count.load(Ordering::Relaxed)
        ));
        output.push_str(&format!(
            "app_strategy_worker_insight_route_duration_seconds_sum{{{labels}}} {:.9}\n",
            self.insight_route_latency_sum_ns.load(Ordering::Relaxed) as f64 / 1_000_000_000.0
        ));
        output.push_str(&format!(
            "app_strategy_worker_insight_route_duration_seconds_count{{{labels}}} {}\n",
            self.insight_route_latency_count.load(Ordering::Relaxed)
        ));
        if self.role == "scout"
            && let Some((open, closed, unavailable)) = self.scout_instance_counts()
        {
            output.push_str("# HELP app_scout_instances Scout instances in the current persisted state, by lifecycle status. Unresolved instances older than seven days are unavailable, not active.\n# TYPE app_scout_instances gauge\n");
            metric(
                &mut output,
                "app_scout_instances",
                &format!("{labels},status=\"open\""),
                open,
            );
            metric(
                &mut output,
                "app_scout_instances",
                &format!("{labels},status=\"closed\""),
                closed,
            );
            metric(
                &mut output,
                "app_scout_instances",
                &format!("{labels},status=\"unavailable\""),
                unavailable,
            );
        }
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

    fn scout_instance_counts(&self) -> Option<(u64, u64, u64)> {
        let path = self.scout_state_path.as_ref()?;
        let mut cache = self.scout_counts_cache.lock().ok()?;
        if let Some((updated, open, closed, unavailable)) = cache.as_ref()
            && updated.elapsed() < Duration::from_secs(60)
        {
            return Some((*open, *closed, *unavailable));
        }
        let counts = read_scout_instance_counts(path)?;
        *cache = Some((Instant::now(), counts.0, counts.1, counts.2));
        Some(counts)
    }
}

fn read_scout_instance_counts(path: &std::path::Path) -> Option<(u64, u64, u64)> {
    let connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).ok()?;
    connection.busy_timeout(Duration::from_millis(250)).ok()?;
    connection
        .execute_batch("PRAGMA query_only=ON; PRAGMA cache_size=-512; PRAGMA mmap_size=0; PRAGMA temp_store=FILE;")
        .ok()?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64;
    let open_since_ms = now_ms.saturating_sub(SCOUT_OUTCOME_TTL_MS);
    let (open, closed, unavailable): (i64, i64, i64) = connection
        .query_row(
            "SELECT
                COALESCE(SUM(CASE WHEN json_extract(payload, '$.scout_net_return_percent') IS NULL
                    AND COALESCE(CAST(json_extract(payload, '$.timestamp') AS INTEGER), CAST(json_extract(payload, '$.signal_timestamp') AS INTEGER), 0) >= ?1
                    AND (COALESCE(json_extract(payload, '$.strategy'), 'unattributed') NOT LIKE 'live-%'
                         OR COALESCE(json_extract(payload, '$.scout_entry_price'), json_extract(payload, '$.price'), json_extract(payload, '$.strategy_decision.observed_close')) IS NOT NULL)
                    THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN json_extract(payload, '$.scout_net_return_percent') IS NOT NULL THEN 1 ELSE 0 END), 0),
                COALESCE(SUM(CASE WHEN json_extract(payload, '$.scout_net_return_percent') IS NULL
                    AND (COALESCE(CAST(json_extract(payload, '$.timestamp') AS INTEGER), CAST(json_extract(payload, '$.signal_timestamp') AS INTEGER), 0) < ?1
                         OR (COALESCE(json_extract(payload, '$.strategy'), 'unattributed') LIKE 'live-%'
                             AND COALESCE(json_extract(payload, '$.scout_entry_price'), json_extract(payload, '$.price'), json_extract(payload, '$.strategy_decision.observed_close')) IS NULL))
                    THEN 1 ELSE 0 END), 0)
             FROM scout_records
             WHERE collection='scout_instances'
               AND COALESCE(json_extract(payload, '$.symbol'), '') != ''
               AND COALESCE(json_extract(payload, '$.direction'), 'unknown') IN ('bullish', 'bearish', 'unknown')",
            [open_since_ms as i64],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .ok()?;
    let orphaned_outcomes: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM scout_records AS outcome
             WHERE outcome.collection='outcomes'
               AND NOT EXISTS (
                   SELECT 1 FROM scout_records AS instance
                   WHERE instance.collection='scout_instances'
                     AND instance.record_key=json_extract(outcome.payload, '$.decision_id')
               )",
            [],
            |row| row.get(0),
        )
        .ok()?;
    let completed = closed.max(0).saturating_add(orphaned_outcomes.max(0));
    Some((
        open.max(0) as u64,
        completed as u64,
        unavailable.max(0) as u64,
    ))
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
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn scout_metrics_separate_active_completed_and_stale_rows() {
        let path =
            std::env::temp_dir().join(format!("scout-metric-counts-{}.sqlite", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE scout_records(collection TEXT NOT NULL, record_key TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(collection,record_key));",
            )
            .unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let payloads = [
            (
                "scout_instances",
                "open",
                serde_json::json!({"decision_id":"open","symbol":"BTC-USDT","strategy":"live-momentum","direction":"bullish","timestamp":now-60_000,"scout_entry_price":100.0}),
            ),
            (
                "scout_instances",
                "closed",
                serde_json::json!({"decision_id":"closed","symbol":"BTC-USDT","strategy":"live-momentum","direction":"bullish","timestamp":now-60_000,"scout_entry_price":100.0,"scout_net_return_percent":0.5}),
            ),
            (
                "scout_instances",
                "old-open",
                serde_json::json!({"decision_id":"old-open","symbol":"BTC-USDT","strategy":"live-momentum","direction":"bullish","timestamp":now-30*86_400_000,"scout_entry_price":100.0}),
            ),
            (
                "outcomes",
                "orphan|strategy|single",
                serde_json::json!({"decision_id":"orphan","symbol":"BTC-USDT","strategy":"live-momentum","signal_timestamp_ms":now-60_000,"evaluated_timestamp_ms":now,"exit_scenario":"strategy","net_return_percent":0.1}),
            ),
        ];
        for (collection, key, payload) in payloads {
            connection
                .execute(
                    "INSERT INTO scout_records VALUES (?1,?2,?3)",
                    rusqlite::params![collection, key, payload.to_string()],
                )
                .unwrap();
        }
        drop(connection);

        assert_eq!(read_scout_instance_counts(&path), Some((1, 2, 1)));
        let metrics = WorkerMetrics::new("scout", "live-momentum", Some(path.clone()));
        let text = metrics.render();
        assert!(text.contains(
            "app_scout_instances{role=\"scout\",strategy=\"live-momentum\",status=\"open\"} 1"
        ));
        assert!(text.contains(
            "app_scout_instances{role=\"scout\",strategy=\"live-momentum\",status=\"closed\"} 2"
        ));
        assert!(text.contains(
            "app_scout_instances{role=\"scout\",strategy=\"live-momentum\",status=\"unavailable\"} 1"
        ));
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn exports_bounded_role_strategy_metrics_and_histogram() {
        let metrics = WorkerMetrics::new("live", "live-momentum", None);
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

    #[test]
    fn exports_insight_route_latency_histogram() {
        let metrics = WorkerMetrics::new("scout", "live-momentum", None);
        metrics.record_insight_route_latency(2_000_000);
        metrics.record_insight_route_latency(400_000_000);

        let text = metrics.render();
        assert!(text.contains("app_strategy_worker_insight_route_duration_seconds_bucket"));
        assert!(text.contains("le=\"0.005\"} 1\n"));
        assert!(text.contains("le=\"0.5\"} 2\n"));
        assert!(text.contains(
            "app_strategy_worker_insight_route_duration_seconds_count{role=\"scout\",strategy=\"live-momentum\"} 2"
        ));
    }

    #[test]
    fn exports_live_report_failures_and_last_successful_report_timestamp() {
        let metrics = WorkerMetrics::new("live", "live-momentum", None);
        metrics.record_decision_report(1_700_000_000_000, true);
        metrics.record_decision_report(1_700_000_001_000, false);
        metrics.record_insight_update_failure(true);
        metrics.record_profile_update(1_700_000_002_000);
        metrics.record_profile_update_failure();

        let text = metrics.render();
        assert!(text.contains(
            "app_strategy_worker_decision_reports_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_decision_report_failures_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_insight_update_failures_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_insight_update_timeouts_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_last_decision_report_timestamp_ms{role=\"live\",strategy=\"live-momentum\"} 1700000000000"
        ));
        assert!(text.contains(
            "app_strategy_worker_profile_updates_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_profile_update_failures_total{role=\"live\",strategy=\"live-momentum\"} 1"
        ));
        assert!(text.contains(
            "app_strategy_worker_last_profile_update_timestamp_ms{role=\"live\",strategy=\"live-momentum\"} 1700000002000"
        ));
    }
}
