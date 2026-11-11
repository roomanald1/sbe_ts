#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if let Ok(strategy_name) = std::env::var("SCOUT_WORKER_STRATEGY") {
        let strategy = strategies::Strategy::from_name(&strategy_name)
            .ok_or_else(|| anyhow::anyhow!("unknown SCOUT_WORKER_STRATEGY {strategy_name}"))?;
        return scouts::serve_strategy_worker(scouts::ScoutWorkerConfig::from_env(strategy)?).await;
    }
    let address = std::env::var("SCOUTS_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3011".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Scouts service listening on http://{address}");
    axum::serve(
        listener,
        scouts::router().merge(service_docs::routes(
            "Scouts",
            &[
                ("/health", &["get"]),
                ("/process-cpu-seconds", &["get"]),
                ("/process-memory-bytes", &["get"]),
            ],
        )),
    )
    .await?;
    Ok(())
}

// Learning owns selection and scheduling. This process either serves the
// existing public Scout API or, when SCOUT_WORKER_STRATEGY is configured,
// runs as one RSocket-only strategy worker.
