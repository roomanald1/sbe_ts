use axum::{Json, Router, routing::get};
use dashmap::DashMap;
use futures_util::StreamExt;
use insights::signals::{NewsAttribution, NewsHeadlineAttribution, SignalEngine, SignalSnapshot};
use rsocket_rust::{prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Notify, mpsc, watch},
    time::{Duration, Instant},
};

const EVENT_SHARDS: usize = 16;
const EVENT_QUEUE_CAPACITY: usize = 1_024;
const INGRESS_QUEUE_CAPACITY: usize = 8_192;
const PUBLISH_CONCURRENCY: usize = 8;

mod api;
mod delivery;
mod feed;
mod market_data;
mod news_context;
mod signal_workers;
mod state;

use api::router;
use delivery::run_delivery_worker;
use feed::feed_consumer;
use market_data::{MarketEvent, SignalEvent, book_value, decode_event, now_ms};
use news_context::{NewsContext, apply_news, poll_research_context};
use signal_workers::run_shards;
use state::{HTTP_CLIENT, LiveState, enqueue_snapshot};

pub(crate) async fn run() -> anyhow::Result<()> {
    let (context_tx, context_rx) = watch::channel(Arc::new(NewsContext::default()));
    let state = Arc::new(LiveState::new(context_rx));
    tokio::spawn(poll_research_context(context_tx));
    tokio::spawn(run_delivery_worker(Arc::clone(&state)));
    let (event_tx, event_rx) = mpsc::channel(INGRESS_QUEUE_CAPACITY);
    tokio::spawn(run_shards(Arc::clone(&state), event_rx));
    tokio::spawn(feed_consumer(event_tx));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3007)).await?;
    crate::service_log!(
        "INFO",
        "Live Signals service listening on http://0.0.0.0:3007; publishes current market signals directly to AutoTrader"
    );
    axum::serve(listener, app).await?;
    Ok(())
}
