use anyhow::{Context, bail};
use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Method, Request, StatusCode},
};
use futures_util::{Stream, stream};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;
use tower::ServiceExt;

const MAX_REQUEST_BODY_BYTES: usize = 2 * 1024 * 1024;
const MAX_RESPONSE_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum RouteRequest {
    Scout(strategies::ScoutCoordinatorRequest),
    Browser(BrowserRouteRequest),
}

#[derive(Debug, Deserialize)]
struct BrowserRouteRequest {
    method: String,
    path: String,
    #[serde(default)]
    body: Option<Value>,
}

#[derive(Debug, Serialize)]
struct BrowserRouteResponse {
    status: u16,
    body: Value,
}

struct AutoTraderResponder(Router);

#[rsocket_rust::async_trait]
impl RSocket for AutoTraderResponder {
    async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
        bail!("AutoTrader accepts application calls using request-response only")
    }

    async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
        bail!("AutoTrader requires request-response so callers receive the result")
    }

    async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
        let bytes = request.data().context("empty AutoTrader route request")?;
        let request: RouteRequest =
            serde_json::from_slice(bytes).context("invalid AutoTrader RSocket route request")?;
        let response = match request {
            RouteRequest::Browser(request) => {
                let result =
                    dispatch_route(&self.0, &request.method, &request.path, request.body).await?;
                serde_json::to_vec(&result)?
            }
            RouteRequest::Scout(strategies::ScoutCoordinatorRequest::WorkerRoute(command)) => {
                if command.strategy.is_some() {
                    bail!("AutoTrader routes are not strategy-partitioned");
                }
                let method = match command.method {
                    strategies::ScoutWorkerHttpMethod::Get => "GET",
                    strategies::ScoutWorkerHttpMethod::Put => "PUT",
                    strategies::ScoutWorkerHttpMethod::Post => "POST",
                    strategies::ScoutWorkerHttpMethod::Delete => "DELETE",
                };
                let result = dispatch_route(
                    &self.0,
                    method,
                    &command.path,
                    (command.payload != Value::Null).then_some(command.payload),
                )
                .await?;
                serde_json::to_vec(&strategies::ScoutCoordinatorResponse::WorkerRoute(
                    strategies::ScoutCoordinatorRouteResult {
                        request_id: command.request_id,
                        status: result.status,
                        body: result.body,
                    },
                ))?
            }
            RouteRequest::Scout(_) => bail!("unsupported AutoTrader RSocket operation"),
        };
        Ok(Some(Payload::builder().set_data(response).build()))
    }

    fn request_stream(
        &self,
        request: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        let route = request
            .data()
            .and_then(|data| serde_json::from_slice::<Value>(data).ok())
            .and_then(|request| {
                request
                    .get("route")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        if route.as_deref() != Some("autotrader:snapshot") {
            return Box::pin(stream::once(async {
                Err(anyhow::anyhow!("unsupported AutoTrader RSocket stream"))
            }));
        }
        let router = self.0.clone();
        Box::pin(stream::unfold(
            (
                router,
                tokio::time::interval(std::time::Duration::from_secs(15)),
            ),
            |(router, mut interval)| async move {
                interval.tick().await;
                let result = build_snapshot(&router)
                    .await
                    .and_then(|snapshot| {
                        serde_json::to_vec(&serde_json::json!({
                            "type": "snapshot",
                            "state": snapshot,
                        }))
                        .map_err(anyhow::Error::from)
                    })
                    .map(|data| Payload::builder().set_data(data).build());
                Some((result, (router, interval)))
            },
        ))
    }

    fn request_channel(
        &self,
        _requests: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "AutoTrader RSocket does not support request-channel"
            ))
        }))
    }
}

async fn dispatch_route(
    router: &Router,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<BrowserRouteResponse> {
    let method = Method::from_bytes(method.as_bytes()).context("invalid HTTP method")?;
    if !is_allowed_route(&method, path) {
        bail!("unsupported AutoTrader RSocket route: {method} {path}");
    }
    let body = body
        .map(|value| serde_json::to_vec(&value))
        .transpose()?
        .unwrap_or_default();
    if body.len() > MAX_REQUEST_BODY_BYTES {
        return Ok(BrowserRouteResponse {
            status: StatusCode::PAYLOAD_TOO_LARGE.as_u16(),
            body: Value::String("AutoTrader request body is too large".into()),
        });
    }
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))?,
        )
        .await?;
    let status = response.status().as_u16();
    let bytes = to_bytes(response.into_body(), MAX_RESPONSE_BODY_BYTES).await?;
    let body = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&bytes).into_owned()));
    Ok(BrowserRouteResponse { status, body })
}

async fn build_snapshot(router: &Router) -> anyhow::Result<Value> {
    let status = dispatch_route(router, "GET", "/positions/auto-trader", None).await?;
    let activity = dispatch_route(router, "GET", "/positions/auto-trader/activity", None).await?;
    let parameters =
        dispatch_route(router, "GET", "/positions/auto-trader/parameters", None).await?;
    let dated_carry =
        dispatch_route(router, "GET", "/positions/auto-trader/dated-carry", None).await?;
    Ok(serde_json::json!({
        "status": status.body,
        "activity": activity.body,
        "parameters": parameters.body,
        "dated_carry": dated_carry.body,
        "execution_audit": { "items": [], "nextCursor": null },
    }))
}

