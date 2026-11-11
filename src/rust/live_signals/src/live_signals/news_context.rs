use super::*;

#[derive(Clone, Deserialize, serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub(super) struct NewsContext {
    #[serde(default)]
    pub(super) news: NewsSnapshot,
    #[serde(default)]
    pub(super) weight: f64,
}

#[derive(Clone, Deserialize, serde::Serialize, Default)]
pub(super) struct NewsSnapshot {
    #[serde(default)]
    pub(super) source: String,
    #[serde(default)]
    pub(super) articles: Vec<NewsArticle>,
}

#[derive(Clone, Deserialize, serde::Serialize)]
pub(super) struct NewsArticle {
    title: String,
    url: String,
    published_at: String,
    received_at_ms: u64,
    matched_asset: String,
    direction: String,
    score: f64,
    signal_reason: String,
    sentiment_model: String,
    sentiment_confidence: f64,
    matched_keywords: Vec<String>,
}

pub(super) fn apply_news(snapshot: &mut SignalSnapshot, context: &NewsContext, now: u64) {
    let asset = snapshot
        .symbol
        .split('-')
        .next()
        .unwrap_or(&snapshot.symbol);
    let relevant = context
        .news
        .articles
        .iter()
        .filter(|article| {
            (article.matched_asset == asset || article.matched_asset == "CRYPTO")
                && article.score != 0.0
                && article.direction != "neutral"
                && now.saturating_sub(article.received_at_ms) <= 30 * 60 * 1_000
        })
        .collect::<Vec<_>>();
    if relevant.is_empty() {
        return;
    }
    let news_score =
        relevant.iter().map(|article| article.score).sum::<f64>() / relevant.len() as f64;
    let pre_news_score = snapshot.score;
    let weight = context.weight.clamp(0.0, 0.20);
    snapshot.score = (snapshot.score * (1.0 - weight) + news_score * weight).clamp(-1.0, 1.0);
    snapshot.refresh_raw_strength();
    snapshot.direction = if snapshot.score > 0.2 {
        "bullish"
    } else if snapshot.score < -0.2 {
        "bearish"
    } else {
        "neutral"
    }
    .into();
    snapshot.news_attribution = Some(NewsAttribution {
        pre_news_score,
        post_news_score: snapshot.score,
        aggregate_score: news_score,
        weight,
        article_count: relevant.len(),
        source: context.news.source.clone(),
        articles: relevant
            .iter()
            .map(|article| NewsHeadlineAttribution {
                title: article.title.clone(),
                url: article.url.clone(),
                published_at: article.published_at.clone(),
                received_at_ms: article.received_at_ms,
                matched_asset: article.matched_asset.clone(),
                direction: article.direction.clone(),
                score: article.score,
                sentiment_model: article.sentiment_model.clone(),
                sentiment_confidence: article.sentiment_confidence,
                matched_keywords: article.matched_keywords.clone(),
                signal_reason: article.signal_reason.clone(),
            })
            .collect(),
    });
    snapshot.reasons.push(format!(
        "strategy_vote:news_sentiment;direction={};vote={news_score:.2};headlines={};weight={weight:.3};source=CoinDesk_RSS;context=headline+description;max_age_seconds=1800;classifier=local_crypto_finbert_or_keyword_fallback",
        if news_score > 0.0 { "bullish" } else { "bearish" }, relevant.len()
    ));
}

pub(super) async fn poll_research_context(sender: watch::Sender<Arc<NewsContext>>) {
    let rsocket_url =
        std::env::var("INSIGHTS_RPC_RSOCKET_URL").unwrap_or_else(|_| "ws://127.0.0.1:10007".into());
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        match refresh_research_context_once(&sender, &rsocket_url).await {
            Ok(()) => {}
            Err(error) => {
                crate::service_log!("WARN", "Could not refresh Insights live context: {error}")
            }
        }
    }
}

