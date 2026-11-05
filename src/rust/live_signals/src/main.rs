#[path = "../../insights/src/observability/latency.rs"]
mod latency;
mod live_signals;
#[path = "../../insights/src/observability/logging.rs"]
mod logging;
#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    live_signals::run().await
}
