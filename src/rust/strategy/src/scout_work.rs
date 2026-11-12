//! Typed assignment and result messages for Learning-coordinated Scout workers.
//!
//! Learning owns candidate selection and scheduling. A worker receives one
//! bounded assignment for exactly one strategy and returns the evaluation
//! result; it does not select follow-up work.

use anyhow::{Context, bail};
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, Mutex, OnceLock},
};

use crate::{Strategy, StrategyMarketSignal};

pub const MAX_REPLAY_SIGNALS: usize = 512;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StrategyReplayPoint {
    pub timestamp_ms: u64,
    pub signal: StrategyMarketSignal,
    #[serde(default)]
    pub top_bid: Option<f64>,
    #[serde(default)]
    pub top_ask: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StrategyReplayRequest {
    pub job_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_symbol: String,
    pub profile_id: String,
    pub parameters: BTreeMap<String, f64>,
    pub entry_timestamp_ms: u64,
    pub actual_exit_timestamp_ms: u64,
    pub observation_through_timestamp_ms: u64,
    pub position_side: String,
    #[serde(default)]
    pub counterfactual_position_open: bool,
    #[serde(default)]
    pub counterfactual_entry_found: bool,
    pub points: Vec<StrategyReplayPoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StrategyReplayDecisionPoint {
    pub timestamp_ms: u64,
    pub entry_decision: ScoutWorkDecision,
    pub position_decision: Option<ScoutWorkDecision>,
    pub mark_price: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StrategyReplayResponse {
    pub job_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub profile_id: String,
    pub input_count: usize,
    pub evaluated_count: usize,
    pub counterfactual_exit_timestamp_ms: Option<u64>,
    pub counterfactual_exit_mark_price: Option<f64>,
    pub counterfactual_position_open: bool,
    pub counterfactual_entry_found: bool,
    pub decisions: Vec<StrategyReplayDecisionPoint>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutWorkAssignment {
    pub assignment_id: String,
    pub strategy: Strategy,
    pub mode: ScoutWorkMode,
    pub symbol: String,
    pub parameters: BTreeMap<String, f64>,
    /// The serialized Insights signal used for this one strategy evaluation.
    pub candidate: Value,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutWorkMode {
    #[default]
    Evaluate,
    EvidenceOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutWorkDecision {
    NotEvaluated,
    Enter,
    Scout,
    Hold,
    Exit,
    Rebalance,
    Reject,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutWorkResult {
    pub assignment_id: String,
    pub strategy: Strategy,
    pub symbol: String,
    pub decision: ScoutWorkDecision,
    pub candidate: Value,
    #[serde(default)]
    pub walk_forward_evidence: Option<crate::StrategyWalkForwardEvidence>,
}

/// Candidate data supplied to Learning. It deliberately omits strategy and
/// parameters; Learning selects the active strategy profiles and dispatches
/// one assignment to the corresponding isolated workers.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutCandidateBatch {
    pub request_id: String,
    #[serde(default)]
    pub mode: ScoutWorkMode,
    pub signals: Vec<Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutCandidateBatchResult {
    pub request_id: String,
    pub assignments: Vec<ScoutWorkResult>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutWorkerHttpMethod {
    Get,
    Post,
    Put,
    Delete,
}

/// Learning-to-worker request for one existing Scout lifecycle operation.
/// `path` is validated by the worker against its route allowlist before it
/// invokes the existing handler locally.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutWorkerRouteCommand {
    pub request_id: String,
    pub strategy: Strategy,
    pub method: ScoutWorkerHttpMethod,
    pub path: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutWorkerRouteResult {
    pub request_id: String,
    pub strategy: Strategy,
    pub status: u16,
    pub body: Value,
}

/// Application-level Scout request received by Learning. When `strategy` is
/// absent, Learning chooses the relevant strategy partitions from the route
/// and payload and aggregates their responses.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutCoordinatorRouteCommand {
    pub request_id: String,
    #[serde(default)]
    pub strategy: Option<Strategy>,
    pub method: ScoutWorkerHttpMethod,
    pub path: String,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutCoordinatorRouteResult {
    pub request_id: String,
    pub status: u16,
    pub body: Value,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutFeedbackAcknowledgement {
    pub event_id: String,
    pub accepted: bool,
    pub duplicate: bool,
    pub status: u16,
    #[serde(default)]
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum ScoutCoordinatorRequest {
    CandidateBatch(ScoutCandidateBatch),
    StrategyReplay(StrategyReplayRequest),
    WorkerRoute(ScoutCoordinatorRouteCommand),
    LearningRoute(ScoutCoordinatorRouteCommand),
    Feedback(crate::FeedbackEvent),
    ProfileSnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "response", rename_all = "snake_case")]
pub enum ScoutCoordinatorResponse {
    CandidateBatch(ScoutCandidateBatchResult),
    StrategyReplay(StrategyReplayResponse),
    WorkerRoute(ScoutCoordinatorRouteResult),
    LearningRoute(ScoutCoordinatorRouteResult),
    Feedback(ScoutFeedbackAcknowledgement),
    ProfileSnapshot(Value),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "request", rename_all = "snake_case")]
pub enum ScoutWorkerRequest {
    Evaluate(ScoutWorkAssignment),
    StrategyReplay(StrategyReplayRequest),
    Route(ScoutWorkerRouteCommand),
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "operation", content = "response", rename_all = "snake_case")]
pub enum ScoutWorkerResponse {
    Evaluation(ScoutWorkResult),
    StrategyReplay(StrategyReplayResponse),
    Route(ScoutWorkerRouteResult),
}

/// Sends a Learning coordination request over RSocket request-response.
/// Non-WebSocket URLs are rejected so callers cannot silently fall back to
/// HTTP during a worker outage.
pub async fn request_scout_candidate_batch(
    learning_rsocket_url: &str,
    request: &ScoutCandidateBatch,
) -> anyhow::Result<ScoutCandidateBatchResult> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(
                &ScoutCoordinatorRequest::CandidateBatch(request.clone()),
            )?)
            .build(),
    )
    .await
    .context("Learning Scout candidate batch request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning coordinator response had no data")?,
    )
    .context("Learning coordinator response was not a valid result")?
    {
        ScoutCoordinatorResponse::CandidateBatch(response) => Ok(response),
        ScoutCoordinatorResponse::StrategyReplay(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::WorkerRoute(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::Feedback(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::ProfileSnapshot(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::LearningRoute(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
    }
}

pub async fn request_scout_strategy_replay(
    learning_rsocket_url: &str,
    request: &StrategyReplayRequest,
) -> anyhow::Result<StrategyReplayResponse> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    if request.points.len() > MAX_REPLAY_SIGNALS {
        bail!("strategy replay exceeds the configured signal limit");
    }
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(
                &ScoutCoordinatorRequest::StrategyReplay(request.clone()),
            )?)
            .build(),
    )
    .await
    .context("Learning strategy replay request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning coordinator response had no data")?,
    )
    .context("Learning coordinator response was not a valid result")?
    {
        ScoutCoordinatorResponse::StrategyReplay(result) => Ok(result),
        _ => bail!("Learning returned the wrong replay operation response"),
    }
}

pub async fn request_scout_worker_route(
    learning_rsocket_url: &str,
    request: &ScoutCoordinatorRouteCommand,
) -> anyhow::Result<ScoutCoordinatorRouteResult> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(&ScoutCoordinatorRequest::WorkerRoute(
                request.clone(),
            ))?)
            .build(),
    )
    .await
    .context("Learning Scout worker route request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning coordinator response had no data")?,
    )
    .context("Learning coordinator response was not a valid result")?
    {
        ScoutCoordinatorResponse::WorkerRoute(response) => Ok(response),
        ScoutCoordinatorResponse::CandidateBatch(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::StrategyReplay(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::Feedback(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::ProfileSnapshot(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
        ScoutCoordinatorResponse::LearningRoute(_) => {
            bail!("Learning returned the wrong Scout operation response")
        }
    }
}

/// Calls Learning-owned application APIs through its RSocket listener. The
/// route is executed by Learning's local Axum router; no HTTP fallback exists.
pub async fn request_learning_route(
    learning_rsocket_url: &str,
    request: &ScoutCoordinatorRouteCommand,
) -> anyhow::Result<ScoutCoordinatorRouteResult> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(
                &ScoutCoordinatorRequest::LearningRoute(request.clone()),
            )?)
            .build(),
    )
    .await
    .context("Learning RSocket route request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning RSocket response had no data")?,
    )
    .context("Learning RSocket response was invalid")?
    {
        ScoutCoordinatorResponse::LearningRoute(response) => Ok(response),
        _ => bail!("Learning returned the wrong RSocket operation response"),
    }
}

