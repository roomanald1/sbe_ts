use axum::{Json, Router, body::Body, http::{Response, StatusCode, header::CONTENT_TYPE}, routing::{get, post}};
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

async fn publish_news_snapshot(Json(snapshot): Json<serde_json::Value>) -> StatusCode {
    let Ok(bytes) = serde_json::to_vec(&snapshot) else { return StatusCode::BAD_REQUEST; };
    if feed_handlers::websocket::publish_news_snapshot(Bytes::from(bytes)) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn publish_signal_snapshot(Json(snapshot): Json<serde_json::Value>) -> StatusCode {
    let Some(symbol) = snapshot.get("symbol").and_then(serde_json::Value::as_str) else {
        return StatusCode::BAD_REQUEST;
    };
    let Ok(bytes) = serde_json::to_vec(&snapshot) else { return StatusCode::BAD_REQUEST; };
    if feed_handlers::websocket::publish_signal_snapshot(symbol, Bytes::from(bytes)) {
        StatusCode::ACCEPTED
    } else {
        StatusCode::NOT_FOUND
    }
}
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
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/instruments", get(get_instruments))
        .route("/internal/signals/snapshot", post(publish_signal_snapshot))
        .route("/internal/news/snapshot", post(publish_news_snapshot));
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
async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat.rsplit_once(')').map(|(_, fields)| fields).unwrap_or("");
    let values: Vec<u64> = fields.split_whitespace().filter_map(|value| value.parse().ok()).collect();
    values.get(11).zip(values.get(12)).map(|(user, system)| (*user + *system) as f64 / 100.0).unwrap_or(0.0).to_string()
}
