pub fn publish_news_snapshot(snapshot: Bytes) -> bool {
    let Some(sender) = NEWS_SNAPSHOT.get() else { return false; };
    sender.send_replace(snapshot);
    true
}

pub fn publish_signal_snapshot(symbol: &str, snapshot: Bytes) -> bool {
    let Some(feeds) = SIGNAL_FEEDS.get() else { return false; };
    let sender = feeds.get(symbol).map(|entry| entry.value().clone());
    if let Some(sender) = sender {
        let _ = sender.send(snapshot);
        true
    } else {
        false
    }
}

fn start_signal_feed(
    instrument: &str,
    feeds: &Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    price_feeds: &Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    events: &broadcast::Sender<Bytes>,
) -> broadcast::Sender<Bytes> {
    let price_sender = price_feeds
        .entry(instrument.to_owned())
        .or_insert_with(|| broadcast::channel(64).0)
        .value()
        .clone();
    let price_receiver = price_sender.subscribe();

    // Reserve the symbol entry first, then start its task after releasing the
    // DashMap shard guard. The task may itself access shared feed maps.
    let (sender, should_start) = match feeds.entry(instrument.to_owned()) {
        dashmap::mapref::entry::Entry::Occupied(entry) => (entry.get().clone(), false),
        dashmap::mapref::entry::Entry::Vacant(entry) => {
            let (sender, _) = broadcast::channel(64);
            entry.insert(sender.clone());
            (sender, true)
        }
    };
    if should_start {
        tokio::spawn(run_signal_feed(instrument.to_owned(), price_receiver, events.clone()));
    }
    sender
}


async fn run_signal_feed(
    instrument: String,
    mut prices: broadcast::Receiver<Bytes>,
    events: broadcast::Sender<Bytes>,
) {
    let mut trades = trade_request_stream(instrument.clone());
    let mut books = book_request_stream(instrument.clone());
    loop {
        tokio::select! {
            incoming = prices.recv() => match incoming {
                Ok(bytes) if has_symbol(&bytes, &instrument) => {
                    if bytes.len() >= 64 && read_f64(&bytes, 40).is_some() && read_f64(&bytes, 48).is_some() {
                        let mut message = BytesMut::with_capacity(bytes.len() + 8);
                        message.extend_from_slice(&bytes);
                        message.extend_from_slice(&now_ms().to_le_bytes());
                        let _ = events.send(message.freeze());
                    }
                }
                Ok(_) => {},
                Err(broadcast::error::RecvError::Lagged(skipped)) => {
                    crate::service_log!("WARN", "Signal price feed for {instrument} lagged; skipped {skipped} ticker updates");
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
            incoming = trades.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if has_symbol(bytes, &instrument) {
                        if bytes.len() >= 97 {
                            let _ = events.send(Bytes::clone(payload.data().expect("checked payload data")));
                        }
                    }
                }
            },
            incoming = books.next() => if let Some(Ok(payload)) = incoming {
                if let Some(bytes) = payload.data() {
                    let bytes = bytes.as_ref();
                    if has_symbol(bytes, &instrument) && bytes.len() >= 272 {
                        let _ = events.send(Bytes::clone(payload.data().expect("checked payload data")));
                    }
                }
            },
        }
    }
}

fn symbol_from_sbe(bytes: &[u8]) -> Option<&str> {
    let field = bytes.get(8..40)?;
    let end = field.iter().position(|byte| *byte == b' ' || *byte == 0).unwrap_or(field.len());
    std::str::from_utf8(&field[..end]).ok()
}

fn has_symbol(bytes: &[u8], symbol: &str) -> bool {
    let Some(field) = bytes.get(8..40) else {
        return false;
    };
    let end = field
        .iter()
        .position(|byte| *byte == b' ' || *byte == 0)
        .unwrap_or(field.len());
    &field[..end] == symbol.as_bytes()
}

fn read_f64(bytes: &[u8], offset: usize) -> Option<f64> {
    let value = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    value.is_finite().then_some(value)
}
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

