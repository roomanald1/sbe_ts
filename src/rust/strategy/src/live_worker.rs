use anyhow::{Context, bail};
use futures_util::{Stream, stream};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{pin::Pin, sync::Arc};
use tokio::sync::Semaphore;

use crate::worker_metrics::{WorkerMetrics, serve_metrics};
use crate::{
    LIVE_EVALUATION_CONTRACT_VERSION, LiveEvaluationError, LiveEvaluationErrorCode,
    LiveEvaluationInput, LiveEvaluationRequest, LiveEvaluationResponse, LiveStrategyDecision,
    Strategy, StrategyDecision, StrategyInput,
};

#[derive(Clone, Debug)]
pub struct LiveWorkerConfig {
    pub strategy: Strategy,
    pub listen_addr: String,
    pub max_in_flight: usize,
    pub max_payload_bytes: usize,
    pub metrics_listen_addr: String,
}

impl LiveWorkerConfig {
    pub fn from_env(strategy: Strategy) -> anyhow::Result<Self> {
        let config = Self {
            strategy,
            listen_addr: std::env::var("STRATEGY_X_LIVE_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:10201".into()),
            max_in_flight: env_usize("STRATEGY_X_LIVE_MAX_IN_FLIGHT", 32)?,
            max_payload_bytes: env_usize("STRATEGY_X_LIVE_MAX_PAYLOAD_BYTES", 262_144)?,
            metrics_listen_addr: std::env::var("STRATEGY_X_METRICS_LISTEN_ADDR")
                .unwrap_or_else(|_| "0.0.0.0:10202".into()),
        };
        if config.max_in_flight == 0 || config.max_payload_bytes == 0 {
            bail!("Live worker limits must be positive");
        }
        Ok(config)
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

struct LiveWorker {
    config: LiveWorkerConfig,
    in_flight: Arc<Semaphore>,
    metrics: Arc<WorkerMetrics>,
}

impl LiveWorker {
    fn new(config: LiveWorkerConfig) -> Self {
        Self {
            in_flight: Arc::new(Semaphore::new(config.max_in_flight)),
            metrics: WorkerMetrics::new("live", config.strategy.as_str()),
            config,
        }
    }

    async fn evaluate(&self, bytes: &[u8]) -> anyhow::Result<LiveEvaluationResponse> {
        self.metrics.begin();
        let started = std::time::Instant::now();
        let result = self.evaluate_inner(bytes).await;
        self.metrics.finish(
            started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
            result
                .as_ref()
                .err()
                .map(|error| WorkerMetrics::classify_error(&error.to_string())),
        );
        result
    }

    async fn evaluate_inner(&self, bytes: &[u8]) -> anyhow::Result<LiveEvaluationResponse> {
        if bytes.len() > self.config.max_payload_bytes {
            bail!("live evaluation exceeds configured payload limit");
        }
        let _permit = Arc::clone(&self.in_flight)
            .try_acquire_owned()
            .map_err(|_| anyhow::anyhow!("live worker is at its configured in-flight limit"))?;
        let request: LiveEvaluationRequest =
            serde_json::from_slice(bytes).context("invalid live evaluation request")?;
        let mut response = LiveEvaluationResponse {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            request_id: request.request_id.clone(),
            strategy: request.strategy,
            symbol: request.symbol.clone(),
            profile_id: request.profile.profile_id.clone(),
            profile_symbol: request.profile.profile_symbol.clone(),
            profile_version: request.profile.version,
            decision: None,
            error: None,
        };
        match evaluate_request(&request, self.config.strategy) {
            Ok(decision) => response.decision = Some(decision),
            Err(error) => response.error = Some(error),
        }
        Ok(response)
    }
}

fn invalid(code: LiveEvaluationErrorCode, message: impl Into<String>) -> LiveEvaluationError {
    LiveEvaluationError {
        code,
        message: message.into(),
    }
}

fn evaluate_request(
    request: &LiveEvaluationRequest,
    worker_strategy: Strategy,
) -> Result<LiveStrategyDecision, LiveEvaluationError> {
    if request.contract_version != LIVE_EVALUATION_CONTRACT_VERSION {
        return Err(invalid(
            LiveEvaluationErrorCode::UnsupportedVersion,
            format!("unsupported contract version {}", request.contract_version),
        ));
    }
    if request.request_id.trim().is_empty() {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidRequest,
            "request ID is required",
        ));
    }
    if request.strategy != worker_strategy {
        return Err(invalid(
            LiveEvaluationErrorCode::WrongStrategy,
            format!(
                "request for {} routed to {} worker",
                request.strategy.as_str(),
                worker_strategy.as_str()
            ),
        ));
    }
    if request.symbol.trim().is_empty() {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidSymbol,
            "symbol is required",
        ));
    }
    if request.profile.profile_id.trim().is_empty()
        || request.profile.profile_symbol.trim().is_empty()
    {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidProfile,
            "profile ID and profile symbol are required",
        ));
    }
    let specs = crate::parameter_specs(worker_strategy);
    for (key, value) in &request.profile.parameters {
        let Some(spec) = specs.iter().find(|spec| spec.key.as_str() == key) else {
            return Err(invalid(
                LiveEvaluationErrorCode::InvalidParameters,
                format!("unknown parameter {key}"),
            ));
        };
        let step_position = (*value - spec.min) / spec.step;
        if !value.is_finite()
            || *value < spec.min
            || *value > spec.max
            || spec.step <= 0.0
            || (step_position - step_position.round()).abs() > 1e-6
        {
            return Err(invalid(
                LiveEvaluationErrorCode::InvalidParameters,
                format!("parameter {key} is outside its bounds"),
            ));
        }
    }
    let expected_profile_id = crate::profile_id(
        &request.profile.profile_symbol,
        request.strategy.as_str(),
        &request.profile.parameters,
    );
    if request.profile.profile_id != expected_profile_id {
        return Err(invalid(
            LiveEvaluationErrorCode::InvalidProfile,
            "profile ID does not match profile symbol, strategy, and parameters",
        ));
    }

    let parameters = &request.profile.parameters;
    let input = match &request.input {
        LiveEvaluationInput::Entry {
            signal,
            daily_features,
            walk_forward_evidence,
        } => {
            if signal.symbol != request.symbol {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "signal symbol does not match request symbol",
                ));
            }
            if signal.strategy.as_deref().and_then(Strategy::from_name) != Some(request.strategy) {
                return Err(invalid(
                    LiveEvaluationErrorCode::WrongStrategy,
                    "signal strategy does not match request strategy",
                ));
            }
            let mut input = StrategyInput::entry(signal, parameters);
            input.daily_features = *daily_features;
            input.walk_forward_evidence = *walk_forward_evidence;
            input
        }
        LiveEvaluationInput::OpenPosition { position } => {
            if !position.is_open {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidRequest,
                    "open-position evaluation requires an open position",
                ));
            }
            let mut input = StrategyInput::position(
                parameters,
                position.strategy_enabled,
                position.signal_active,
                position.rebalance_required,
            );
            input.daily_features = position.daily_features;
            input
        }
        LiveEvaluationInput::DatedCarryEntry {
            emitted_at_ms,
            evaluated_at_ms,
            candidate,
        } => {
            if request.strategy != Strategy::DatedCarry || candidate.spot_symbol != request.symbol {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "dated-carry candidate does not match strategy or symbol",
                ));
            }
            StrategyInput::dated_carry_entry(
                candidate,
                *emitted_at_ms,
                *evaluated_at_ms,
                parameters,
            )
        }
        LiveEvaluationInput::Settlement {
            emitted_at_ms,
            evaluated_at_ms,
            spot_symbol,
            future_symbol,
            expiry_ms,
            spot_exit_twap,
            settlement_price,
            settlement_source,
        } => {
            if request.strategy != Strategy::DatedCarry
                || spot_symbol != &request.symbol
                || future_symbol.trim().is_empty()
            {
                return Err(invalid(
                    LiveEvaluationErrorCode::InvalidSymbol,
                    "settlement does not match dated-carry strategy or request symbol",
                ));
            }
            StrategyInput::dated_carry_settlement(
                *emitted_at_ms,
                *evaluated_at_ms,
                spot_symbol,
                future_symbol,
                *expiry_ms,
                *spot_exit_twap,
                *settlement_price,
                *settlement_source,
                parameters,
            )
        }
    };
    let evaluated = worker_strategy
        .evaluate(&input)
        .map_err(|message| invalid(LiveEvaluationErrorCode::EvaluationFailed, message))?;
    Ok(match evaluated {
        StrategyDecision::Enter => LiveStrategyDecision::Enter,
        StrategyDecision::Hold => LiveStrategyDecision::Hold,
        StrategyDecision::Exit => LiveStrategyDecision::Exit,
        StrategyDecision::Rebalance => LiveStrategyDecision::Rebalance,
        // Scout is a strategy-internal research result, never a live action.
        StrategyDecision::Scout | StrategyDecision::Reject => LiveStrategyDecision::Reject,
    })
}

