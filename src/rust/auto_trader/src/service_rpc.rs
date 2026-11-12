//! Request-response clients for application services. These calls use RSocket
//! so domain traffic does not depend on the HTTP listener used by browsers and
//! operational tooling.

use anyhow::Context;
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex as StdMutex, OnceLock},
    time::Duration,
};
use strategy::{LiveEvaluationRequest, LiveEvaluationResponse, LiveStrategyDecision, Strategy};

const DEFAULT_LIVE_EVALUATION_TIMEOUT_MS: u64 = 100;
const DEFAULT_LIVE_EVALUATION_CONCURRENCY: usize = 32;
const MAX_LIVE_EVALUATION_BATCH: usize = 256;

static LIVE_EVALUATION_REQUEST_SEQUENCE: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);
static LIVE_EVALUATION_SLOTS: tokio::sync::Semaphore =
    tokio::sync::Semaphore::const_new(DEFAULT_LIVE_EVALUATION_CONCURRENCY);

/// Build an owned live request with a process-unique correlation id.
pub fn live_evaluation_request(
    strategy: Strategy,
    symbol: impl Into<String>,
    profile_id: impl Into<String>,
    profile_symbol: impl Into<String>,
    profile_version: u64,
    parameters: BTreeMap<String, f64>,
    input: strategy::LiveEvaluationInput,
) -> LiveEvaluationRequest {
    let symbol = symbol.into();
    let sequence =
        LIVE_EVALUATION_REQUEST_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    LiveEvaluationRequest {
        contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
        request_id: format!("live:{now_ms}:{sequence}:{}:{symbol}", strategy.as_str()),
        strategy,
        symbol,
        profile: strategy::LiveStrategyProfile {
            profile_id: profile_id.into(),
            profile_symbol: profile_symbol.into(),
            version: profile_version,
            parameters,
        },
        input,
    }
}

pub fn live_entry_evaluation_request(
    strategy: Strategy,
    signal: &strategy::Signal,
    parameters: BTreeMap<String, f64>,
) -> anyhow::Result<LiveEvaluationRequest> {
    let profile_version = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("learning_profile_version"))
        .and_then(Value::as_u64)
        .unwrap_or_default();
    let profile_symbol = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| {
            decision
                .get("learning_profile_symbol")
                .or_else(|| decision.get("scout_parameter_profile_symbol"))
        })
        .and_then(Value::as_str)
        .unwrap_or(&signal.symbol)
        .to_owned();
    let profile_id = strategy::profile_id(&profile_symbol, strategy.as_str(), &parameters);
    live_entry_evaluation_request_for_profile(
        strategy,
        signal,
        profile_id,
        profile_symbol,
        profile_version,
        parameters,
    )
}

pub fn live_entry_evaluation_request_for_profile(
    strategy: Strategy,
    signal: &strategy::Signal,
    profile_id: impl Into<String>,
    profile_symbol: impl Into<String>,
    profile_version: u64,
    parameters: BTreeMap<String, f64>,
) -> anyhow::Result<LiveEvaluationRequest> {
    anyhow::ensure!(
        signal.strategy.as_deref().and_then(Strategy::from_name) == Some(strategy),
        "live entry signal strategy does not match request strategy"
    );
    let decision = signal.strategy_decision.as_ref();
    let daily_features = decision
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    let walk_forward_evidence = decision
        .and_then(|value| value.get("strategy_walk_forward"))
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok());
    Ok(live_evaluation_request(
        strategy,
        signal.symbol.clone(),
        profile_id,
        profile_symbol,
        profile_version,
        parameters,
        strategy::LiveEvaluationInput::Entry {
            signal: signal.clone(),
            daily_features,
            walk_forward_evidence,
        },
    ))
}

pub fn live_position_evaluation_request(
    strategy: Strategy,
    symbol: impl Into<String>,
    profile_id: impl Into<String>,
    profile_symbol: impl Into<String>,
    profile_version: u64,
    parameters: BTreeMap<String, f64>,
    position: strategy::LivePositionContext,
) -> LiveEvaluationRequest {
    live_evaluation_request(
        strategy,
        symbol,
        profile_id,
        profile_symbol,
        profile_version,
        parameters,
        strategy::LiveEvaluationInput::OpenPosition { position },
    )
}

