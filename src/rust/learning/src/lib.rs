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
#[cfg(test)]
use strategies::StrategyThresholdObservation;
use strategies::{
    ContextLearningStat, ContextMissedOpportunityBand, ContextThresholdInput,
    ContextThresholdRequest, ContextThresholdResponse, ContextThresholds, ExecutionLearningRequest,
    ExecutionLearningResponse, ExecutionLearningStat, ExecutionOutcomeEvent, FeedbackEvent,
    LearnedProfileSnapshot, ParameterSpec, ScoutComparisonMetadata, StrategyProfile,
    StrategyProfileRegistration,
};
use tokio::sync::Mutex;

mod context;
mod execution;
mod feedback;
mod profiles;
mod scout_coordinator;
mod thresholds;

use context::*;
use execution::*;
use feedback::*;
use profiles::*;
pub use scout_coordinator::ScoutCoordinator;
use thresholds::*;

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
    #[serde(default)]
    evidence_group_id: Option<String>,
    #[serde(default)]
    learning_context: Option<String>,
    source: LearningOutcomeSource,
    #[serde(default)]
    realized_pnl_usd: Option<f64>,
    #[serde(default)]
    paired_comparison: bool,
    #[serde(default)]
    tested_parameter_keys: Vec<String>,
    #[serde(default)]
    baseline_parameters: BTreeMap<String, f64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum LearningOutcomeSource {
    Scout,
    Trade,
}

#[derive(Clone, Serialize, Deserialize)]
struct ThresholdTrainingSample {
    symbol: String,
    strategy: String,
    parameter_key: String,
    observed_at_ms: u64,
    threshold: f64,
    feature_value: f64,
    net_return_percent: f64,
}