#[derive(Clone)]
struct Responder(Arc<LiveWorker>);

#[rsocket_rust::async_trait]
impl RSocket for Responder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("live evaluation requires request-response")
    }
    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("live evaluation requires correlated request-response")
    }
    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let bytes = req.data().context("live evaluation payload is empty")?;
        let response = self.0.evaluate(bytes).await?;
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
            Err(anyhow::anyhow!("live worker does not stream work"))
        }))
    }
    fn request_channel(
        &self,
        _reqs: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!("live worker accepts request-response only"))
        }))
    }
}

pub async fn serve_live_worker(config: LiveWorkerConfig) -> anyhow::Result<()> {
    let worker = Arc::new(LiveWorker::new(config.clone()));
    let metrics_server = serve_metrics(config.metrics_listen_addr, Arc::clone(&worker.metrics));
    eprintln!(
        "Strategy X live worker for {} listening on ws://{} (max in-flight {}, max payload {} bytes)",
        config.strategy.as_str(),
        config.listen_addr,
        config.max_in_flight,
        config.max_payload_bytes
    );
    let rsocket_server = RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(config.listen_addr))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(Responder(Arc::clone(&worker))))
        }))
        .serve();
    tokio::try_join!(
        async {
            rsocket_server
                .await
                .context("Strategy X live RSocket worker stopped")
        },
        metrics_server
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SignalComponents, StrategyMarketSignal};
    use rsocket_rust::utils::EchoRSocket;
    use rsocket_rust_transport_websocket::WebsocketClientTransport;
    use std::collections::BTreeMap;
    use tokio::time::sleep;

    fn request() -> LiveEvaluationRequest {
        let parameters = BTreeMap::from([("minimum_momentum".into(), 0.005)]);
        LiveEvaluationRequest {
            contract_version: LIVE_EVALUATION_CONTRACT_VERSION,
            request_id: "r-1".into(),
            strategy: Strategy::LiveMomentum,
            symbol: "BTC-USDT".into(),
            profile: crate::LiveStrategyProfile {
                profile_id: crate::profile_id(
                    "BTC-USDT",
                    Strategy::LiveMomentum.as_str(),
                    &parameters,
                ),
                profile_symbol: "BTC-USDT".into(),
                version: 0,
                parameters,
            },
            input: LiveEvaluationInput::Entry {
                signal: StrategyMarketSignal {
                    symbol: "BTC-USDT".into(),
                    strategy: Some("live-momentum".into()),
                    direction: "bullish".into(),
                    components: SignalComponents {
                        momentum_10s_percent: Some(0.01),
                        ..Default::default()
                    },
                    ..Default::default()
                },
                daily_features: None,
                walk_forward_evidence: None,
            },
        }
    }

    #[test]
    fn validates_strategy_profile_and_correlates_result() {
        let req = request();
        let response = evaluate_request(&req, Strategy::LiveMomentum).unwrap();
        assert_eq!(response, LiveStrategyDecision::Enter);
        let mut wrong = req.clone();
        wrong.strategy = Strategy::DatedCarry;
        assert_eq!(
            evaluate_request(&wrong, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::WrongStrategy
        );
        let mut invalid = req.clone();
        invalid
            .profile
            .parameters
            .insert("minimum_momentum".into(), 1e20);
        assert_eq!(
            evaluate_request(&invalid, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidParameters
        );
        let mut invalid_step = req.clone();
        invalid_step
            .profile
            .parameters
            .insert("minimum_momentum".into(), 0.0055);
        assert_eq!(
            evaluate_request(&invalid_step, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidParameters
        );
        let mut invalid_profile = req.clone();
        invalid_profile.profile.profile_symbol = "ETH-USDT".into();
        assert_eq!(
            evaluate_request(&invalid_profile, Strategy::LiveMomentum)
                .unwrap_err()
                .code,
            LiveEvaluationErrorCode::InvalidProfile
        );
    }

    #[tokio::test]
    async fn live_role_returns_profile_attributed_correlated_rsocket_response() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let config = LiveWorkerConfig {
            strategy: Strategy::LiveMomentum,
            listen_addr: address.to_string(),
            max_in_flight: 1,
            max_payload_bytes: 64 * 1024,
            metrics_listen_addr: "127.0.0.1:0".into(),
        };
        let server = tokio::spawn(async move { serve_live_worker(config).await });
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
        let client = client.expect("live worker should accept an RSocket connection");
        let response = client
            .request_response(
                Payload::builder()
                    .set_data(serde_json::to_vec(&request()).unwrap())
                    .build(),
            )
            .await
            .unwrap()
            .expect("live worker should return request-response result");
        let result: LiveEvaluationResponse =
            serde_json::from_slice(response.data().unwrap()).unwrap();
        assert_eq!(result.request_id, "r-1");
        assert_eq!(result.strategy, Strategy::LiveMomentum);
        assert_eq!(result.profile_id, request().profile.profile_id);
        assert_eq!(result.profile_symbol, "BTC-USDT");
        assert_eq!(result.profile_version, 0);
        assert_eq!(result.decision, Some(LiveStrategyDecision::Enter));
        assert_eq!(result.error, None);
        server.abort();
        drop(client);
    }
}