/// Calls a service-owned Scout route responder directly, without routing
/// through Learning. Used for durable registration callbacks into Insights.
pub async fn request_scout_service_route(
    service_rsocket_url: &str,
    request: &ScoutCoordinatorRouteCommand,
) -> anyhow::Result<ScoutCoordinatorRouteResult> {
    validate_learning_rsocket_url(service_rsocket_url)?;
    let response = pooled_request(
        service_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(&ScoutCoordinatorRequest::WorkerRoute(
                request.clone(),
            ))?)
            .build(),
    )
    .await
    .context("Scout service route request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Scout service route response had no data")?,
    )
    .context("Scout service route response was not valid")?
    {
        ScoutCoordinatorResponse::WorkerRoute(response) => Ok(response),
        _ => bail!("Scout service returned the wrong operation response"),
    }
}

type RSocketClientPool = HashMap<String, Arc<Client>>;

fn rsocket_client_pool() -> &'static Mutex<RSocketClientPool> {
    static POOL: OnceLock<Mutex<RSocketClientPool>> = OnceLock::new();
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

async fn pooled_rsocket_client(service_rsocket_url: &str) -> anyhow::Result<Arc<Client>> {
    if let Some(client) = rsocket_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(service_rsocket_url)
        .cloned()
    {
        return Ok(client);
    }

    let connected = Arc::new(
        RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(service_rsocket_url))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .context("could not connect to the RSocket endpoint")?,
    );
    let mut pool = rsocket_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Ok(pool
        .entry(service_rsocket_url.to_owned())
        .or_insert(connected)
        .clone())
}

