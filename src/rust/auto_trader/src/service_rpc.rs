//! Request-response clients for application services. These calls use RSocket
//! so domain traffic does not depend on the HTTP listener used by browsers and
//! operational tooling.

use anyhow::Context;
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex as StdMutex, OnceLock},
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcRequest {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RpcResponse {
    pub status: u16,
    pub body: Value,
    #[serde(default)]
    pub headers: std::collections::BTreeMap<String, String>,
}

pub async fn request(endpoint: &str, request: &RpcRequest) -> anyhow::Result<RpcResponse> {
    anyhow::ensure!(
        endpoint.starts_with("ws://") || endpoint.starts_with("wss://"),
        "RSocket endpoint must use ws or wss"
    );
    let client = pooled_client(endpoint).await?;
    let payload = match client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(request)?)
                .build(),
        )
        .await
    {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            invalidate_client(endpoint, &client);
            anyhow::bail!("application service returned an empty RSocket response");
        }
        Err(error) => {
            invalidate_client(endpoint, &client);
            return Err(error).context("application service RSocket request failed");
        }
    };
    let bytes = payload
        .data()
        .context("application service RSocket response had no data")?;
    let response: RpcResponse = serde_json::from_slice(bytes.as_ref())
        .context("application service RSocket response was invalid")?;
    Ok(response)
}

type RpcClientPool = HashMap<String, Arc<Client>>;

fn rpc_client_pool() -> &'static StdMutex<RpcClientPool> {
    static CLIENTS: OnceLock<StdMutex<RpcClientPool>> = OnceLock::new();
    CLIENTS.get_or_init(|| StdMutex::new(HashMap::new()))
}

async fn pooled_client(endpoint: &str) -> anyhow::Result<Arc<Client>> {
    if let Some(client) = rpc_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(endpoint)
        .cloned()
    {
        return Ok(client);
    }
    let connected = Arc::new(
        RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(endpoint))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .with_context(|| format!("could not connect to application service at {endpoint}"))?,
    );
    let mut clients = rpc_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(clients
        .entry(endpoint.to_owned())
        .or_insert(connected)
        .clone())
}

fn invalidate_client(endpoint: &str, failed_client: &Arc<Client>) {
    let mut clients = rpc_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if clients
        .get(endpoint)
        .is_some_and(|client| Arc::ptr_eq(client, failed_client))
    {
        clients.remove(endpoint);
    }
}

pub async fn position_request(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<RpcResponse> {
    let endpoint = std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
        .context("POSITION_MANAGEMENT_RSOCKET_URL is required")?;
    position_request_at(&endpoint, method, path, body).await
}

pub async fn position_request_at(
    endpoint: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<RpcResponse> {
    request(
        &endpoint,
        &RpcRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            body,
        },
    )
    .await
}

pub async fn position_get(path: &str) -> anyhow::Result<Value> {
    let endpoint = std::env::var("POSITION_MANAGEMENT_RSOCKET_URL")
        .context("POSITION_MANAGEMENT_RSOCKET_URL is required")?;
    position_get_at(&endpoint, path).await
}

pub async fn position_get_at(endpoint: &str, path: &str) -> anyhow::Result<Value> {
    let response = request(
        endpoint,
        &RpcRequest {
            method: "GET".into(),
            path: path.into(),
            body: None,
        },
    )
    .await?;
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "Position Management returned status {}: {}",
        response.status,
        response.body
    );
    Ok(response.body)
}

pub async fn position_send(method: &str, path: &str, body: Value) -> anyhow::Result<Value> {
    let response = position_request(method, path, Some(body)).await?;
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "Position Management returned status {}: {}",
        response.status,
        response.body
    );
    Ok(response.body)
}

