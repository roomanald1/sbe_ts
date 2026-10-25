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
    sync::{Mutex, Notify, mpsc},
    task::JoinSet,
    time::{Duration, interval},
};

include!("auto_trader/constants.rs");
include!("auto_trader/market_data.rs");
include!("auto_trader/research_market_data.rs");
include!("auto_trader/settings.rs");
include!("auto_trader/api_types.rs");
include!("auto_trader/state.rs");
include!("auto_trader/learning_state.rs");
include!("auto_trader/cycle_types.rs");
include!("auto_trader/http_api.rs");
include!("auto_trader/workers.rs");
include!("auto_trader/strategy.rs");
include!("auto_trader/position_exits.rs");
include!("auto_trader/hedging.rs");
include!("auto_trader/fills_and_audit.rs");
include!("auto_trader/learning_and_selection.rs");
include!("auto_trader/execution.rs");
include!("auto_trader/diagnostics.rs");
include!("auto_trader/persistence.rs");
