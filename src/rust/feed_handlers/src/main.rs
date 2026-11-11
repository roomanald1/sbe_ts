#[path = "../../service_docs.rs"]
mod service_docs;
use axum::{Json, Router, routing::get};
async fn serve_http() -> anyhow::Result<()> {
    let app = Router::new()
        .merge(service_docs::routes(
            "Feed Handlers",
            &[("/health", &["get"])],
        ))
        .route("/health", get(|| async { "ok" }))
        .route("/internal/message-counts", get(message_counts))
        .route("/internal/queue-metrics", get(queue_metrics))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3001)).await?;
    feed_handlers::service_log!(
        "INFO",
        "FeedHandlers HTTP API listening on http://0.0.0.0:3001"
    );
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("PORT").is_none() {
        unsafe {
            std::env::set_var("PORT", "10001");
        }
    }
    tokio::try_join!(feed_handlers::websocket::serve(), serve_http(),)?;
    Ok(())
}
async fn message_counts() -> Json<serde_json::Value> {
    let (trades, books, tickers) = feed_handlers::metrics::snapshot();
    Json(serde_json::json!({"trades": trades, "books": books, "tickers": tickers}))
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

async fn queue_metrics() -> Json<Vec<serde_json::Value>> {
    Json(feed_handlers::metrics::queue_snapshot().into_iter().map(|(queue, depth, dropped)| serde_json::json!({"queue": queue, "depth": depth, "dropped": dropped})).collect())
}