fn is_allowed_route(method: &Method, path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    matches!(
        (method, path),
        (&Method::GET, "/positions/auto-trader")
            | (&Method::PUT, "/positions/auto-trader")
            | (&Method::PUT, "/positions/auto-trader/settings")
            | (&Method::GET, "/positions/auto-trader/dated-carry")
            | (&Method::PUT, "/positions/auto-trader/dated-carry")
            | (&Method::GET, "/positions/auto-trader/parameters")
            | (&Method::GET, "/positions/auto-trader/activity")
            | (&Method::GET, "/positions/auto-trader/learning-summary")
            | (&Method::POST, "/internal/auto-trader/signal")
            | (
                &Method::POST,
                "/internal/auto-trader/daily-strategy-signals"
            )
            | (&Method::POST, "/internal/auto-trader/dated-carry-signal")
    )
}

pub async fn serve(router: Router, address: String) -> anyhow::Result<()> {
    ::auto_trader::service_log!("INFO", "AutoTrader RSocket API listening on ws://{address}");
    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(AutoTraderResponder(router.clone())))
        }))
        .serve()
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, routing::get};

    async fn status() -> Json<Value> {
        Json(serde_json::json!({ "enabled": true }))
    }

    async fn echo_payload(Json(payload): Json<Value>) -> Json<Value> {
        Json(payload)
    }

    #[tokio::test]
    async fn dispatches_allowed_request_response_route_and_preserves_json() {
        let router = Router::new().route("/positions/auto-trader/activity", get(status));
        let response = dispatch_route(&router, "GET", "/positions/auto-trader/activity", None)
            .await
            .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, serde_json::json!({ "enabled": true }));
    }

    #[tokio::test]
    async fn refuses_operations_and_routes_outside_the_allowlist() {
        let router = Router::new();
        assert!(
            dispatch_route(&router, "GET", "/health", None)
                .await
                .is_err()
        );
        assert!(
            dispatch_route(&router, "POST", "/positions/auto-trader", None)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn dispatches_mutation_body_and_preserves_the_route_response() {
        let router = Router::new().route(
            "/internal/auto-trader/signal",
            axum::routing::post(echo_payload),
        );
        let payload = serde_json::json!({ "symbol": "BTC-USDT", "side": "buy" });
        let response = dispatch_route(
            &router,
            "POST",
            "/internal/auto-trader/signal",
            Some(payload.clone()),
        )
        .await
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body, payload);
    }

    #[test]
    fn scout_contract_accepts_activity_and_parameter_reads_only_after_path_validation() {
        assert!(is_allowed_route(
            &Method::GET,
            "/positions/auto-trader/activity"
        ));
        assert!(is_allowed_route(
            &Method::GET,
            "/positions/auto-trader/parameters"
        ));
        assert!(!is_allowed_route(&Method::GET, "/health"));
    }

    #[tokio::test]
    async fn snapshot_contains_service_owned_auto_trader_state() {
        let router = Router::new()
            .route(
                "/positions/auto-trader",
                get(|| async { axum::Json(serde_json::json!({"enabled": true})) }),
            )
            .route(
                "/positions/auto-trader/activity",
                get(|| async { axum::Json(serde_json::json!({"orders": []})) }),
            )
            .route(
                "/positions/auto-trader/parameters",
                get(|| async { axum::Json(serde_json::json!({"thresholds": []})) }),
            )
            .route(
                "/positions/auto-trader/dated-carry",
                get(|| async { axum::Json(serde_json::json!({"positions": []})) }),
            );
        let snapshot = build_snapshot(&router).await.unwrap();
        assert_eq!(snapshot["status"]["enabled"], true);
        assert_eq!(snapshot["activity"]["orders"], serde_json::json!([]));
        assert_eq!(snapshot["parameters"]["thresholds"], serde_json::json!([]));
        assert_eq!(snapshot["dated_carry"]["positions"], serde_json::json!([]));
        assert!(snapshot["execution_audit"]["items"].is_array());
    }

    #[tokio::test]
    async fn request_stream_sends_an_initial_snapshot_for_the_supported_route() {
        use futures_util::StreamExt;

        let router = Router::new()
            .route(
                "/positions/auto-trader",
                get(|| async { axum::Json(serde_json::json!({"enabled": true})) }),
            )
            .route(
                "/positions/auto-trader/activity",
                get(|| async { axum::Json(serde_json::json!({"orders": []})) }),
            )
            .route(
                "/positions/auto-trader/parameters",
                get(|| async { axum::Json(serde_json::json!({"thresholds": []})) }),
            )
            .route(
                "/positions/auto-trader/dated-carry",
                get(|| async { axum::Json(serde_json::json!({"positions": []})) }),
            );
        let responder = AutoTraderResponder(router);
        let payload = Payload::builder()
            .set_data(
                serde_json::to_vec(&serde_json::json!({
                    "route": "autotrader:snapshot"
                }))
                .unwrap(),
            )
            .build();
        let mut stream = responder.request_stream(payload);
        let first = stream.next().await.unwrap().unwrap();
        let message: Value = serde_json::from_slice(first.data().unwrap()).unwrap();
        assert_eq!(message["type"], "snapshot");
        assert_eq!(message["state"]["status"]["enabled"], true);
    }
}