fn invalidate_pooled_rsocket_client(service_rsocket_url: &str, failed_client: &Arc<Client>) {
    let mut pool = rsocket_client_pool()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if pool
        .get(service_rsocket_url)
        .is_some_and(|current| Arc::ptr_eq(current, failed_client))
    {
        pool.remove(service_rsocket_url);
    }
}

async fn pooled_request(service_rsocket_url: &str, request: Payload) -> anyhow::Result<Payload> {
    let client = pooled_rsocket_client(service_rsocket_url).await?;
    match client.request_response(request).await {
        Ok(Some(response)) => Ok(response),
        Ok(None) => {
            invalidate_pooled_rsocket_client(service_rsocket_url, &client);
            bail!("RSocket endpoint returned an empty response");
        }
        Err(error) => {
            invalidate_pooled_rsocket_client(service_rsocket_url, &client);
            Err(error).context("RSocket endpoint request failed")
        }
    }
}

pub async fn request_scout_feedback(
    learning_rsocket_url: &str,
    event: &crate::FeedbackEvent,
) -> anyhow::Result<ScoutFeedbackAcknowledgement> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(&ScoutCoordinatorRequest::Feedback(
                event.clone(),
            ))?)
            .build(),
    )
    .await
    .context("Learning Scout feedback request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning coordinator response had no data")?,
    )
    .context("Learning coordinator response was not a valid result")?
    {
        ScoutCoordinatorResponse::Feedback(acknowledgement) => Ok(acknowledgement),
        _ => bail!("Learning returned the wrong Scout operation response"),
    }
}

pub async fn request_scout_profile_snapshot(learning_rsocket_url: &str) -> anyhow::Result<Value> {
    validate_learning_rsocket_url(learning_rsocket_url)?;
    let response = pooled_request(
        learning_rsocket_url,
        Payload::builder()
            .set_data(serde_json::to_vec(
                &ScoutCoordinatorRequest::ProfileSnapshot,
            )?)
            .build(),
    )
    .await
    .context("Learning Scout profile snapshot request failed")?;
    match serde_json::from_slice::<ScoutCoordinatorResponse>(
        response
            .data()
            .context("Learning coordinator response had no data")?,
    )
    .context("Learning coordinator response was not a valid result")?
    {
        ScoutCoordinatorResponse::ProfileSnapshot(snapshot) => Ok(snapshot),
        _ => bail!("Learning returned the wrong Scout operation response"),
    }
}

