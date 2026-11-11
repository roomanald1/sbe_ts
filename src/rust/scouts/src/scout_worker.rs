use anyhow::{Context, bail};
use axum::{Router, body::Body};
use futures_util::{Stream, stream};
use http_body_util::BodyExt;
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use rusqlite::{Connection, OptionalExtension, params};
use sha2::{Digest, Sha256};
use std::{path::PathBuf, pin::Pin, sync::Arc};
use strategies::{
    ScoutWorkAssignment, ScoutWorkDecision, ScoutWorkMode, ScoutWorkResult, ScoutWorkerHttpMethod,
    ScoutWorkerRequest, ScoutWorkerResponse, ScoutWorkerRouteCommand, ScoutWorkerRouteResult,
    Strategy, StrategyDecision, StrategyInput,
};
use tokio::sync::{Mutex, Semaphore};
use tower::ServiceExt;

/// Per-process bounds and storage settings for one strategy partition.
#[derive(Clone, Debug)]
pub struct ScoutWorkerConfig {
    pub strategy: Strategy,
    pub listen_addr: String,
    pub state_path: PathBuf,
    pub max_in_flight: usize,
    pub max_payload_bytes: usize,
    pub max_persisted_results: usize,
}

impl ScoutWorkerConfig {
    pub fn from_env(strategy: Strategy) -> anyhow::Result<Self> {
        let strategy_name = strategy.as_str();
        let listen_addr =
            std::env::var("SCOUT_WORKER_LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:10200".into());
        let state_path = std::env::var_os("SCOUT_WORKER_STATE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("scout-worker-{strategy_name}.sqlite")));
        let max_in_flight = env_usize("SCOUT_WORKER_MAX_IN_FLIGHT", 1)?;
        let max_payload_bytes = env_usize("SCOUT_WORKER_MAX_PAYLOAD_BYTES", 1_048_576)?;
        let max_persisted_results = env_usize("SCOUT_WORKER_MAX_PERSISTED_RESULTS", 10_000)?;
        if max_in_flight == 0
            || max_payload_bytes == 0
            || max_persisted_results == 0
            || max_persisted_results > i64::MAX as usize
        {
            bail!("Scout worker limits must be positive");
        }
        Ok(Self {
            strategy,
            listen_addr,
            state_path,
            max_in_flight,
            max_payload_bytes,
            max_persisted_results,
        })
    }
}

fn env_usize(name: &str, default: usize) -> anyhow::Result<usize> {
    match std::env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} must be a positive integer")),
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(error).with_context(|| format!("could not read {name}")),
    }
}

struct StrategyWorker {
    config: ScoutWorkerConfig,
    connection: Arc<Mutex<Connection>>,
    in_flight: Arc<Semaphore>,
    router: Router,
}

