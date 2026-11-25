//! AutoTrader implementation, grouped by responsibility in adjacent source files.
//! These include fragments share this module so private state and helpers keep
//! their existing visibility and behavior.

use axum::{Json, Router, extract::State, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Instant, SystemTime, UNIX_EPOCH},
};
use strategy::StrategyDecision;
#[cfg(test)]
pub(crate) use strategy::StrategyParameterScoutVariant;
pub use strategy::benchmark_strategy_evaluation;
pub(crate) use strategy::daily::{StrategyDiagnostic, parse_scout_combination};
#[cfg(test)]
pub(crate) use strategy::scout_parameter_specs;
#[cfg(test)]
use strategy::{AbsorptionPressureValue, DepthSignalValue, WindowedSignalValue};
pub(crate) use strategy::{
    DatedCarryCandidate, DatedCarrySettlementSource, FeedbackEvent, Strategy, StrategyParameterKey,
    StrategyParameterProfile, StrategyParameterValue, StrategySignal, confident_win_rate_edge,
    effective_parameter_values, legacy_strategy_parameter_name, signal_strategy_parameters,
    strategy_parameter_key, strategy_parameter_value, strategy_profile_value,
    validate_strategy_parameter_overrides,
};
use tokio::{
    sync::{Mutex, Notify, broadcast},
    task::JoinSet,
    time::{Duration, interval},
};

include!("auto_trader/constants.rs");
include!("auto_trader/market_data.rs");

include!("auto_trader/settings.rs");
include!("auto_trader/exit_protection_policy.rs");
include!("auto_trader/profile_settings_adapter.rs");
include!("auto_trader/api_types.rs");
include!("auto_trader/dated_carry.rs");
include!("auto_trader/state.rs");
include!("auto_trader/feedback/legacy_learning_compat.rs");
include!("auto_trader/cycle_types.rs");
include!("auto_trader/api/http_api.rs");
include!("auto_trader/workers/mod.rs");
include!("auto_trader/entry/mod.rs");
include!("auto_trader/exits/mod.rs");
include!("auto_trader/hedging/portfolio_hedge_candidates.rs");
include!("auto_trader/hedging/mod.rs");
include!("auto_trader/fills_and_audit.rs");
include!("auto_trader/execution/mod.rs");
include!("auto_trader/diagnostics/mod.rs");
include!("auto_trader/persistence.rs");
include!("auto_trader/execution_benchmarks.rs");

#[cfg(test)]
mod strategy_cleanup_integration_tests {
    use super::*;
    use anyhow::Context;
    use rsocket_rust::prelude::*;
    use rsocket_rust_transport_websocket::WebsocketServerTransport;
    use std::pin::Pin;

    #[derive(Clone)]
    struct DeleteBarrierResponder(Arc<Mutex<Vec<String>>>);