fn validate_learning_rsocket_url(url: &str) -> anyhow::Result<()> {
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        bail!("Learning Scout coordination requires an RSocket WebSocket URL");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assignment_and_result_round_trip_as_typed_messages() {
        let assignment = ScoutWorkAssignment {
            assignment_id: "assignment-1".into(),
            strategy: Strategy::LiveMomentum,
            mode: ScoutWorkMode::Evaluate,
            symbol: "BTC-USDT".into(),
            parameters: BTreeMap::from([("minimum_momentum".into(), 0.25)]),
            candidate: serde_json::json!({"symbol":"BTC-USDT"}),
        };
        let bytes = serde_json::to_vec(&assignment).unwrap();
        let decoded: ScoutWorkAssignment = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(decoded, assignment);

        let result = ScoutWorkResult {
            assignment_id: decoded.assignment_id,
            strategy: decoded.strategy,
            symbol: decoded.symbol,
            decision: ScoutWorkDecision::Enter,
            candidate: serde_json::json!({"symbol":"BTC-USDT"}),
            walk_forward_evidence: None,
        };
        let decoded: ScoutWorkResult =
            serde_json::from_slice(&serde_json::to_vec(&result).unwrap()).unwrap();
        assert_eq!(decoded, result);
    }

    #[test]
    fn coordinator_url_validation_refuses_http_fallback() {
        assert!(validate_learning_rsocket_url("http://learning:3010").is_err());
        assert!(validate_learning_rsocket_url("ws://learning:10010").is_ok());
    }

    #[tokio::test]
    async fn service_route_requests_reuse_a_pooled_rsocket_connection() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct ServiceResponder;

        #[rsocket_rust::async_trait]
        impl RSocket for ServiceResponder {
            async fn metadata_push(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }

            async fn fire_and_forget(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }

            async fn request_response(
                &self,
                request: Payload,
            ) -> rsocket_rust::Result<Option<Payload>> {
                let request: ScoutCoordinatorRequest =
                    serde_json::from_slice(request.data().expect("request payload"))
                        .expect("valid request");
                let ScoutCoordinatorRequest::WorkerRoute(request) = request else {
                    bail!("expected worker route")
                };
                let response = ScoutCoordinatorResponse::WorkerRoute(ScoutCoordinatorRouteResult {
                    request_id: request.request_id,
                    status: 204,
                    body: Value::Null,
                });
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response).expect("serialize response"))
                        .build(),
                ))
            }

            fn request_stream(&self, _request: Payload) -> Flux<rsocket_rust::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }

            fn request_channel(
                &self,
                requests: Flux<rsocket_rust::Result<Payload>>,
            ) -> Flux<rsocket_rust::Result<Payload>> {
                requests
            }
        }

        let accepted_connections = Arc::new(AtomicUsize::new(0));
        let accept_counter = Arc::clone(&accepted_connections);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                accept_counter.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(ServiceResponder))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let endpoint = format!("ws://{address}");
        for request_id in ["insights-1", "insights-2"] {
            let response = request_scout_service_route(
                &endpoint,
                &ScoutCoordinatorRouteCommand {
                    request_id: request_id.into(),
                    strategy: None,
                    method: ScoutWorkerHttpMethod::Post,
                    path: "/internal/auto-trader/signal".into(),
                    payload: serde_json::json!({"symbol":request_id}),
                },
            )
            .await
            .unwrap();
            assert_eq!(response.status, 204);
            assert_eq!(response.request_id, request_id);
        }

        assert_eq!(accepted_connections.load(Ordering::Relaxed), 1);
        rsocket_client_pool().lock().unwrap().remove(&endpoint);
        server_task.abort();
    }

    #[tokio::test]
    async fn learning_route_requests_reuse_a_pooled_rsocket_connection() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct LearningResponder;

        #[rsocket_rust::async_trait]
        impl RSocket for LearningResponder {
            async fn metadata_push(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }
            async fn fire_and_forget(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }
            async fn request_response(
                &self,
                request: Payload,
            ) -> rsocket_rust::Result<Option<Payload>> {
                let request: ScoutCoordinatorRequest =
                    serde_json::from_slice(request.data().expect("request payload"))
                        .expect("valid request");
                let ScoutCoordinatorRequest::LearningRoute(request) = request else {
                    bail!("expected Learning route")
                };
                let response =
                    ScoutCoordinatorResponse::LearningRoute(ScoutCoordinatorRouteResult {
                        request_id: request.request_id,
                        status: 200,
                        body: serde_json::json!({"ok":true}),
                    });
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response).expect("serialize response"))
                        .build(),
                ))
            }
            fn request_stream(&self, _request: Payload) -> Flux<rsocket_rust::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
            fn request_channel(
                &self,
                requests: Flux<rsocket_rust::Result<Payload>>,
            ) -> Flux<rsocket_rust::Result<Payload>> {
                requests
            }
        }

        let accepted_connections = Arc::new(AtomicUsize::new(0));
        let accept_counter = Arc::clone(&accepted_connections);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                accept_counter.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(LearningResponder))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let endpoint = format!("ws://{address}");
        for request_id in ["learning-1", "learning-2"] {
            let response = request_learning_route(
                &endpoint,
                &ScoutCoordinatorRouteCommand {
                    request_id: request_id.into(),
                    strategy: None,
                    method: ScoutWorkerHttpMethod::Get,
                    path: "/summary".into(),
                    payload: Value::Null,
                },
            )
            .await
            .unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.request_id, request_id);
        }

        assert_eq!(accepted_connections.load(Ordering::Relaxed), 1);
        rsocket_client_pool().lock().unwrap().remove(&endpoint);
        server_task.abort();
    }

    #[tokio::test]
    async fn transport_error_evicts_pooled_client_for_the_next_request() {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct FlakyLearningResponder(Arc<AtomicUsize>);

        #[rsocket_rust::async_trait]
        impl RSocket for FlakyLearningResponder {
            async fn metadata_push(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }
            async fn fire_and_forget(&self, _request: Payload) -> rsocket_rust::Result<()> {
                bail!("unsupported")
            }
            async fn request_response(
                &self,
                request: Payload,
            ) -> rsocket_rust::Result<Option<Payload>> {
                let request: ScoutCoordinatorRequest =
                    serde_json::from_slice(request.data().expect("request payload"))
                        .expect("valid request");
                let ScoutCoordinatorRequest::LearningRoute(request) = request else {
                    bail!("expected Learning route")
                };
                if self.0.fetch_add(1, Ordering::Relaxed) == 0 {
                    bail!("simulated stale transport");
                }
                let response =
                    ScoutCoordinatorResponse::LearningRoute(ScoutCoordinatorRouteResult {
                        request_id: request.request_id,
                        status: 200,
                        body: Value::Null,
                    });
                Ok(Some(
                    Payload::builder()
                        .set_data(serde_json::to_vec(&response).expect("serialize response"))
                        .build(),
                ))
            }
            fn request_stream(&self, _request: Payload) -> Flux<rsocket_rust::Result<Payload>> {
                Box::pin(futures_util::stream::empty())
            }
            fn request_channel(
                &self,
                requests: Flux<rsocket_rust::Result<Payload>>,
            ) -> Flux<rsocket_rust::Result<Payload>> {
                requests
            }
        }

        let accepted_connections = Arc::new(AtomicUsize::new(0));
        let accept_counter = Arc::clone(&accepted_connections);
        let requests = Arc::new(AtomicUsize::new(0));
        let request_counter = Arc::clone(&requests);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                accept_counter.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(FlakyLearningResponder(Arc::clone(
                    &request_counter,
                ))))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let endpoint = format!("ws://{address}");
        let request = |request_id: &str| ScoutCoordinatorRouteCommand {
            request_id: request_id.into(),
            strategy: None,
            method: ScoutWorkerHttpMethod::Get,
            path: "/summary".into(),
            payload: Value::Null,
        };
        assert!(
            request_learning_route(&endpoint, &request("failed"))
                .await
                .is_err()
        );
        let response = request_learning_route(&endpoint, &request("reconnected"))
            .await
            .unwrap();
        assert_eq!(response.request_id, "reconnected");
        assert_eq!(accepted_connections.load(Ordering::Relaxed), 2);
        assert_eq!(requests.load(Ordering::Relaxed), 2);

        rsocket_client_pool().lock().unwrap().remove(&endpoint);
        server_task.abort();
    }
}
