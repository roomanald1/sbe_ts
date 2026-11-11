mod logging;
#[path = "../../service_docs.rs"]
mod service_docs;
use axum::{
    Router,
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    response::Response,
    routing::{any, get},
};
use futures_util::StreamExt;
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::{WebsocketClientTransport, WebsocketServerTransport};
use serde::{Deserialize, Serialize};
use std::{pin::Pin, sync::Arc};
use tower::ServiceExt;
#[derive(Clone)]
struct GatewayState {
    position_management_rsocket: String,
    allowed_prefixes: Arc<Vec<String>>,
}
async fn proxy(
    State(state): State<GatewayState>,
    request: Request<Body>,
) -> Result<Response, StatusCode> {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path();
    if !state
        .allowed_prefixes
        .iter()
        .any(|prefix| path == prefix || path.starts_with(&format!("{prefix}/")))
    {
        return Err(StatusCode::NOT_FOUND);
    }
    let body = axum::body::to_bytes(body, 16 * 1024 * 1024)
        .await
        .map_err(|_| StatusCode::BAD_REQUEST)?;
    let body = if body.is_empty() {
        None
    } else {
        Some(
            serde_json::from_slice::<serde_json::Value>(&body)
                .map_err(|_| StatusCode::BAD_REQUEST)?,
        )
    };
    let request = PositionManagementRequest {
        method: parts.method.to_string(),
        path: parts.uri.to_string(),
        body,
    };
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(
            state.position_management_rsocket.as_str(),
        ))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?;
    let reply = client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(&request).map_err(|_| StatusCode::BAD_REQUEST)?)
                .build(),
        )
        .await
        .map_err(|_| StatusCode::BAD_GATEWAY)?
        .ok_or(StatusCode::BAD_GATEWAY)?;
    let data = reply.data().ok_or(StatusCode::BAD_GATEWAY)?;
    let response: PositionManagementResponse =
        serde_json::from_slice(data).map_err(|_| StatusCode::BAD_GATEWAY)?;
    let mut outgoing = Response::new(Body::from(
        serde_json::to_vec(&response.body).map_err(|_| StatusCode::BAD_GATEWAY)?,
    ));
    *outgoing.status_mut() =
        StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
    outgoing.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        "application/json".parse().unwrap(),
    );
    Ok(outgoing)
}

#[derive(Deserialize, Serialize)]
struct PositionManagementRequest {
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct PositionManagementResponse {
    status: u16,
    body: serde_json::Value,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let gateway_state = GatewayState {
        position_management_rsocket: std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
            .unwrap_or_else(|_| "ws://127.0.0.1:10005".to_owned()),
        allowed_prefixes: Arc::new(vec![
            "/positions/orders".to_owned(),
            "/positions/vwap".to_owned(),
            "/positions/twap".to_owned(),
            "/positions/close-profitable".to_owned(),
            "/positions/barriers".to_owned(),
        ]),
    };
    let gateway = Router::new().fallback(any(proxy)).with_state(gateway_state);
    let application_router = Router::new()
        .merge(gateway)
        .merge(execution::vwap_api::router())
        .merge(execution::orders_api::router());
    start_execution_rsocket(application_router);
    let app = Router::new()
        .merge(service_docs::routes("Execution", &[("/health", &["get"])]))
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes));
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3003)).await?;
    crate::service_log!("INFO", "Execution service listening on http://0.0.0.0:3003");
    axum::serve(listener, app).await?;
    Ok(())
}

struct ExecutionRSocketResponder {
    router: Router,
}

