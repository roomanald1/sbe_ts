use ::auto_trader::auto_trader;
#[path = "../../service_docs.rs"]
mod service_docs;

use axum::routing::get;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = auto_trader::router()
        .merge(service_docs::routes(
            "AutoTrader",
            &[
                ("/health", &["get"]),
                ("/positions/auto-trader", &["get", "put"]),
                ("/positions/auto-trader/settings", &["put"]),
                ("/positions/auto-trader/dated-carry", &["get", "put"]),
                ("/positions/auto-trader/parameters", &["get"]),
                ("/positions/auto-trader/activity", &["get"]),
                ("/positions/auto-trader/learning-summary", &["get"]),
            ],
        ))
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3005)).await?;
    ::auto_trader::service_log!(
        "INFO",
        "AutoTrader service listening on http://0.0.0.0:3005"
    );
    axum::serve(listener, app).await?;
    Ok(())
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
