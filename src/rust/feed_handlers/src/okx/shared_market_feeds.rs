#[derive(Clone, Copy)]
enum MarketFeedKind {
    Trades,
    Books,
}

impl MarketFeedKind {
    fn label(self) -> &'static str {
        match self {
            Self::Trades => "trades",
            Self::Books => "books",
        }
    }
}

enum MarketFeedCommand {
    Subscribe(String),
    Unsubscribe(String),
}

const SUBSCRIPTION_BATCH_WINDOW: Duration = Duration::from_millis(25);

struct MarketFeedEntry {
    updates: broadcast::Sender<Bytes>,
    subscribers: usize,
}

struct SharedMarketFeed {
    kind: MarketFeedKind,
    commands: mpsc::UnboundedSender<MarketFeedCommand>,
    entries: StdMutex<HashMap<String, MarketFeedEntry>>,
}

pub(crate) struct MarketFeedSubscription {
    instrument: String,
    receiver: broadcast::Receiver<Bytes>,
    feed: Arc<SharedMarketFeed>,
}

impl MarketFeedSubscription {
    pub(crate) async fn recv(&mut self) -> std::result::Result<Bytes, broadcast::error::RecvError> {
        let result = self.receiver.recv().await;
        let queue = format!("shared_{}/{}", self.feed.kind.label(), self.instrument);
        crate::metrics::record_queue_depth(&queue, self.receiver.len());
        if let Err(broadcast::error::RecvError::Lagged(skipped)) = result {
            crate::metrics::record_queue_lagged(&queue, skipped);
        }
        result
    }
}

impl Drop for MarketFeedSubscription {
    fn drop(&mut self) {
        let mut entries = self
            .feed
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(entry) = entries.get_mut(&self.instrument) else {
            return;
        };
        if entry.subscribers > 1 {
            entry.subscribers -= 1;
            return;
        }
        entries.remove(&self.instrument);
        let _ = self
            .feed
            .commands
            .send(MarketFeedCommand::Unsubscribe(self.instrument.clone()));
    }
}

impl SharedMarketFeed {
    fn new(kind: MarketFeedKind) -> Arc<Self> {
        let (commands, receiver) = mpsc::unbounded_channel();
        let feed = Arc::new(Self {
            kind,
            commands,
            entries: StdMutex::new(HashMap::new()),
        });
        tokio::spawn(run_shared_market_feed(kind, receiver, Arc::clone(&feed)));
        feed
    }

    fn subscribe(self: &Arc<Self>, instrument: &str) -> MarketFeedSubscription {
        let mut entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let receiver = match entries.entry(instrument.to_owned()) {
            std::collections::hash_map::Entry::Occupied(mut entry) => {
                entry.get_mut().subscribers += 1;
                entry.get().updates.subscribe()
            }
            std::collections::hash_map::Entry::Vacant(entry) => {
                let (updates, _) = broadcast::channel(match self.kind {
                    MarketFeedKind::Trades => SHARED_TRADE_UPDATE_CAPACITY,
                    MarketFeedKind::Books => SHARED_BOOK_UPDATE_CAPACITY,
                });
                let receiver = updates.subscribe();
                crate::metrics::record_queue_depth(
                    &format!("shared_{}/{}", self.kind.label(), instrument),
                    updates.len(),
                );
                entry.insert(MarketFeedEntry {
                    updates,
                    subscribers: 1,
                });
                let _ = self
                    .commands
                    .send(MarketFeedCommand::Subscribe(instrument.to_owned()));
                receiver
            }
        };
        MarketFeedSubscription {
            instrument: instrument.to_owned(),
            receiver,
            feed: Arc::clone(self),
        }
    }

    fn publish(&self, instrument: &str, update: Bytes) {
        let entries = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(entry) = entries.get(instrument) {
            let _ = entry.updates.send(update);
            crate::metrics::record_queue_depth(&format!("shared_{}/{}", self.kind.label(), instrument), entry.updates.len());
        }
    }
}

