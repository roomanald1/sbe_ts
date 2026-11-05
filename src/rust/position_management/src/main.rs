#[path = "observability/logging.rs"]
mod logging;
#[path = "../../service_docs.rs"]
mod service_docs;
use axum::routing::get;
#[path = "feeds/feed_client.rs"]
mod feed_client;
mod positions;
#[path = "users/users.rs"]
mod users;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = positions::router()
        .await?
        .merge(service_docs::routes(
            "Position Management",
            &[
                ("/health", &["get"]),
                ("/positions", &["get"]),
                ("/positions/realized-pnl", &["get"]),
                ("/positions/fee-summary", &["get"]),
                ("/positions/account", &["get"]),
                ("/positions/account/topups", &["post"]),
                ("/positions/account/leverage", &["post"]),
                ("/positions/usd-rates", &["get"]),
                ("/positions/fee-rates", &["get"]),
                ("/positions/hedge-candidates", &["get"]),
                ("/positions/trades", &["get"]),
                ("/positions/vwap", &["get", "post"]),
                ("/positions/vwap/{id}", &["delete"]),
                ("/positions/twap", &["post"]),
                ("/positions/barriers", &["get"]),
                ("/positions/barriers/profit-target", &["post"]),
                ("/positions/close-profitable", &["post"]),
                ("/users/{page}", &["get"]),
                ("/account", &["get"]),
                ("/account/topups", &["post"]),
            ],
        ))
        .merge(users::router())
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes));
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
