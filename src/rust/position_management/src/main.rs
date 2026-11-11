#[path = "feeds/feed_client.rs"]
mod feed_client;
#[path = "observability/logging.rs"]
mod logging;
mod positions;
#[path = "../../service_docs.rs"]
mod service_docs;
#[path = "users/users.rs"]
mod users;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _internal_position_router = positions::router().await?;
    let app = axum::Router::new()
        .merge(service_docs::routes(
            "Position Management",
            &[("/health", &["get"])],
        ))
        .route("/health", axum::routing::get(|| async { "ok" }))
        .route(
            "/process-cpu-seconds",
            axum::routing::get(process_cpu_seconds),
        )
        .route(
            "/process-memory-bytes",
            axum::routing::get(process_memory_bytes),
        );
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3002)).await?;
    crate::service_log!(
        "INFO",
        "Position Management service listening on http://0.0.0.0:3002"
    );
    axum::serve(listener, app).await?;
    Ok(())
}
async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat
        .rsplit_once(')')
        .map(|(_, fields)| fields)
        .unwrap_or("");
    let values: Vec<u64> = fields
        .split_whitespace()
        .filter_map(|value| value.parse().ok())
        .collect();
    values
        .get(11)
        .zip(values.get(12))
        .map(|(user, system)| (*user + *system) as f64 / 100.0)
        .unwrap_or(0.0)
        .to_string()
}

//TODO put this in and the cpu func above in a lib crate and reuse in all services
async fn process_memory_bytes() -> String {
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
        .unwrap_or_else(|| "0".to_owned())
}
