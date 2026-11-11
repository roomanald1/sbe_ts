use anyhow::{Context, bail};
use axum::{Router, body::Body};
use futures_util::{Stream, stream};
use http_body_util::BodyExt;
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::{WebsocketClientTransport, WebsocketServerTransport};
use std::{collections::BTreeMap, pin::Pin, time::Duration};
use strategies::{
    ScoutCandidateBatch, ScoutCandidateBatchResult, ScoutCoordinatorRequest,
    ScoutCoordinatorResponse, ScoutCoordinatorRouteCommand, ScoutCoordinatorRouteResult,
    ScoutFeedbackAcknowledgement, ScoutWorkAssignment, ScoutWorkResult, ScoutWorkerHttpMethod,
    ScoutWorkerRequest, ScoutWorkerResponse, ScoutWorkerRouteCommand, ScoutWorkerRouteResult,
    Strategy, StrategyProfile,
};
use tower::ServiceExt;

use crate::ServiceState;

const DEFAULT_MAX_SCOUT_BATCH_SIGNALS: usize = 64;
const DEFAULT_MAX_SCOUT_ASSIGNMENTS: usize = 512;
const DEFAULT_MAX_SCOUT_BATCH_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_WORKER_TIMEOUT_MS: usize = 3_000;

#[derive(Clone)]
pub struct ScoutCoordinator {
    state: ServiceState,
    local_router: Router,
    worker_endpoints: BTreeMap<String, String>,
    max_batch_signals: usize,
    max_assignments: usize,
    max_batch_bytes: usize,
    worker_timeout: Duration,
    enabled: bool,
}

impl ScoutCoordinator {
    pub(crate) fn new(
        state: ServiceState,
        worker_endpoints: &str,
        local_router: Router,
    ) -> Result<Self, String> {
        let worker_endpoints = parse_worker_endpoints(worker_endpoints)?;
        Ok(Self {
            state,
            local_router,
            worker_endpoints,
            max_batch_signals: env_limit(
                "SCOUT_COORDINATOR_MAX_BATCH_SIGNALS",
                DEFAULT_MAX_SCOUT_BATCH_SIGNALS,
            )?,
            max_assignments: env_limit(
                "SCOUT_COORDINATOR_MAX_ASSIGNMENTS",
                DEFAULT_MAX_SCOUT_ASSIGNMENTS,
            )?,
            max_batch_bytes: env_limit(
                "SCOUT_COORDINATOR_MAX_BATCH_BYTES",
                DEFAULT_MAX_SCOUT_BATCH_BYTES,
            )?,
            worker_timeout: Duration::from_millis(env_limit(
                "SCOUT_COORDINATOR_WORKER_TIMEOUT_MS",
                DEFAULT_WORKER_TIMEOUT_MS,
            )? as u64),
            enabled: std::env::var("SCOUT_WORKERS_ENABLED")
                .map(|value| value.eq_ignore_ascii_case("true"))
                .unwrap_or(true),
        })
    }