static SHARED_TRADE_FEED: OnceLock<Arc<SharedMarketFeed>> = OnceLock::new();
static SHARED_BOOK_FEED: OnceLock<Arc<SharedMarketFeed>> = OnceLock::new();

pub(crate) fn subscribe_shared_trades(instrument: &str) -> MarketFeedSubscription {
    SHARED_TRADE_FEED
        .get_or_init(|| SharedMarketFeed::new(MarketFeedKind::Trades))
        .subscribe(instrument)
}

pub(crate) fn subscribe_shared_books(instrument: &str) -> MarketFeedSubscription {
    SHARED_BOOK_FEED
        .get_or_init(|| SharedMarketFeed::new(MarketFeedKind::Books))
        .subscribe(instrument)
}

async fn run_shared_market_feed(
    kind: MarketFeedKind,
    mut commands: mpsc::UnboundedReceiver<MarketFeedCommand>,
    feed: Arc<SharedMarketFeed>,
) {
    let mut active = HashSet::new();
    let mut retry_delay = INITIAL_RECONNECT_DELAY;
    loop {
        while active.is_empty() {
            let Some(command) = commands.recv().await else {
                return;
            };
            apply_market_feed_command(command, &mut active);
        }
        while let Ok(command) = commands.try_recv() {
            apply_market_feed_command(command, &mut active);
        }
        if active.is_empty() {
            continue;
        }
        sleep(SUBSCRIPTION_BATCH_WINDOW).await;
        while let Ok(command) = commands.try_recv() {
            apply_market_feed_command(command, &mut active);
        }
        if active.is_empty() {
            continue;
        }

        let attempt_started = Instant::now();
        match run_shared_market_feed_connection(kind, &mut commands, &feed, &mut active).await {
            Ok(()) if active.is_empty() => retry_delay = INITIAL_RECONNECT_DELAY,
            Ok(()) => crate::service_log!(
                "WARN",
                "Shared OKX {} connection ended; reconnecting in {}s",
                kind.label(),
                retry_delay.as_secs()
            ),
            Err(error) if !active.is_empty() => crate::service_log!(
                "ERROR",
                "Shared OKX {} connection failed: {error:#}; reconnecting in {}s",
                kind.label(),
                retry_delay.as_secs()
            ),
            Err(_) => {}
        }
        if active.is_empty() {
            continue;
        }
        if attempt_started.elapsed() >= STABLE_CONNECTION_DURATION {
            retry_delay = INITIAL_RECONNECT_DELAY;
        }
        sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

fn apply_market_feed_command(command: MarketFeedCommand, active: &mut HashSet<String>) {
    match command {
        MarketFeedCommand::Subscribe(instrument) => {
            active.insert(instrument);
        }
        MarketFeedCommand::Unsubscribe(instrument) => {
            active.remove(&instrument);
        }
    }
}

async fn send_market_feed_request(
    write: &mut (impl SinkExt<tokio_tungstenite::tungstenite::Message> + Unpin),
    kind: MarketFeedKind,
    operation: &str,
    instruments: &[String],
) -> Result<()> {
    for request in market_feed_requests(kind, operation, instruments) {
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                request,
            ))
            .await
            .map_err(|_| {
                anyhow::anyhow!("failed to send OKX {} {operation} request", kind.label())
            })?;
    }
    Ok(())
}

fn market_feed_requests(
    kind: MarketFeedKind,
    operation: &str,
    instruments: &[String],
) -> Vec<Utf8Bytes> {
    instruments
        .chunks(240)
        .map(|batch| {
            let args = batch
                .iter()
                .map(|instrument| json!({ "channel": kind.label(), "instId": instrument }))
                .collect::<Vec<_>>();
            Utf8Bytes::from(json!({ "op": operation, "args": args }).to_string())
        })
        .collect()
}


