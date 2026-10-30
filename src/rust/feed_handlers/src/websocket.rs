use anyhow::Result;
use bytes::{Bytes, BytesMut};
use dashmap::DashMap;
use futures_util::{Stream, StreamExt};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use std::{
    collections::HashSet,
    env,
    pin::Pin,
    sync::{Arc, OnceLock},
};
use tokio::sync::{broadcast, watch};
use tokio::time::{Duration, sleep};

use crate::encoders::{SYMBOL_LENGTH, encode_symbol_price_bytes};
use crate::okx::connectivity::{
    RollingVolatility, get_all_spot_instruments, get_instrument_data, subscribe,
    run_usd_rate_stream, subscribe_shared_books, subscribe_shared_trades,
};

const PRICE_UPDATE_CAPACITY: usize = 4096;
const TICKER_BATCH_INTERVAL: Duration = Duration::from_millis(10);
const SYMBOL_PRICE_MESSAGE_LENGTH: usize = 64;
const MAX_TICKER_BATCH_MESSAGES: usize = 256;
const TICKER_BATCH_CAPACITY: usize = SYMBOL_PRICE_MESSAGE_LENGTH * MAX_TICKER_BATCH_MESSAGES;
// Generate signals across the full top-volume spot universe so Auto Trader can
// find fresh candidates beyond the first 30 instruments.
const SIGNAL_INSTRUMENT_LIMIT: usize = 100;
static SIGNAL_FEEDS: OnceLock<Arc<DashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static SIGNAL_PRICE_FEEDS: OnceLock<Arc<DashMap<String, broadcast::Sender<Bytes>>>> = OnceLock::new();
static NEWS_SNAPSHOT: OnceLock<watch::Sender<Bytes>> = OnceLock::new();


include!("websocket_server.rs");
include!("websocket_responder.rs");
include!("websocket_stream_lifecycle.rs");
include!("websocket_publishers.rs");
include!("trade_stream_requests.rs");
include!("book_stream_requests.rs");
include!("synthetic_feed.rs");
