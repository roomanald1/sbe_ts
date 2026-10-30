impl SignalStore {
    pub fn router(self) -> Router {
        let dashboard_store = self.clone();
        let history_store = self.clone();
        let performance_store = self.clone();
        let evaluation_store = self.clone();
        let raw_evaluation_store = self.clone();
        let book_snapshot_store = self.clone();
        let research_feature_store = self.clone();
        let external_market_store = self.clone();
        let auto_trader_store = self.clone();
        let audit_store = self.clone();
        let execution_audit_store = self.clone();
        let ui_stream_store = self.clone();
        let best_store = self;
        Router::new()
            .route("/signals/stream", get(move |websocket: axum::extract::ws::WebSocketUpgrade| {
                let store = ui_stream_store.clone();
                async move {
                    websocket.on_upgrade(move |socket| async move {
                        use axum::extract::ws::Message;
                        use futures_util::{SinkExt, StreamExt};
                        let (mut sender, mut receiver) = socket.split();
                        let mut refresh = tokio::time::interval(std::time::Duration::from_secs(15));
                        refresh.tick().await;
                        let mut previous = signal_ui_snapshot(&store).await;
                        let initial = serde_json::json!({ "type": "snapshot", "state": previous }).to_string();
                        if sender.send(Message::Text(initial.into())).await.is_err() { return; }
                        loop {
                            let should_refresh = tokio::select! {
                                _ = refresh.tick() => true,
                                incoming = receiver.next() => if incoming.is_none() { break } else { false },
                            };
                            if should_refresh {
                                let state = signal_ui_snapshot(&store).await;
                                let mut changes = serde_json::Map::new();
                                if let (Some(current), Some(old)) = (state.as_object(), previous.as_object()) {
                                    for (key, value) in current {
                                        if old.get(key) != Some(value) { changes.insert(key.clone(), value.clone()); }
                                    }
                                }
                                previous = state;
                                if !changes.is_empty() {
                                    let payload = serde_json::json!({ "type": "delta", "changes": changes }).to_string();
                                    if sender.send(Message::Text(payload.into())).await.is_err() { break; }
                                }
                            }
                        }
                    })
                }
            }))
            .route("/positions/insights", get(get_position_insights))
            .route(
                "/signals/dashboard",
                get(move || {
                    let store = dashboard_store.clone();
                    async move {
                        match store.dashboard().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/history",
                get(move || {
                    let store = history_store.clone();
                    async move {
                        match store.recent(200).await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/performance",
                get(move || {
                    let store = performance_store.clone();
                    async move {
                        match store.performance().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-dashboard",
                get(move || {
                    let store = auto_trader_store.clone();
                    async move {
                        match store.auto_trader_dashboard().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-decision-quality",
                get({
                    let store = audit_store.clone();
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = store.clone();
                        async move {
                            match store.auto_trader_decision_quality(query.since).await {
                                Ok(summary) => Ok(Json(summary)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-execution-quality",
                get({
                    let store = audit_store.clone();
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = store.clone();
                        async move {
                            match store.auto_trader_execution_quality(query.since).await {
                                Ok(summary) => Ok(Json(summary)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-execution-audit",
                get(move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                    let store = execution_audit_store.clone();
                    async move {
                        match store
                            .load_auto_trader_execution_audit_page(
                                query.cursor,
                                query.since,
                                query.limit.unwrap_or(500),
                            )
                            .await
                        {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/auto-trader-audit",
                get({
                    let store = audit_store.clone();
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = store.clone();
                        async move {
                            match store
                                .load_auto_trader_audit(query.since, query.limit.unwrap_or(10_000))
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    }
                })
                .post({
                    let store = audit_store.clone();
                    move |Json(payload): Json<serde_json::Value>| {
                        let store = store.clone();
                        async move {
                            match store.save_auto_trader_audit(payload).await {
                                Ok(()) => Ok(StatusCode::NO_CONTENT),
                                Err(error) => Err((StatusCode::BAD_REQUEST, error.to_string())),
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/best",
                get(move || {
                    let store = best_store.clone();
                    async move {
                        match store.best(10).await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/walk-forward",
                get(move || {
                    let store = evaluation_store.clone();
                    async move {
                        match store.walk_forward().await {
                            Ok(items) => Ok(Json(items)),
                            Err(error) => {
                                Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                            }
                        }
                    }
                }),
            )
            .route(
                "/signals/evaluations",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = raw_evaluation_store.clone();
                        async move {
                            match store
                                .evaluations(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/book-snapshots",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = book_snapshot_store.clone();
                        async move {
                            match store
                                .book_snapshots(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/research-features",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = research_feature_store.clone();
                        async move {
                            match store
                                .research_features(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
            .route(
                "/signals/external-market",
                get(
                    move |axum::extract::Query(query): axum::extract::Query<EvaluationQuery>| {
                        let store = external_market_store.clone();
                        async move {
                            match store
                                .external_market_features(
                                    query.since,
                                    query.limit.unwrap_or(10_000).clamp(1, 50_000),
                                )
                                .await
                            {
                                Ok(items) => Ok(Json(items)),
                                Err(error) => {
                                    Err((StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
                                }
                            }
                        }
                    },
                ),
            )
    }
}

async fn signal_ui_snapshot(store: &SignalStore) -> serde_json::Value {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64;
    let since = Some(now / 86_400_000 * 86_400_000);
    let signal_dashboard = store.dashboard().await.ok();
    let auto_trader_dashboard = store.auto_trader_dashboard().await.ok();
    let execution_audit = store.load_auto_trader_execution_audit_page(None, None, 500).await.ok();
    let execution_quality = store.auto_trader_execution_quality(since).await.ok();
    let decision_quality = store.auto_trader_decision_quality(since).await.ok();
    let close_insights = get_position_insights().await.ok().map(|Json(response)| response);
    serde_json::json!({
        "signal_dashboard": signal_dashboard,
        "auto_trader_dashboard": auto_trader_dashboard,
        "execution_audit": execution_audit,
        "execution_quality": execution_quality,
        "decision_quality": decision_quality,
        "close_insights": close_insights,
    })
}