    pub async fn serve(self, address: String) -> anyhow::Result<()> {
        let responder = CoordinatorResponder(self);
        eprintln!("Learning Scout coordinator listening on ws://{address}");
        RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(responder.clone()))
            }))
            .serve()
            .await
            .context("Learning Scout coordinator stopped")
    }

    async fn coordinate(
        &self,
        request: ScoutCandidateBatch,
    ) -> anyhow::Result<ScoutCandidateBatchResult> {
        if request.request_id.trim().is_empty() {
            bail!("Scout batch request ID is required");
        }
        if request.signals.len() > self.max_batch_signals {
            bail!("Scout batch exceeds the configured signal limit");
        }
        if !self.enabled {
            return Ok(ScoutCandidateBatchResult {
                request_id: request.request_id,
                assignments: Vec::new(),
            });
        }

        let state = self.state.state.lock().await;
        let mut assignments = Vec::new();
        for (signal_index, signal) in request.signals.iter().enumerate() {
            let symbol = signal["symbol"]
                .as_str()
                .filter(|symbol| !symbol.is_empty())
                .context("Scout signal is missing its symbol")?;
            for profile in selected_profiles(&state, symbol, signal)? {
                let strategy = Strategy::from_name(&profile.strategy).with_context(|| {
                    format!("Learning profile has unknown strategy {}", profile.strategy)
                })?;
                let endpoint = self
                    .worker_endpoints
                    .get(strategy.as_str())
                    .with_context(|| {
                        format!("no RSocket worker configured for {}", strategy.as_str())
                    })?;
                let candidate = candidates_for_profile(signal, &profile)?;
                for candidate in candidate {
                    if assignments.len() >= self.max_assignments {
                        bail!("Scout batch exceeds the configured assignment limit");
                    }
                    let direction = candidate["direction"].as_str().unwrap_or("neutral");
                    let assignment_id = format!(
                        "{}:{}:{}:{}:{}:{}",
                        request.request_id,
                        signal_index,
                        symbol,
                        strategy.as_str(),
                        profile.profile_id,
                        direction
                    );
                    assignments.push((
                        endpoint.clone(),
                        ScoutWorkAssignment {
                            assignment_id,
                            strategy,
                            mode: request.mode,
                            symbol: symbol.to_owned(),
                            parameters: profile.parameters.clone(),
                            candidate,
                        },
                        walk_forward_evidence(&state, &profile, signal_timestamp_ms(signal)),
                    ));
                }
            }
        }
        drop(state);

        let mut results = Vec::with_capacity(assignments.len());
        for (endpoint, assignment, evidence) in assignments {
            let expected_id = assignment.assignment_id.clone();
            let expected_strategy = assignment.strategy;
            let mut result = tokio::time::timeout(
                self.worker_timeout,
                dispatch_assignment(&endpoint, &assignment),
            )
            .await
            .with_context(|| format!("Scout worker {} timed out", expected_strategy.as_str()))?
            .with_context(|| format!("Scout worker {} failed", expected_strategy.as_str()))?;
            if result.assignment_id != expected_id || result.strategy != expected_strategy {
                bail!("Scout worker returned a result for a different assignment");
            }
            result.walk_forward_evidence = evidence;
            results.push(result);
        }
        Ok(ScoutCandidateBatchResult {
            request_id: request.request_id,
            assignments: results,
        })
    }

    async fn route_worker_command(
        &self,
        command: ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<ScoutCoordinatorRouteResult> {
        if !self.enabled {
            bail!("Scout execution is disabled");
        }
        let targets = self.route_targets(&command).await?;
        if targets.is_empty() {
            return Ok(ScoutCoordinatorRouteResult {
                request_id: command.request_id,
                status: 204,
                body: serde_json::Value::Null,
            });
        }
        let mut results = Vec::with_capacity(targets.len());
        for strategy in targets {
            let endpoint = self
                .worker_endpoints
                .get(strategy.as_str())
                .with_context(|| {
                    format!("no RSocket worker configured for {}", strategy.as_str())
                })?;
            let mut payload = command.payload.clone();
            if command.path == "/configuration" && command.method == ScoutWorkerHttpMethod::Post {
                payload["strategies"] = serde_json::json!([strategy.as_str()]);
            }
            let worker_path = if command.path.split('?').next() == Some("/outcomes") {
                replace_query_parameter(&command.path, "page", "0", "page_size", "100")
            } else {
                command.path.clone()
            };
            let worker_command = ScoutWorkerRouteCommand {
                request_id: format!("{}:{}", command.request_id, strategy.as_str()),
                strategy,
                method: command.method,
                path: worker_path,
                payload,
            };
            results.push(self.dispatch_route(endpoint, worker_command).await?);
        }
        Ok(aggregate_route_results(command, results))
    }

    async fn accept_scout_feedback(
        &self,
        event: strategies::FeedbackEvent,
    ) -> anyhow::Result<ScoutFeedbackAcknowledgement> {
        let event_id = crate::event_parts(&event).0.to_owned();
        match crate::accept_feedback(axum::extract::State(self.state.clone()), axum::Json(event))
            .await
        {
            Ok(result) => Ok(ScoutFeedbackAcknowledgement {
                event_id,
                accepted: result.0.accepted,
                duplicate: result.0.duplicate,
                status: 200,
                detail: None,
            }),
            Err((status, message)) => Ok(ScoutFeedbackAcknowledgement {
                event_id,
                accepted: false,
                duplicate: false,
                status: status.as_u16(),
                detail: Some(message),
            }),
        }
    }

    async fn profile_snapshot(&self) -> anyhow::Result<serde_json::Value> {
        let state = self.state.state.lock().await;
        let profiles = state
            .active_profiles
            .values()
            .map(serde_json::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(serde_json::json!({"profiles": profiles}))
    }

    async fn dispatch_route(
        &self,
        endpoint: &str,
        command: ScoutWorkerRouteCommand,
    ) -> anyhow::Result<ScoutWorkerRouteResult> {
        let client = RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(endpoint))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .with_context(|| format!("could not connect to Scout worker at {endpoint}"))?;
        let response = tokio::time::timeout(
            self.worker_timeout,
            client.request_response(
                Payload::builder()
                    .set_data(serde_json::to_vec(&ScoutWorkerRequest::Route(
                        command.clone(),
                    ))?)
                    .build(),
            ),
        )
        .await
        .context("Scout worker route request timed out")??
        .context("Scout worker route returned an empty response")?;
        let response: ScoutWorkerResponse = serde_json::from_slice(
            response
                .data()
                .context("Scout worker route response had no data")?,
        )
        .context("Scout worker route response was invalid")?;
        let ScoutWorkerResponse::Route(result) = response else {
            bail!("Scout worker returned the wrong operation response");
        };
        if result.request_id != command.request_id || result.strategy != command.strategy {
            bail!("Scout worker returned a result for a different lifecycle command");
        }
        Ok(result)
    }

    async fn route_targets(
        &self,
        command: &ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<Vec<Strategy>> {
        if let Some(strategy) = command.strategy {
            return Ok(self
                .worker_endpoints
                .contains_key(strategy.as_str())
                .then_some(strategy)
                .into_iter()
                .collect());
        }
        let mut names = route_strategy_names(command);
        if names.is_empty() {
            let state = self.state.state.lock().await;
            names = state
                .active_profiles
                .values()
                .map(|profile| profile.strategy.clone())
                .collect();
        }
        if names.is_empty() {
            names.extend(Strategy::live_strategies().map(|strategy| strategy.as_str().to_owned()));
        }
        let mut strategies = names
            .into_iter()
            .filter_map(|name| Strategy::from_name(&name))
            .filter(|strategy| self.worker_endpoints.contains_key(strategy.as_str()))
            .collect::<Vec<_>>();
        let path = command.path.split('?').next().unwrap_or(&command.path);
        if path == "/outcomes" || path == "/status" || path == "/configuration" {
            strategies = self
                .worker_endpoints
                .keys()
                .filter_map(|name| Strategy::from_name(name))
                .collect();
        }
        strategies.sort_by_key(|strategy| strategy.as_str());
        strategies.dedup();
        Ok(strategies)
    }

    async fn local_route(
        &self,
        command: ScoutCoordinatorRouteCommand,
    ) -> anyhow::Result<ScoutCoordinatorRouteResult> {
        let path = command.path.split('?').next().unwrap_or(&command.path);
        if !learning_route_allowed(path, command.method) {
            bail!("unsupported Learning RSocket route: {path}");
        }
        let method = match command.method {
            ScoutWorkerHttpMethod::Get => http::Method::GET,
            ScoutWorkerHttpMethod::Post => http::Method::POST,
            ScoutWorkerHttpMethod::Put => http::Method::PUT,
            ScoutWorkerHttpMethod::Delete => http::Method::DELETE,
        };
        let body = if method == http::Method::GET || method == http::Method::DELETE {
            Vec::new()
        } else {
            serde_json::to_vec(&command.payload)?
        };
        let response = self
            .local_router
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(&command.path)
                    .header(http::header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))?,
            )
            .await?;
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await?.to_bytes();
        let body = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| serde_json::Value::String(String::from_utf8_lossy(&bytes).into()));
        Ok(ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status,
            body,
        })
    }
}

