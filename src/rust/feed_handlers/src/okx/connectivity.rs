use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use reqwest::header::USER_AGENT;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::{
        Arc, Mutex as StdMutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{
    broadcast::{self, Sender},
    mpsc, watch,
};
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Utf8Bytes};

use crate::encoders::{SYMBOL_LENGTH, encode_book_depth, encode_symbol_price, encode_trade_tick};

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const STABLE_CONNECTION_DURATION: Duration = Duration::from_secs(60);
const OKX_IDLE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);
const OKX_PONG_TIMEOUT: Duration = Duration::from_secs(10);
const OKX_PUBLIC_WS_URL: &str = "wss://ws.okx.com/ws/v5/public";
const OKX_CONNECT_MIN_INTERVAL: Duration = Duration::from_millis(350);
const SHARED_TRADE_UPDATE_CAPACITY: usize = 512;
const SHARED_BOOK_UPDATE_CAPACITY: usize = 32;
static OKX_NEXT_CONNECT_SLOT: OnceLock<tokio::sync::Mutex<Instant>> = OnceLock::new();

async fn wait_for_okx_connection_slot() {
    let next_slot = OKX_NEXT_CONNECT_SLOT.get_or_init(|| tokio::sync::Mutex::new(Instant::now()));
    let mut next_slot = next_slot.lock().await;
    let now = Instant::now();
    if *next_slot > now {
        tokio::time::sleep_until(*next_slot).await;
    }
    *next_slot = Instant::now() + OKX_CONNECT_MIN_INTERVAL;
}

include!("instrument_catalog.rs");
include!("okx_message_types.rs");
include!("usd_rate_stream.rs");
include!("ticker_trade_streams.rs");
include!("market_book_stream.rs");
include!("book_update_helpers.rs");
include!("okx_connection_loop.rs");
include!("reconnect_policy.rs");
include!("shared_market_feeds.rs");

#[cfg(test)]
mod connection_slot_tests {
    use super::wait_for_okx_connection_slot;
    use std::time::{Duration, Instant};

    #[tokio::test]
    async fn spaces_connection_attempts_by_the_configured_minimum_interval() {
        wait_for_okx_connection_slot().await;
        let started = Instant::now();
        wait_for_okx_connection_slot().await;
        assert!(started.elapsed() >= Duration::from_millis(350));
    }
}