fn process_shared_market_message(
    kind: MarketFeedKind,
    text: &str,
    feed: &SharedMarketFeed,
    book_state: &mut HashMap<String, (Vec<(f64, f64)>, Vec<(f64, f64)>)>,
) -> Result<()> {
    match kind {
                MarketFeedKind::Trades => {
                    let update = match serde_json::from_str::<TradeMessage<'_>>(text) {
                        Ok(update) => update,
                        Err(error) => {
                            crate::service_log!("ERROR", "Ignoring invalid shared OKX trades JSON: {error}");
                            return Ok(());
                        }
                    };
                    if update.event == Some("error") {
                        anyhow::bail!("OKX shared trades subscription error: {}", update.msg.unwrap_or("unknown error"));
                    }
                    for trade in update.data.unwrap_or_default() {
                        let (Ok(price), Ok(size), Ok(timestamp)) = (
                            trade.px.parse::<f64>(), trade.sz.parse::<f64>(), trade.ts.parse::<u64>()
                        ) else { continue; };
                        let encoded = encode_trade_tick(
                            trade.instrument,
                            trade.trade_id,
                            timestamp,
                            price,
                            size,
                            trade.side,
                        )?;
                        crate::metrics::record_trade();
                        feed.publish(trade.instrument, encoded);
                    }
                }
                MarketFeedKind::Books => {
                    let update = match serde_json::from_str::<BookMessage<'_>>(text) {
                        Ok(update) => update,
                        Err(error) => {
                            crate::service_log!("ERROR", "Ignoring invalid shared OKX books JSON: {error}");
                            return Ok(());
                        }
                    };
                    if update.event == Some("error") {
                        anyhow::bail!("OKX shared books subscription error: {}", update.msg.unwrap_or("unknown error"));
                    }
                    let (Some(argument), Some(items)) = (update.arg, update.data) else { return Ok(()); };
                    let state = book_state.entry(argument.instrument.to_owned()).or_default();
                    for item in items {
                        if update.action == Some("snapshot") {
                            load_book_snapshot(&mut state.0, &item.bids)?;
                            load_book_snapshot(&mut state.1, &item.asks)?;
                        } else {
                            apply_book_side(&mut state.0, &item.bids, true)?;
                            apply_book_side(&mut state.1, &item.asks, false)?;
                        }
                        let bids = top_book_side(&state.0);
                        let asks = top_book_side(&state.1);
                        let timestamp = item.ts.parse::<u64>()
                            .map_err(|error| anyhow::anyhow!("invalid OKX book timestamp: {error}"))?;
                        let encoded = encode_book_depth(argument.instrument, timestamp, &bids, &asks)?;
                        crate::metrics::record_book();
                        feed.publish(argument.instrument, encoded);
                    }
                }
    }
    Ok(())
}