fn learning_route_allowed(path: &str, method: ScoutWorkerHttpMethod) -> bool {
    matches!(
        (method, path),
        (ScoutWorkerHttpMethod::Get, "/profiles")
            | (ScoutWorkerHttpMethod::Get, "/decision-snapshot")
            | (ScoutWorkerHttpMethod::Get, "/summary")
            | (ScoutWorkerHttpMethod::Post, "/feedback")
            | (ScoutWorkerHttpMethod::Post, "/exit-timing")
            | (ScoutWorkerHttpMethod::Post, "/context-thresholds")
            | (ScoutWorkerHttpMethod::Post, "/execution-decision")
            | (ScoutWorkerHttpMethod::Post, "/execution-outcomes")
    )
}

fn route_strategy_names(command: &ScoutCoordinatorRouteCommand) -> Vec<String> {
    fn collect(value: &serde_json::Value, names: &mut std::collections::BTreeSet<String>) {
        match value {
            serde_json::Value::Object(object) => {
                for (key, value) in object {
                    if matches!(key.as_str(), "strategy" | "strategy_name") {
                        if let Some(name) = value.as_str() {
                            if Strategy::from_name(name).is_some() {
                                names.insert(name.to_owned());
                            }
                        }
                    }
                    collect(value, names);
                }
            }
            serde_json::Value::Array(values) => {
                for value in values {
                    collect(value, names);
                }
            }
            _ => {}
        }
    }
    let mut names = std::collections::BTreeSet::new();
    collect(&command.payload, &mut names);
    if command.path == "/live-inputs" {
        return Strategy::live_strategies()
            .map(|strategy| strategy.as_str().to_owned())
            .collect();
    }
    if command.path == "/dated-carry-inputs" {
        return vec![Strategy::DatedCarry.as_str().to_owned()];
    }
    if command.path == "/variants" || command.path == "/variants/batch" {
        return Vec::new();
    }
    names.into_iter().collect()
}

