#[derive(Clone, Deserialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    entry_fees: f64,
    estimated_exit_fee_rate: f64,
}
#[derive(Clone, Deserialize)]
struct BookLevel {
    price: f64,
    size: f64,
}
#[derive(Deserialize)]
struct PositionMarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}
#[derive(Deserialize)]
struct PositionInsightsInput {
    positions: Vec<Position>,
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Deserialize)]
struct PositionList {
    positions: Vec<Position>,
}
#[derive(Deserialize)]
struct MarketInputs {
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Deserialize)]
struct PositionRpcResponse {
    status: u16,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    body: serde_json::Value,
}

fn market_service_clients() -> &'static std::sync::Mutex<HashMap<String, Arc<rsocket_rust::Client>>> {
    static CLIENTS: std::sync::OnceLock<
        std::sync::Mutex<HashMap<String, Arc<rsocket_rust::Client>>>,
    > = std::sync::OnceLock::new();
    CLIENTS.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}
fn parse_service_rpc_response(bytes: &[u8], service: &str) -> Result<serde_json::Value, String> {
    let envelope: PositionRpcResponse = serde_json::from_slice(bytes)
        .map_err(|error| format!("Invalid {service} RSocket response: {error}"))?;
    if !(200..300).contains(&envelope.status) {
        return Err(format!("{service} returned status {}", envelope.status));
    }
    if let Some(content_type) = envelope.headers.get("content-type")
        && !content_type.starts_with("application/json")
    {
        return Err(format!("{service} returned unsupported content type {content_type}"));
    }
    Ok(envelope.body)
}
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum TradeSide {
    Buy,
    Sell,
}
#[derive(Serialize)]
struct CloseInsight {
    symbol: String,
    close_side: TradeSide,
    close_quantity: f64,
    close_price: Option<f64>,
    current_price: Option<f64>,
    breakeven_price: Option<f64>,
    close_pnl: Option<f64>,
    close_fee: Option<f64>,
    depth_covers_position: bool,
    close_depth_quantity: f64,
    profitable_close_probability_5m: Option<f64>,
    probability_samples: usize,
}
#[derive(Serialize)]
struct CloseInsightsResponse {
    insights: Vec<CloseInsight>,
}

async fn connect_market_service(endpoint: &str, service: &str) -> Result<Arc<rsocket_rust::Client>, (StatusCode, String)> {
    if let Some(client) = market_service_clients()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(endpoint)
        .cloned()
    {
        return Ok(client);
    }
    use rsocket_rust::{prelude::*, utils::EchoRSocket};
    use rsocket_rust_transport_websocket::WebsocketClientTransport;

    let connected = Arc::new(RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(endpoint))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("{service} RSocket connection failed: {error}")))?);
    let mut clients = market_service_clients()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(clients
        .entry(endpoint.to_owned())
        .or_insert(connected)
        .clone())
}

fn invalidate_market_service_client(endpoint: &str, failed_client: &Arc<rsocket_rust::Client>) {
    let mut clients = market_service_clients()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if clients
        .get(endpoint)
        .is_some_and(|client| Arc::ptr_eq(client, failed_client))
    {
        clients.remove(endpoint);
    }
}

async fn market_service_get(
    endpoint: &str,
    client: &Arc<rsocket_rust::Client>,
    service: &str,
    path: &str,
) -> Result<serde_json::Value, (StatusCode, String)> {
    use rsocket_rust::prelude::*;

    let payload = serde_json::to_vec(&serde_json::json!({ "method": "GET", "path": path }))
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Could not encode {service} request: {error}")))?;
    let response = match client
        .request_response(Payload::builder().set_data(payload).build())
        .await
    {
        Ok(Some(response)) => response,
        Ok(None) => {
            invalidate_market_service_client(endpoint, client);
            return Err((StatusCode::BAD_GATEWAY, format!("{service} returned an empty RSocket response")));
        }
        Err(error) => {
            invalidate_market_service_client(endpoint, client);
            return Err((StatusCode::BAD_GATEWAY, format!("{service} RSocket request failed: {error}")));
        }
    };
    let data = response.data().ok_or_else(|| (StatusCode::BAD_GATEWAY, format!("{service} returned no response data")))?;
    parse_service_rpc_response(data.as_ref(), service)
        .map_err(|error| (StatusCode::BAD_GATEWAY, error))
}