impl StrategyWorker {
    fn open(config: ScoutWorkerConfig) -> anyhow::Result<Self> {
        if let Some(parent) = config.state_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| {
                format!(
                    "could not create worker state directory {}",
                    parent.display()
                )
            })?;
        }
        let connection = Connection::open(&config.state_path).with_context(|| {
            format!(
                "could not open worker state {}",
                config.state_path.display()
            )
        })?;
        connection.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA synchronous=NORMAL;
             PRAGMA journal_size_limit=1048576;
             CREATE TABLE IF NOT EXISTS scout_worker_results (
                 assignment_id TEXT PRIMARY KEY,
                 request_hash BLOB NOT NULL,
                 result_json BLOB NOT NULL,
                 completed_at_ms INTEGER NOT NULL
             );",
        )?;
        Ok(Self {
            in_flight: Arc::new(Semaphore::new(config.max_in_flight)),
            connection: Arc::new(Mutex::new(connection)),
            router: crate::strategy_worker_router(config.strategy, config.state_path.clone()),
            config,
        })
    }

    async fn execute(&self, payload: &[u8]) -> anyhow::Result<ScoutWorkResult> {
        if payload.len() > self.config.max_payload_bytes {
            bail!("Scout assignment exceeds the configured payload limit");
        }
        let assignment: ScoutWorkAssignment =
            serde_json::from_slice(payload).context("invalid Scout work assignment payload")?;
        if assignment.assignment_id.trim().is_empty() || assignment.symbol.trim().is_empty() {
            bail!("Scout assignment ID and symbol are required");
        }
        if assignment.strategy != self.config.strategy {
            bail!(
                "assignment for {} was sent to {} worker",
                assignment.strategy.as_str(),
                self.config.strategy.as_str()
            );
        }
        if assignment.candidate["symbol"].as_str() != Some(&assignment.symbol) {
            bail!("assignment symbol does not match candidate symbol");
        }
        if let Some(candidate_strategy) = assignment.candidate["strategy"].as_str() {
            let candidate_strategy = Strategy::from_name(candidate_strategy)
                .context("candidate contains an unknown strategy")?;
            if candidate_strategy != assignment.strategy {
                bail!("assignment strategy does not match candidate strategy");
            }
        }

        // Reject excess work immediately instead of retaining an unbounded
        // queue of decoded request payloads in each isolated process.
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("strategy worker is at its configured in-flight limit"))?;
        let config = self.config.clone();
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || execute_and_persist(connection, config, assignment))
            .await
            .context("Scout evaluation task failed")?
    }

    async fn route(
        &self,
        command: ScoutWorkerRouteCommand,
    ) -> anyhow::Result<ScoutWorkerRouteResult> {
        if command.strategy != self.config.strategy {
            bail!("lifecycle command was routed to the wrong strategy worker");
        }
        if command.request_id.trim().is_empty() || !worker_route_allowed(&command.path) {
            bail!("invalid Scout worker lifecycle command");
        }
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("strategy worker is at its configured in-flight limit"))?;
        let method = match command.method {
            ScoutWorkerHttpMethod::Get => http::Method::GET,
            ScoutWorkerHttpMethod::Post => http::Method::POST,
            ScoutWorkerHttpMethod::Put => http::Method::PUT,
            ScoutWorkerHttpMethod::Delete => http::Method::DELETE,
        };
        let bytes = if method == http::Method::GET || method == http::Method::DELETE {
            Vec::new()
        } else {
            serde_json::to_vec(&command.payload)?
        };
        if bytes.len() > self.config.max_payload_bytes {
            bail!("Scout lifecycle command exceeds the configured payload limit");
        }
        let request = http::Request::builder()
            .method(method)
            .uri(&command.path)
            .header(http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))?;
        let response = self
            .router
            .clone()
            .oneshot(request)
            .await
            .context("local Scout lifecycle route failed")?;
        let status = response.status().as_u16();
        let body = response.into_body().collect().await?.to_bytes();
        let body = serde_json::from_slice(&body)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&body).into()));
        Ok(ScoutWorkerRouteResult {
            request_id: command.request_id,
            strategy: command.strategy,
            status,
            body,
        })
    }
}

fn worker_route_allowed(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    matches!(
        path,
        "/configuration"
            | "/status"
            | "/outcomes"
            | "/samples"
            | "/scout-outcome"
            | "/scout-markout"
            | "/scout-plan"
            | "/daily-inputs"
            | "/live-inputs"
            | "/dated-carry-inputs"
            | "/scout-lifecycle/state"
            | "/scout-lifecycle/observe"
            | "/variants"
            | "/variants/batch"
            | "/live-candidates"
            | "/select-entry"
    )
}