async fn run_shared_market_feed_connection(
    kind: MarketFeedKind,
    commands: &mut mpsc::UnboundedReceiver<MarketFeedCommand>,
    feed: &SharedMarketFeed,
    active: &mut HashSet<String>,
) -> Result<()> {
    wait_for_okx_connection_slot().await;
    let (socket, _) = connect_async(OKX_PUBLIC_WS_URL).await.map_err(|error| {
        anyhow::anyhow!(
            "failed to connect to shared OKX {} stream: {error}",
            kind.label()
        )
    })?;
    crate::service_log!("INFO", "Shared OKX {} WebSocket connected", kind.label());
    let (mut write, mut read) = socket.split();
    let instruments = active.iter().cloned().collect::<Vec<_>>();
    send_market_feed_request(&mut write, kind, "subscribe", &instruments).await?;
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    let pong_timeout = tokio::time::sleep(Duration::from_secs(60 * 60 * 24 * 365));
    tokio::pin!(heartbeat);
    tokio::pin!(pong_timeout);
    let mut awaiting_pong = false;
    let mut book_state = HashMap::<String, (Vec<(f64, f64)>, Vec<(f64, f64)>)>::new();

    loop {
        tokio::select! {
            _ = &mut pong_timeout, if awaiting_pong => anyhow::bail!(
                "OKX shared {} WebSocket did not answer ping within {}s",
                kind.label(),
                OKX_PONG_TIMEOUT.as_secs()
            ),
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                awaiting_pong = true;
                pong_timeout.as_mut().reset(Instant::now() + OKX_PONG_TIMEOUT);
                heartbeat.as_mut().reset(Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            command = commands.recv() => {
                let Some(command) = command else { return Ok(()); };
                let previous = active.clone();
                apply_market_feed_command(command, active);
                sleep(SUBSCRIPTION_BATCH_WINDOW).await;
                while let Ok(command) = commands.try_recv() {
                    apply_market_feed_command(command, active);
                }
                let added = active.difference(&previous).cloned().collect::<Vec<_>>();
                let removed = previous.difference(active).cloned().collect::<Vec<_>>();
                if !added.is_empty() {
                    send_market_feed_request(&mut write, kind, "subscribe", &added).await?;
                }
                if !removed.is_empty() {
                    send_market_feed_request(&mut write, kind, "unsubscribe", &removed).await?;
                    for instrument in removed {
                        book_state.remove(&instrument);
                    }
                }
                if active.is_empty() {
                    write.close().await?;
                    return Ok(());
                }
            }
            message = read.next() => {
                let Some(message) = message else {
                    anyhow::bail!("OKX shared {} WebSocket stream ended", kind.label());
                };
                let message = message?;
                heartbeat.as_mut().reset(Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                if !message.is_text() { continue; }
                let text = message.to_text()?;
                if text == "pong" {
                    awaiting_pong = false;
                    pong_timeout.as_mut().reset(Instant::now() + Duration::from_secs(60 * 60 * 24 * 365));
                    continue;
                }
                process_shared_market_message(kind, text, feed, &mut book_state)?;
            }
        }
    }
}

#[cfg(test)]
mod shared_market_feed_tests {
    use super::{
        MarketFeedCommand, MarketFeedKind, SharedMarketFeed, apply_market_feed_command,
        market_feed_requests, process_shared_market_message, send_market_feed_request,
    };
    use bytes::Bytes;
    use std::collections::{HashMap, HashSet};
    use std::sync::{Arc, Mutex as StdMutex};
    use tokio::sync::mpsc;

    #[test]
    fn feed_kinds_have_stable_channel_labels() {
        assert_eq!(MarketFeedKind::Trades.label(), "trades");
        assert_eq!(MarketFeedKind::Books.label(), "books");
    }

    #[test]
    fn subscription_commands_add_and_remove_active_instruments() {
        let mut active = HashSet::new();
        apply_market_feed_command(MarketFeedCommand::Subscribe("BTC-USDT".into()), &mut active);
        apply_market_feed_command(MarketFeedCommand::Subscribe("ETH-USDT".into()), &mut active);
        apply_market_feed_command(MarketFeedCommand::Subscribe("BTC-USDT".into()), &mut active);
        assert_eq!(active.len(), 2);
        apply_market_feed_command(MarketFeedCommand::Unsubscribe("BTC-USDT".into()), &mut active);
        assert_eq!(active, HashSet::from(["ETH-USDT".to_owned()]));
        apply_market_feed_command(MarketFeedCommand::Unsubscribe("MISSING".into()), &mut active);
        assert_eq!(active, HashSet::from(["ETH-USDT".to_owned()]));
    }

    #[tokio::test]
    async fn subscriptions_share_updates_and_unsubscribe_after_the_last_drop() {
        let (commands, mut command_receiver) = mpsc::unbounded_channel();
        let feed = Arc::new(SharedMarketFeed {
            kind: MarketFeedKind::Trades,
            commands,
            entries: StdMutex::new(HashMap::new()),
        });
        let mut first = feed.subscribe("BTC-USDT");
        assert!(matches!(command_receiver.try_recv(), Ok(MarketFeedCommand::Subscribe(symbol)) if symbol == "BTC-USDT"));
        let mut second = feed.subscribe("BTC-USDT");
        assert!(command_receiver.try_recv().is_err());

        feed.publish("BTC-USDT", Bytes::from_static(b"trade"));
        assert_eq!(first.recv().await.unwrap(), Bytes::from_static(b"trade"));
        assert_eq!(second.recv().await.unwrap(), Bytes::from_static(b"trade"));
        feed.publish("UNKNOWN", Bytes::from_static(b"ignored"));

        drop(first);
        assert!(command_receiver.try_recv().is_err());
        drop(second);
        assert!(matches!(command_receiver.try_recv(), Ok(MarketFeedCommand::Unsubscribe(symbol)) if symbol == "BTC-USDT"));
    }

    #[tokio::test]
    async fn processes_trade_and_book_messages_and_rejects_subscription_errors() {
        let (commands, _command_receiver) = mpsc::unbounded_channel();
        let feed = Arc::new(SharedMarketFeed {
            kind: MarketFeedKind::Trades,
            commands,
            entries: StdMutex::new(HashMap::new()),
        });
        let mut trades = feed.subscribe("BTC-USDT");
        let mut book_state = HashMap::new();
        process_shared_market_message(
            MarketFeedKind::Trades,
            r#"{"data":[{"instId":"BTC-USDT","tradeId":"1","px":"100","sz":"2","side":"buy","ts":"1700000000000"}]}"#,
            &feed,
            &mut book_state,
        )
        .unwrap();
        let trade = trades.recv().await.unwrap();
        assert_eq!(trade.len(), 97);
        assert!(process_shared_market_message(
            MarketFeedKind::Trades,
            r#"{"event":"error","msg":"denied"}"#,
            &feed,
            &mut book_state,
        )
        .is_err());
        process_shared_market_message(MarketFeedKind::Trades, "invalid json", &feed, &mut book_state)
            .unwrap();

        let mut books = feed.subscribe("BTC-USDT");
        process_shared_market_message(
            MarketFeedKind::Books,
            r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":[["101","3"]],"bids":[["99","2"]],"ts":"1700000000001"}]}"#,
            &feed,
            &mut book_state,
        )
        .unwrap();
        assert_eq!(books.recv().await.unwrap().len(), 272);
        assert_eq!(book_state["BTC-USDT"].0, [(99.0, 2.0)]);
        assert_eq!(book_state["BTC-USDT"].1, [(101.0, 3.0)]);
        assert!(process_shared_market_message(
            MarketFeedKind::Books,
            r#"{"event":"error","msg":"denied"}"#,
            &feed,
            &mut book_state,
        )
        .is_err());
    }

    #[tokio::test]
    async fn sends_subscription_and_unsubscription_requests_in_batches() {
        use futures_util::SinkExt;
        use std::{convert::Infallible, sync::Mutex};
        use tokio_tungstenite::tungstenite::Message;

        let captured = Arc::new(Mutex::new(Vec::<Message>::new()));
        let captured_sink = Arc::clone(&captured);
        let mut sink = Box::pin(futures_util::sink::unfold(
            captured_sink,
            |state, message| async move {
                state.lock().unwrap().push(message);
                Ok::<_, Infallible>(state)
            },
        ));
        let instruments = (0..241)
            .map(|index| format!("COIN{index}-USDT"))
            .collect::<Vec<_>>();

        send_market_feed_request(&mut sink, MarketFeedKind::Books, "subscribe", &instruments)
            .await
            .unwrap();
        send_market_feed_request(&mut sink, MarketFeedKind::Books, "unsubscribe", &[])
            .await
            .unwrap();

        let messages = captured.lock().unwrap();
        assert_eq!(messages.len(), 2);
        let Message::Text(first) = &messages[0] else { panic!("expected text request"); };
        let Message::Text(second) = &messages[1] else { panic!("expected text request"); };
        let first: serde_json::Value = serde_json::from_str(first).unwrap();
        let second: serde_json::Value = serde_json::from_str(second).unwrap();
        assert_eq!(first["op"], "subscribe");
        assert_eq!(first["args"].as_array().unwrap().len(), 240);
        assert_eq!(first["args"][0]["channel"], "books");
        assert_eq!(second["args"][0]["instId"], "COIN240-USDT");
        assert_eq!(market_feed_requests(MarketFeedKind::Trades, "subscribe", &[]).len(), 0);
    }
}
