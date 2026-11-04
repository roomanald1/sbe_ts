//! Learning service and profile update engine. It is intentionally independent
//! of the trader's decision and order-execution state.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};
use strategy_core::{FeedbackEvent, ParameterSpec, StrategyProfile, StrategyProfileRegistration};
use tokio::sync::Mutex;

#[derive(Clone, Serialize, Deserialize)]
struct RegisteredProfile {
    profile: StrategyProfile,
    specs: Vec<ParameterSpec>,
    samples: usize,
    positive: usize,
    realized_pnl: f64,
    #[serde(default)]
    outcomes: Vec<LearningOutcome>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LearningOutcome {
    observed_at_ms: u64,
    net_return_percent: f64,
}

#[derive(Serialize)]
struct StrategySummary {
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct LearningState {
    variants: BTreeMap<String, RegisteredProfile>,
    active_profiles: BTreeMap<String, StrategyProfile>,
    seen_event_ids: BTreeSet<String>,
    accepted_events: u64,
}

#[derive(Clone)]
struct ServiceState {
    state: Arc<Mutex<LearningState>>,
    path: PathBuf,
}

#[derive(Deserialize, Serialize)]
struct FeedbackResult {
    accepted: bool,
    duplicate: bool,
    profile: Option<StrategyProfile>,
}

fn key(symbol: &str, strategy: &str) -> String {
    format!("{symbol}:{strategy}")
}

fn variant_key(profile: &StrategyProfile) -> String {
    format!(
        "{}:{}:{}:{:?}",
        profile.symbol, profile.strategy, profile.profile_id, profile.parameters
    )
}

fn event_parts(event: &FeedbackEvent) -> (&str, &str, &str, &BTreeMap<String, f64>, String) {
    match event {
        FeedbackEvent::ScoutOutcome {
            event_id,
            symbol,
            strategy,
            profile_id,
            parameters,
            ..
        } => (event_id, symbol, strategy, parameters, profile_id.clone()),
        FeedbackEvent::TradeOutcome {
            event_id,
            symbol,
            strategy,
            profile_id,
            parameters,
            ..
        } => (event_id, symbol, strategy, parameters, profile_id.clone()),
    }
}

pub fn router(path: PathBuf) -> Result<Router, String> {
    let stored = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice::<LearningState>(&bytes).map_err(|error| {
            format!(
                "could not decode learning state {}: {error}",
                path.display()
            )
        })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => LearningState::default(),
        Err(error) => {
            return Err(format!(
                "could not read learning state {}: {error}",
                path.display()
            ));
        }
    };
    let service = ServiceState {
        state: Arc::new(Mutex::new(stored)),
        path,
    };
    Ok(Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/profiles", get(get_profiles).post(register_profile))
        .route("/summary", get(get_summary))
        .route("/feedback", post(accept_feedback))
        .with_state(service))
}

async fn get_profiles(State(service): State<ServiceState>) -> Json<Vec<StrategyProfile>> {
    let state = service.state.lock().await;
    Json(state.active_profiles.values().cloned().collect())
}

async fn get_summary(State(service): State<ServiceState>) -> Json<Vec<StrategySummary>> {
    let state = service.state.lock().await;
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    let mut summaries: BTreeMap<String, Vec<f64>> = BTreeMap::new();
    for profile in state.variants.values() {
        summaries
            .entry(profile.profile.strategy.clone())
            .or_default()
            .extend(
                profile
                    .outcomes
                    .iter()
                    .filter(|outcome| outcome.observed_at_ms >= cutoff)
                    .map(|outcome| outcome.net_return_percent),
            );
    }
    Json(
        summaries
            .into_iter()
            .map(|(strategy, returns)| {
                let positive = returns.iter().filter(|value| **value > 0.0).count();
                let count = returns.len();
                StrategySummary {
                    strategy,
                    learning_samples: count,
                    positive_rate: (count > 0).then_some(positive as f64 / count as f64),
                    average_net_return_percent: (count > 0)
                        .then_some(returns.iter().sum::<f64>() / count as f64),
                }
            })
            .collect(),
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn event_observed_at(event: &FeedbackEvent) -> u64 {
    match event {
        FeedbackEvent::ScoutOutcome { observed_at_ms, .. }
        | FeedbackEvent::TradeOutcome { observed_at_ms, .. } => *observed_at_ms,
    }
}

async fn register_profile(
    State(service): State<ServiceState>,
    Json(request): Json<StrategyProfileRegistration>,
) -> Result<StatusCode, (StatusCode, String)> {
    if request.profile.profile_id.is_empty()
        || request
            .profile
            .parameters
            .values()
            .any(|value| !value.is_finite())
        || request.specs.iter().any(|spec| !valid_spec(spec))
        || request
            .specs
            .iter()
            .any(|spec| !request.profile.parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid strategy profile registration".into(),
        ));
    }
    let profile_key = key(&request.profile.symbol, &request.profile.strategy);
    let mut state = service.state.lock().await;
    let id = variant_key(&request.profile);
    state.variants.entry(id).or_insert(RegisteredProfile {
        profile: request.profile.clone(),
        specs: request.specs,
        samples: request.profile.learning_samples,
        positive: request.profile.positive_samples,
        realized_pnl: request.profile.realized_pnl,
        outcomes: Vec::new(),
    });
    let current = state
        .active_profiles
        .entry(profile_key)
        .or_insert(request.profile.clone());
    if request.profile.version > current.version {
        *current = request.profile;
    }
    persist(&service.path, &state)?;
    Ok(StatusCode::ACCEPTED)
}

async fn accept_feedback(
    State(service): State<ServiceState>,
    Json(event): Json<FeedbackEvent>,
) -> Result<Json<FeedbackResult>, (StatusCode, String)> {
    let (event_id, symbol, strategy, parameters, profile_id) = event_parts(&event);
    if event_id.is_empty() || parameters.values().any(|value| !value.is_finite()) {
        return Err((StatusCode::BAD_REQUEST, "invalid feedback event".into()));
    }
    let mut state = service.state.lock().await;
    if state.seen_event_ids.contains(event_id) {
        return Ok(Json(FeedbackResult {
            accepted: true,
            duplicate: true,
            profile: None,
        }));
    }
    let profile_key = key(symbol, strategy);
    let source_profile = StrategyProfile {
        profile_id: profile_id.clone(),
        version: 0,
        symbol: symbol.to_owned(),
        strategy: strategy.to_owned(),
        parameters: parameters.clone(),
        learning_samples: 0,
        positive_samples: 0,
        realized_pnl: 0.0,
    };
    let id = variant_key(&source_profile);
    let LearningState {
        variants,
        active_profiles,
        ..
    } = &mut *state;
    let registered = variants.entry(id).or_insert_with(|| RegisteredProfile {
        profile: source_profile,
        specs: event_specs(&event).to_vec(),
        samples: 0,
        positive: 0,
        realized_pnl: 0.0,
        outcomes: Vec::new(),
    });
    if registered
        .specs
        .iter()
        .any(|spec| !valid_spec(spec) || !parameters.contains_key(&spec.key))
    {
        return Err((
            StatusCode::CONFLICT,
            "feedback does not include valid parameter bounds".into(),
        ));
    }
    if event_specs(&event).iter().any(|spec| {
        !valid_spec(spec) || !parameters.contains_key(&spec.key) || !registered.specs.contains(spec)
    }) {
        return Err((
            StatusCode::CONFLICT,
            "Scout feedback references an unregistered parameter probe".into(),
        ));
    }
    if registered.profile.profile_id != profile_id || &registered.profile.parameters != parameters {
        return Err((
            StatusCode::CONFLICT,
            "feedback profile does not match registered variant".into(),
        ));
    }
    registered.samples = registered.samples.saturating_add(1);
    if event_is_positive(&event) {
        registered.positive = registered.positive.saturating_add(1);
    }
    registered.realized_pnl += event_value(&event);
    let net_return = match &event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        } => *net_return_percent,
        FeedbackEvent::TradeOutcome {
            net_return_percent, ..
        } => *net_return_percent,
    };
    registered.outcomes.push(LearningOutcome {
        observed_at_ms: event_observed_at(&event),
        net_return_percent: net_return,
    });
    let cutoff = now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    registered
        .outcomes
        .retain(|outcome| outcome.observed_at_ms >= cutoff);
    let updated = update_profile(&registered.profile, event_specs(&event), &event).ok_or((
        StatusCode::UNPROCESSABLE_ENTITY,
        "feedback did not match profile".into(),
    ))?;
    let should_promote = should_promote_variant(
        registered.samples,
        registered.positive,
        registered.realized_pnl,
    );
    if should_promote {
        let active = active_profiles
            .entry(profile_key)
            .or_insert(updated.clone());
        let mut promoted = updated.clone();
        promoted.version = active.version.saturating_add(1);
        promoted.profile_id =
            strategy_core::profile_id(&promoted.symbol, &promoted.strategy, &promoted.parameters);
        promoted.learning_samples = registered.samples;
        promoted.positive_samples = registered.positive;
        promoted.realized_pnl = registered.realized_pnl;
        *active = promoted;
    }
    state.seen_event_ids.insert(event_id.to_owned());
    while state.seen_event_ids.len() > 100_000 {
        let Some(oldest) = state.seen_event_ids.iter().next().cloned() else {
            break;
        };
        state.seen_event_ids.remove(&oldest);
    }
    state.accepted_events = state.accepted_events.saturating_add(1);
    persist(&service.path, &state)?;
    Ok(Json(FeedbackResult {
        accepted: true,
        duplicate: false,
        profile: Some(updated),
    }))
}

