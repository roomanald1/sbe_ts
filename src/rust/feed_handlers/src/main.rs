use axum::{Router, body::Body, http::{Response, header::CONTENT_TYPE}, routing::get};
use bytes::Bytes;
use serde::Serialize;
use std::{sync::OnceLock, time::Duration};
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Serialize)]
struct InstrumentsResponse {
    symbols: Vec<String>,
    instruments: Vec<feed_handlers::okx::connectivity::SpotInstrumentPrecision>,
}

static INSTRUMENTS_CACHE: OnceLock<Mutex<Option<(Instant, Bytes)>>> = OnceLock::new();
const INSTRUMENTS_CACHE_TTL: Duration = Duration::from_secs(300);

async fn get_instruments() -> Response<Body> {
    let cache = INSTRUMENTS_CACHE.get_or_init(|| Mutex::new(None));
    let mut cached = cache.lock().await;
    if let Some((created, response)) = cached.as_ref() {
        if created.elapsed() < INSTRUMENTS_CACHE_TTL {
            return json_response(response.clone());
        }
    }
    let symbols = match feed_handlers::okx::connectivity::get_all_spot_instruments().await {
        Ok(symbols) => symbols,
        Err(error) => {
            feed_handlers::service_log!("ERROR", "Unable to load full spot instrument list: {error:#}; using ticker symbols");
            feed_handlers::okx::connectivity::get_instrument_data().await.unwrap_or_default()
        }
    };
    let instruments = feed_handlers::okx::connectivity::get_spot_instrument_precisions()
        .await
        .unwrap_or_default();
    let response = InstrumentsResponse { symbols, instruments };
    let body = Bytes::from(serde_json::to_vec(&response).expect("instrument response serialization failed"));
    *cached = Some((Instant::now(), body.clone()));
    json_response(body)
}

fn json_response(body: Bytes) -> Response<Body> {
    Response::builder()
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .expect("valid instrument response")
}

async fn serve_http() -> anyhow::Result<()> {
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/instruments", get(get_instruments));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3001)).await?;
    feed_handlers::service_log!("INFO", "FeedHandlers HTTP API listening on http://0.0.0.0:3001");
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