fn replace_query_parameter(
    path: &str,
    first_key: &str,
    first_value: &str,
    second_key: &str,
    second_value: &str,
) -> String {
    let (base, query) = path.split_once('?').unwrap_or((path, ""));
    let mut parameters = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .collect::<BTreeMap<_, _>>();
    parameters.insert(first_key, first_value);
    parameters.insert(second_key, second_value);
    let query = parameters
        .into_iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    format!("{base}?{query}")
}

fn aggregate_route_results(
    command: ScoutCoordinatorRouteCommand,
    results: Vec<ScoutWorkerRouteResult>,
) -> ScoutCoordinatorRouteResult {
    if results.len() == 1 && command.path.split('?').next() != Some("/outcomes") {
        let result = results.into_iter().next().unwrap();
        return ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status: result.status,
            body: result.body,
        };
    }
    if let Some(error) = results.iter().find(|result| result.status >= 400) {
        return ScoutCoordinatorRouteResult {
            request_id: command.request_id,
            status: error.status,
            body: error.body.clone(),
        };
    }
    let path = command.path.split('?').next().unwrap_or(&command.path);
    let bodies = results
        .into_iter()
        .map(|result| (result.strategy.as_str().to_owned(), result.body))
        .collect::<BTreeMap<_, _>>();
    let body = match path {
        "/live-inputs" | "/daily-inputs" | "/dated-carry-inputs" | "/scout-markout" => {
            serde_json::Value::Null
        }
        "/live-candidates" => serde_json::Value::Array(
            bodies
                .values()
                .flat_map(|body| body.as_array().cloned().unwrap_or_default())
                .collect(),
        ),
        "/configuration" => {
            let mut configuration = bodies.values().next().cloned().unwrap_or_default();
            configuration["strategies"] = serde_json::json!(bodies.keys().collect::<Vec<_>>());
            configuration
        }
        "/outcomes" => aggregate_outcomes(&bodies, &command.path),
        "/status" => aggregate_status(&bodies),
        _ => serde_json::json!({"by_strategy": bodies}),
    };
    ScoutCoordinatorRouteResult {
        request_id: command.request_id,
        status: 200,
        body,
    }
}