fn parse_live_worker_endpoints(value: &str) -> anyhow::Result<HashMap<String, String>> {
    let mut endpoints = HashMap::new();
    for entry in value
        .split(',')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
    {
        let (strategy, endpoint) = entry.split_once('=').ok_or_else(|| {
            anyhow::anyhow!("live worker endpoint must use strategy=ws://host:port")
        })?;
        let strategy = strategy.trim();
        let endpoint = endpoint.trim();
        anyhow::ensure!(
            Strategy::from_name(strategy).is_some(),
            "unknown live worker strategy '{strategy}'"
        );
        let uri = endpoint.parse::<axum::http::Uri>().map_err(|error| {
            anyhow::anyhow!("live worker endpoint for '{strategy}' is invalid: {error}")
        })?;
        anyhow::ensure!(
            matches!(uri.scheme_str(), Some("ws" | "wss")),
            "live worker endpoint for '{strategy}' must use ws or wss"
        );
        anyhow::ensure!(
            uri.authority()
                .and_then(|authority| authority.port_u16())
                .is_some(),
            "live worker endpoint for '{strategy}' must include a valid port"
        );
        anyhow::ensure!(
            uri.path_and_query().is_none_or(|path| {
                (path.path().is_empty() || path.path() == "/") && path.query().is_none()
            }),
            "live worker endpoint for '{strategy}' must not include a path or query"
        );
        anyhow::ensure!(
            endpoints
                .insert(strategy.to_owned(), endpoint.to_owned())
                .is_none(),
            "duplicate live worker endpoint for '{strategy}'"
        );
    }
    anyhow::ensure!(
        !endpoints.is_empty(),
        "AUTO_TRADER_LIVE_WORKER_ENDPOINTS is empty"
    );
    Ok(endpoints)
}

fn live_worker_endpoint(strategy: Strategy) -> anyhow::Result<String> {
    let configured = std::env::var("AUTO_TRADER_LIVE_WORKER_ENDPOINTS")
        .context("AUTO_TRADER_LIVE_WORKER_ENDPOINTS is required")?;
    let endpoints = parse_live_worker_endpoints(&configured)?;
    endpoints.get(strategy.as_str()).cloned().ok_or_else(|| {
        anyhow::anyhow!(
            "no live Strategy X worker configured for '{}'",
            strategy.as_str()
        )
    })
}

fn live_evaluation_timeout() -> Duration {
    let millis = std::env::var("AUTO_TRADER_LIVE_EVALUATION_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=2_000).contains(value))
        .unwrap_or(DEFAULT_LIVE_EVALUATION_TIMEOUT_MS);
    Duration::from_millis(millis)
}

fn live_evaluation_concurrency() -> usize {
    std::env::var("AUTO_TRADER_LIVE_EVALUATION_CONCURRENCY")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .filter(|value| (1..=256).contains(value))
        .unwrap_or(DEFAULT_LIVE_EVALUATION_CONCURRENCY)
}

fn live_evaluation_batch_limits(total: usize) -> (usize, usize) {
    (
        total.min(MAX_LIVE_EVALUATION_BATCH),
        total.saturating_sub(MAX_LIVE_EVALUATION_BATCH),
    )
}

fn validate_live_evaluation_response(
    request: &LiveEvaluationRequest,
    response: LiveEvaluationResponse,
) -> anyhow::Result<LiveStrategyDecision> {
    anyhow::ensure!(
        response.contract_version == request.contract_version,
        "live Strategy X response contract version mismatch"
    );
    anyhow::ensure!(
        response.request_id == request.request_id,
        "live Strategy X response request id mismatch"
    );
    anyhow::ensure!(
        response.strategy == request.strategy,
        "live Strategy X response strategy mismatch"
    );
    anyhow::ensure!(
        response.symbol == request.symbol,
        "live Strategy X response symbol mismatch"
    );
    anyhow::ensure!(
        response.profile_id == request.profile.profile_id,
        "live Strategy X response profile id mismatch"
    );
    anyhow::ensure!(
        response.profile_symbol == request.profile.profile_symbol,
        "live Strategy X response profile symbol mismatch"
    );
    anyhow::ensure!(
        response.profile_version == request.profile.version,
        "live Strategy X response profile version mismatch"
    );
    if let Some(error) = response.error {
        anyhow::bail!(
            "Strategy X evaluation failed ({:?}): {}",
            error.code,
            error.message
        );
    }
    response.decision.ok_or_else(|| {
        anyhow::anyhow!("live Strategy X response had neither a decision nor an error")
    })
}

async fn request_live_evaluation_at(
    endpoint: &str,
    request: &LiveEvaluationRequest,
) -> anyhow::Result<LiveStrategyDecision> {
    request_live_evaluation_at_with_timeout(endpoint, request, live_evaluation_timeout()).await
}

async fn request_live_evaluation_at_with_timeout(
    endpoint: &str,
    request: &LiveEvaluationRequest,
    timeout: Duration,
) -> anyhow::Result<LiveStrategyDecision> {
    anyhow::ensure!(
        endpoint.starts_with("ws://") || endpoint.starts_with("wss://"),
        "live Strategy X endpoint must use ws or wss"
    );
    tokio::time::timeout(timeout, async {
        let _slot = LIVE_EVALUATION_SLOTS
            .acquire()
            .await
            .map_err(|_| anyhow::anyhow!("live Strategy X request slots are unavailable"))?;
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
                anyhow::bail!("Strategy X returned an empty RSocket response");
            }
            Err(error) => {
                invalidate_client(endpoint, &client);
                return Err(anyhow::anyhow!(
                    "Strategy X RSocket request failed: {error}"
                ));
            }
        };
        let bytes = payload.data().context("Strategy X response had no data")?;
        let response: LiveEvaluationResponse =
            serde_json::from_slice(bytes.as_ref()).context("Strategy X response was invalid")?;
        validate_live_evaluation_response(request, response)
    })
    .await
    .map_err(|_| {
        anyhow::anyhow!(
            "Strategy X live evaluation timed out after {} ms",
            timeout.as_millis()
        )
    })?
}

