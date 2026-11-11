use super::*;

pub(super) async fn health() -> &'static str {
    "ok"
}

pub(super) async fn process_cpu_seconds() -> String {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return "0".into();
    }
    let usage = unsafe { usage.assume_init() };
    (usage.ru_utime.tv_sec as f64
        + usage.ru_utime.tv_usec as f64 / 1_000_000.0
        + usage.ru_stime.tv_sec as f64
        + usage.ru_stime.tv_usec as f64 / 1_000_000.0)
        .to_string()
}

pub(super) async fn process_memory_bytes() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| {
            status.lines().find_map(|line| {
                line.strip_prefix("VmRSS:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        })
        .map(|kilobytes| (kilobytes * 1024).to_string())
        .unwrap_or_else(|| "0".into())
}

pub(super) fn router() -> Router {
    Router::new()
        .merge(crate::service_docs::routes(
            "Live Signals",
            &[("/health", &["get"])],
        ))
        .route("/health", get(health))
        .route(
            "/internal/latency",
            get(|| async { Json(crate::latency::snapshot()) }),
        )
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;

    #[tokio::test]
    async fn health_route_returns_ok() {
        let response = router()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/health")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
    }

    #[tokio::test]
    async fn process_resource_handlers_return_numeric_values() {
        let cpu = process_cpu_seconds().await.parse::<f64>().unwrap();
        let memory = process_memory_bytes().await.parse::<u64>().unwrap();
        assert!(cpu.is_finite() && cpu >= 0.0);
        assert!(memory > 0);
    }
}