fn aggregate_outcomes(
    bodies: &BTreeMap<String, serde_json::Value>,
    path: &str,
) -> serde_json::Value {
    let mut rows = Vec::new();
    let mut learning_rows = Vec::new();
    let mut learning_outcomes = Vec::new();
    let mut pending_probes = Vec::new();
    let mut symbols = std::collections::BTreeSet::new();
    let mut summaries = serde_json::json!({"all":{"total":0,"open":0,"resolved":0,"wins":0,"losses":0,"flats":0,"unavailable":0},"live":{"total":0,"open":0,"resolved":0,"wins":0,"losses":0,"flats":0,"unavailable":0}});
    for body in bodies.values() {
        rows.extend(body["rows"].as_array().cloned().unwrap_or_default());
        learning_rows.extend(body["learningRows"].as_array().cloned().unwrap_or_default());
        learning_outcomes.extend(
            body["learningOutcomes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        pending_probes.extend(
            body["pendingParameterProbes"]
                .as_array()
                .cloned()
                .unwrap_or_default(),
        );
        symbols.extend(
            body["symbols"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str().map(str::to_owned)),
        );
        for group in ["all", "live"] {
            for key in [
                "total",
                "open",
                "resolved",
                "wins",
                "losses",
                "flats",
                "unavailable",
            ] {
                summaries[group][key] = serde_json::json!(
                    summaries[group][key]
                        .as_u64()
                        .unwrap_or_default()
                        .saturating_add(body["summary"][group][key].as_u64().unwrap_or_default())
                );
            }
        }
    }
    let params = path
        .split_once('?')
        .map(|(_, query)| query)
        .unwrap_or_default()
        .split('&')
        .filter_map(|part| part.split_once('='))
        .collect::<BTreeMap<_, _>>();
    let page = params
        .get("page")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or_default();
    let page_size = params
        .get("page_size")
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(50)
        .clamp(1, 100);
    let sort_by = params.get("sort_by").copied().unwrap_or("openedAt");
    let descending = params.get("sort_direction").copied() != Some("asc");
    rows.sort_by(|left, right| {
        let key = match sort_by {
            "symbol" => "symbol",
            "strategy" => "strategy",
            "status" => "status",
            "exitScenario" => "exitScenario",
            "scoutLevelPercent" => "scoutLevelPercent",
            "durationLabel" => "durationMs",
            "entryPrice" => "entryPrice",
            "markPrice" => "markPrice",
            "netReturnLabel" | "displayNetReturnPercent" => "displayNetReturnPercent",
            "pairedDeltaPercent" => "pairedDeltaPercent",
            "strength" => "strength",
            _ => "openedAt",
        };
        let ordering = if let (Some(a), Some(b)) = (left[key].as_f64(), right[key].as_f64()) {
            a.total_cmp(&b)
        } else if let (Some(a), Some(b)) = (left[key].as_str(), right[key].as_str()) {
            a.cmp(b)
        } else {
            left["openedAt"].as_u64().cmp(&right["openedAt"].as_u64())
        };
        let ordering = if descending {
            ordering.reverse()
        } else {
            ordering
        };
        ordering.then_with(|| {
            left["decisionId"]
                .as_str()
                .cmp(&right["decisionId"].as_str())
        })
    });
    let total_rows = rows.len();
    let page_count = total_rows.div_ceil(page_size);
    let page = page.min(page_count.saturating_sub(1));
    let page_rows = rows
        .drain(page.saturating_mul(page_size)..)
        .take(page_size)
        .collect::<Vec<_>>();
    serde_json::json!({
        "rows": page_rows, "summary": summaries, "learningRows": learning_rows,
        "learningOutcomes": learning_outcomes, "pendingParameterProbes": pending_probes,
        "symbols": symbols, "learningSymbol": params.get("learning_symbol").filter(|symbol| **symbol != "all").copied(), "updatedAt": crate::now_ms(),
        "source": params.get("source").copied().unwrap_or("live"),
        "status": params.get("status").copied().unwrap_or("all"),
        "sortBy": sort_by,
        "sortDirection": if descending { "desc" } else { "asc" },
        "page": page, "pageSize": page_size, "pageCount": page_count,
        "totalRows": total_rows, "pageNumber": page + 1,
        "hasPrevious": page > 0, "hasNext": page + 1 < page_count,
    })
}

fn aggregate_status(bodies: &BTreeMap<String, serde_json::Value>) -> serde_json::Value {
    let total = |key: &str| {
        bodies
            .values()
            .map(|body| body[key].as_u64().unwrap_or_default())
            .sum::<u64>()
    };
    serde_json::json!({
        "active_samples": total("active_samples"),
        "learning_samples": total("learning_samples"),
        "positive_samples": total("positive_samples"),
        "realized_pnl_usd": bodies.values().map(|body| body["realized_pnl_usd"].as_f64().unwrap_or_default()).sum::<f64>(),
    })
}

fn signal_timestamp_ms(signal: &serde_json::Value) -> u64 {
    signal["book_updated_at_ms"]
        .as_u64()
        .or_else(|| signal["timestamp"].as_u64())
        .unwrap_or_default()
}

fn selected_profiles(
    state: &crate::LearningState,
    symbol: &str,
    signal: &serde_json::Value,
) -> anyhow::Result<Vec<StrategyProfile>> {
    if signal["strategy_candidate"].as_bool() == Some(true) {
        return Ok(Vec::new());
    }
    let strategy_names = match signal["strategy"].as_str() {
        Some(name) => {
            if Strategy::from_name(name).is_none() {
                bail!("Scout signal names unknown strategy {name}");
            }
            vec![name.to_owned()]
        }
        None => Strategy::live_strategies()
            .map(|strategy| strategy.as_str().to_owned())
            .collect(),
    };
    let mut profiles = Vec::new();
    for strategy_name in strategy_names {
        let Some(strategy) = Strategy::from_name(&strategy_name) else {
            continue;
        };
        let key = format!("{symbol}:{strategy_name}");
        let baseline = state.active_profiles.get(&key).cloned().unwrap_or_else(|| {
            let parameters = strategy
                .parameter_specs()
                .into_iter()
                .map(|spec| (spec.key.as_str().to_owned(), spec.default))
                .collect::<BTreeMap<_, _>>();
            StrategyProfile {
                profile_id: strategies::profile_id(symbol, &strategy_name, &parameters),
                version: 0,
                symbol: symbol.to_owned(),
                strategy: strategy_name.clone(),
                parameters,
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            }
        });
        profiles.push(baseline.clone());
        let mut variants = state
            .variants
            .values()
            .filter(|registered| {
                registered.profile.symbol == symbol
                    && registered.profile.strategy == strategy_name
                    && registered.profile.parameters != baseline.parameters
            })
            .map(|registered| registered.profile.clone())
            .collect::<Vec<_>>();
        variants.sort_by(|left, right| left.profile_id.cmp(&right.profile_id));
        profiles.extend(variants);
    }
    Ok(profiles)
}

fn candidates_for_profile(
    signal: &serde_json::Value,
    profile: &StrategyProfile,
) -> anyhow::Result<Vec<serde_json::Value>> {
    if signal["symbol"].as_str() != Some(profile.symbol.as_str()) {
        bail!("Scout signal symbol does not match its selected Learning profile");
    }
    let strategy = Strategy::from_name(&profile.strategy)
        .with_context(|| format!("unknown strategy {}", profile.strategy))?;
    let mut candidate = signal.clone();
    candidate["strategy"] = serde_json::json!(profile.strategy);
    let mut direction = signal["direction"].as_str().unwrap_or_default().to_owned();
    if !matches!(direction.as_str(), "bullish" | "bearish") {
        let components = &signal["components"];
        let values = match strategy {
            Strategy::LiveMomentum => vec![
                components["momentum_10s_percent"].as_f64(),
                components["trend_5m_volatility_units"].as_f64(),
            ],
            Strategy::LiveTradeFlow => vec![components["trade_flow_imbalance"].as_f64()],
            Strategy::LiveOrderBook => vec![components["book_imbalance"].as_f64()],
            Strategy::LiveVwapDislocation => vec![components["mispricing_vs_vwap_bps"].as_f64()],
            Strategy::LiveAbsorptionPressure => vec![
                components["pressure_without_price_move"]
                    .as_f64()
                    .map(|value| -value),
            ],
            _ => Vec::new(),
        };
        let directions = values
            .into_iter()
            .flatten()
            .filter(|value| value.is_finite() && *value != 0.0)
            .map(|value| if value > 0.0 { "bullish" } else { "bearish" })
            .collect::<std::collections::BTreeSet<_>>();
        if directions.len() != 1 {
            return Ok(Vec::new());
        }
        direction = directions.into_iter().next().unwrap_or_default().to_owned();
    }
    if direction.is_empty() {
        return Ok(Vec::new());
    }
    let side = if direction == "bullish" { 1.0 } else { -1.0 };
    candidate["direction"] = serde_json::json!(direction);
    candidate["score"] =
        serde_json::json!(signal["score"].as_f64().unwrap_or_default().abs() * side);
    candidate["strength"] = signal["strength"].clone();
    let mut decision = candidate["strategy_decision"]
        .as_object()
        .cloned()
        .unwrap_or_default();
    decision.insert("strategy".into(), serde_json::json!(profile.strategy));
    decision.insert(
        "strategy_parameters".into(),
        serde_json::to_value(&profile.parameters)?,
    );
    decision.insert("direction".into(), serde_json::json!(direction));
    decision.insert(
        "direction_source".into(),
        serde_json::json!("strategy_insight"),
    );
    candidate["strategy_decision"] = serde_json::Value::Object(decision);
    Ok(vec![candidate])
}

fn walk_forward_evidence(
    state: &crate::LearningState,
    profile: &StrategyProfile,
    signal_timestamp_ms: u64,
) -> Option<strategies::StrategyWalkForwardEvidence> {
    let registered = state.variants.values().find(|registered| {
        registered.profile.symbol == profile.symbol
            && registered.profile.strategy == profile.strategy
            && registered.profile.parameters == profile.parameters
    })?;
    let samples = registered
        .outcomes
        .iter()
        .filter(|outcome| {
            outcome.observed_at_ms < signal_timestamp_ms
                && outcome.net_return_percent.is_finite()
                && matches!(outcome.source, crate::LearningOutcomeSource::Scout)
        })
        .collect::<Vec<_>>();
    if samples.is_empty() {
        return None;
    }
    let sample_count = samples.len();
    let positive_rate = samples
        .iter()
        .filter(|outcome| outcome.net_return_percent > 0.0)
        .count() as f64
        / sample_count as f64;
    let average_return = samples
        .iter()
        .map(|outcome| outcome.net_return_percent)
        .sum::<f64>()
        / sample_count as f64;
    Some(strategies::StrategyWalkForwardEvidence {
        sample_count,
        positive_net_rate: Some(positive_rate),
        average_net_return_percent: Some(average_return),
        last_sample_timestamp_ms: samples.last()?.observed_at_ms,
    })
}

fn parse_worker_endpoints(input: &str) -> Result<BTreeMap<String, String>, String> {
    let mut endpoints = BTreeMap::new();
    if input.trim().is_empty() {
        return Ok(endpoints);
    }
    for entry in input.split(';') {
        let (strategy_name, endpoint) = entry.split_once('=').ok_or_else(|| {
            "Scout worker endpoints must use strategy=ws://host:port entries".to_owned()
        })?;
        let strategy = Strategy::from_name(strategy_name)
            .ok_or_else(|| format!("unknown Scout worker strategy {strategy_name}"))?;
        if !(endpoint.starts_with("ws://") || endpoint.starts_with("wss://")) {
            return Err(format!(
                "Scout worker {} endpoint must use RSocket WebSocket",
                strategy.as_str()
            ));
        }
        if endpoints
            .insert(strategy.as_str().to_owned(), endpoint.to_owned())
            .is_some()
        {
            return Err(format!(
                "duplicate Scout worker endpoint for {}",
                strategy.as_str()
            ));
        }
    }
    Ok(endpoints)
}

fn env_limit(name: &str, default: usize) -> Result<usize, String> {
    match std::env::var(name) {
        Ok(value) => {
            let parsed = value
                .parse::<usize>()
                .map_err(|_| format!("{name} must be a positive integer"))?;
            if parsed == 0 {
                return Err(format!("{name} must be a positive integer"));
            }
            Ok(parsed)
        }
        Err(std::env::VarError::NotPresent) => Ok(default),
        Err(error) => Err(format!("could not read {name}: {error}")),
    }
}

async fn dispatch_assignment(
    endpoint: &str,
    assignment: &ScoutWorkAssignment,
) -> anyhow::Result<ScoutWorkResult> {
    let client = RSocketFactory::connect()
        .transport(WebsocketClientTransport::from(endpoint))
        .acceptor(Box::new(|| Box::new(EchoRSocket)))
        .start()
        .await
        .with_context(|| format!("could not connect to Scout worker at {endpoint}"))?;
    let response = client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(&ScoutWorkerRequest::Evaluate(
                    assignment.clone(),
                ))?)
                .build(),
        )
        .await?
        .context("Scout worker returned an empty response")?;
    let response: ScoutWorkerResponse = serde_json::from_slice(
        response
            .data()
            .context("Scout worker response had no data")?,
    )
    .context("Scout worker response was not a valid result")?;
    let ScoutWorkerResponse::Evaluation(result) = response else {
        bail!("Scout worker returned the wrong operation response");
    };
    Ok(result)
}

