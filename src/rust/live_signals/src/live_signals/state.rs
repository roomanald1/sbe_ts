use super::*;

pub(super) struct LiveState {
    pub(super) latest: DashMap<String, Value>,
    pub(super) markets: DashMap<String, Value>,
    pub(super) pending_delivery: DashMap<String, Value>,
    pub(super) delivery_notify: Notify,
    pub(super) news_context: watch::Receiver<Arc<NewsContext>>,
}

impl LiveState {
    pub(super) fn new(news_context: watch::Receiver<Arc<NewsContext>>) -> Self {
        Self {
            latest: DashMap::new(),
            markets: DashMap::new(),
            pending_delivery: DashMap::new(),
            delivery_notify: Notify::new(),
            news_context,
        }
    }
}

pub(super) static HTTP_CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();

pub(super) fn enqueue_snapshot(state: &LiveState, symbol: &str, snapshot: &SignalSnapshot) {
    let context = Arc::clone(&state.news_context.borrow());
    let mut snapshot = snapshot.clone();
    apply_news(&mut snapshot, &context, now_ms());
    let mut markets = serde_json::Map::new();
    if let Some(market) = state.markets.get(symbol) {
        markets.insert(symbol.to_owned(), market.value().clone());
    }
    let dashboard = json!({
        "best": [snapshot.clone()],
        "walkForward": context.walk_forward,
        "missedOpportunityBands": context.missed_opportunity_bands,
        "markets": Value::Object(markets)
    });
    state.latest.insert(
        symbol.to_owned(),
        serde_json::to_value(&snapshot).unwrap_or(Value::Null),
    );
    state.pending_delivery.insert(symbol.to_owned(), dashboard);
    state.delivery_notify.notify_one();
}

#[cfg(test)]
mod tests {
    use super::*;
    use insights::signals::SignalComponents;

    fn snapshot(symbol: &str) -> SignalSnapshot {
        SignalSnapshot {
            symbol: symbol.into(),
            timestamp: 1_000,
            direction: "bullish".into(),
            score: 0.7,
            strength: 0.7,
            components: SignalComponents::default(),
            reasons: vec!["test".into()],
            news_attribution: None,
            stale: false,
            ranging_probability: None,
            range_lower_bound: None,
            range_upper_bound: None,
        }
    }

    #[test]
    fn live_state_initializes_empty_caches_and_keeps_context_receiver() {
        let (sender, receiver) = watch::channel(Arc::new(NewsContext::default()));
        let state = LiveState::new(receiver);
        assert!(sender.send(Arc::new(NewsContext::default())).is_ok());

        assert!(state.latest.is_empty());
        assert!(state.markets.is_empty());
        assert!(state.pending_delivery.is_empty());
        assert!(state.news_context.has_changed().is_ok());
    }

    #[test]
    fn enqueue_snapshot_builds_autotrader_payload_and_uses_cached_market() {
        let (_sender, receiver) = watch::channel(Arc::new(NewsContext {
            walk_forward: vec![json!({"symbol": "BTC-USDT"})],
            missed_opportunity_bands: vec![json!({"gate": "score"})],
            ..NewsContext::default()
        }));
        let state = LiveState::new(receiver);
        state.markets.insert(
            "BTC-USDT".into(),
            json!({"bids": [{"price": 99.0, "size": 2.0}]}),
        );

        enqueue_snapshot(&state, "BTC-USDT", &snapshot("BTC-USDT"));

        assert_eq!(state.latest.get("BTC-USDT").unwrap()["score"], 0.7);
        let payload = state.pending_delivery.get("BTC-USDT").unwrap();
        assert_eq!(payload["best"][0]["symbol"], "BTC-USDT");
        assert_eq!(payload["walkForward"][0]["symbol"], "BTC-USDT");
        assert_eq!(payload["missedOpportunityBands"][0]["gate"], "score");
        assert_eq!(payload["markets"]["BTC-USDT"]["bids"][0]["price"], 99.0);
    }
}