fn should_promote_variant(samples: usize, positive: usize, net_outcome: f64) -> bool {
    samples >= 5
        && positive.saturating_mul(2) > samples
        && net_outcome.is_finite()
        && net_outcome > 0.0
}

fn persist(path: &Path, state: &LearningState) -> Result<(), (StatusCode, String)> {
    let bytes = serde_json::to_vec(state)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let temporary = path.with_extension("json.tmp");
    std::fs::write(&temporary, bytes)
        .and_then(|()| std::fs::rename(&temporary, path))
        .map_err(|error| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("could not persist learning state: {error}"),
            )
        })
}

fn valid_spec(spec: &ParameterSpec) -> bool {
    spec.default.is_finite()
        && spec.min.is_finite()
        && spec.max.is_finite()
        && spec.step.is_finite()
        && spec.learning_direction.is_finite()
        && spec.min <= spec.default
        && spec.default <= spec.max
        && spec.min < spec.max
        && spec.step > 0.0
}

fn event_specs(event: &FeedbackEvent) -> &[ParameterSpec] {
    match event {
        FeedbackEvent::ScoutOutcome { specs, .. } | FeedbackEvent::TradeOutcome { specs, .. } => {
            specs
        }
    }
}

fn event_is_positive(event: &FeedbackEvent) -> bool {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        } => *net_return_percent > 0.0,
        FeedbackEvent::TradeOutcome {
            realized_pnl_usd, ..
        } => *realized_pnl_usd > 0.0,
    }
}

