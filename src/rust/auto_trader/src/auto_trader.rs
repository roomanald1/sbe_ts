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
use strategies::StrategyDecision;
#[cfg(test)]
pub(crate) use strategies::StrategyParameterScoutVariant;
pub use strategies::benchmark_strategy_evaluation;
pub(crate) use strategies::daily::{
    DailyStrategySignalBatch, DailyStrategySnapshot, StrategyDiagnostic, parse_scout_combination,
};
#[cfg(test)]
pub(crate) use strategies::scout_parameter_specs;
#[cfg(test)]
use strategies::{AbsorptionPressureValue, DepthSignalValue, WindowedSignalValue};
pub(crate) use strategies::{
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
include!("auto_trader/profile_settings_adapter.rs");
include!("auto_trader/api_types.rs");
include!("auto_trader/dated_carry.rs");
include!("auto_trader/state.rs");
include!("auto_trader/daily_strategy_route.rs");
include!("auto_trader/feedback/legacy_learning_compat.rs");
include!("auto_trader/cycle_types.rs");
include!("auto_trader/api/http_api.rs");
include!("auto_trader/workers/mod.rs");
include!("auto_trader/entry/mod.rs");
include!("auto_trader/exits/mod.rs");
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
        let audit_tx = AuditQueue::new(DECISION_AUDIT_QUEUE_CAPACITY);
        let execution_audit_tx = AuditQueue::new(EXECUTION_AUDIT_QUEUE_CAPACITY);
        let (state_updates, _) = tokio::sync::broadcast::channel(1);
        let path =
            std::env::temp_dir().join(format!("auto-trader-cleanup-{}.json", std::process::id()));
        AutoTraderState {
            latest_daily_strategy_snapshot: Arc::new(Mutex::new(None)),
            latest_live_strategy_signals: Arc::new(Mutex::new(BTreeMap::new())),
            strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
            latest_strategy_blockers: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            settings: Arc::new(Mutex::new(settings)),
            settings_persist_notify: Arc::new(Notify::new()),
            price_path_last_checkpoint_requested_at: Arc::new(std::sync::Mutex::new(None)),
            state_updates,
            settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            latest_missed_opportunity_bands: Arc::new(Mutex::new(Arc::new(Vec::new()))),
            latest_live_insights_context: Arc::new(
                Mutex::new(SharedLiveInsightsContext::default()),
            ),
            latest_execution_parameters: Arc::new(Mutex::new(None)),
            learning_response_cache: Arc::new(Mutex::new(LearningResponseCache::default())),
            cycle_lock: Arc::new(Mutex::new(())),
            pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
            daily_signal_notify: Arc::new(Notify::new()),
            pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
            live_signal_notify: Arc::new(Notify::new()),
            signal_book_prewarms: Arc::new(Mutex::new(BTreeMap::new())),
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

    pub(super) fn order(strategy: &str, timestamp: u64) -> TraderOrder {
        TraderOrder {
            timestamp,
            triggered_at: None,
            symbol: "BTC-USDT".into(),
            side: "sell".into(),
            order_type: "stop-market".into(),
            purpose: "stop-loss".into(),
            strategy: Some(strategy.into()),
            decision_id: None,
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
}

#[cfg(test)]
#[path = "auto_trader/dated_carry_tests.rs"]
mod dated_carry_tests;
