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
