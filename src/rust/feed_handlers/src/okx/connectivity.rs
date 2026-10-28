use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use reqwest::header::USER_AGENT;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashSet, VecDeque},
    sync::{atomic::{AtomicU64, Ordering}, Arc},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast::Sender, mpsc, watch};
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Utf8Bytes};

use crate::encoders::{SYMBOL_LENGTH, encode_book_depth, encode_symbol_price, encode_trade_tick};

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const STABLE_CONNECTION_DURATION: Duration = Duration::from_secs(60);
const VOLATILITY_HISTORY_MS: u64 = 300_000;
const OKX_IDLE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const OKX_PONG_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(crate) struct RollingVolatility {
    samples: VecDeque<(u64, f64)>,
    minute_returns: VecDeque<(u64, f64)>,
    window_returns: VecDeque<(u64, f64)>,
    minute_squared_sum: f64,
    window_squared_sum: f64,
}


include!("volatility_tracking.rs");
include!("instrument_catalog.rs");
include!("okx_message_types.rs");
include!("usd_rate_stream.rs");
include!("ticker_trade_streams.rs");
include!("market_book_stream.rs");
include!("book_update_helpers.rs");
include!("okx_connection_loop.rs");
include!("reconnect_policy.rs");
