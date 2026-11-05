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
    RollingVolatility, get_all_spot_instruments, get_instrument_data, run_usd_rate_stream,
    subscribe, subscribe_shared_books, subscribe_shared_trades,
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
static SIGNAL_PRICE_FEEDS: OnceLock<Arc<DashMap<String, broadcast::Sender<Bytes>>>> =
    OnceLock::new();
static NEWS_SNAPSHOT: OnceLock<watch::Sender<Bytes>> = OnceLock::new();

include!("server.rs");
include!("responder.rs");
include!("stream_lifecycle.rs");
include!("publishers.rs");
include!("trade_stream_requests.rs");
include!("book_stream_requests.rs");
include!("synthetic_feed.rs");

#[cfg(test)]
mod websocket_request_tests {
    use super::{
        publish_news_snapshot, publish_signal_snapshot, requested_book_instrument,
        requested_signal_instrument, requested_trade_instrument,
    };
    use bytes::Bytes;
    use rsocket_rust::prelude::Payload;

    #[tokio::test]
    async fn stream_lifecycle_preserves_payloads_and_ends_cleanly() {
        use super::log_stream_end;
        use futures_util::{StreamExt, stream};

        let payload = Payload::builder().set_data(b"tick".to_vec()).build();
        let mut wrapped = log_stream_end(
            stream::iter(vec![Ok(payload.clone())]),
            "test stream ended".into(),
        );
        let received = wrapped.next().await.unwrap().unwrap();
        assert_eq!(received.data(), payload.data());
        assert!(wrapped.next().await.is_none());
    }

    #[test]
    fn request_parsers_accept_only_their_own_utf8_prefixes() {
        let signal = Payload::builder()
            .set_data(b"signals:BTC-USDT".to_vec())
            .build();
        let trades = Payload::builder()
            .set_data(b"trades:ETH-USDT".to_vec())
            .build();
        let book = Payload::builder()
            .set_data(b"book:SOL-USDT".to_vec())
            .build();
        let invalid_utf8 = Payload::builder().set_data(vec![0xff]).build();

        assert_eq!(requested_signal_instrument(&signal), Some("BTC-USDT"));
        assert_eq!(requested_trade_instrument(&trades), Some("ETH-USDT"));
        assert_eq!(requested_book_instrument(&book), Some("SOL-USDT"));
        assert_eq!(requested_signal_instrument(&trades), None);
        assert_eq!(requested_trade_instrument(&book), None);
        assert_eq!(requested_book_instrument(&signal), None);
        assert_eq!(requested_signal_instrument(&invalid_utf8), None);
        assert_eq!(requested_trade_instrument(&Payload::new(None, None)), None);
    }

    #[test]
    fn publishers_report_unavailable_before_server_initialization() {
        assert!(!publish_news_snapshot(Bytes::new()));
        assert!(!publish_signal_snapshot("BTC-USDT", Bytes::new()));
    }
}
