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
use strategies::{
    ContextLearningStat, ContextMissedOpportunityBand, ContextThresholdInput,
    ContextThresholdRequest, ContextThresholdResponse, ContextThresholds, ExecutionLearningRequest,
    ExecutionLearningResponse, ExecutionLearningStat, ExecutionOutcomeEvent, FeedbackEvent,
    LearnedProfileSnapshot, ParameterSpec, StrategyProfile, StrategyProfileRegistration,
    StrategyThresholdObservation,
};
use tokio::sync::Mutex;

mod context;
mod execution;
mod feedback;
mod profiles;
mod thresholds;

use context::*;
use execution::*;
use feedback::*;
use profiles::*;
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
    tested_parameter_keys: Vec<String>,
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
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct LearningState {
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
        .route("/decision-snapshot", get(get_decision_snapshot))
        .route("/summary", get(get_summary))
        .route("/feedback", post(accept_feedback))
        .route("/context-thresholds", post(context_thresholds))
        .route("/execution-decision", post(execution_decision))
        .route("/execution-outcomes", post(execution_outcome))
        .with_state(service))
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
    async fn paired_scout_promotion_pools_symbols_and_keeps_the_tested_value() {
        let path =
            std::env::temp_dir().join(format!("learning-paired-test-{}.json", std::process::id()));
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

        for (symbol_index, symbol) in ["BTC-USDT", "ETH-USDT"].into_iter().enumerate() {
            let parameters = BTreeMap::from([("threshold".into(), 0.6)]);
            let profile_id = format!("{symbol}-candidate");
            for sample in 0..3 {
                let event = FeedbackEvent::ScoutOutcome {
                    event_id: format!("paired-{symbol}-{sample}"),
                    observed_at_ms: now_ms() + symbol_index as u64 * 10 + sample,
                    learning_context: None,
                    symbol: symbol.into(),
                    strategy: "live-momentum".into(),
                    profile_id: profile_id.clone(),
                    parameters: parameters.clone(),
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
                .all(|profile| profile.parameters["threshold"] == 0.6)
        );

        let response = json_request(app, "GET", "/decision-snapshot", None).await;
        let snapshot: LearnedProfileSnapshot =
            serde_json::from_slice(&response.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(snapshot.scout_parameter_variants.len(), 2);
        assert!(snapshot.scout_parameter_variants.iter().all(|variant| {
            variant.sample_count == 6
                && variant.minimum_samples == 6
                && variant.promoted
                && variant.lower_90_one_sided_improvement_percent == Some(0.02)
        }));
        let _ = std::fs::remove_file(path);
    }

    #[tokio::test]
    async fn trade_threshold_observations_train_and_validate_threshold_in_learning_service() {
        let path = std::env::temp_dir().join(format!(
            "learning-threshold-test-{}.json",
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
            specs: vec![spec.clone()],
            net_return_percent: -1.0,
        };

        let updated = update_profile(&profile, &[spec], &event).unwrap();
        assert_eq!(updated.parameters["threshold"], 0.9);
        assert_eq!(updated.parameters["take_profit_percent"], 0.8);
    }

    #[test]
    fn trade_profile_statistics_publish_after_repeated_positive_outcomes() {
        assert!(!should_publish_trade_profile(5, 2, 1.0));
        assert!(!should_publish_trade_profile(5, 3, -0.1));
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
}