pub async fn request_live_evaluation(
    request: &LiveEvaluationRequest,
) -> anyhow::Result<LiveStrategyDecision> {
    let endpoint = live_worker_endpoint(request.strategy)?;
    request_live_evaluation_at(&endpoint, request).await
}

/// Evaluate a bounded candidate batch concurrently while preserving result order.
pub async fn request_live_evaluations(
    requests: Vec<LiveEvaluationRequest>,
) -> Vec<anyhow::Result<LiveStrategyDecision>> {
    use futures_util::StreamExt;
    let concurrency = live_evaluation_concurrency();
    let (bounded_count, oversized_count) = live_evaluation_batch_limits(requests.len());
    let mut results =
        futures_util::stream::iter(requests.into_iter().take(bounded_count).enumerate().map(
            |(index, request)| async move {
                let result = request_live_evaluation(&request).await;
                (index, result)
            },
        ))
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await;
    results.sort_by_key(|(index, _)| *index);
    let mut outcomes = results
        .into_iter()
        .map(|(_, result)| result)
        .collect::<Vec<_>>();
    outcomes.extend((0..oversized_count).map(|_| {
        Err(anyhow::anyhow!(
            "Strategy X live evaluation batch exceeds {MAX_LIVE_EVALUATION_BATCH} requests"
        ))
    }));
    outcomes
}

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
    let response = strategy::request_scout_service_route(
        &endpoint,
        &strategy::ScoutCoordinatorRouteCommand {
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

fn scout_worker_method(method: &str) -> anyhow::Result<strategy::ScoutWorkerHttpMethod> {
    match method {
        "GET" => Ok(strategy::ScoutWorkerHttpMethod::Get),
        "POST" => Ok(strategy::ScoutWorkerHttpMethod::Post),
        "PUT" => Ok(strategy::ScoutWorkerHttpMethod::Put),
        "DELETE" => Ok(strategy::ScoutWorkerHttpMethod::Delete),
        _ => anyhow::bail!("unsupported Insights service method {method}"),
    }
}

pub async fn learning_request(
    method: strategy::ScoutWorkerHttpMethod,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<strategy::ScoutCoordinatorRouteResult> {
    static REQUEST_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let endpoint =
        std::env::var("LEARNING_RSOCKET_URL").context("LEARNING_RSOCKET_URL is required")?;
    strategy::request_learning_route(
        &endpoint,
        &strategy::ScoutCoordinatorRouteCommand {
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

    fn live_request() -> LiveEvaluationRequest {
        let signal = strategy::Signal {
            symbol: "BTC-USDT".into(),
            strategy: Some(Strategy::LiveMomentum.as_str().into()),
            ..Default::default()
        };
        live_entry_evaluation_request(Strategy::LiveMomentum, &signal, BTreeMap::new()).unwrap()
    }

    fn live_response(request: &LiveEvaluationRequest) -> LiveEvaluationResponse {
        LiveEvaluationResponse {
            contract_version: request.contract_version,
            request_id: request.request_id.clone(),
            strategy: request.strategy,
            symbol: request.symbol.clone(),
            profile_id: request.profile.profile_id.clone(),
            profile_symbol: request.profile.profile_symbol.clone(),
            profile_version: request.profile.version,
            decision: Some(LiveStrategyDecision::Enter),
            error: None,
        }
    }

    #[test]
    fn live_worker_routes_parse_and_select_the_named_strategy() {
        let routes = parse_live_worker_endpoints(
            "live-momentum=ws://momentum:9000, donchian-20-sma200=wss://daily:9001",
        )
        .unwrap();
        assert_eq!(routes["live-momentum"], "ws://momentum:9000");
        assert_eq!(routes["donchian-20-sma200"], "wss://daily:9001");
        assert!(parse_live_worker_endpoints("unknown=ws://worker:9000").is_err());
        assert!(parse_live_worker_endpoints("live-momentum=http://worker:9000").is_err());
        assert!(
            parse_live_worker_endpoints(
                "live-momentum=ws://momentum:9000;live-trade-flow=ws://flow:9000"
            )
            .is_err()
        );
        assert!(parse_live_worker_endpoints("live-momentum=ws://worker:not-a-port").is_err());
        assert!(
            parse_live_worker_endpoints("live-momentum=ws://one:9000,live-momentum=ws://two:9000")
                .is_err()
        );
    }

    #[test]
    fn live_entry_request_preserves_the_learned_profile_identity() {
        let signal = strategy::Signal {
            symbol: "BTC-USDT".into(),
            strategy: Some(Strategy::LiveMomentum.as_str().into()),
            strategy_decision: Some(serde_json::json!({
                "learning_profile_symbol": "BTC-USDT-PERP",
                "learning_profile_version": 7,
            })),
            ..Default::default()
        };
        let parameters = BTreeMap::from([("minimum_momentum".to_owned(), 0.01)]);
        let request =
            live_entry_evaluation_request(Strategy::LiveMomentum, &signal, parameters.clone())
                .unwrap();
        assert_eq!(request.profile.profile_symbol, "BTC-USDT-PERP");
        assert_eq!(request.profile.version, 7);
        assert_eq!(
            request.profile.profile_id,
            strategy::profile_id("BTC-USDT-PERP", "live-momentum", &parameters)
        );
    }

    #[test]
    fn live_worker_response_identity_must_match_request() {
        let request = live_request();
        assert_eq!(
            validate_live_evaluation_response(&request, live_response(&request)).unwrap(),
            LiveStrategyDecision::Enter
        );
        let mut wrong_profile = live_response(&request);
        wrong_profile.profile_id = "other-profile".into();
        assert!(validate_live_evaluation_response(&request, wrong_profile).is_err());
        let mut wrong_request = live_response(&request);
        wrong_request.request_id.push_str("-other");
        assert!(validate_live_evaluation_response(&request, wrong_request).is_err());
        let mut missing_decision = live_response(&request);
        missing_decision.decision = None;
        assert!(validate_live_evaluation_response(&request, missing_decision).is_err());
        let mut worker_error = live_response(&request);
        worker_error.decision = None;
        worker_error.error = Some(strategy::LiveEvaluationError {
            code: strategy::LiveEvaluationErrorCode::EvaluationFailed,
            message: "missing required learned parameter".into(),
        });
        assert!(
            validate_live_evaluation_response(&request, worker_error)
                .unwrap_err()
                .to_string()
                .contains("missing required learned parameter")
        );
    }

    #[test]
    fn live_batch_limits_cap_requests_and_count_omitted_candidates() {
        assert_eq!(
            live_evaluation_batch_limits(MAX_LIVE_EVALUATION_BATCH),
            (MAX_LIVE_EVALUATION_BATCH, 0)
        );
        assert_eq!(
            live_evaluation_batch_limits(MAX_LIVE_EVALUATION_BATCH + 1),
            (MAX_LIVE_EVALUATION_BATCH, 1)
        );
    }

    #[tokio::test]
    async fn live_worker_timeout_is_an_evaluation_failure() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;

        struct DelayedResponder;

        #[rsocket_rust::async_trait]
        impl RSocket for DelayedResponder {
            async fn metadata_push(&self, _request: Payload) -> rsocket_rust::Result<()> {
                anyhow::bail!("unsupported")
            }

            async fn fire_and_forget(&self, _request: Payload) -> rsocket_rust::Result<()> {
                anyhow::bail!("unsupported")
            }

            async fn request_response(
                &self,
                _request: Payload,
            ) -> rsocket_rust::Result<Option<Payload>> {
                tokio::time::sleep(Duration::from_millis(100)).await;
                Ok(None)
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

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(|_, _| Ok(Box::new(DelayedResponder))))
            .serve();
        let server_task = tokio::spawn(server);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let error = request_live_evaluation_at_with_timeout(
            &format!("ws://{address}"),
            &live_request(),
            Duration::from_millis(5),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("timed out"));
        server_task.abort();
    }

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
            strategy::ScoutWorkerHttpMethod::Post
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
