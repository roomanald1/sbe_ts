use arc_swap::ArcSwapOption;
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, Query, State},
    http::{Request, StatusCode, header::HeaderName},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, OnceLock, RwLock as StdRwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, broadcast, mpsc},
    time::{Duration, Instant, timeout},
};

const SYMBOL_PRICE_MESSAGE_LENGTH: usize = 64;
const BOOK_DEPTH: usize = 7;
const NUM_VWAP_BINS: usize = 78;
const EPSILON: f64 = 1e-12;
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static USD_RATE_CACHE: OnceLock<Mutex<Option<(Instant, UsdRatesResponse)>>> = OnceLock::new();
static USD_RATE_REFRESHING: AtomicBool = AtomicBool::new(false);
static USD_RATE_STREAM_CACHE: OnceLock<StdRwLock<Option<UsdRatesResponse>>> = OnceLock::new();
static USD_RATE_ORDER_SNAPSHOT: OnceLock<ArcSwapOption<UsdRatesResponse>> = OnceLock::new();
const USD_RATE_CACHE_TTL: Duration = Duration::from_secs(15);
const USD_RATE_STREAM_MAX_AGE: Duration = Duration::from_secs(120);

include!("positions/positions_routes.rs");

include!("market/market_book_workers.rs");

include!("market/usd_rates.rs");

include!("orders/order_types.rs");

include!("market/book_matching.rs");

include!("ledger/position_store_load.rs");

include!("ledger/position_store_subscriptions.rs");

include!("scheduling/scheduled_run_lifecycle.rs");

include!("scheduling/twap_execution.rs");

include!("scheduling/vwap_execution.rs");

include!("ledger/ledger_trade_index.rs");

include!("account/account_snapshot.rs");

include!("account/account_capacity.rs");

include!("account/account_routes.rs");

include!("positions/position_reporting.rs");

include!("positions/hedge_candidates.rs");

include!("market/market_insights_inputs.rs");

include!("positions/position_trade_history.rs");

include!("orders/order_queries_and_barriers.rs");

include!("orders/order_submission.rs");

include!("orders/order_cancellation.rs");

include!("ledger/ledger_helpers.rs");

#[cfg(test)]
mod coverage_tests {
    use super::*;

    fn assert_status<T>(result: Result<T, (StatusCode, String)>, expected: StatusCode) {
        match result {
            Ok(_) => panic!("expected {expected}, got success"),
            Err((status, _)) => assert_eq!(status, expected),
        }
    }