fn event_value(event: &FeedbackEvent) -> f64 {
    match event {
        FeedbackEvent::ScoutOutcome {
            net_return_percent, ..
        } => *net_return_percent,
        FeedbackEvent::TradeOutcome {
            realized_pnl_usd, ..
        } => *realized_pnl_usd,
    }
}

/// Apply one event only to the exact profile variant that generated it.
pub fn update_profile(
    profile: &StrategyProfile,
    specs: &[ParameterSpec],
    event: &FeedbackEvent,
) -> Option<StrategyProfile> {
    let (event_profile, strategy, symbol, parameters, outcome) = match event {
        FeedbackEvent::ScoutOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            net_return_percent,
            ..
        } => (
            profile_id,
            strategy,
            symbol,
            parameters,
            *net_return_percent,
        ),
        FeedbackEvent::TradeOutcome {
            profile_id,
            strategy,
            symbol,
            parameters,
            realized_pnl_usd,
            ..
        } => (profile_id, strategy, symbol, parameters, *realized_pnl_usd),
    };
    if event_profile != &profile.profile_id
        || strategy != &profile.strategy
        || symbol != &profile.symbol
        || parameters != &profile.parameters
        || !outcome.is_finite()
    {
        return None;
    }
    let mut updated = profile.clone();
    let positive = outcome > 0.0;
    for spec in specs {
        if !valid_spec(spec) {
            continue;
        }
        let Some(value) = updated.parameters.get_mut(&spec.key) else {
            continue;
        };
        let direction = if positive {
            spec.learning_direction.signum()
        } else {
            -spec.learning_direction.signum()
        };
        *value = (*value + direction * spec.step).clamp(spec.min, spec.max);
    }
    updated.version = updated.version.saturating_add(1);
    updated.profile_id =
        strategy_core::profile_id(&updated.symbol, &updated.strategy, &updated.parameters);
    Some(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    async fn json_request(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<Vec<u8>>,
    ) -> axum::response::Response {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        app.oneshot(builder.body(Body::from(body.unwrap_or_default())).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn service_persists_profiles_deduplicates_feedback_and_publishes_bounded_updates() {
        let path = std::env::temp_dir().join(format!("learning-test-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let profile = StrategyProfile {
            profile_id: "profile-a".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([
                ("threshold".into(), 0.5),
                ("take_profit_percent".into(), 0.8),
            ]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.1,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let unrelated_spec = ParameterSpec {
            key: "take_profit_percent".into(),
            label: "Take profit".into(),
            default: 0.8,
            min: 0.1,
            max: 1.5,
            step: 0.1,
            unit: "%".into(),
            learning_direction: 1.0,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/profiles",
            Some(
                serde_json::to_vec(&StrategyProfileRegistration {
                    profile: profile.clone(),
                    specs: vec![spec.clone(), unrelated_spec],
                })
                .unwrap(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        for index in 0..5 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("trade-{index}"),
                observed_at_ms: now_ms(),
                symbol: profile.symbol.clone(),
                strategy: profile.strategy.clone(),
                profile_id: profile.profile_id.clone(),
                parameters: profile.parameters.clone(),
                specs: vec![spec.clone()],
                scout: false,
                realized_pnl_usd: 1.0,
                net_return_percent: 1.0,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/feedback",
                Some(serde_json::to_vec(&event).unwrap()),
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
        }
        let duplicate = FeedbackEvent::TradeOutcome {
            event_id: "trade-4".into(),
            observed_at_ms: now_ms(),
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            specs: vec![spec],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/feedback",
            Some(serde_json::to_vec(&duplicate).unwrap()),
        )
        .await;
        let result: FeedbackResult =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert!(result.duplicate);

        let response = json_request(app, "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].parameters["threshold"], 0.6);
        assert_eq!(profiles[0].parameters["take_profit_percent"], 0.8);
        assert_eq!(profiles[0].learning_samples, 5);
        let _ = std::fs::remove_file(path);
    }

    fn profile() -> StrategyProfile {
        StrategyProfile {
            profile_id: "p1".into(),
            version: 3,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.9)]),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        }
    }

    #[test]
    fn feedback_updates_are_variant_scoped_and_bounded() {
        let profile = profile();
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.2,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "e1".into(),
            observed_at_ms: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "p1".into(),
            parameters: profile.parameters.clone(),
            specs: vec![spec.clone()],
            net_return_percent: 2.0,
        };
        let updated = update_profile(&profile, &[spec.clone()], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 1.0);
        assert_eq!(updated.version, 4);
        let FeedbackEvent::ScoutOutcome { parameters, .. } = event else {
            unreachable!()
        };
        let mut wrong = profile;
        wrong.parameters = parameters.into_iter().map(|(key, _)| (key, 0.1)).collect();
        assert!(
            update_profile(
                &wrong,
                &[],
                &FeedbackEvent::TradeOutcome {
                    event_id: "e2".into(),
                    observed_at_ms: 2,
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    profile_id: "p1".into(),
                    parameters: BTreeMap::from([("threshold".into(), 0.9)]),
                    specs: vec![spec],
                    scout: false,
                    realized_pnl_usd: 1.0,
                    net_return_percent: 1.0,
                }
            )
            .is_none()
        );
    }

    #[test]
    fn scout_feedback_changes_only_the_parameter_that_was_probed() {
        let mut profile = profile();
        profile.parameters.insert("take_profit_percent".into(), 0.8);
        let spec = ParameterSpec {
            key: "threshold".into(),
            label: "Threshold".into(),
            default: 0.5,
            min: 0.0,
            max: 1.0,
            step: 0.2,
            unit: "ratio".into(),
            learning_direction: 1.0,
        };
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "scout-probe-loss".into(),
            observed_at_ms: 1,
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            specs: vec![spec.clone()],
            net_return_percent: -1.0,
        };

        let updated = update_profile(&profile, &[spec], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.7);
        assert_eq!(updated.parameters["take_profit_percent"], 0.8);
    }

    #[test]
    fn a_parameter_variant_needs_more_positive_than_negative_scouts_to_promote() {
        assert!(!should_promote_variant(5, 2, 1.0));
        assert!(!should_promote_variant(5, 3, -0.1));
        assert!(should_promote_variant(5, 3, 0.1));
        assert!(!should_promote_variant(4, 4, 1.0));
    }
}