fn execute_and_persist(
    connection: Arc<Mutex<Connection>>,
    config: ScoutWorkerConfig,
    assignment: ScoutWorkAssignment,
) -> anyhow::Result<ScoutWorkResult> {
    let request_json = serde_json::to_vec(&assignment)?;
    let request_hash = Sha256::digest(&request_json).to_vec();
    let db = connection.blocking_lock();
    let existing = db
        .query_row(
            "SELECT request_hash, result_json FROM scout_worker_results WHERE assignment_id = ?1",
            [&assignment.assignment_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, Vec<u8>>(1)?)),
        )
        .optional()?;
    if let Some((stored_request, result)) = existing {
        if stored_request != request_hash {
            bail!("assignment ID was reused with a different Scout request");
        }
        return serde_json::from_slice(&result).context("stored Scout result is invalid");
    }

    let decision = match assignment.mode {
        ScoutWorkMode::Evaluate => {
            let input = StrategyInput::from_candidate_json(
                assignment.candidate.clone(),
                assignment.parameters.clone(),
            )
            .map_err(anyhow::Error::msg)?;
            Some(
                config
                    .strategy
                    .evaluate(&input)
                    .map_err(anyhow::Error::msg)?,
            )
        }
        ScoutWorkMode::EvidenceOnly => None,
    };
    let result = ScoutWorkResult {
        assignment_id: assignment.assignment_id.clone(),
        strategy: assignment.strategy,
        symbol: assignment.symbol,
        decision: decision.map_or(ScoutWorkDecision::NotEvaluated, |decision| match decision {
            StrategyDecision::Enter => ScoutWorkDecision::Enter,
            StrategyDecision::Scout => ScoutWorkDecision::Scout,
            StrategyDecision::Hold => ScoutWorkDecision::Hold,
            StrategyDecision::Exit => ScoutWorkDecision::Exit,
            StrategyDecision::Rebalance => ScoutWorkDecision::Rebalance,
            StrategyDecision::Reject => ScoutWorkDecision::Reject,
        }),
        candidate: assignment.candidate.clone(),
        walk_forward_evidence: None,
    };
    let result_json = serde_json::to_vec(&result)?;
    let completed_at_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64;
    db.execute(
        "INSERT INTO scout_worker_results (assignment_id, request_hash, result_json, completed_at_ms)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(assignment_id) DO NOTHING",
        params![assignment.assignment_id, request_hash, result_json, completed_at_ms],
    )?;
    db.execute(
        "DELETE FROM scout_worker_results WHERE assignment_id IN (
             SELECT assignment_id FROM scout_worker_results
             ORDER BY completed_at_ms DESC, assignment_id DESC LIMIT -1 OFFSET ?1
         )",
        [config.max_persisted_results as i64],
    )?;
    let stored = db.query_row(
        "SELECT result_json FROM scout_worker_results WHERE assignment_id = ?1",
        [&result.assignment_id],
        |row| row.get::<_, Vec<u8>>(0),
    )?;
    serde_json::from_slice(&stored).context("could not decode persisted Scout result")
}

#[derive(Clone)]
struct WorkerResponder(Arc<StrategyWorker>);

#[rsocket_rust::async_trait]
impl RSocket for WorkerResponder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Scout workers accept assigned work using request-response only")
    }

    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Scout workers require request-response so Learning receives the result")
    }

    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let bytes = req.data().context("Scout assignment payload is empty")?;
        let request: ScoutWorkerRequest =
            serde_json::from_slice(bytes).context("invalid typed Scout worker request")?;
        let response = match request {
            ScoutWorkerRequest::Evaluate(assignment) => {
                let result = self.0.execute(&serde_json::to_vec(&assignment)?).await?;
                ScoutWorkerResponse::Evaluation(result)
            }
            ScoutWorkerRequest::Route(command) => {
                ScoutWorkerResponse::Route(self.0.route(command).await?)
            }
        };
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&response)?)
                .build(),
        ))
    }

    fn request_stream(
        &self,
        _req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Scout workers do not select or stream work"
            ))
        }))
    }

    fn request_channel(
        &self,
        _reqs: rsocket_rust::prelude::Flux<anyhow::Result<Payload>>,
    ) -> rsocket_rust::prelude::Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Scout workers do not accept request channels"
            ))
        }))
    }
}