    fn store() -> Arc<PositionStore> {
        let (state_updates, _) = broadcast::channel(8);
        Arc::new(PositionStore {
            path: PathBuf::from("/tmp/position-management-coverage-test.sqlite"),
            ledger: Arc::new(Mutex::new(Ledger::default())),
            persistence_notify: Arc::new(Notify::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            books: StdRwLock::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols: Arc::new(vec!["BTC-USDT".to_owned()]),
            vwap_runs: Mutex::new(HashMap::new()),
        })
    }

    #[tokio::test]
    async fn reporting_and_order_read_handlers_cover_empty_ledger_paths() {
        let store = store();
        let fees = get_fee_summary(State(Arc::clone(&store))).await.0;
        assert_eq!(fees.fills, 0);
        assert!(fees.rows.is_empty());
        let positions = get_positions(
            State(Arc::clone(&store)),
            Query(PositionSourceQuery { source: None }),
        )
        .await
        .unwrap();
        assert!(positions.positions.is_empty());
        assert_eq!(positions.trade_count, 0);
        assert!(
            cached_position_summary(&store, &Ledger::default(), None)
                .positions
                .is_empty()
        );
        assert!(
            get_orders(State(Arc::clone(&store)))
                .await
                .0
                .orders
                .is_empty()
        );
        assert!(
            get_barrier_orders(State(Arc::clone(&store)))
                .await
                .0
                .is_empty()
        );
        let trades = get_position_trades(State(store), Query(PositionTradesQuery::default()))
            .await
            .0;
        assert!(trades.trades.is_empty());
        assert_eq!(trades.total, None);
    }

    #[tokio::test]
    async fn reporting_handlers_cover_trade_grouping_source_filters_and_pagination() {
        let store = store();
        let trades = vec![
            serde_json::from_value::<PositionTrade>(serde_json::json!({
                "id": "a", "symbol": "BTC-USDT", "side": "buy", "quantity": 2.0,
                "execution_price": 100.0, "timestamp": 10, "source": "auto", "fee": -0.2
            }))
            .unwrap(),
            serde_json::from_value::<PositionTrade>(serde_json::json!({
                "id": "b", "symbol": "BTC-USDT", "side": "sell", "quantity": 1.0,
                "execution_price": 110.0, "timestamp": 20, "source": "manual", "fee": -0.11
            }))
            .unwrap(),
        ];
        store.ledger.lock().await.trades = trades;
        let fees = get_fee_summary(State(Arc::clone(&store))).await.0;
        assert_eq!(fees.fills, 2);
        assert_eq!(fees.rows.len(), 2);
        assert_eq!(fees.first_fill_ms, Some(10));
        assert_eq!(fees.last_fill_ms, Some(20));

        let positions = get_positions(
            State(Arc::clone(&store)),
            Query(PositionSourceQuery {
                source: Some("auto".into()),
            }),
        )
        .await
        .unwrap();
        assert_eq!(positions.trade_count, 1);
        assert_eq!(positions.positions[0].quantity, 2.0);
        let page = get_position_trades(
            State(store),
            Query(PositionTradesQuery {
                symbol: Some("btc-usdt".into()),
                source: Some("manual".into()),
                since_ms: Some(10),
                before_ms: Some(30),
                limit: Some(1),
                offset: Some(0),
            }),
        )
        .await
        .0;
        assert_eq!(page.trades.len(), 1);
        assert_eq!(page.total, Some(1));
        assert_eq!(page.trades[0].fee, -0.11);
        assert_eq!(page.next_offset, None);
        assert_eq!(page.has_older, Some(false));
    }

    #[tokio::test]
    async fn handlers_reject_invalid_requests_before_external_dependencies() {
        let store = store();
        assert_status(
            get_realized_pnl(
                State(Arc::clone(&store)),
                Query(RealizedPnlQuery {
                    source: Some("invalid".into()),
                    since_ms: None,
                }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            create_profit_barrier(
                State(Arc::clone(&store)),
                Json(ProfitBarrierRequest {
                    symbol: "invalid".into(),
                    target_profit_usd: 1.0,
                }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            close_profitable(
                State(Arc::clone(&store)),
                Json(CloseProfitableRequest {
                    symbol: "invalid".into(),
                    strategy_instance_id: None,
                }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            set_account_leverage(
                State(Arc::clone(&store)),
                Json(LeverageRequest {
                    max_leverage: f64::NAN,
                }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            top_up_account(
                State(Arc::clone(&store)),
                Json(TopupRequest { amount: 0.0 }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            cancel_order(State(store), Path("missing".into())).await,
            StatusCode::NOT_FOUND,
        );
    }

    #[tokio::test]
    async fn insights_handlers_cover_empty_and_invalid_market_paths() {
        let store = store();
        assert!(
            get_insights_inputs(
                State(Arc::clone(&store)),
                Query(InsightsInputQuery::default())
            )
            .await
            .unwrap()
            .0
            .markets
            .is_empty()
        );
        assert_status(
            get_market_book(
                State(store),
                Query(InsightsInputQuery {
                    symbol: Some("INVALID".into()),
                }),
            )
            .await,
            StatusCode::BAD_REQUEST,
        );
    }

    #[tokio::test]
    async fn scheduling_subscription_and_ui_snapshot_cover_in_memory_paths() {
        let store = store();
        assert!(
            get_vwap_runs(State(Arc::clone(&store)))
                .await
                .0
                .runs
                .is_empty()
        );
        let request = || NewVwapRun {
            symbol: "bad symbol".into(),
            side: TradeSide::Buy,
            quantity: 1.0,
            duration_minutes: 30,
            max_participation_rate: 0.5,
            strategy: ExecutionStrategy::Vwap,
            source: "manual".into(),
            client_order_id: None,
        };
        assert_status(
            start_vwap(State(Arc::clone(&store)), Json(request())).await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            start_twap(State(Arc::clone(&store)), Json(request())).await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            cancel_vwap(State(Arc::clone(&store)), Path("absent".into())).await,
            StatusCode::NOT_FOUND,
        );
        assert_eq!(
            subscribe_market_book(State(Arc::clone(&store)), Path("bad symbol".into())).await,
            StatusCode::BAD_REQUEST
        );

        store
            .persistence_timings_ms
            .lock()
            .unwrap()
            .insert("snapshot".into(), vec![1.0, 3.0, 2.0]);
        let timing = get_persistence_timing(State(Arc::clone(&store))).await.0;
        assert_eq!(timing["snapshot"].samples, 3);
        assert_eq!(timing["snapshot"].p95_ms, 3.0);
        let account =
            account_ui_snapshot(&store, &serde_json::json!({"rates": {"USD": 1.0}})).await;
        assert_eq!(account["currency"], "USD");
        assert!(
            position_ui_snapshot(&store).await["positions"]["positions"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(!hedge_candidates_snapshot(&store).await.is_null());
    }

    #[tokio::test]
    async fn scheduled_run_helpers_cover_cancelled_and_missing_book_paths() {
        let store = store();
        let status = Arc::new(Mutex::new(VwapRunStatus {
            id: "cancelled-run".into(),
            symbol: "BTC-USDT".into(),
            side: TradeSide::Buy,
            quantity: 2.0,
            executed_quantity: 0.0,
            strategy: "twap".into(),
            duration_minutes: 5,
            started_at: 1,
            status: "running".into(),
            market_vwap: 0.0,
            execution_vwap: 0.0,
            slippage: 0.0,
            error: None,
        }));
        run_twap(
            Arc::clone(&store),
            NewVwapRun {
                symbol: "BTC-USDT".into(),
                side: TradeSide::Buy,
                quantity: 2.0,
                duration_minutes: 5,
                max_participation_rate: 1.0,
                strategy: ExecutionStrategy::Twap,
                source: "manual".into(),
                client_order_id: None,
            },
            "BTC-USDT".into(),
            Arc::clone(&status),
            Arc::new(AtomicBool::new(true)),
        )
        .await;
        assert_eq!(status.lock().await.status, "cancelled");
        let slot = store.book_slot("BTC-USDT");
        assert_eq!(
            execute_vwap_child_from_slot(
                &store,
                "BTC-USDT",
                &TradeSide::Buy,
                1.0,
                &slot,
                "manual",
                "parent",
            )
            .await
            .unwrap_err(),
            "Live order book unavailable for VWAP child order"
        );
        store.vwap_runs.lock().await.insert(
            "cancelled-run".into(),
            VwapRunEntry {
                status: Arc::clone(&status),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        );
        assert_eq!(
            cancel_vwap(State(store), Path("cancelled-run".into()))
                .await
                .unwrap(),
            StatusCode::NO_CONTENT
        );
    }

    #[tokio::test]
    async fn market_and_barrier_submission_reject_invalid_requests() {
        let store = store();
        let invalid_order = NewOrder {
            symbol: "bad symbol".into(),
            side: TradeSide::Buy,
            order_type: OrderType::Ioc,
            quantity: 1.0,
            limit_price: None,
            measure_execution_quality: false,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
            source: "manual".into(),
            client_order_id: None,
            reduce_only: false,
        };
        assert_status(
            submit_order(State(Arc::clone(&store)), Json(invalid_order)).await,
            StatusCode::BAD_REQUEST,
        );
        let invalid_barrier = NewOrder {
            symbol: "BTC-USDT".into(),
            side: TradeSide::Sell,
            order_type: OrderType::Barrier,
            quantity: 1.0,
            limit_price: None,
            measure_execution_quality: false,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
            source: "manual".into(),
            client_order_id: None,
            reduce_only: false,
        };
        assert_status(
            submit_barrier_order(Arc::clone(&store), "BTC-USDT".into(), invalid_barrier).await,
            StatusCode::BAD_REQUEST,
        );
        assert_status(
            cancel_order(State(store), Path("not-present".into())).await,
            StatusCode::NOT_FOUND,
        );
    }

    #[test]
    fn persistence_sample_ring_buffer_keeps_only_latest_five_hundred_values() {
        let samples = StdMutex::new(BTreeMap::new());
        for sample in 0..501 {
            record_position_persistence_sample(&samples, "write", sample as f64);
        }
        let samples = samples.lock().unwrap();
        assert_eq!(samples["write"].len(), 500);
        assert_eq!(samples["write"][0], 1.0);
        assert_eq!(samples["write"][499], 500.0);
    }

    #[tokio::test]
    async fn ticker_samples_are_throttled_and_stale_history_is_pruned() {
        let store = store();
        store
            .record_ticker_sample("BTC-USDT".into(), 100.0, 10_000)
            .await;
        store
            .record_ticker_sample("BTC-USDT".into(), 101.0, 15_000)
            .await;
        store
            .record_ticker_sample("BTC-USDT".into(), 102.0, 40 * 60_000)
            .await;
        let samples = store.hedge_samples.lock().await;
        assert_eq!(samples["BTC-USDT"], vec![(40 * 60_000, 102.0)]);
    }
}