async fn get_position_insights() -> Result<Json<CloseInsightsResponse>, (StatusCode, String)> {
    let position_endpoint = std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10005".to_owned());
    let position_client = connect_market_service(&position_endpoint, "Position Management").await?;
    let position_body = market_service_get(&position_endpoint, &position_client, "Position Management", "/positions").await?;
    let positions: PositionList = serde_json::from_value(position_body)
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid position list: {error}")))?;
    let mut markets = HashMap::new();
    if positions.positions.iter().any(|position| position.quantity.abs() > 1e-12) {
        let paper_endpoint = std::env::var("PAPER_EXCHANGE_RSOCKET_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:10006".to_owned());
        let paper_client = connect_market_service(&paper_endpoint, "Paper Exchange").await?;
        for position in &positions.positions {
            if position.quantity.abs() <= 1e-12 { continue; }
            let path = format!("/internal/insights-inputs?symbol={}", position.symbol);
            let body = market_service_get(&paper_endpoint, &paper_client, "Paper Exchange", &path).await?;
            let market: MarketInputs = serde_json::from_value(body)
                .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid Paper Exchange market input: {error}")))?;
            markets.extend(market.markets);
        }
    }
    let source = PositionInsightsInput { positions: positions.positions, markets };
    let insights = source
        .positions
        .into_iter()
        .filter(|p| p.quantity.abs() > 1e-12)
        .map(|position| {
            let market = source.markets.get(&position.symbol);
            let closing_long = position.quantity > 0.0;
            let close_side = if closing_long {
                TradeSide::Sell
            } else {
                TradeSide::Buy
            };
            let levels = market
                .map(|m| if closing_long { &m.bids } else { &m.asks })
                .cloned()
                .unwrap_or_default();
            let mut remaining = position.quantity.abs();
            let mut notional: f64 = 0.0;
            for level in &levels {
                let qty = remaining.min(level.size);
                notional += qty * level.price;
                remaining -= qty;
                if remaining <= 1e-12 {
                    break;
                }
            }
            let close_depth_quantity = (position.quantity.abs() - remaining).max(0.0);
            let depth_covers_position = remaining <= 1e-12;
            let current_price = market.and_then(|m| {
                let bid = m.bids.first()?.price;
                let ask = m.asks.first()?.price;
                let mid = bid + (ask - bid) * 0.5;
                (bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid && mid.is_finite())
                    .then_some(mid)
            });
            let close_price = if depth_covers_position && position.quantity.abs() > 1e-12 {
                let average = notional / position.quantity.abs();
                (notional.is_finite() && notional > 0.0 && average.is_finite() && average > 0.0)
                    .then_some(average)
            } else {
                None
            };
            let breakeven_price = {
                let quantity = position.quantity.abs();
                let fee_rate = position.estimated_exit_fee_rate;
                let (numerator, denominator) = if closing_long {
                    (
                        position.average_entry_price * quantity + position.entry_fees,
                        quantity * (1.0 - fee_rate),
                    )
                } else {
                    (
                        position.average_entry_price * quantity - position.entry_fees,
                        quantity * (1.0 + fee_rate),
                    )
                };
                (numerator.is_finite()
                    && denominator.is_finite()
                    && numerator > 0.0
                    && denominator > 0.0)
                    .then_some(numerator / denominator)
                    .filter(|price| price.is_finite() && *price > 0.0)
            };
            let close_fee = close_price
                .map(|price| position.quantity.abs() * price * position.estimated_exit_fee_rate);
            let close_pnl = close_price
                .map(|price| {
                    (price - position.average_entry_price) * position.quantity
                        - position.entry_fees
                        - position.quantity.abs() * price * position.estimated_exit_fee_rate
                })
                .filter(|pnl| pnl.is_finite());
            let samples = market.map(|m| m.price_samples.as_slice()).unwrap_or(&[]);
            let probability = if samples.len() >= 31 && close_price.is_some() {
                let returns: Vec<f64> = samples
                    .windows(2)
                    .filter_map(|pair| {
                        let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
                        (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0)
                            .then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
                    })
                    .collect();
                if returns.len() >= 30 {
                    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                    let variance = returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>()
                        / (returns.len() - 1) as f64;
                    let projected_mean = close_price.unwrap().ln() + mean * 30.0;
                    let projected_sd = (variance * 30.0).sqrt();
                    if projected_sd > 0.0 {
                        let below = normal_cdf(
                            (position.average_entry_price.ln() - projected_mean) / projected_sd,
                        );
                        Some(if closing_long { 1.0 - below } else { below })
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };
            CloseInsight {
                symbol: position.symbol,
                close_side,
                close_quantity: position.quantity.abs(),
                close_price,
                current_price,
                breakeven_price,
                close_pnl,
                close_fee,
                depth_covers_position,
                close_depth_quantity,
                profitable_close_probability_5m: probability,
                probability_samples: samples.len(),
            }
        })
        .collect();
    Ok(Json(CloseInsightsResponse { insights }))
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0
        - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t
            + 0.254829592)
            * t)
            * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}

#[cfg(test)]
mod position_insights_rpc_tests {
    use super::parse_service_rpc_response;

    #[test]
    fn parses_position_management_rpc_envelope() {
        let response = br#"{"status":200,"headers":{"content-type":"application/json"},"body":{"positions":[],"markets":{}}}"#;
        let parsed = parse_service_rpc_response(response, "Position Management").unwrap();
        assert!(parsed["positions"].as_array().unwrap().is_empty());
        assert!(parsed["markets"].as_object().unwrap().is_empty());
    }

    #[test]
    fn rejects_non_success_status_and_unsupported_content_type() {
        let status = br#"{"status":503,"headers":{},"body":{}}"#;
        assert!(parse_service_rpc_response(status, "Paper Exchange")
            .err()
            .unwrap()
            .contains("status 503"));
        let content_type = br#"{"status":200,"headers":{"content-type":"text/plain"},"body":{}}"#;
        assert!(parse_service_rpc_response(content_type, "Paper Exchange")
            .err()
            .unwrap()
            .contains("content type"));
    }
}
