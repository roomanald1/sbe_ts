#[path = "../../service_docs.rs"]
mod service_docs;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let address = std::env::var("LEARNING_LISTEN_ADDR").unwrap_or_else(|_| "127.0.0.1:3010".into());
    let path = std::env::var("LEARNING_STATE_FILE")
        .map(Into::into)
        .unwrap_or_else(|_| "learning_state.json".into());
    let listener = tokio::net::TcpListener::bind(&address).await?;
    println!("Learning service listening on http://{address}");
    let app = learning::router(path)
        .map_err(anyhow::Error::msg)?
        .merge(service_docs::routes(
            "Learning",
            &[
                ("/health", &["get"]),
                ("/profiles", &["get", "post"]),
                ("/summary", &["get"]),
                ("/feedback", &["post"]),
            ],
        ));
    axum::serve(listener, app).await?;
    Ok(())
}
