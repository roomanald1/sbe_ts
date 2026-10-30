use arc_swap::ArcSwapOption;
use axum::{
    body::Body,
    extract::{Path, Query, State},
    http::{header::HeaderName, Request, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, post},
    Json, Router,
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex as StdMutex, OnceLock, RwLock as StdRwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{broadcast, mpsc, Mutex, Notify},
    time::{timeout, Duration, Instant},
};

const SYMBOL_PRICE_MESSAGE_LENGTH: usize = 64;
const BOOK_DEPTH: usize = 7;
const NUM_VWAP_BINS: usize = 78;
const EPSILON: f64 = 1e-12;
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
static USD_RATE_CACHE: OnceLock<Mutex<Option<(Instant, UsdRatesResponse)>>> = OnceLock::new();
static USD_RATE_REFRESHING: AtomicBool = AtomicBool::new(false);
static USD_RATE_STREAM_CACHE: OnceLock<StdRwLock<Option<UsdRatesResponse>>> = OnceLock::new();
const USD_RATE_CACHE_TTL: Duration = Duration::from_secs(15);
const USD_RATE_STREAM_MAX_AGE: Duration = Duration::from_secs(120);


include!("positions_routes.rs");

include!("market_book_workers.rs");

include!("usd_rates.rs");

include!("order_types.rs");

include!("book_matching.rs");

include!("position_store_load.rs");

include!("position_store_subscriptions.rs");

include!("scheduled_run_lifecycle.rs");

include!("twap_execution.rs");

include!("vwap_execution.rs");

include!("account_snapshot.rs");

include!("account_capacity.rs");

include!("account_routes.rs");

include!("position_reporting.rs");

include!("hedge_candidates.rs");

include!("market_insights_inputs.rs");

include!("position_trade_history.rs");

include!("order_queries_and_barriers.rs");

include!("order_submission.rs");

include!("order_cancellation.rs");

include!("ledger_helpers.rs");