#[derive(Clone)]
struct CoordinatorResponder(ScoutCoordinator);

#[rsocket_rust::async_trait]
impl RSocket for CoordinatorResponder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Learning Scout coordinator accepts request-response only")
    }

    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Learning Scout coordination requires request-response results")
    }

    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let data = req.data().context("Scout candidate batch is empty")?;
        if data.len() > self.0.max_batch_bytes {
            bail!("Scout candidate batch exceeds the configured payload limit");
        }
        let request: ScoutCoordinatorRequest =
            serde_json::from_slice(data).context("invalid Scout coordinator request")?;
        let result = match request {
            ScoutCoordinatorRequest::CandidateBatch(request) => {
                ScoutCoordinatorResponse::CandidateBatch(self.0.coordinate(request).await?)
            }
            ScoutCoordinatorRequest::WorkerRoute(command) => {
                ScoutCoordinatorResponse::WorkerRoute(self.0.route_worker_command(command).await?)
            }
            ScoutCoordinatorRequest::Feedback(event) => {
                ScoutCoordinatorResponse::Feedback(self.0.accept_scout_feedback(event).await?)
            }
            ScoutCoordinatorRequest::LearningRoute(command) => {
                ScoutCoordinatorResponse::LearningRoute(self.0.local_route(command).await?)
            }
            ScoutCoordinatorRequest::ProfileSnapshot => {
                ScoutCoordinatorResponse::ProfileSnapshot(self.0.profile_snapshot().await?)
            }
        };
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&result)?)
                .build(),
        ))
    }

    fn request_stream(
        &self,
        _req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Learning Scout coordinator does not stream work"
            ))
        }))
    }

    fn request_channel(
        &self,
        _reqs: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Learning Scout coordinator does not accept request channels"
            ))
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LearningState;
    use rsocket_rust_transport_websocket::WebsocketServerTransport;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex as StdMutex};
    use strategies::StrategyProfile;
    use tokio::sync::Mutex;

    struct AssignmentResponder(Arc<StdMutex<Vec<ScoutWorkAssignment>>>);

    #[rsocket_rust::async_trait]
    impl RSocket for AssignmentResponder {
        async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }
        async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
            bail!("unsupported")
        }
        async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
            let request: ScoutWorkerRequest =
                serde_json::from_slice(req.data().context("missing test assignment")?)?;
            let ScoutWorkerRequest::Evaluate(assignment) = request else {
                bail!("expected evaluation assignment")
            };
            self.0.lock().unwrap().push(assignment.clone());
            let result = ScoutWorkResult {
                assignment_id: assignment.assignment_id,
                strategy: assignment.strategy,
                symbol: assignment.symbol,
                decision: strategies::ScoutWorkDecision::Enter,
                candidate: assignment.candidate,
                walk_forward_evidence: None,
            };
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&ScoutWorkerResponse::Evaluation(
                        result,
                    ))?)
                    .build(),
            ))
        }
        fn request_stream(
            &self,
            _req: Payload,
        ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
            Box::pin(stream::empty())
        }
        fn request_channel(
            &self,
            reqs: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            reqs
        }
    }

    #[test]
    fn worker_endpoint_configuration_accepts_only_known_strategy_rsocket_addresses() {
        let parsed = parse_worker_endpoints(
            "live-momentum=ws://scout-live-momentum:10200;dated-futures-carry=wss://carry:10200",
        )
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert!(parse_worker_endpoints("not-a-strategy=ws://worker:10200").is_err());
        assert!(parse_worker_endpoints("live-momentum=http://worker:10200").is_err());
        assert!(parse_worker_endpoints("live-momentum=ws://a:1;live-momentum=ws://b:2").is_err());
    }

    #[test]
    fn outcome_aggregation_merges_worker_pages_and_preserves_global_pagination() {
        let bodies = BTreeMap::from([
            (
                "live-momentum".to_owned(),
                serde_json::json!({
                    "rows":[{"decisionId":"b","openedAt":20,"strategy":"live-momentum"}],
                    "summary":{"all":{"total":1,"wins":1},"live":{"total":1,"wins":1}},
                    "learningRows":[],"learningOutcomes":[],"pendingParameterProbes":[],"symbols":["BTC-USDT"]
                }),
            ),
            (
                "live-trade-flow".to_owned(),
                serde_json::json!({
                    "rows":[{"decisionId":"a","openedAt":10,"strategy":"live-trade-flow"}],
                    "summary":{"all":{"total":1,"wins":0},"live":{"total":1,"wins":0}},
                    "learningRows":[],"learningOutcomes":[],"pendingParameterProbes":[],"symbols":["ETH-USDT"]
                }),
            ),
        ]);
        let response = aggregate_outcomes(&bodies, "/outcomes?source=all&page=1&page_size=1");
        assert_eq!(response["totalRows"], 2);
        assert_eq!(response["pageCount"], 2);
        assert_eq!(response["rows"][0]["decisionId"], "a");
        assert_eq!(response["summary"]["all"]["total"], 2);
        assert_eq!(response["summary"]["all"]["wins"], 1);
        assert_eq!(response["symbols"].as_array().unwrap().len(), 2);
    }

    #[tokio::test]
    async fn learning_selects_active_profile_and_dispatches_typed_assignment_over_rsocket() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let assignments = Arc::new(StdMutex::new(Vec::new()));
        let worker_assignments = Arc::clone(&assignments);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(AssignmentResponder(Arc::clone(
                    &worker_assignments,
                ))))
            }))
            .serve();
        let server_task = tokio::spawn(async move { server.await });

        let mut state = LearningState::default();
        state.active_profiles.insert(
            "BTC-USDT:live-momentum".into(),
            StrategyProfile {
                profile_id: "profile-v2".into(),
                version: 2,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("minimum_momentum".into(), 0.45)]),
                learning_samples: 12,
                positive_samples: 9,
                realized_pnl: 2.3,
            },
        );
        let service = ServiceState {
            state: Arc::new(Mutex::new(state)),
            path: PathBuf::new(),
        };
        let endpoint = format!("live-momentum=ws://{address}");
        let coordinator = ScoutCoordinator::new(service, &endpoint, Router::new()).unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        let response = coordinator
            .coordinate(ScoutCandidateBatch {
                request_id: "signal-batch-1".into(),
                mode: strategies::ScoutWorkMode::Evaluate,
                signals: vec![serde_json::json!({
                    "symbol":"BTC-USDT", "strategy":"live-momentum", "direction":"bullish",
                    "components":{"momentum_10s_percent":0.8}
                })],
            })
            .await
            .unwrap();
        assert_eq!(response.request_id, "signal-batch-1");
        assert_eq!(response.assignments.len(), 1);
        assert_eq!(
            response.assignments[0].decision,
            strategies::ScoutWorkDecision::Enter
        );
        let received = assignments.lock().unwrap();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0].strategy, Strategy::LiveMomentum);
        assert_eq!(received[0].parameters["minimum_momentum"], 0.45);
        assert_eq!(received[0].symbol, "BTC-USDT");

        server_task.abort();
    }
}