async fn refresh_research_context_once(
    sender: &watch::Sender<Arc<NewsContext>>,
    rsocket_url: &str,
) -> anyhow::Result<()> {
    let context = fetch_research_context(rsocket_url).await?;
    let previous = serde_json::to_value(sender.borrow().as_ref())?;
    let next = serde_json::to_value(&context)?;
    if previous != next {
        let _ = sender.send(Arc::new(context));
    }
    Ok(())
}

async fn fetch_research_context(rsocket_url: &str) -> anyhow::Result<NewsContext> {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        strategies::request_scout_service_route(
            rsocket_url,
            &strategies::ScoutCoordinatorRouteCommand {
                request_id: format!("live-signals-context:{}", now_ms()),
                strategy: None,
                method: strategies::ScoutWorkerHttpMethod::Get,
                path: "/internal/live-news-context".into(),
                payload: Value::Null,
            },
        ),
    )
    .await??;
    if response.status >= 400 {
        anyhow::bail!(
            "Insights rejected live-context query: status {}",
            response.status
        );
    }
    Ok(serde_json::from_value(response.body)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use insights::signals::SignalComponents;
    struct TestInsightsResponder {
        body: Value,
        status: u16,
    }

    #[rsocket_rust::async_trait]
    impl RSocket for TestInsightsResponder {
        async fn metadata_push(&self, _request: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn fire_and_forget(&self, _request: Payload) -> anyhow::Result<()> {
            anyhow::bail!("unsupported")
        }
        async fn request_response(&self, request: Payload) -> anyhow::Result<Option<Payload>> {
            let request: strategies::ScoutCoordinatorRequest =
                serde_json::from_slice(request.data().unwrap())?;
            let strategies::ScoutCoordinatorRequest::WorkerRoute(request) = request else {
                anyhow::bail!("expected route command")
            };
            assert_eq!(request.method, strategies::ScoutWorkerHttpMethod::Get);
            assert_eq!(request.path, "/internal/live-news-context");
            let response = strategies::ScoutCoordinatorResponse::WorkerRoute(
                strategies::ScoutCoordinatorRouteResult {
                    request_id: request.request_id,
                    status: self.status,
                    body: self.body.clone(),
                },
            );
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&response)?)
                    .build(),
            ))
        }
        fn request_stream(
            &self,
            _request: Payload,
        ) -> futures_util::stream::BoxStream<'static, anyhow::Result<Payload>> {
            futures_util::stream::empty().boxed()
        }
        fn request_channel(
            &self,
            requests: Flux<anyhow::Result<Payload>>,
        ) -> Flux<anyhow::Result<Payload>> {
            requests
        }
    }

    async fn start_insights_responder(
        body: Value,
        status: u16,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use rsocket_rust_transport_websocket::WebsocketServerTransport;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = RSocketFactory::receive()
            .transport(WebsocketServerTransport::from(address.to_string()))
            .acceptor(Box::new(move |_setup, _socket| {
                Ok(Box::new(TestInsightsResponder {
                    body: body.clone(),
                    status,
                }))
            }))
            .serve();
        let task = tokio::spawn(async move {
            let _ = server.await;
        });
        tokio::time::sleep(Duration::from_millis(20)).await;
        (format!("ws://{address}"), task)
    }

    fn article(asset: &str, direction: &str, score: f64, received_at_ms: u64) -> NewsArticle {
        NewsArticle {
            title: format!("{asset} market news"),
            url: "https://example.test/news".into(),
            published_at: "today".into(),
            received_at_ms,
            matched_asset: asset.into(),
            direction: direction.into(),
            score,
            signal_reason: "unit test".into(),
            sentiment_model: "test".into(),
            sentiment_confidence: 0.9,
            matched_keywords: vec!["market".into()],
        }
    }

    fn snapshot(symbol: &str, score: f64) -> SignalSnapshot {
        SignalSnapshot {
            symbol: symbol.into(),
            timestamp: 2_000_000,
            direction: "neutral".into(),
            score,
            strength: score.abs(),
            components: SignalComponents::default(),
            reasons: Vec::new(),
            news_attribution: None,
            stale: false,
            ranging_probability: None,
            range_lower_bound: None,
            range_upper_bound: None,
            evidence_coverage: Some(1.0),
            estimated_accuracy: None,
        }
    }

    #[test]
    fn apply_news_filters_by_asset_age_and_direction() {
        let context = NewsContext {
            news: NewsSnapshot {
                source: "test".into(),
                articles: vec![
                    article("BTC", "bullish", 1.0, 2_000_000),
                    article("ETH", "bearish", -1.0, 2_000_000),
                    article("BTC", "neutral", 0.0, 2_000_000),
                    article("BTC", "bearish", -1.0, 0),
                ],
            },
            weight: 0.2,
        };
        let mut signal = snapshot("BTC-USDT", 0.5);

        apply_news(&mut signal, &context, 2_000_000);

        assert!((signal.score - 0.6).abs() < 1e-12);
        assert!((signal.strength - 0.6).abs() < 1e-12);
        assert_eq!(signal.direction, "bullish");
        let attribution = signal.news_attribution.unwrap();
        assert_eq!(attribution.article_count, 1);
        assert_eq!(attribution.articles[0].matched_asset, "BTC");
        assert!(
            signal
                .reasons
                .iter()
                .any(|reason| reason.contains("news_sentiment"))
        );
    }

    #[test]
    fn apply_news_caps_weight_and_leaves_unmatched_signals_unchanged() {
        let context = NewsContext {
            news: NewsSnapshot {
                source: "test".into(),
                articles: vec![article("CRYPTO", "bullish", 1.0, 2_000_000)],
            },
            weight: 4.0,
        };
        let mut signal = snapshot("ETH-USDT", 0.5);

        apply_news(&mut signal, &context, 2_000_000);
        assert!((signal.score - 0.6).abs() < 1e-12);
        assert!((signal.strength - 0.6).abs() < 1e-12);
        assert_eq!(signal.news_attribution.unwrap().weight, 0.2);

        let mut unmatched = snapshot("ETH-USDT", 0.5);
        let unrelated = NewsContext {
            news: NewsSnapshot {
                source: "test".into(),
                articles: vec![article("BTC", "bullish", 1.0, 2_000_000)],
            },
            ..context
        };
        apply_news(&mut unmatched, &unrelated, 2_000_000);
        assert_eq!(unmatched.score, 0.5);
        assert_eq!(unmatched.strength, 0.5);
        assert!(unmatched.news_attribution.is_none());
    }

    #[tokio::test]
    async fn fetch_research_context_decodes_the_service_contract() {
        let (url, server) = start_insights_responder(
            json!({
                "news": {"source": "test", "articles": []},
                "weight": 0.15,
                "walkForward": [{"symbol": "BTC-USDT"}],
                "missedOpportunityBands": [{"gate": "score"}]
            }),
            200,
        )
        .await;
        let context = fetch_research_context(&url).await.unwrap();

        assert_eq!(context.weight, 0.15);
        assert_eq!(context.news.source, "test");
        server.abort();
    }

    #[tokio::test]
    async fn fetch_research_context_rejects_server_errors() {
        let (url, server) = start_insights_responder(Value::Null, 503).await;
        assert!(fetch_research_context(&url).await.is_err());
        server.abort();
    }

    #[tokio::test]
    async fn refresh_research_context_updates_watch_state() {
        let (url, server) = start_insights_responder(
            json!({
                "news": {"source": "test", "articles": []},
                "weight": 0.1
            }),
            200,
        )
        .await;
        let (sender, mut receiver) = watch::channel(Arc::new(NewsContext::default()));

        refresh_research_context_once(&sender, &url).await.unwrap();

        assert!(receiver.has_changed().unwrap());
        assert!((receiver.borrow_and_update().weight - 0.1).abs() < 1e-12);
        server.abort();
    }
}
