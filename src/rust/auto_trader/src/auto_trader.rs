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
use tokio::{
    sync::{broadcast, Mutex, Notify, mpsc},
    task::JoinSet,
    time::{Duration, interval},
};

include!("auto_trader/constants.rs");
include!("auto_trader/market_data.rs");
include!("auto_trader/daily_strategy_models.rs");
include!("auto_trader/strategies/mod.rs");

include!("auto_trader/settings.rs");
include!("auto_trader/strategy_parameter_types.rs");
include!("auto_trader/strategy_parameter_catalog.rs");
include!("auto_trader/strategy_parameters.rs");
include!("auto_trader/api_types.rs");
include!("auto_trader/dated_carry.rs");
include!("auto_trader/state.rs");
include!("auto_trader/daily_strategy_receiver.rs");
include!("auto_trader/learning/learning_state.rs");
include!("auto_trader/cycle_types.rs");
include!("auto_trader/api/http_api.rs");
include!("auto_trader/workers/mod.rs");
include!("auto_trader/entry/mod.rs");
include!("auto_trader/exits/mod.rs");
include!("auto_trader/hedging/mod.rs");
include!("auto_trader/fills_and_audit.rs");
include!("auto_trader/learning/mod.rs");
include!("auto_trader/execution/mod.rs");
include!("auto_trader/diagnostics/mod.rs");
include!("auto_trader/persistence.rs");

#[cfg(test)]
mod strategy_cleanup_integration_tests {
    use super::*;
    use axum::{extract::{Path, State as AxumState}, http::StatusCode, routing::delete};

    async fn delete_barrier(
        AxumState(deleted): AxumState<Arc<Mutex<Vec<String>>>>,
        Path(id): Path<String>,
    ) -> StatusCode {
        deleted.lock().await.push(id);
        StatusCode::NO_CONTENT
    }

    fn test_state(settings: Settings) -> AutoTraderState {
        let (audit_tx, _) = mpsc::channel(1);
        let (execution_audit_tx, _) = mpsc::unbounded_channel();
        let (state_updates, _) = tokio::sync::broadcast::channel(1);
        let path = std::env::temp_dir().join(format!("auto-trader-cleanup-{}.json", std::process::id()));
        AutoTraderState {
            client: reqwest::Client::new(),
            latest_daily_strategy_snapshot: Arc::new(Mutex::new(None)),
            strategy_diagnostic: Arc::new(std::sync::Mutex::new(Vec::new())),
            enabled: Arc::new(AtomicBool::new(true)),
            settings: Arc::new(Mutex::new(settings)),
            settings_persist_notify: Arc::new(Notify::new()),
            state_updates,
            settings_persist_timings_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            latest_missed_opportunity_bands: Arc::new(Mutex::new(Vec::new())),
            latest_execution_parameters: Arc::new(Mutex::new(None)),
            cycle_lock: Arc::new(Mutex::new(())),
            pending_daily_signals: Arc::new(Mutex::new(BTreeMap::new())),
            daily_signal_notify: Arc::new(Notify::new()),
            pending_live_signals: Arc::new(Mutex::new(BTreeMap::new())),
            live_signal_notify: Arc::new(Notify::new()),
            signal_book_prewarms: Arc::new(Mutex::new(BTreeMap::new())),
            audit_tx,
            execution_audit_tx,
            gap_summary: Arc::new(std::sync::Mutex::new(GapSummary::default())),
            insight_processing_ms: Arc::new(Mutex::new(Vec::new())),
            cycle_lock_wait_ms: Arc::new(Mutex::new(BTreeMap::new())),
            cycle_lock_hold_ms: Arc::new(std::sync::Mutex::new(BTreeMap::new())),
            settings_path: path.clone(),
            dated_carry: Arc::new(Mutex::new(DatedCarryBook::default())),
            dated_carry_work_lock: Arc::new(Mutex::new(())),
            dated_carry_path: path.with_extension("dated_carry.json"),
        }
    }

    fn order(strategy: &str, timestamp: u64) -> TraderOrder {
        TraderOrder {
            timestamp, triggered_at: None, symbol: "BTC-USDT".into(), side: "sell".into(),
            order_type: "stop-market".into(), purpose: "stop-loss".into(), strategy: Some(strategy.into()),
            decision_id: None, quantity: 1.0, filled_quantity: 0.0, fill_rate: 0.0, average_price: None,
            fees_paid: 0.0, reserved_notional: 0.0, execution_order_id: None,
            processed_fill_trade_ids: Vec::new(), learning_context: None, book_bid_depth_10bps: None,
            book_ask_depth_10bps: None, book_depth_imbalance_10bps: None, estimated_sweep_bps: None,
            estimated_ioc_all_in_bps: None, decision_bid: None, decision_ask: None, decision_mid: None,
            submitted_limit_price: None, target_price: None, horizon_take_profit_percent: None,
            limit_to_mid_bps: None, limit_to_crossing_quote_bps: None, expires_at: None,
            failure_reason: None, stage_timings_ms: BTreeMap::new(), status: "armed".into(),
            realized_pnl: 0.0, strategy_realized_pnl: None, strategy_realized_pnl_today: None,
            strategy_realized_pnl_day: None,
        }
    }

    #[tokio::test]
    async fn every_registered_strategy_cancels_its_protective_orders_on_exit() {
        let deleted = Arc::new(Mutex::new(Vec::new()));
        let app = Router::new().route("/positions/orders/{id}", delete(delete_barrier)).with_state(Arc::clone(&deleted));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let state = test_state(Settings::default());

        for (index, strategy) in Strategy::all().enumerate() {
            let strategy_name = strategy.as_str();
            let barrier_id = format!("barrier-{index}");
            let timestamp = 10_000 + index as u64;
            {
                let mut settings = state.settings.lock().await;
                let stop = ProtectiveStop {
                    barrier_id: barrier_id.clone(), strategy: strategy_name.into(), order_timestamp: timestamp,
                    quantity: 1.0, processed_quantity: 0.0, processed_notional: 0.0, processed_fee: 0.0,
                };
                let mut position = TraderPosition { quantity: 1.0, ..TraderPosition::default() };
                position.protective_stops.push(stop.clone());
                settings.positions.insert("BTC-USDT".into(), position.clone());
                settings.strategy_positions.insert("BTC-USDT".into(), BTreeMap::from([(strategy_name.into(), position)]));
                settings.orders.push(order(strategy_name, timestamp));
            }

            assert!(cancel_protective_stops(&state.client, &format!("http://{address}"), &state, "BTC-USDT", Some(strategy_name)).await.unwrap(), "{strategy_name}");

            let settings = state.settings.lock().await;
            assert!(settings.positions["BTC-USDT"].protective_stops.is_empty(), "aggregate stop retained for {strategy_name}");
            assert!(settings.strategy_positions["BTC-USDT"][strategy_name].protective_stops.is_empty(), "strategy stop retained for {strategy_name}");
            assert_eq!(settings.orders.last().unwrap().status, "cancelled", "local order remained active for {strategy_name}");
            drop(settings);
        }

        server.abort();
        let deleted = deleted.lock().await;
        assert_eq!(deleted.len(), Strategy::all().count(), "not every strategy barrier reached Position Manager");
        assert_eq!(deleted.iter().collect::<BTreeSet<_>>().len(), deleted.len(), "a strategy barrier was canceled more than once");
    }
}