pub async fn insights_request(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<RpcResponse> {
    static REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let endpoint = std::env::var("INSIGHTS_RPC_RSOCKET_URL")
        .context("INSIGHTS_RPC_RSOCKET_URL is required")?;
    let response = strategies::request_scout_service_route(
        &endpoint,
        &strategies::ScoutCoordinatorRouteCommand {
            request_id: format!(
                "autotrader-insights-rpc-{}",
                REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            strategy: None,
            method: scout_worker_method(method)?,
            path: path.to_owned(),
            payload: body.unwrap_or(Value::Null),
        },
    )
    .await?;
    Ok(RpcResponse {
        status: response.status,
        body: response.body,
        headers: std::collections::BTreeMap::new(),
    })
}

pub async fn insights_browser_request(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<RpcResponse> {
    let endpoint = std::env::var("INSIGHTS_BROWSER_RSOCKET_URL")
        .context("INSIGHTS_BROWSER_RSOCKET_URL is required")?;
    request(
        &endpoint,
        &RpcRequest {
            method: method.to_owned(),
            path: path.to_owned(),
            body,
        },
    )
    .await
}

fn scout_worker_method(method: &str) -> anyhow::Result<strategies::ScoutWorkerHttpMethod> {
    match method {
        "GET" => Ok(strategies::ScoutWorkerHttpMethod::Get),
        "POST" => Ok(strategies::ScoutWorkerHttpMethod::Post),
        "PUT" => Ok(strategies::ScoutWorkerHttpMethod::Put),
        "DELETE" => Ok(strategies::ScoutWorkerHttpMethod::Delete),
        _ => anyhow::bail!("unsupported Insights service method {method}"),
    }
}

pub async fn learning_request(
    method: strategies::ScoutWorkerHttpMethod,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<strategies::ScoutCoordinatorRouteResult> {
    static REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let endpoint =
        std::env::var("LEARNING_RSOCKET_URL").context("LEARNING_RSOCKET_URL is required")?;
    strategies::request_learning_route(
        &endpoint,
        &strategies::ScoutCoordinatorRouteCommand {
            request_id: format!(
                "autotrader-rpc-{}",
                REQUEST_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ),
            strategy: None,
            method,
            path: path.to_owned(),
            payload: body.unwrap_or(Value::Null),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rpc_contract_round_trips_request_and_response_json() {
        let request = RpcRequest {
            method: "POST".into(),
            path: "/signals/auto-trader-audit".into(),
            body: Some(serde_json::json!({ "symbol": "BTC-USDT" })),
        };
        let encoded = serde_json::to_vec(&request).unwrap();
        assert_eq!(
            serde_json::from_slice::<RpcRequest>(&encoded).unwrap(),
            request
        );
        let response = RpcResponse {
            status: 202,
            body: Value::Null,
            headers: std::collections::BTreeMap::new(),
        };
        assert_eq!(
            serde_json::from_slice::<RpcResponse>(&serde_json::to_vec(&response).unwrap()).unwrap(),
            response
        );
    }

    #[test]
    fn insights_rpc_method_maps_to_scout_coordinator_route_method() {
        assert_eq!(
            scout_worker_method("POST").unwrap(),
            strategies::ScoutWorkerHttpMethod::Post
        );
        assert!(scout_worker_method("PATCH").is_err());
    }

    #[tokio::test]
    async fn rpc_rejects_http_endpoints_before_connecting() {
        let req = RpcRequest {
            method: "GET".into(),
            path: "/positions".into(),
            body: None,
        };
        assert!(request("http://localhost:3002", &req).await.is_err());
    }

    #[tokio::test]
    async fn rpc_reuses_a_pooled_rsocket_connection() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct Responder;

        #[rsocket_rust::async_trait]
        impl RSocket for Responder {
            async fn metadata_push(&self, _request: Payload) -> rsocket_rust::Result<()> {
                anyhow::bail!("unsupported")
            }

            async fn fire_and_forget(&self, _request: Payload) -> rsocket_rust::Result<()> {
                anyhow::bail!("unsupported")
            }

            async fn request_response(
                &self,
                request: Payload,
            ) -> rsocket_rust::Result<Option<Payload>> {
                let request: RpcRequest = serde_json::from_slice(request.data().unwrap())?;
                let response = RpcResponse {
                    status: 202,
                    body: serde_json::json!({"path": request.path}),
                    headers: std::collections::BTreeMap::new(),
                };
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response).unwrap())
                        .build(),
                ))
            }

            fn request_stream(&self, _request: Payload) -> Flux<rsocket_rust::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }

            fn request_channel(
                &self,
                _requests: Flux<rsocket_rust::Result<Payload>>,
            ) -> Flux<rsocket_rust::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
        }

        let connections = Arc::new(AtomicUsize::new(0));
        let accepted = Arc::clone(&connections);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_, _| {
                accepted.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(Responder))
            }))
            .serve();
        let server_task = tokio::spawn(server);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let endpoint = format!("ws://{address}");
        for path in ["/first", "/second"] {
            let response = request(
                &endpoint,
                &RpcRequest {
                    method: "GET".into(),
                    path: path.into(),
                    body: None,
                },
            )
            .await
            .unwrap();
            assert_eq!(response.status, 202);
            assert_eq!(response.body["path"], path);
        }
        assert_eq!(connections.load(Ordering::Relaxed), 1);
        server_task.abort();
    }
}