/// Starts one RSocket request-response worker. The process owns only the
/// assigned strategy's evaluator and its bounded idempotency result database.
pub async fn serve_strategy_worker(config: ScoutWorkerConfig) -> anyhow::Result<()> {
    let worker = Arc::new(StrategyWorker::open(config.clone())?);
    let address = config.listen_addr.clone();
    eprintln!(
        "Scouts worker for {} listening on ws://{} (max in-flight {}, state {})",
        config.strategy.as_str(),
        address,
        config.max_in_flight,
        config.state_path.display()
    );
    let server = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(WorkerResponder(Arc::clone(&worker))))
        }))
        .serve();
    server.await.context("Scout RSocket worker stopped")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsocket_rust::utils::EchoRSocket;
    use rsocket_rust_transport_websocket::WebsocketClientTransport;
    use serde_json::json;
    use std::collections::BTreeMap;
    use tokio::time::sleep;

    fn config(strategy: Strategy, path: PathBuf) -> ScoutWorkerConfig {
        ScoutWorkerConfig {
            strategy,
            listen_addr: "127.0.0.1:0".into(),
            state_path: path,
            max_in_flight: 1,
            max_payload_bytes: 64 * 1024,
            max_persisted_results: 2,
        }
    }

    fn assignment(id: &str, strategy: Strategy) -> ScoutWorkAssignment {
        ScoutWorkAssignment {
            assignment_id: id.into(),
            strategy,
            mode: ScoutWorkMode::Evaluate,
            symbol: "BTC-USDT".into(),
            parameters: BTreeMap::from([
                ("minimum_momentum".into(), 0.5),
                ("minimum_flow_imbalance".into(), 0.2),
            ]),
            candidate: json!({
                "symbol":"BTC-USDT", "strategy":strategy.as_str(),
                "direction":"bullish", "components":{
                    "momentum_10s_percent":0.75,
                    "trade_flow_imbalance":0.4
                }
            }),
        }
    }

    #[tokio::test]
    async fn strategy_partition_rejects_misrouted_work() {
        let path = std::env::temp_dir().join(format!(
            "scout-worker-routing-{}.sqlite",
            std::process::id()
        ));
        let worker = StrategyWorker::open(config(Strategy::LiveMomentum, path.clone())).unwrap();
        let error = worker
            .execute(&serde_json::to_vec(&assignment("a", Strategy::LiveTradeFlow)).unwrap())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("sent to live-momentum worker"));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn lifecycle_command_uses_the_local_strategy_scoped_router() {
        let path =
            std::env::temp_dir().join(format!("scout-worker-route-{}.sqlite", std::process::id()));
        let worker = StrategyWorker::open(config(Strategy::LiveMomentum, path.clone())).unwrap();
        let result = worker
            .route(ScoutWorkerRouteCommand {
                request_id: "configuration-read".into(),
                strategy: Strategy::LiveMomentum,
                method: ScoutWorkerHttpMethod::Get,
                path: "/configuration".into(),
                payload: serde_json::Value::Null,
            })
            .await
            .unwrap();
        assert_eq!(result.status, 200);
        assert_eq!(result.body["strategies"], json!(["live-momentum"]));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn evidence_only_assignment_skips_strategy_evaluation() {
        let path = std::env::temp_dir().join(format!(
            "scout-worker-evidence-only-{}.sqlite",
            std::process::id()
        ));
        let worker = StrategyWorker::open(config(Strategy::DatedCarry, path.clone())).unwrap();
        let mut request = assignment("evidence-only", Strategy::DatedCarry);
        request.mode = ScoutWorkMode::EvidenceOnly;
        request.candidate = json!({"symbol":"BTC-USDT", "strategy":"dated-futures-carry"});
        let result = worker
            .execute(&serde_json::to_vec(&request).unwrap())
            .await
            .unwrap();
        assert_eq!(result.decision, ScoutWorkDecision::NotEvaluated);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn each_strategy_partition_is_independent_across_failure_and_restart() {
        let root =
            std::env::temp_dir().join(format!("scout-worker-isolation-{}", std::process::id()));
        let momentum_path = root.with_extension("momentum.sqlite");
        let flow_path = root.with_extension("flow.sqlite");
        let momentum =
            StrategyWorker::open(config(Strategy::LiveMomentum, momentum_path.clone())).unwrap();
        let flow =
            StrategyWorker::open(config(Strategy::LiveTradeFlow, flow_path.clone())).unwrap();
        assert!(!Arc::ptr_eq(&momentum.connection, &flow.connection));

        let momentum_request = assignment("same-local-id", Strategy::LiveMomentum);
        let momentum_result = momentum
            .execute(&serde_json::to_vec(&momentum_request).unwrap())
            .await
            .unwrap();
        let flow_request = assignment("same-local-id", Strategy::LiveTradeFlow);
        let flow_result = flow
            .execute(&serde_json::to_vec(&flow_request).unwrap())
            .await
            .unwrap();
        assert_eq!(momentum_result.strategy, Strategy::LiveMomentum);
        assert_eq!(flow_result.strategy, Strategy::LiveTradeFlow);
        assert_eq!(
            Connection::open(&momentum_path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM scout_worker_results", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );
        assert_eq!(
            Connection::open(&flow_path)
                .unwrap()
                .query_row("SELECT COUNT(*) FROM scout_worker_results", [], |row| row
                    .get::<_, i64>(
                    0
                ))
                .unwrap(),
            1
        );

        // Removing one worker's state must not affect another strategy's state
        // or ability to keep processing its own assignments.
        drop(momentum);
        std::fs::remove_file(&momentum_path).unwrap();
        let next_flow_request = assignment("next-flow", Strategy::LiveTradeFlow);
        flow.execute(&serde_json::to_vec(&next_flow_request).unwrap())
            .await
            .unwrap();
        drop(flow);
        let restarted_flow =
            StrategyWorker::open(config(Strategy::LiveTradeFlow, flow_path.clone())).unwrap();
        let replay = restarted_flow
            .execute(&serde_json::to_vec(&flow_request).unwrap())
            .await
            .unwrap();
        assert_eq!(replay, flow_result);

        for index in 0..3 {
            let request = assignment(&format!("flow-result-{index}"), Strategy::LiveTradeFlow);
            restarted_flow
                .execute(&serde_json::to_vec(&request).unwrap())
                .await
                .unwrap();
        }
        let retained: i64 = Connection::open(&flow_path)
            .unwrap()
            .query_row("SELECT COUNT(*) FROM scout_worker_results", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(
            retained, 2,
            "worker result persistence must honor its configured row cap"
        );

        let _ = std::fs::remove_file(momentum_path);
        let _ = std::fs::remove_file(flow_path);
    }

    #[tokio::test]
    async fn worker_restart_reuses_persisted_assignment_result() {
        let path = std::env::temp_dir().join(format!(
            "scout-worker-restart-{}.sqlite",
            std::process::id()
        ));
        let cfg = config(Strategy::LiveMomentum, path.clone());
        let work = assignment("a", Strategy::LiveMomentum);
        let first_worker = StrategyWorker::open(cfg.clone()).unwrap();
        let first = first_worker
            .execute(&serde_json::to_vec(&work).unwrap())
            .await
            .unwrap();
        drop(first_worker);
        let restarted_worker = StrategyWorker::open(cfg.clone()).unwrap();
        let second = restarted_worker
            .execute(&serde_json::to_vec(&work).unwrap())
            .await
            .unwrap();
        assert_eq!(first, second);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn worker_accepts_typed_assignments_over_rsocket_request_response() {
        let path = std::env::temp_dir().join(format!(
            "scout-worker-rsocket-{}.sqlite",
            std::process::id()
        ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let worker =
            Arc::new(StrategyWorker::open(config(Strategy::LiveMomentum, path.clone())).unwrap());
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(WorkerResponder(Arc::clone(&worker))))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });

        let url = format!("ws://{address}");
        let mut client = None;
        for _ in 0..25 {
            match RSocketFactory::connect()
                .transport(WebsocketClientTransport::from(url.as_str()))
                .acceptor(Box::new(|| Box::new(EchoRSocket)))
                .start()
                .await
            {
                Ok(connected) => {
                    client = Some(connected);
                    break;
                }
                Err(_) => sleep(std::time::Duration::from_millis(20)).await,
            }
        }
        let client = client.expect("RSocket worker should accept a connection");
        let assignment = assignment("wire-test", Strategy::LiveMomentum);
        let response = client
            .request_response(
                Payload::builder()
                    .set_data(
                        serde_json::to_vec(&ScoutWorkerRequest::Evaluate(assignment)).unwrap(),
                    )
                    .build(),
            )
            .await
            .unwrap()
            .expect("worker responds with its evaluation");
        let ScoutWorkerResponse::Evaluation(result) =
            serde_json::from_slice(response.data().unwrap()).unwrap()
        else {
            panic!("worker should return an evaluation response");
        };
        assert_eq!(result.assignment_id, "wire-test");
        assert_eq!(result.strategy, Strategy::LiveMomentum);
        assert_eq!(result.decision, ScoutWorkDecision::Enter);

        server_task.abort();
        drop(client);
        let _ = std::fs::remove_file(path);
    }
}
