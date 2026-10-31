//! Read-only provider ingress. Strategy selection and profitability belong in Insights.
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

mod simulation;
#[derive(Clone)]
struct Feeds {
    client: reqwest::Client,
    binance: String,
    deribit: String,
    ethereum: String,
    cache: Arc<Mutex<BTreeMap<String, (Instant, Value)>>>,
}
#[derive(Serialize, Deserialize, Clone)]
struct RawFeed {
    provider: String,
    requested_at_ms: u64,
    received_at_ms: u64,
    data: Value,
}
type Error = (StatusCode, String);
fn error(e: impl std::fmt::Display) -> Error {
    (StatusCode::BAD_GATEWAY, e.to_string())
}
fn bad(message: &str) -> Error {
    (StatusCode::BAD_REQUEST, message.into())
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn router() -> Router {
    let feeds = Feeds {
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(12))
            .build()
            .expect("provider client"),
        binance: std::env::var("BINANCE_MARKET_DATA_URL")
            .unwrap_or_else(|_| "https://data-api.binance.vision".into()),
        deribit: std::env::var("DERIBIT_MARKET_DATA_URL")
            .unwrap_or_else(|_| "https://www.deribit.com".into()),
        ethereum: std::env::var("ETHEREUM_RPC_URL")
            .unwrap_or_else(|_| "https://ethereum.publicnode.com".into()),
        cache: Arc::new(Mutex::new(BTreeMap::new())),
    };
    Router::new()
        .route("/market-data/alternative/binance", get(binance))
        .route("/market-data/alternative/deribit", get(deribit))
        .route("/market-data/alternative/ethereum", post(ethereum))
        .route(
            "/market-data/alternative/liquidation-simulation",
            post(simulation::simulate),
        )
        .route(
            "/market-data/alternative/staking-simulation",
            post(simulation::staking),
        )
        .route(
            "/market-data/alternative/simulation-capability",
            get(simulation::capability),
        )
        .layer(axum::extract::DefaultBodyLimit::max(256 * 1024))
        .with_state(feeds)
}
async fn cached_get(
    s: &Feeds,
    provider: &str,
    url: reqwest::Url,
    ttl: u64,
) -> Result<Value, Error> {
    let key = url.as_str().to_owned();
    if let Some((created, value)) = s.cache.lock().await.get(&key) {
        if created.elapsed() < Duration::from_secs(ttl) {
            return Ok(value.clone());
        }
    }
    let start = now();
    let response = s
        .client
        .get(url)
        .send()
        .await
        .map_err(error)?
        .error_for_status()
        .map_err(error)?;
    let data = response.json::<Value>().await.map_err(error)?;
    if !data["error"].is_null() {
        return Err(error(format!("{provider}: {}", data["error"])));
    }
    let value = serde_json::to_value(RawFeed {
        provider: provider.into(),
        requested_at_ms: start,
        received_at_ms: now(),
        data,
    })
    .map_err(error)?;
    let mut cache = s.cache.lock().await;
    cache.retain(|_, (at, _)| at.elapsed() < Duration::from_secs(600));
    if cache.len() >= 512 {
        cache.clear();
    }
    cache.insert(key, (Instant::now(), value.clone()));
    Ok(value)
}
const SPOT_SYMBOLS: [&str; 11] = [
    "BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT", "ADAUSDT", "DOGEUSDT", "LTCUSDT",
    "BTCUSDC", "ETHUSDC", "USDCUSDT",
];
async fn binance(
    State(s): State<Feeds>,
    Query(mut q): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, Error> {
    let endpoint = q.remove("endpoint").unwrap_or_else(|| "depth".into());
    if !matches!(endpoint.as_str(), "depth" | "klines") {
        return Err(bad("Unsupported Binance feed"));
    }
    if !q
        .get("symbol")
        .is_some_and(|v| SPOT_SYMBOLS.contains(&v.as_str()))
    {
        return Err(bad("Unsupported spot symbol"));
    }
    let allowed = if endpoint == "depth" {
        vec!["symbol", "limit"]
    } else {
        vec!["symbol", "interval", "limit", "startTime", "endTime"]
    };
    if q.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(bad("Unsupported Binance parameter"));
    }
    let limit = q
        .get("limit")
        .and_then(|n| n.parse::<usize>().ok())
        .unwrap_or(100);
    if limit == 0 || limit > 1000 {
        return Err(bad("Limit must be 1–1000"));
    }
    if endpoint == "klines"
        && !q
            .get("interval")
            .is_some_and(|v| matches!(v.as_str(), "1h" | "1d"))
    {
        return Err(bad("Only hourly/daily klines are supported"));
    }
    let mut url =
        reqwest::Url::parse(&format!("{}/api/v3/{endpoint}", s.binance)).map_err(error)?;
    url.query_pairs_mut().extend_pairs(q.iter());
    Ok(Json(
        cached_get(&s, "binance", url, if endpoint == "depth" { 2 } else { 30 }).await?,
    ))
}
async fn deribit(
    State(s): State<Feeds>,
    Query(mut q): Query<BTreeMap<String, String>>,
) -> Result<Json<Value>, Error> {
    let method = q.remove("method").ok_or(bad("Deribit method required"))?;
    if !matches!(
        method.as_str(),
        "get_instruments"
            | "get_instrument"
            | "get_order_book"
            | "get_volatility_index_data"
            | "get_delivery_prices"
            | "get_index_price"
    ) {
        return Err(bad("Unsupported Deribit public feed"));
    }
    if q.len() > 8 || q.iter().any(|(k, v)| k.len() > 40 || v.len() > 100) {
        return Err(bad("Invalid Deribit parameters"));
    }
    if let Some(v) = q.get("currency") {
        if !matches!(v.as_str(), "BTC" | "USDC") {
            return Err(bad("Unsupported currency"));
        }
    }
    if let Some(v) = q.get("instrument_name") {
        if !(v.starts_with("BTC-") || v.starts_with("BTC_USDC-"))
            || !v
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        {
            return Err(bad("Only BTC options supported"));
        }
    }
    if q.get("count")
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|n| n > 1000)
    {
        return Err(bad("Count exceeds 1000"));
    }
    let mut url =
        reqwest::Url::parse(&format!("{}/api/v2/public/{method}", s.deribit)).map_err(error)?;
    url.query_pairs_mut().extend_pairs(q.iter());
    let ttl = match method.as_str() {
        "get_instruments" | "get_instrument" => 300,
        "get_volatility_index_data" => 30,
        "get_delivery_prices" => 30,
        _ => 2,
    };
    Ok(Json(cached_get(&s, "deribit", url, ttl).await?))
}
fn validate_rpc(v: &Value) -> Result<(), Error> {
    if let Some(batch) = v.as_array() {
        if batch.is_empty() || batch.len() > 100 {
            return Err(bad("RPC batch must contain 1–100 reads"));
        }
        for item in batch {
            validate_rpc(item)?;
        }
        return Ok(());
    }
    let method = v["method"].as_str().ok_or(bad("RPC method required"))?;
    if !matches!(
        method,
        "eth_chainId"
            | "eth_blockNumber"
            | "eth_getBlockByNumber"
            | "eth_getLogs"
            | "eth_call"
            | "eth_gasPrice"
            | "eth_getCode"
            | "web3_sha3"
    ) {
        return Err(bad("Only read-only Ethereum RPC methods are allowed"));
    }
    let params = v["params"]
        .as_array()
        .ok_or(bad("RPC params must be an array"))?;
    if method == "eth_call" {
        if params.len() != 2 {
            return Err(bad(
                "State overrides are available only through the simulation feed",
            ));
        }
        if params[0]["gas"]
            .as_str()
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
            .is_some_and(|n| n > 2_000_000)
        {
            return Err(bad("Read gas limit exceeds 2 million"));
        }
    }
    if method == "eth_getLogs" {
        let from = params
            .first()
            .and_then(|p| p["fromBlock"].as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());
        let to = params
            .first()
            .and_then(|p| p["toBlock"].as_str())
            .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());
        if !matches!((from,to),(Some(a),Some(b)) if b>=a && b-a<=2048) {
            return Err(bad("Log ranges must be explicit and at most 2048 blocks"));
        }
    }
    Ok(())
}
async fn rpc(s: &Feeds, payload: Value) -> Result<Value, Error> {
    s.client
        .post(&s.ethereum)
        .json(&payload)
        .send()
        .await
        .map_err(error)?
        .error_for_status()
        .map_err(error)?
        .json::<Value>()
        .await
        .map_err(error)
}
async fn ethereum(
    State(s): State<Feeds>,
    Json(payload): Json<Value>,
) -> Result<Json<RawFeed>, Error> {
    validate_rpc(&payload)?;
    let start = now();
    let data = rpc(&s, payload).await?;
    Ok(Json(RawFeed {
        provider: "ethereum-mainnet".into(),
        requested_at_ms: start,
        received_at_ms: now(),
        data,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rpc_never_accepts_transaction_submission_or_client_overrides() {
        assert!(validate_rpc(&json!({"method":"eth_sendRawTransaction","params":["0x"]})).is_err());
        assert!(validate_rpc(&json!({"method":"eth_call","params":[{},"latest",{}]})).is_err());
        assert!(
            validate_rpc(
                &json!({"method":"eth_getLogs","params":[{"fromBlock":"0x1","toBlock":"latest"}]})
            )
            .is_err()
        );
        assert!(
            validate_rpc(
                &json!({"method":"eth_getLogs","params":[{"fromBlock":"0x1","toBlock":"0x100"}]})
            )
            .is_ok()
        );
        assert!(validate_rpc(&json!([{"method":"eth_chainId","params":[]},{"method":"eth_sendTransaction","params":[]}])).is_err());
    }

    #[tokio::test]
    async fn cached_raw_feed_keeps_original_acquisition_time() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/",
            get(move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::Relaxed);
                    Json(json!({"bids":[["1","2"]],"asks":[["2","3"]]}))
                }
            }),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let feeds = Feeds {
            client: reqwest::Client::new(),
            binance: String::new(),
            deribit: String::new(),
            ethereum: String::new(),
            cache: Arc::new(Mutex::new(BTreeMap::new())),
        };
        let url = reqwest::Url::parse(&format!("http://{address}/")).unwrap();
        let first = cached_get(&feeds, "test", url.clone(), 30).await.unwrap();
        let second = cached_get(&feeds, "test", url, 30).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::Relaxed), 1);
        assert!(
            first["requested_at_ms"].as_u64().unwrap() <= first["received_at_ms"].as_u64().unwrap()
        );
        task.abort();
    }
}
