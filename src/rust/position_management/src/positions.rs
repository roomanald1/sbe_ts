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