    #[rsocket_rust::async_trait]
    impl RSocket for DeleteBarrierResponder {
        async fn metadata_push(&self, _: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn fire_and_forget(&self, _: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
            let request: crate::service_rpc::RpcRequest =
                serde_json::from_slice(request.data().context("request data missing")?.as_ref())?;
            anyhow::ensure!(request.method == "DELETE", "expected DELETE request");
            let barrier_id = request.path.rsplit('/').next().unwrap_or_default();
            self.0.lock().await.push(barrier_id.to_owned());
            let response = serde_json::json!({"status": 204, "body": null});
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ))
        }
        fn request_stream(
            &self,
            _: Payload,
        ) -> Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>>
        {
            Box::pin(futures_util::stream::empty())
        }
        fn request_channel(
            &self,
            _: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            Box::pin(futures_util::stream::empty())
        }
    }

    pub(super) fn test_state(settings: Settings) -> AutoTraderState {
        let audit_tx = AuditQueue::new(
            DECISION_AUDIT_QUEUE_CAPACITY,
            DECISION_AUDIT_QUEUE_MAX_BYTES,
        );
        let execution_audit_tx = AuditQueue::new(
            EXECUTION_AUDIT_QUEUE_CAPACITY,
            EXECUTION_AUDIT_QUEUE_MAX_BYTES,
        );
        let (state_updates, _) = tokio::sync::broadcast::channel(1);
        let path =
            std::env::temp_dir().join(format!("auto-trader-cleanup-{}.json", std::process::id()));
        AutoTraderState {
            latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
            strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
            latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            latest_worker_decisions: Arc::new(Mutex::new(BTreeMap::new())),
            latest_worker_diagnostics: Arc::new(Mutex::new(BTreeMap::new())),
            seen_worker_decision_events: Arc::new(Mutex::new(BTreeMap::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            settings: Arc::new(Mutex::new(settings)),
            activity_snapshot: Arc::new(Mutex::new(None)),
            settings_persist_notify: Arc::new(Notify::new()),
            settings_persist_write_lock: Arc::new(Mutex::new(())),
            durable_feedback_event_ids: Arc::new(Mutex::new(BTreeSet::new())),
            price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
            state_updates,
            live_profile_updates: tokio::sync::broadcast::channel(1).0,
            settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
            latest_execution_parameters: Arc::new(Mutex::new(None)),
            cycle_lock: Arc::new(Mutex::new(())),
            pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
            daily_signal_notify: Arc::new(Notify::new()),
            pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
            live_signal_notify: Arc::new(Notify::new()),
            audit_tx,
            execution_audit_tx,
            audit_health: Arc::new(AtomicU64::new(3)),
            audit_failure_latched: Arc::new(AtomicBool::new(false)),
            gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
            stale_signal_candidates_total: Arc::new(AtomicU64::new(0)),
            stale_signal_cycles_total: Arc::new(AtomicU64::new(0)),
            insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
            cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
            cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            cycle_lock_scope_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            settings_path: path.clone(),
            dated_carry: Arc::new(Mutex::new(DatedCarryBook::default())),
            dated_carry_work_lock: Arc::new(Mutex::new(())),
            dated_carry_path: path.with_extension("dated_carry.json"),
        }
    }

    #[tokio::test]
    async fn scout_feedback_flows_through_learning_snapshot_into_live_parameter_api() {
        use axum::{body::Body, http::Request};
        use http_body_util::BodyExt;
        use tower::ServiceExt;

        let learning_path = std::env::temp_dir().join(format!(
            "walk-forward-integration-{}-{}.sqlite",
            std::process::id(),
            current_time_ms()
        ));
        let learning_app = learning::router(learning_path.clone()).unwrap();
        let strategy = Strategy::LiveMomentum;
        let symbol = "BTC-USDT";
        let specs = strategy
            .parameter_specs()
            .into_iter()
            .map(|spec| strategy::ParameterSpec {
                key: spec.key.as_str().to_owned(),
                label: spec.label.to_owned(),
                default: spec.default,
                min: spec.min,
                max: spec.max,
                step: spec.step,
                unit: spec.unit.to_owned(),
                learning_direction: spec.learning_direction,
            })
            .collect::<Vec<_>>();
        let parameters = specs
            .iter()
            .map(|spec| (spec.key.clone(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile_id = strategy::profile_id(symbol, strategy.as_str(), &parameters);
        let event = FeedbackEvent::ScoutOutcome {
            event_id: "walk-forward-integration-scout-1".into(),
            observed_at_ms: current_time_ms(),
            learning_context: Some("scout:BTC:trade".into()),
            market_regime: Some("trend".into()),
            symbol: symbol.into(),
            strategy: strategy.as_str().into(),
            profile_id: profile_id.clone(),
            parameters,
            baseline_parameters: BTreeMap::new(),
            comparison: strategy::ScoutComparisonMetadata::Independent,
            specs,
            net_return_percent: 0.25,
        };
        let feedback_response = learning_app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/feedback")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&event).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(feedback_response.status(), axum::http::StatusCode::OK);

        let snapshot_response = learning_app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/decision-snapshot")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(snapshot_response.status(), axum::http::StatusCode::OK);
        let snapshot = serde_json::from_slice::<strategy::LearnedProfileSnapshot>(
            &snapshot_response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes(),
        )
        .unwrap();
        assert_eq!(
            snapshot.walk_forward_evidence[&format!("{symbol}|{}", strategy.as_str())].sample_count,
            1
        );
        let exact_regime_key = format!("{profile_id}|regime:trend");
        assert_eq!(
            snapshot.walk_forward_evidence[&exact_regime_key].sample_count,
            1
        );

        let state = test_state(Settings::default());
        AutoTraderSocketApi {
            state: state.clone(),
        }
        .apply_learning_snapshot(snapshot)
        .await;
        let Json(daily_scan) = get_daily_strategy_parameters(State(state.clone())).await;
        assert!(
            daily_scan
                .strategy_profiles
                .iter()
                .all(|profile| profile.strategy != strategy.as_str())
        );
        assert!(
            state
                .settings
                .lock()
                .await
                .learned_walk_forward_evidence
                .contains_key(&exact_regime_key)
        );
        let Json(response) = get_decision_parameters(State(state.clone())).await;
        let profile = response
            .strategy_profiles
            .iter()
            .find(|profile| profile.symbol == symbol && profile.strategy == strategy.as_str())
            .expect("live parameter API includes the sampled live strategy");
        let api_profile = serde_json::to_value(profile).unwrap();
        assert_eq!(
            api_profile["walk_forward_evidence"]["sampleCount"],
            serde_json::json!(1)
        );
        let evidence = profile
            .walk_forward_evidence
            .as_ref()
            .expect("AutoTrader exposes Learning walk-forward evidence");
        assert_eq!(evidence.sample_count, 1);
        assert_eq!(evidence.positive_net_rate, Some(1.0));
        assert_eq!(evidence.average_net_return_percent, Some(0.25));
        assert_eq!(
            profile.walk_forward_evidence_by_regime["trend"].sample_count,
            1
        );

        let _ = std::fs::remove_file(&learning_path);
        let _ = std::fs::remove_file(learning_path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(learning_path.with_extension("sqlite-shm"));
    }

    pub(super) fn order(strategy: &str, timestamp: u64) -> TraderOrder {
        TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: "BTC-USDT".into(),
            side: "sell".into(),
            order_type: "stop-market".into(),
            purpose: "stop-loss".into(),
            exploratory: false,
            strategy: Some(strategy.into()),
            decision_id: None,
            revalidation_snapshot: None,
            strength: 0.0,
            ioc_fallback_for: None,
            quantity: 1.0,
            filled_quantity: 0.0,
            fill_rate: 0.0,
            average_price: None,
            fees_paid: 0.0,
            reserved_notional: 0.0,
            execution_order_id: None,
            processed_fill_trade_ids: Vec::new(),
            learning_context: None,
            book_bid_depth_10bps: None,
            book_ask_depth_10bps: None,
            book_depth_imbalance_10bps: None,
            estimated_sweep_bps: None,
            estimated_ioc_all_in_bps: None,
            decision_bid: None,
            decision_ask: None,
            decision_mid: None,
            submitted_limit_price: None,
            target_price: None,
            horizon_take_profit_percent: None,
            limit_to_mid_bps: None,
            limit_to_crossing_quote_bps: None,
            expires_at: None,
            failure_reason: None,
            stage_timings_ms: BTreeMap::new(),
            status: "armed".into(),
            realized_pnl: 0.0,
            strategy_realized_pnl: None,
            strategy_realized_pnl_today: None,
            strategy_realized_pnl_day: None,
        }
    }

    #[tokio::test]
    async fn every_registered_strategy_cancels_its_protective_orders_on_exit() {
        let deleted = Arc::new(Mutex::new(Vec::new()));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let endpoint = format!("ws://{address}");
        let responder = DeleteBarrierResponder(Arc::clone(&deleted));
        let server = tokio::spawn(async move {
            RSocketFactory::receive()
                .transport(WebsocketServerTransport::from(address.to_string()))
                .acceptor(Box::new(move |_, _| Ok(Box::new(responder.clone()))))
                .serve()
                .await
                .unwrap();
        });
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let state = test_state(Settings::default());

        for (index, strategy) in Strategy::all().enumerate() {
            let strategy_name = strategy.as_str();
            let barrier_id = format!("barrier-{index}");
            let timestamp = 10_000 + index as u64;
            {
                let mut settings = state.settings.lock().await;
                let stop = ProtectiveStop {
                    barrier_id: barrier_id.clone(),
                    strategy: strategy_name.into(),
                    order_timestamp: timestamp,
                    quantity: 1.0,
                    processed_quantity: 0.0,
                    processed_notional: 0.0,
                    processed_fee: 0.0,
                };
                let mut position = TraderPosition {
                    quantity: 1.0,
                    ..TraderPosition::default()
                };
                position.protective_stops.push(stop.clone());
                settings
                    .positions
                    .insert("BTC-USDT".into(), position.clone());
                settings.strategy_positions.insert(
                    "BTC-USDT".into(),
                    BTreeMap::from([(strategy_name.into(), position)]),
                );
                settings.orders.push(order(strategy_name, timestamp));
            }

            assert!(
                cancel_protective_stops_at(
                    Some(&endpoint),
                    &state,
                    "BTC-USDT",
                    Some(strategy_name)
                )
                .await
                .unwrap(),
                "{strategy_name}"
            );

            let settings = state.settings.lock().await;
            assert!(
                settings.positions["BTC-USDT"].protective_stops.is_empty(),
                "aggregate stop retained for {strategy_name}"
            );
            assert!(
                settings.strategy_positions["BTC-USDT"][strategy_name]
                    .protective_stops
                    .is_empty(),
                "strategy stop retained for {strategy_name}"
            );
            assert_eq!(
                settings.orders.last().unwrap().status,
                "cancelled",
                "local order remained active for {strategy_name}"
            );
            drop(settings);
        }

        server.abort();
        let deleted = deleted.lock().await;
        assert_eq!(
            deleted.len(),
            Strategy::all().count(),
            "not every strategy barrier reached Position Manager"
        );
        assert_eq!(
            deleted.iter().collect::<BTreeSet<_>>().len(),
            deleted.len(),
            "a strategy barrier was canceled more than once"
        );
    }

    #[tokio::test]
    async fn typed_live_decision_socket_uses_service_owned_state_and_push_profiles() {
        let strategy = Strategy::LiveMomentum;
        let parameters = strategy
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let profile = strategy::LiveStrategyProfile {
            profile_id: strategy::profile_id("BTC-USDT", strategy.as_str(), &parameters),
            profile_symbol: "BTC-USDT".into(),
            version: 7,
            parameters: parameters.clone(),
            walk_forward_evidence: None,
            market_regime: None,
            market_state_key: None,
            walk_forward_evidence_by_regime: Default::default(),
        };
        let first_spec = &strategy.parameter_specs()[0];
        let mut second_parameters = parameters.clone();
        let second_value = if (first_spec.default - first_spec.min).abs() > f64::EPSILON {
            first_spec.min
        } else {
            first_spec.max
        };
        second_parameters.insert(first_spec.key.as_str().to_owned(), second_value);
        let second_profile = strategy::LiveStrategyProfile {
            profile_id: strategy::profile_id("BTC-USDT", strategy.as_str(), &second_parameters),
            profile_symbol: "BTC-USDT".into(),
            version: 8,
            parameters: second_parameters.clone(),
            walk_forward_evidence: None,
            market_regime: None,
            market_state_key: None,
            walk_forward_evidence_by_regime: Default::default(),
        };
        let mut settings = Settings::default();
        settings.strategies = vec![strategy];
        settings.active_live_strategy_profiles.insert(
            strategy.as_str().to_owned(),
            vec![profile.clone(), second_profile.clone()],
        );
        let state = test_state(settings);
        let socket_api = AutoTraderSocketApi {
            state: state.clone(),
        };

        let profile_sets = socket_api.live_profile_sets().await;
        let profile_set = profile_sets
            .iter()
            .find(|set| set.strategy == strategy)
            .expect("AutoTrader publishes a set for each live worker");
        assert!(profile_set.enabled);
        assert_eq!(profile_set.profiles.len(), 2);
        assert!(
            profile_set
                .profiles
                .iter()
                .any(|configured| configured.profile_id == profile.profile_id)
        );
        assert!(
            profile_set
                .profiles
                .iter()
                .any(|configured| configured.profile_id == second_profile.profile_id)
        );

        let timestamp = current_time_ms();
        let mut signal = strategy::StrategyMarketSignal::default();
        signal.symbol = "BTC-USDT".into();
        signal.strategy = Some(strategy.as_str().into());
        signal.timestamp = timestamp;
        signal.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": parameters,
            "learning_profile_id": profile.profile_id,
            "learning_profile_symbol": profile.profile_symbol,
            "learning_profile_version": profile.version,
        }));
        let status = socket_api
            .report_decision(strategy::LiveStrategyDecisionReport {
                contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: "socket-decision-test".into(),
                event_identity_version: 0,
                episode_id: String::new(),
                strategy,
                symbol: "BTC-USDT".into(),
                profile_id: profile.profile_id.clone(),
                profile_version: profile.version,
                parameter_hash: profile.profile_id.clone(),
                event_time_ms: timestamp,
                decision: strategy::LiveStrategyDecision::Reject,
                reason: Some("test rejection".into()),
                walk_forward_evidence: None,
                signal,
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            })
            .await
            .unwrap();
        assert_eq!(status, axum::http::StatusCode::ACCEPTED.as_u16());
        let mut second_signal = strategy::StrategyMarketSignal::default();
        second_signal.symbol = "BTC-USDT".into();
        second_signal.strategy = Some(strategy.as_str().into());
        second_signal.timestamp = timestamp;
        second_signal.strategy_decision = Some(serde_json::json!({
            "strategy_parameters": second_parameters,
            "learning_profile_id": second_profile.profile_id,
            "learning_profile_symbol": second_profile.profile_symbol,
            "learning_profile_version": second_profile.version,
        }));
        let second_status = socket_api
            .report_decision(strategy::LiveStrategyDecisionReport {
                contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: "socket-decision-test".into(),
                event_identity_version: 0,
                episode_id: String::new(),
                strategy,
                symbol: "BTC-USDT".into(),
                profile_id: second_profile.profile_id.clone(),
                profile_version: second_profile.version,
                parameter_hash: second_profile.profile_id.clone(),
                event_time_ms: timestamp,
                decision: strategy::LiveStrategyDecision::Reject,
                reason: Some("second profile rejection".into()),
                walk_forward_evidence: None,
                signal: second_signal,
                dated_carry_scan: None,
                dated_carry_candidate: None,
                dated_carry_settlement: None,
            })
            .await
            .unwrap();
        assert_eq!(second_status, axum::http::StatusCode::ACCEPTED.as_u16());
        let decisions = state.latest_worker_decisions.lock().await;
        assert_eq!(decisions.len(), 2);
        assert!(decisions.contains_key(&(
            "BTC-USDT".into(),
            strategy.as_str().into(),
            profile.profile_id.clone(),
        )));
        assert!(decisions.contains_key(&(
            "BTC-USDT".into(),
            strategy.as_str().into(),
            second_profile.profile_id.clone(),
        )));
        drop(decisions);
        let diagnostic_status = socket_api
            .report_worker_diagnostic(strategy::LiveStrategyWorkerDiagnostic {
                contract_version: strategy::LIVE_EVALUATION_CONTRACT_VERSION,
                event_id: "socket-worker-skip-test".into(),
                strategy,
                symbol: "BTC-USDT".into(),
                profile_id: Some(second_profile.profile_id.clone()),
                event_time_ms: timestamp + 1,
                reason: "Feature preparation skipped this profile".into(),
            })
            .await
            .unwrap();
        assert_eq!(diagnostic_status, axum::http::StatusCode::ACCEPTED.as_u16());
        assert!(state.latest_worker_diagnostics.lock().await.contains_key(&(
            "BTC-USDT".into(),
            strategy.as_str().into(),
            second_profile.profile_id,
        )));
    }
}

#[cfg(test)]
#[path = "auto_trader/dated_carry_tests.rs"]
mod dated_carry_tests;

#[cfg(test)]
#[path = "auto_trader/lifecycle_recovery_tests.rs"]
mod lifecycle_recovery_tests;
