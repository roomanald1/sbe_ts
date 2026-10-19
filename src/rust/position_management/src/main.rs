use axum::routing::get;
mod feed_client;
mod positions;
mod users;
mod auto_trader;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = positions::router().await?.merge(users::router()).merge(auto_trader::router()).route("/health", get(|| async { "ok" }));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3002)).await?;
    println!("Position Management service listening on http://0.0.0.0:3002");
    axum::serve(listener, app).await?;
    Ok(())
}