#[derive(Serialize)]
struct StrategySummary {
    strategy: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    average_net_return_percent: Option<f64>,
    scout_outcome_samples: usize,
    scout_positive_rate: Option<f64>,
    scout_average_net_return_percent: Option<f64>,
    active_profile_trade_samples: usize,
    active_profile_trade_positive_rate: Option<f64>,
    active_profile_average_net_return_percent: Option<f64>,
    other_profile_trade_samples: usize,
    other_profile_trade_positive_rate: Option<f64>,
    other_profile_average_net_return_percent: Option<f64>,
    paired_scout_samples: usize,
    paired_scout_positive_rate: Option<f64>,
    average_paired_scout_lift_percent: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct LearningState {
    #[serde(default = "default_live_scout_parameter_samples")]
    minimum_live_scout_parameter_samples: usize,
    variants: BTreeMap<String, RegisteredProfile>,
    active_profiles: BTreeMap<String, StrategyProfile>,
    #[serde(default)]
    context_learning: BTreeMap<String, ContextLearningStat>,
    #[serde(default)]
    execution_learning: BTreeMap<String, ExecutionLearningStat>,
    #[serde(default)]
    threshold_samples: Vec<ThresholdTrainingSample>,
    #[serde(default)]
    seen_execution_event_ids: BTreeSet<String>,
    #[serde(default)]
    exit_timing_observations: BTreeMap<String, serde_json::Value>,
    seen_event_ids: BTreeSet<String>,
    accepted_events: u64,
}

fn default_live_scout_parameter_samples() -> usize {
    3
}

impl Default for LearningState {
    fn default() -> Self {
        Self {
            minimum_live_scout_parameter_samples: default_live_scout_parameter_samples(),
            variants: BTreeMap::new(),
            active_profiles: BTreeMap::new(),
            context_learning: BTreeMap::new(),
            execution_learning: BTreeMap::new(),
            threshold_samples: Vec::new(),
            seen_execution_event_ids: BTreeSet::new(),
            exit_timing_observations: BTreeMap::new(),
            seen_event_ids: BTreeSet::new(),
            accepted_events: 0,
        }
    }
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
    let service = service_state(path)?;
    Ok(service_router(service))
}

pub fn router_with_scout_coordinator(
    path: PathBuf,
    worker_endpoints: &str,
) -> Result<(Router, ScoutCoordinator), String> {
    let service = service_state(path)?;
    let app = service_router(service.clone());
    let coordinator = ScoutCoordinator::new(service, worker_endpoints, app.clone())?;
    Ok((app, coordinator))
}

fn service_state(path: PathBuf) -> Result<ServiceState, String> {
    let mut stored = load_state(&path)?;
    rebuild_trade_profile_metrics(&mut stored, now_ms());
    Ok(ServiceState {
        state: Arc::new(Mutex::new(stored)),
        path,
    })
}

fn service_router(service: ServiceState) -> Router {
    Router::new()
        .route("/profiles", get(get_profiles).post(register_profile))
        .route("/decision-snapshot", get(get_decision_snapshot))
        .route("/summary", get(get_summary))
        .route("/feedback", post(accept_feedback))
        .route("/exit-timing", post(accept_exit_timing_observation))
        .route(
            "/config/scout-parameter-samples",
            axum::routing::put(update_scout_parameter_samples),
        )
        .route("/context-thresholds", post(context_thresholds))
        .route("/execution-decision", post(execution_decision))
        .route("/execution-outcomes", post(execution_outcome))
        .with_state(service)
}

/// Operations-only HTTP surface. Domain requests are served through the
/// Learning RSocket coordinator.
pub fn operational_router() -> Router {
    Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/process-memory-bytes", get(process_memory_bytes))
}

async fn process_cpu_seconds() -> String {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
        return "0".to_owned();
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    (user + system).to_string()
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

fn open_state_db(path: &Path) -> Result<rusqlite::Connection, String> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create learning state directory {}: {error}",
                parent.display()
            )
        })?;
    }
    let connection = rusqlite::Connection::open(path).map_err(|error| {
        format!(
            "could not open learning database {}: {error}",
            path.display()
        )
    })?;
    //TODO - do we really need to do then whenever we open the db
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL;
             CREATE TABLE IF NOT EXISTS learning_metadata (id INTEGER PRIMARY KEY CHECK (id = 1), accepted_events INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_config (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_variants (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_active_profiles (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_context_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_execution_stats (id TEXT PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_threshold_samples (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);
             CREATE TABLE IF NOT EXISTS learning_seen_event_ids (id TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS learning_seen_execution_event_ids (id TEXT PRIMARY KEY);
             CREATE TABLE IF NOT EXISTS learning_exit_timing_observations (id TEXT PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .map_err(|error| {
            format!(
                "could not initialize learning database {}: {error}",
                path.display()
            )
        })?;
    Ok(connection)
}

async fn accept_exit_timing_observation(
    State(service): State<ServiceState>,
    Json(observation): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let event_id = observation["event_id"].as_str().unwrap_or_default();
    let valid_number = |key: &str| observation[key].as_f64().is_some_and(f64::is_finite);
    if event_id.is_empty()
        || event_id.len() > 256 //TODO remove hardcoding
        || observation["symbol"].as_str().is_none()
        || observation["strategy"].as_str().is_none()
        || !valid_number("exit_return_percent")
        || !valid_number("post_exit_return_percent")
        || !valid_number("missed_return_percent")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "invalid exit-timing observation".into(),
        ));
    }
    let mut state = service.state.lock().await;
    let duplicate = state.exit_timing_observations.contains_key(event_id);
    if !duplicate {
        state
            .exit_timing_observations
            .insert(event_id.to_owned(), observation);
        while state.exit_timing_observations.len() > 10_000 {
            //TODO remove hardcoding
            if let Some(oldest) = state.exit_timing_observations.keys().next().cloned() {
                state.exit_timing_observations.remove(&oldest);
            }
        }
        persist(&service.path, &state)?;
    }
    Ok(Json(
        serde_json::json!({"accepted": true, "duplicate": duplicate}),
    ))
}

fn load_state(path: &Path) -> Result<LearningState, String> {
    use rusqlite::OptionalExtension;

    let connection = open_state_db(path)?;
    if let Some(accepted_events) = connection
        .query_row(
            "SELECT accepted_events FROM learning_metadata WHERE id = 1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("could not read learning metadata: {error}"))?
    {
        let mut state = LearningState {
            accepted_events: accepted_events.max(0) as u64,
            ..LearningState::default()
        };
        //TODO we need to remove direct DB queries in the code. Should be behind a set of helper functions. Dont like it in the code like this
        state.minimum_live_scout_parameter_samples = connection.query_row(
            "SELECT value FROM learning_config WHERE key = 'minimum_live_scout_parameter_samples'",
            [], |row| row.get::<_, i64>(0),
        ).optional().map_err(|error| format!("could not read learning configuration: {error}"))?
            .and_then(|value| usize::try_from(value).ok())
            .filter(|value| {
                (MINIMUM_SCOUT_PARAMETER_SAMPLES..=MAXIMUM_SCOUT_PARAMETER_SAMPLES)
                    .contains(value)
            })
            .unwrap_or_else(default_live_scout_parameter_samples);
        load_map(&connection, "learning_variants", &mut state.variants)?;
        load_map(
            &connection,
            "learning_active_profiles",
            &mut state.active_profiles,
        )?;
        load_map(
            &connection,
            "learning_context_stats",
            &mut state.context_learning,
        )?;
        load_map(
            &connection,
            "learning_execution_stats",
            &mut state.execution_learning,
        )?;
        state.threshold_samples = load_rows(&connection, "learning_threshold_samples")?;
        state.seen_event_ids = load_ids(&connection, "learning_seen_event_ids")?;
        state.seen_execution_event_ids =
            load_ids(&connection, "learning_seen_execution_event_ids")?;
        load_map(
            &connection,
            "learning_exit_timing_observations",
            &mut state.exit_timing_observations,
        )?;
        return Ok(state);
    }
    Ok(LearningState::default())
}

fn load_map<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    table: &str,
    target: &mut BTreeMap<String, T>,
) -> Result<(), String> {
    let mut statement = connection
        .prepare(&format!("SELECT id, payload FROM {table}"))
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (id, payload) = row.map_err(|error| error.to_string())?;
        target.insert(
            id,
            serde_json::from_str(&payload).map_err(|error| error.to_string())?,
        );
    }
    Ok(())
}

fn load_rows<T: for<'de> Deserialize<'de>>(
    connection: &rusqlite::Connection,
    table: &str,
) -> Result<Vec<T>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT payload FROM {table} ORDER BY id"))
        .map_err(|error| error.to_string())?;
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .map(|row| {
            let payload = row.map_err(|error| error.to_string())?;
            serde_json::from_str(&payload).map_err(|error| error.to_string())
        })
        .collect()
}

fn load_ids(connection: &rusqlite::Connection, table: &str) -> Result<BTreeSet<String>, String> {
    let mut statement = connection
        .prepare(&format!("SELECT id FROM {table}"))
        .map_err(|error| error.to_string())?;
    statement
        .query_map([], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .map(|row| row.map_err(|error| error.to_string()))
        .collect()
}

pub(crate) fn persist_normalized(
    connection: &rusqlite::Connection,
    state: &LearningState,
) -> rusqlite::Result<()> {
    let transaction = connection.unchecked_transaction()?;
    transaction.execute(
        "INSERT INTO learning_metadata (id, accepted_events) VALUES (1, ?1)
         ON CONFLICT(id) DO UPDATE SET accepted_events = excluded.accepted_events
         WHERE accepted_events != excluded.accepted_events",
        [state.accepted_events.min(i64::MAX as u64) as i64],
    )?;
    transaction.execute(
        "INSERT INTO learning_config (key, value) VALUES ('minimum_live_scout_parameter_samples', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [state.minimum_live_scout_parameter_samples as i64],
    )?;
    write_map(&transaction, "learning_variants", &state.variants)?;
    write_map(
        &transaction,
        "learning_active_profiles",
        &state.active_profiles,
    )?;
    write_map(
        &transaction,
        "learning_context_stats",
        &state.context_learning,
    )?;
    write_map(
        &transaction,
        "learning_execution_stats",
        &state.execution_learning,
    )?;
    {
        let rows = state
            .threshold_samples
            .iter()
            .enumerate()
            .map(|(id, value)| (id as i64, value))
            .collect::<Vec<_>>();
        sync_rows(&transaction, "learning_threshold_samples", &rows)?;
    }
    sync_ids(
        &transaction,
        "learning_seen_event_ids",
        &state.seen_event_ids,
    )?;
    write_map(
        &transaction,
        "learning_exit_timing_observations",
        &state.exit_timing_observations,
    )?;
    sync_ids(
        &transaction,
        "learning_seen_execution_event_ids",
        &state.seen_execution_event_ids,
    )?;
    transaction.commit()
}

fn write_map<T: Serialize>(
    connection: &rusqlite::Connection,
    table: &str,
    values: &BTreeMap<String, T>,
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(&format!(
        "INSERT INTO {table} (id, payload) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET payload = excluded.payload
         WHERE payload != excluded.payload"
    ))?;
    for (id, value) in values {
        statement.execute(rusqlite::params![
            id,
            serde_json::to_string(value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
        ])?;
    }
    drop(statement);
    let existing = {
        let mut statement = connection.prepare_cached(&format!("SELECT id FROM {table}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut delete = connection.prepare_cached(&format!("DELETE FROM {table} WHERE id = ?1"))?;
    for id in existing {
        if !values.contains_key(&id) {
            delete.execute([id])?;
        }
    }
    Ok(())
}

fn sync_rows<T: Serialize>(
    connection: &rusqlite::Connection,
    table: &str,
    values: &[(i64, &T)],
) -> rusqlite::Result<()> {
    let mut statement = connection.prepare_cached(&format!(
        "INSERT INTO {table} (id, payload) VALUES (?1, ?2)
         ON CONFLICT(id) DO UPDATE SET payload = excluded.payload
         WHERE payload != excluded.payload"
    ))?;
    for (id, value) in values {
        statement.execute(rusqlite::params![
            id,
            serde_json::to_string(value)
                .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?
        ])?;
    }
    drop(statement);
    let expected_len = values.len() as i64;
    connection.execute(
        &format!("DELETE FROM {table} WHERE id >= ?1"),
        [expected_len],
    )?;
    Ok(())
}

fn sync_ids(
    connection: &rusqlite::Connection,
    table: &str,
    values: &BTreeSet<String>,
) -> rusqlite::Result<()> {
    let mut statement =
        connection.prepare_cached(&format!("INSERT OR IGNORE INTO {table} (id) VALUES (?1)"))?;
    for id in values {
        statement.execute([id])?;
    }
    drop(statement);
    let existing = {
        let mut statement = connection.prepare_cached(&format!("SELECT id FROM {table}"))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut delete = connection.prepare_cached(&format!("DELETE FROM {table} WHERE id = ?1"))?;
    for id in existing {
        if !values.contains(&id) {
            delete.execute([id])?;
        }
    }
    Ok(())
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
        let path =
            std::env::temp_dir().join(format!("learning-test-{}.sqlite", std::process::id()));
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
                learning_context: None,
                symbol: profile.symbol.clone(),
                strategy: profile.strategy.clone(),
                profile_id: profile.profile_id.clone(),
                parameters: profile.parameters.clone(),
                specs: vec![spec.clone()],
                scout: false,
                realized_pnl_usd: 1.0,
                net_return_percent: 1.0,
                threshold_observations: Vec::new(),
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
            learning_context: None,
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            specs: vec![spec],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 1.0,
            threshold_observations: Vec::new(),
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

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].parameters["threshold"], 0.5);
        assert_eq!(profiles[0].parameters["take_profit_percent"], 0.8);
        assert_eq!(profiles[0].learning_samples, 5);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn paired_scout_promotion_requires_symbol_specific_evidence() {
        let path = std::env::temp_dir().join(format!(
            "learning-paired-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
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
        for symbol in ["BTC-USDT", "ETH-USDT"] {
            let baseline = StrategyProfile {
                profile_id: format!("{symbol}-baseline"),
                version: 0,
                symbol: symbol.into(),
                strategy: "live-momentum".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                learning_samples: 0,
                positive_samples: 0,
                realized_pnl: 0.0,
            };
            let response = json_request(
                app.clone(),
                "POST",
                "/profiles",
                Some(
                    serde_json::to_vec(&StrategyProfileRegistration {
                        profile: baseline,
                        specs: vec![spec.clone()],
                    })
                    .unwrap(),
                ),
            )
            .await;
            assert_eq!(response.status(), StatusCode::ACCEPTED);
        }

        let response = json_request(
            app.clone(),
            "PUT",
            "/config/scout-parameter-samples",
            Some(br#"{"minimum_samples":6}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);

        for (symbol_index, symbol) in ["BTC-USDT", "ETH-USDT"].into_iter().enumerate() {
            let parameters = BTreeMap::from([("threshold".into(), 0.4)]);
            let profile_id = format!("{symbol}-candidate");
            for sample in 0..3 {
                for horizon in [60, 300, 900, 3_600] {
                    let event = FeedbackEvent::ScoutOutcome {
                        event_id: format!("fixture-event-{symbol}-{sample}-{horizon}"),
                        observed_at_ms: now_ms() + symbol_index as u64 * 10 + sample,
                        learning_context: None,
                        symbol: symbol.into(),
                        strategy: "live-momentum".into(),
                        profile_id: profile_id.clone(),
                        parameters: parameters.clone(),
                        baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                        comparison: ScoutComparisonMetadata::Paired {
                            group_id: format!("{symbol}-{sample}"),
                            horizon_seconds: Some(horizon),
                        },
                        specs: vec![spec.clone()],
                        net_return_percent: 0.02,
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
                if symbol_index == 0 {
                    let response = json_request(app.clone(), "GET", "/profiles", None).await;
                    let profiles: Vec<StrategyProfile> = serde_json::from_slice(
                        &response.into_body().collect().await.unwrap().to_bytes(),
                    )
                    .unwrap();
                    assert!(
                        profiles
                            .iter()
                            .all(|profile| profile.parameters["threshold"] == 0.5)
                    );
                }
            }
        }

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 2);
        assert!(
            profiles
                .iter()
                .all(|profile| profile.parameters["threshold"] == 0.5)
        );

        // BTC now has enough paired outcomes on its own. ETH's samples must
        // not contribute to BTC's promotion decision or be changed with it.
        for sample in 3..6 {
            let event = FeedbackEvent::ScoutOutcome {
                event_id: format!("fixture-event-BTC-USDT-extra-{sample}"),
                observed_at_ms: now_ms() + 100 + sample,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "BTC-USDT-candidate".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.4)]),
                baseline_parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                comparison: ScoutComparisonMetadata::Paired {
                    group_id: format!("BTC-USDT-extra-{sample}"),
                    horizon_seconds: None,
                },
                specs: vec![spec.clone()],
                net_return_percent: 0.02,
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

        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].parameters["threshold"], 0.4);
        assert_eq!(profiles[1].parameters["threshold"], 0.5);

        let response = json_request(app.clone(), "GET", "/decision-snapshot", None).await;
        let snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(snapshot.scout_parameter_variants.len(), 2);
        let btc = snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "BTC-USDT")
            .unwrap();
        assert_eq!(btc.sample_count, 6);
        assert!(btc.promoted);
        assert_eq!(btc.paired_baseline_decrease_samples, 6);
        assert_eq!(btc.paired_baseline_delta_samples, 6);
        assert_eq!(btc.lower_90_one_sided_improvement_percent, Some(0.02));
        let eth = snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "ETH-USDT")
            .unwrap();
        assert_eq!(eth.sample_count, 3);
        assert!(!eth.promoted);
        assert_eq!(eth.paired_baseline_decrease_samples, 3);
        assert_eq!(eth.paired_baseline_delta_samples, 3);
        assert_eq!(eth.lower_90_one_sided_improvement_percent, None);
        for index in 0..5 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("old-profile-close-{index}"),
                observed_at_ms: now_ms(),
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "BTC-USDT-baseline".into(),
                parameters: BTreeMap::from([("threshold".into(), 0.5)]),
                specs: vec![],
                scout: false,
                realized_pnl_usd: -1.0,
                net_return_percent: -0.1,
                threshold_observations: vec![],
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
        let response = json_request(app.clone(), "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(
            profiles
                .iter()
                .find(|profile| profile.symbol == "BTC-USDT")
                .unwrap()
                .parameters["threshold"],
            0.4
        );
        let response = json_request(
            app.clone(),
            "PUT",
            "/config/scout-parameter-samples",
            Some(br#"{"minimum_samples":3}"#.to_vec()),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let restarted_app = router(path.clone()).unwrap();
        let response = json_request(restarted_app, "GET", "/decision-snapshot", None).await;
        let restarted_snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let btc = restarted_snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "BTC-USDT")
            .unwrap();
        assert_eq!(btc.minimum_samples, 3);
        assert_eq!(btc.paired_baseline_decrease_samples, 6);
        let eth = restarted_snapshot
            .scout_parameter_variants
            .iter()
            .find(|variant| variant.symbol == "ETH-USDT")
            .unwrap();
        assert_eq!(eth.minimum_samples, 3);
        assert_eq!(eth.paired_baseline_decrease_samples, 3);
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trade_threshold_observations_train_and_validate_threshold_in_learning_service() {
        let path = std::env::temp_dir().join(format!(
            "learning-threshold-test-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let spec = ParameterSpec {
            key: "minimum_flow_imbalance".into(),
            label: "Minimum flow".into(),
            default: 0.02,
            min: 0.02,
            max: 0.8,
            step: 0.01,
            unit: "ratio".into(),
            learning_direction: -1.0,
        };
        let parameters = BTreeMap::from([(spec.key.clone(), 0.02)]);
        let profile = StrategyProfile {
            profile_id: "flow-profile".into(),
            version: 0,
            symbol: "BTC-USDT".into(),
            strategy: "live-trade-flow".into(),
            parameters: parameters.clone(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let response = json_request(
            app.clone(),
            "POST",
            "/profiles",
            Some(
                serde_json::to_vec(&StrategyProfileRegistration {
                    profile,
                    specs: vec![spec.clone()],
                })
                .unwrap(),
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);

        for index in 0..70_u64 {
            let (feature_value, net_return_percent) = if index % 2 == 0 {
                (0.025, -0.2)
            } else {
                (0.075, 1.0)
            };
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("threshold-trade-{index}"),
                observed_at_ms: now_ms() + index,
                learning_context: None,
                symbol: "BTC-USDT".into(),
                strategy: "live-trade-flow".into(),
                profile_id: "flow-profile".into(),
                parameters: parameters.clone(),
                specs: Vec::new(),
                scout: false,
                realized_pnl_usd: net_return_percent,
                net_return_percent,
                threshold_observations: vec![StrategyThresholdObservation {
                    parameter_key: spec.key.clone(),
                    threshold: 0.02,
                    feature_value,
                }],
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

        let response = json_request(app, "GET", "/profiles", None).await;
        let profiles: Vec<StrategyProfile> =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(profiles[0].parameters[&spec.key], 0.03);
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
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            profile_id: "p1".into(),
            parameters: profile.parameters.clone(),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec.clone()],
            net_return_percent: 2.0,
        };
        let updated = update_profile(&profile, &[spec.clone()], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.9);
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
                    learning_context: None,
                    symbol: "BTC-USDT".into(),
                    strategy: "live-momentum".into(),
                    profile_id: "p1".into(),
                    parameters: BTreeMap::from([("threshold".into(), 0.9)]),
                    specs: vec![spec],
                    scout: false,
                    realized_pnl_usd: 1.0,
                    net_return_percent: 1.0,
                    threshold_observations: Vec::new(),
                }
            )
            .is_none()
        );
    }

    #[test]
    fn scout_feedback_keeps_the_exact_tested_candidate_value() {
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
            learning_context: None,
            symbol: profile.symbol.clone(),
            strategy: profile.strategy.clone(),
            profile_id: profile.profile_id.clone(),
            parameters: profile.parameters.clone(),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![spec.clone()],
            net_return_percent: -1.0,
        };

        let updated = update_profile(&profile, &[spec], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.9);
        assert_eq!(updated.parameters["take_profit_percent"], 0.8);
    }

    #[test]
    fn trade_profile_statistics_publish_after_minimum_trade_samples_even_when_losing() {
        assert!(!should_publish_trade_profile(5, 2, f64::NAN));
        assert!(!should_publish_trade_profile(5, 6, -1.0));
        assert!(should_publish_trade_profile(5, 0, -1.0));
        assert!(should_publish_trade_profile(5, 3, 0.1));
        assert!(!should_publish_trade_profile(4, 4, 1.0));
    }

    #[test]
    fn profile_keys_include_identity_and_parameter_variant() {
        let first = profile();
        assert_eq!(key("BTC-USDT", "live-momentum"), "BTC-USDT:live-momentum");
        assert!(variant_key(&first).contains("BTC-USDT:live-momentum:p1:"));
        let mut second = first.clone();
        second.parameters.insert("threshold".into(), 0.8);
        assert_ne!(variant_key(&first), variant_key(&second));
    }

    #[test]
    fn event_parts_extract_the_shared_feedback_identity_for_both_event_kinds() {
        let scout = FeedbackEvent::ScoutOutcome {
            event_id: "scout-id".into(),
            observed_at_ms: 1,
            learning_context: None,
            symbol: "BTC-USDT".into(),
            strategy: "momentum".into(),
            profile_id: "p1".into(),
            parameters: BTreeMap::from([("threshold".into(), 0.5)]),
            baseline_parameters: BTreeMap::new(),
            comparison: ScoutComparisonMetadata::Independent,
            specs: vec![],
            net_return_percent: 1.0,
        };
        let parts = event_parts(&scout);
        assert_eq!(
            (&parts.0, parts.1, parts.2, parts.4.as_str()),
            (&"scout-id", "BTC-USDT", "momentum", "p1")
        );
        assert_eq!(parts.3["threshold"], 0.5);
        let trade = FeedbackEvent::TradeOutcome {
            event_id: "trade-id".into(),
            observed_at_ms: 2,
            learning_context: None,
            symbol: "ETH-USDT".into(),
            strategy: "carry".into(),
            profile_id: "p2".into(),
            parameters: BTreeMap::new(),
            specs: vec![],
            scout: false,
            realized_pnl_usd: 1.0,
            net_return_percent: 0.5,
            threshold_observations: Vec::new(),
        };
        let parts = event_parts(&trade);
        assert_eq!(parts.0, "trade-id");
        assert_eq!(parts.1, "ETH-USDT");
        assert_eq!(parts.2, "carry");
        assert_eq!(parts.4, "p2");
    }

    #[tokio::test]
    async fn actual_trade_feedback_reaches_the_profile_context_snapshot() {
        let path = std::env::temp_dir().join(format!(
            "learning-risk-feedback-{}.sqlite",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let app = router(path.clone()).unwrap();
        let timestamp = now_ms().saturating_sub(100);
        for index in 0..3 {
            let event = FeedbackEvent::TradeOutcome {
                event_id: format!("risk-trade-{index}"),
                observed_at_ms: timestamp + index,
                learning_context: Some("long:mixed:BTC-USDT:live-momentum:P0:depth:lt05".into()),
                symbol: "BTC-USDT".into(),
                strategy: "live-momentum".into(),
                profile_id: "profile-risk".into(),
                parameters: BTreeMap::new(),
                specs: vec![],
                scout: false,
                realized_pnl_usd: -1.0,
                net_return_percent: -0.2,
                threshold_observations: vec![],
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
        let response = json_request(app, "GET", "/decision-snapshot", None).await;
        let snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let values = &snapshot.recent_profile_context_returns["profile-risk|long:mixed:BTC-USDT:live-momentum"];
        assert_eq!(values.len(), 3);
        assert!(values.iter().all(|sample| sample.1 == -0.2));
        let _ = std::fs::remove_file(path);
    }
}
