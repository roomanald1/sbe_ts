mod logging;
use axum::{Router, body::Body, extract::State, http::{Request, StatusCode, header::{CONTENT_LENGTH, HOST, TRANSFER_ENCODING}}, response::Response, routing::{any, get}};
use std::sync::Arc;
#[derive(Clone)]
struct GatewayState { client: reqwest::Client, upstream: String, allowed_prefixes: Arc<Vec<String>> }
async fn proxy(State(state): State<GatewayState>, request: Request<Body>) -> Result<Response, StatusCode> {
 let (parts, body) = request.into_parts();
 let path = parts.uri.path();
 if !state.allowed_prefixes.iter().any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/"))) { return Err(StatusCode::NOT_FOUND); }
 let query = parts.uri.query().map(|q| format!("?{q}")).unwrap_or_default();
 let url = format!("{}{path}{query}", state.upstream);
 let body = axum::body::to_bytes(body, 16 * 1024 * 1024).await.map_err(|_| StatusCode::BAD_REQUEST)?;
 let mut outgoing = state.client.request(parts.method, url).body(body);
 for (name, value) in &parts.headers { if name != HOST && name != CONTENT_LENGTH { outgoing = outgoing.header(name, value); } }
 let upstream = outgoing.send().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
 let status = upstream.status(); let headers = upstream.headers().clone();
 let body = upstream.bytes().await.map_err(|_| StatusCode::BAD_GATEWAY)?;
 let mut response = Response::new(Body::from(body)); *response.status_mut() = status;
 for (name, value) in &headers { if name != CONTENT_LENGTH && name != TRANSFER_ENCODING { response.headers_mut().append(name, value.clone()); } }
 Ok(response)
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
 let gateway_state = GatewayState { client: reqwest::Client::new(), upstream: std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".to_owned()), allowed_prefixes: Arc::new(vec!["/positions/orders".to_owned(), "/positions/vwap".to_owned(), "/positions/twap".to_owned(), "/positions/close-profitable".to_owned(), "/positions/barriers".to_owned(), "/positions/auto-trader".to_owned()]) };
 let gateway = Router::new().fallback(any(proxy)).with_state(gateway_state);
 let app = Router::new().route("/health", get(|| async { "ok" })).merge(gateway).merge(execution::vwap_api::router()).merge(execution::orders_api::router());
 let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3003)).await?;
 crate::service_log!("INFO", "Execution service listening on http://0.0.0.0:3003");
 axum::serve(listener, app).await?; Ok(())
}
