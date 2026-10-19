use axum::{Json, Router, routing::get};
use serde::Serialize;

#[derive(Serialize)]
struct InstrumentsResponse {
    symbols: Vec<String>,
    instruments: Vec<feed_handlers::okx::connectivity::SpotInstrumentPrecision>,
}

async fn get_instruments() -> Json<InstrumentsResponse> {
    let symbols = match feed_handlers::okx::connectivity::get_all_spot_instruments().await {
        Ok(symbols) => symbols,
        Err(error) => {
            eprintln!("Unable to load full spot instrument list: {error:#}; using ticker symbols");
            feed_handlers::okx::connectivity::get_instrument_data().await.unwrap_or_default()
        }
    };
    let instruments = feed_handlers::okx::connectivity::get_spot_instrument_precisions()
        .await
        .unwrap_or_default();
    Json(InstrumentsResponse { symbols, instruments })
}

async fn serve_http() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/instruments", get(get_instruments));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3001)).await?;
    println!("FeedHandlers HTTP API listening on http://0.0.0.0:3001");
    axum::serve(listener, app).await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("PORT").is_none() {
        unsafe { std::env::set_var("PORT", "10001"); }
    }
    tokio::try_join!(
        feed_handlers::websocket::serve(),
        serve_http(),
    )?;
    Ok(())
}