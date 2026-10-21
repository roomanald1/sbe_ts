mod logging;
use axum::routing::get;
mod feed_client;
mod positions;
mod users;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = positions::router().await?.merge(users::router()).route("/health", get(|| async { "ok" })).route("/process-cpu-seconds", get(process_cpu_seconds));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3002)).await?;
    crate::service_log!("INFO", "Position Management service listening on http://0.0.0.0:3002");
    axum::serve(listener, app).await?;
    Ok(())
}
async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat.rsplit_once(')').map(|(_, fields)| fields).unwrap_or("");
    let values: Vec<u64> = fields.split_whitespace().filter_map(|value| value.parse().ok()).collect();
    values.get(11).zip(values.get(12)).map(|(user, system)| (*user + *system) as f64 / 100.0).unwrap_or(0.0).to_string()
}
