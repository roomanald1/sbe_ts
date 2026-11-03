mod alternative_market_data;
use axum::{
    Json, Router,
    body::Body,
    extract::Query,
    http::{Response, StatusCode, header::CONTENT_TYPE},
    response::IntoResponse,
    routing::{get, post},
};
use bytes::Bytes;
use futures_util::future::join_all;
use serde::Serialize;
use std::{sync::OnceLock, time::Duration};
use tokio::sync::Mutex;
use tokio::time::Instant;

#[derive(Serialize)]
struct InstrumentsResponse {
    symbols: Vec<String>,
    instruments: Vec<feed_handlers::okx::connectivity::SpotInstrumentPrecision>,
}

#[derive(Serialize)]
struct DailyCandle {
    close_time_ms: u64,
    high: f64,
    close: f64,
}

async fn get_daily_candles(
    Query(query): Query<std::collections::BTreeMap<String, String>>,
) -> Response<Body> {
    let Some(symbols) = query.get("symbols") else {
        return (
            StatusCode::BAD_REQUEST,
            "symbols query parameter is required",
        )
            .into_response();
    };
    let pairs = [
        ("BTC-USDT", "BTCUSDT"),
        ("ETH-USDT", "ETHUSDT"),
        ("BNB-USDT", "BNBUSDT"),
        ("XRP-USDT", "XRPUSDT"),
        ("SOL-USDT", "SOLUSDT"),
        ("ADA-USDT", "ADAUSDT"),
        ("DOGE-USDT", "DOGEUSDT"),
        ("LTC-USDT", "LTCUSDT"),
        ("TRX-USDT", "TRXUSDT"),
        ("LINK-USDT", "LINKUSDT"),
        ("BCH-USDT", "BCHUSDT"),
        ("XLM-USDT", "XLMUSDT"),
        ("ETC-USDT", "ETCUSDT"),
        ("FIL-USDT", "FILUSDT"),
        ("AVAX-USDT", "AVAXUSDT"),
        ("DOT-USDT", "DOTUSDT"),
    ];
    let mut requested = Vec::new();
    for symbol in symbols.split(',') {
        let Some((_, pair)) = pairs
            .iter()
            .find(|(name, pair)| *name == symbol || *pair == symbol)
        else {
            return (
                StatusCode::BAD_REQUEST,
                format!("unsupported symbol: {symbol}"),
            )
                .into_response();
        };
        requested.push((symbol.to_owned(), (*pair).to_owned()));
    }
    if requested.is_empty() {
        return (StatusCode::BAD_REQUEST, "at least one symbol is required").into_response();
    }
    let client = DAILY_CANDLE_CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone();
    let responses = join_all(requested.into_iter().map(|(symbol, pair)| {
        let client = client.clone();
        async move {
            let url = format!(
                "https://data-api.binance.vision/api/v3/klines?symbol={pair}&interval=1d&limit=260"
            );
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            let rows = response
                .json::<Vec<Vec<serde_json::Value>>>()
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((symbol, rows))
        }
    }))
    .await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut result = serde_json::Map::new();
    for response in responses {
        let (symbol, rows) = match response {
            Ok(value) => value,
            Err(error) => return (StatusCode::BAD_GATEWAY, error).into_response(),
        };
        let candles = rows
            .into_iter()
            .filter_map(|row| {
                let close_time_ms = row.get(6)?.as_u64()?;
                if close_time_ms >= now_ms {
                    return None;
                }
                let high = row.get(2)?.as_str()?.parse::<f64>().ok()?;
                let close = row.get(4)?.as_str()?.parse::<f64>().ok()?;
                (high.is_finite() && high > 0.0 && close.is_finite() && close > 0.0).then_some(
                    DailyCandle {
                        close_time_ms,
                        high,
                        close,
                    },
                )
            })
            .collect::<Vec<_>>();
        result.insert(
            symbol,
            serde_json::to_value(candles).expect("candle serialization"),
        );
    }
    json_response(Bytes::from(
        serde_json::to_vec(&result).expect("candle map serialization"),
    ))
}

static INSTRUMENTS_CACHE: OnceLock<Mutex<Option<(Instant, Bytes)>>> = OnceLock::new();
static DAILY_CANDLE_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

async fn publish_news_snapshot(Json(snapshot): Json<serde_json::Value>) -> StatusCode {
    let Ok(bytes) = serde_json::to_vec(&snapshot) else {
        return StatusCode::BAD_REQUEST;
    };
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
    let Ok(bytes) = serde_json::to_vec(&snapshot) else {
        return StatusCode::BAD_REQUEST;
    };
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
            feed_handlers::service_log!(
                "ERROR",
                "Unable to load full spot instrument list: {error:#}; using ticker symbols"
            );
            feed_handlers::okx::connectivity::get_instrument_data()
                .await
                .unwrap_or_default()
        }
    };
    let instruments = feed_handlers::okx::connectivity::get_spot_instrument_precisions()
        .await
        .unwrap_or_default();
    let response = InstrumentsResponse {
        symbols,
        instruments,
    };
    let body = Bytes::from(
        serde_json::to_vec(&response).expect("instrument response serialization failed"),
    );
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
        .merge(alternative_market_data::router())
        .route("/health", get(|| async { "ok" }))
        .route("/internal/message-counts", get(message_counts))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
        .route("/instruments", get(get_instruments))
        .route("/market-data/daily-candles", get(get_daily_candles))
        .route("/internal/signals/snapshot", post(publish_signal_snapshot))
        .route("/internal/news/snapshot", post(publish_news_snapshot));
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