impl RSocket for ExecutionRSocketResponder {
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        Self: 'async_trait,
        'life0: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket metadata push is unsupported")) })
    }

    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        Self: 'async_trait,
        'life0: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket fire-and-forget is unsupported")) })
    }

    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        request: Payload,
    ) -> Pin<
        Box<
            dyn futures_util::Future<Output = anyhow::Result<Option<Payload>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        Self: 'async_trait,
        'life0: 'async_trait,
    {
        let router = self.router.clone();
        Box::pin(async move {
            let bytes = request
                .data()
                .ok_or_else(|| anyhow::anyhow!("RSocket request data is required"))?;
            let rpc: PositionManagementRequest = serde_json::from_slice(bytes.as_ref())?;
            let path = rpc.path.split('?').next().unwrap_or(&rpc.path);
            let allowed = [
                "/positions/orders",
                "/positions/vwap",
                "/positions/twap",
                "/positions/barriers",
                "/positions/close-profitable",
            ];
            if !allowed
                .iter()
                .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
            {
                anyhow::bail!("unsupported Execution RSocket path");
            }
            let method = rpc.method.parse::<axum::http::Method>()?;
            let body = rpc
                .body
                .map(|value| serde_json::to_vec(&value))
                .transpose()?
                .unwrap_or_default();
            let request = Request::builder()
                .method(method)
                .uri(&rpc.path)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))?;
            let response = router.oneshot(request).await?;
            let status = response.status().as_u16();
            let body = axum::body::to_bytes(response.into_body(), 16 * 1024 * 1024).await?;
            let body = serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into_owned())
            });
            let bytes = serde_json::to_vec(&serde_json::json!({ "status": status, "body": body }))?;
            Ok(Some(Payload::builder().set_data(bytes).build()))
        })
    }

    fn request_stream(
        &self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>>
    {
        Box::pin(futures_util::stream::once(async {
            Err(anyhow::anyhow!("Execution RSocket streams are unsupported"))
        }))
    }

    fn request_channel(
        &self,
        _requests: Flux<Result<Payload, anyhow::Error>>,
    ) -> Flux<Result<Payload, anyhow::Error>> {
        futures_util::stream::once(async {
            Err(anyhow::anyhow!(
                "Execution RSocket request-channel is unsupported"
            ))
        })
        .boxed()
    }
}

fn start_execution_rsocket(router: Router) {
    let port = std::env::var("EXECUTION_RSOCKET_PORT").unwrap_or_else(|_| "10006".to_owned());
    tokio::spawn(async move {
        let address = format!("0.0.0.0:{port}");
        crate::service_log!(
            "INFO",
            "Execution RSocket server listening on ws://{address}"
        );
        if let Err(error) = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(ExecutionRSocketResponder {
                    router: router.clone(),
                }))
            }))
            .serve()
            .await
        {
            crate::service_log!("ERROR", "Execution RSocket server stopped: {error:#}");
        }
    });
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn process_metrics_return_numeric_values() {
        assert!(process_memory_bytes().await.parse::<u64>().is_ok());
        assert!(
            process_cpu_seconds()
                .await
                .parse::<f64>()
                .unwrap()
                .is_finite()
        );
    }
    #[tokio::test]
    async fn proxy_rejects_paths_outside_allowlist() {
        let state = GatewayState {
            position_management_rsocket: "ws://127.0.0.1:1".into(),
            allowed_prefixes: Arc::new(vec!["/positions/orders".into()]),
        };
        let request = Request::builder()
            .uri("/private")
            .body(Body::empty())
            .unwrap();
        assert!(matches!(
            proxy(State(state), request).await,
            Err(StatusCode::NOT_FOUND)
        ));
    }

    #[tokio::test]
    async fn execution_rsocket_dispatches_json_rpc_to_application_router() {
        let router = Router::new().route(
            "/positions/orders",
            get(|| async { axum::Json(serde_json::json!({ "orders": [] })) }),
        );
        let responder = ExecutionRSocketResponder { router };
        let request = Payload::builder()
            .set_data(
                serde_json::to_vec(&PositionManagementRequest {
                    method: "GET".into(),
                    path: "/positions/orders".into(),
                    body: None,
                })
                .unwrap(),
            )
            .build();
        let response = responder.request_response(request).await.unwrap().unwrap();
        let response: serde_json::Value = serde_json::from_slice(response.data().unwrap()).unwrap();
        assert_eq!(response["status"], 200);
        assert_eq!(response["body"]["orders"], serde_json::json!([]));
    }
}
