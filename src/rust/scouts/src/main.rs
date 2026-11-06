#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = std::env::var("SCOUTS_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3011".into());
    let learning_url =
        std::env::var("LEARNING_SERVICE_URL").unwrap_or_else(|_| "http://127.0.0.1:3010".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Scouts service listening on http://{address}");
    axum::serve(
        listener,
        scouts::router(learning_url).merge(service_docs::routes(
            "Scouts",
            &[
                ("/health", &["get"]),
                ("/process-cpu-seconds", &["get"]),
                ("/process-memory-bytes", &["get"]),
                ("/configuration", &["get", "post"]),
                ("/status", &["get"]),
                ("/outcomes", &["get", "post"]),
                ("/samples", &["post"]),
                ("/scout-outcome", &["post"]),
                ("/scout-markout", &["post"]),
                ("/scout-plan", &["post"]),
                ("/daily-inputs", &["post"]),
                ("/live-inputs", &["post"]),
                ("/dated-carry-inputs", &["post"]),
                ("/scout-lifecycle/state", &["get", "post"]),
                ("/scout-lifecycle/observe", &["post"]),
                ("/variants", &["post"]),
                ("/variants/batch", &["post"]),
                ("/live-candidates", &["post"]),
                ("/select-entry", &["post"]),
            ],
        )),
    )
    .await?;
    Ok(())
}
