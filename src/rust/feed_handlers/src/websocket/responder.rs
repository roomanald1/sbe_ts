struct ServerResponder {
    price_updates: tokio::sync::broadcast::Sender<Bytes>,
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    signal_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_price_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_events: broadcast::Sender<Bytes>,
    usd_rate_snapshot: watch::Sender<Bytes>,
}

impl RSocket for ServerResponder {
    fn request_stream(
        &self,
        payload: Payload,
    ) -> Pin<
        Box<
            dyn Stream<Item = Result<rsocket_rust::prelude::Payload, anyhow::Error>>
                + Send
                + 'static,
        >,
    > {
        if payload
            .data()
            .and_then(|data| std::str::from_utf8(data.as_ref()).ok())
            == Some("tickers:*")
        {
            let stream = futures_util::stream::unfold(
                self.price_updates.subscribe(),
                |mut updates| async move {
                    loop {
                        match updates.recv().await {
                            Ok(data) => {
                                crate::metrics::record_queue_depth("ticker_batches", updates.len());
                                break Some((Ok(Payload::new(Some(data), None)), updates));
                            },
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                crate::metrics::record_queue_lagged("ticker_batches", skipped);
                                crate::service_log!("WARN", "RSocket ticker subscriber lagged; skipped {skipped} updates");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                },
            )
            .boxed();
            return log_stream_end(
                stream,
                "FeedHandlers ticker API client disconnected".to_owned(),
            );
        }

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("usd-rates:*") {
            let stream = futures_util::stream::unfold(
                (self.usd_rate_snapshot.subscribe(), true),
                |(mut snapshot, initial)| async move {
                    if !initial && snapshot.changed().await.is_err() { return None; }
                    let data = snapshot.borrow_and_update().clone();
                    Some((Ok(Payload::new(Some(data), None)), (snapshot, false)))
                },
            ).boxed();
            return log_stream_end(stream, "FeedHandlers USD rate stream disconnected".to_owned());
        }

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("signal-events:*") {
            let stream = futures_util::stream::unfold(
                self.signal_events.subscribe(),
                |mut events| async move {
                    loop {
                        match events.recv().await {
                            Ok(data) => {
                                crate::metrics::record_queue_depth("signal_events", events.len());
                                break Some((Ok(Payload::new(Some(data), None)), events));
                            },
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                crate::metrics::record_queue_lagged("signal_events", skipped);
                                crate::service_log!("WARN", "Insights signal-event subscriber lagged; skipped {skipped} events");
                                continue;
                            }
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                },
            ).boxed();
            return log_stream_end(stream, "Insights signal-event stream disconnected".to_owned());
        }

        if let Some(instrument) = requested_signal_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid signal instrument request"))
                }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let missing = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move {
                    Err(anyhow::anyhow!("instrument is unavailable: {missing}"))
                }));
            };
            let instrument = instrument.to_string();
            let sender = start_signal_feed(&instrument, &self.signal_feeds, &self.signal_price_feeds, &self.signal_events);
            let stream =
                futures_util::stream::unfold(sender.subscribe(), |mut receiver| async move {
                    loop {
                        match receiver.recv().await {
                            Ok(data) => {
                                crate::metrics::record_queue_depth("signal_feeds", receiver.len());
                                break Some((Ok(Payload::new(Some(data), None)), receiver));
                            },
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                                crate::metrics::record_queue_lagged("signal_feeds", skipped);
                                continue;
                            },
                            Err(broadcast::error::RecvError::Closed) => break None,
                        }
                    }
                })
                .boxed();
            return log_stream_end(
                stream,
                format!("RSocket client disconnected from signal stream for {instrument}"),
            );
        }

        if let Some(instrument) = requested_trade_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid OKX trades instrument request"))
                }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let instrument = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move {
                    Err(anyhow::anyhow!(
                        "instrument is not available in the current OKX subscription: {instrument}"
                    ))
                }));
            };
            crate::service_log!("INFO", "Subscribing to OKX trades for {instrument}");
            return log_stream_end(
                trade_request_stream(instrument.to_string()),
                format!(
                    "RSocket UI disconnected or cancelled the OKX trades stream for {instrument}"
                ),
            );
        }

        if let Some(instrument) = requested_book_instrument(&payload) {
            if !instrument.is_ascii() || instrument.is_empty() || instrument.len() > SYMBOL_LENGTH {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("invalid OKX book instrument request"))
                }));
            }
            let Some(instrument) = self.instrument_lookup.get(instrument) else {
                let instrument = instrument.to_owned();
                return Box::pin(futures_util::stream::once(async move {
                    Err(anyhow::anyhow!(
                        "instrument is not available in the current OKX subscription: {instrument}"
                    ))
                }));
            };
            crate::service_log!("INFO", "Subscribing to OKX books for {instrument}");
            return log_stream_end(
                book_request_stream(instrument.to_string()),
                format!(
                    "RSocket UI disconnected or cancelled the OKX book stream for {instrument}"
                ),
            );
        }

        let synthetic = is_synthetic_request(&payload);
        crate::service_log!("INFO", 
            "RSocket request stream source: {}",
            if synthetic { "synthetic" } else { "OKX" }
        );
        let stream = if synthetic {
            futures_util::stream::unfold((0_u64, 0_usize), |(round, symbol_index)| async move {
                    if symbol_index == 0 {
                        sleep(Duration::from_millis(10)).await;
                    }

                    let (symbol, price) = synthetic_price_update(round, symbol_index);
                    let result = Ok(Payload::builder()
                        .set_data(encode_symbol_price_bytes(
                            &symbol,
                            price,
                            round.saturating_mul(1_000),
                        ))
                        .build());
                    let next_state = if symbol_index == 99 {
                        (round.wrapping_add(1), 0)
                    } else {
                        (round, symbol_index + 1)
                    };
                    Some((result, next_state))
            })
            .boxed()
        } else {
            futures_util::stream::unfold(self.price_updates.subscribe(), |mut updates| async move {
                loop {
                    match updates.recv().await {
                        Ok(data) => {
                            crate::metrics::record_queue_depth("price_updates", updates.len());
                            break Some((Ok(Payload::new(Some(data), None)), updates));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            crate::metrics::record_queue_lagged("price_updates", skipped);
                            crate::service_log!("ERROR", 
                                "RSocket price stream recovered after skipping {skipped} updates"
                            );
                        }
                        Err(broadcast::error::RecvError::Closed) => break None,
                    }
                }
            })
            .boxed()
        };

        log_stream_end(
            stream,
            "RSocket UI disconnected or cancelled the market price stream".to_owned(),
        )
    }

    #[doc = " Metadata-Push interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        _req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket metadata push is unsupported")) })
    }

    #[doc = " Fire and Forget interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        _req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket fire-and-forget is unsupported")) })
    }

    #[doc = " Request-Response interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<Option<Payload>>>
                + ::core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            if let Some(data) = req.data() {
                if let Ok(strategy::ScoutCoordinatorRequest::WorkerRoute(command)) =
                    serde_json::from_slice::<strategy::ScoutCoordinatorRequest>(data)
                {
                    if command.path != "/internal/signals/snapshot"
                        || command.method != strategy::ScoutWorkerHttpMethod::Post
                    {
                        anyhow::bail!("unsupported FeedHandlers RSocket route: {}", command.path);
                    }
                    let symbol = command.payload["symbol"]
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("signal snapshot is missing symbol"))?;
                    let bytes = serde_json::to_vec(&command.payload)?;
                    let status = if crate::websocket::publish_signal_snapshot(
                        symbol,
                        Bytes::from(bytes),
                    ) {
                        202
                    } else {
                        404
                    };
                    let result = strategy::ScoutCoordinatorResponse::WorkerRoute(
                        strategy::ScoutCoordinatorRouteResult {
                            request_id: command.request_id,
                            status,
                            body: serde_json::Value::Null,
                        },
                    );
                    return Ok(Some(
                        Payload::builder()
                            .set_data(serde_json::to_vec(&result)?)
                            .build(),
                    ));
                }
            }
            let route = req
                .data()
                .and_then(|data| std::str::from_utf8(data.as_ref()).ok())
                .ok_or_else(|| anyhow::anyhow!("invalid FeedHandlers request-response route"))?;
            if !is_instrument_list_request(&req) {
                anyhow::bail!("unsupported FeedHandlers request-response route: {route}");
            }
            let symbols = match get_all_spot_instruments().await {
                Ok(symbols) => symbols,
                Err(error) => {
                    crate::service_log!("ERROR", "Unable to load full spot instrument list: {error:#}; using ticker symbols");
                    get_instrument_data().await.unwrap_or_default()
                }
            };
            let instruments = crate::okx::connectivity::get_spot_instrument_precisions()
                .await
                .unwrap_or_default();
            let data = serde_json::to_vec(&serde_json::json!({"symbols": symbols, "instruments": instruments}))?;
            Ok(Some(Payload::new(Some(Bytes::from(data)), None)))
        })
    }

    #[doc = " Request-Channel interaction model of RSocket."]
    fn request_channel(&self, _reqs: Flux<Result<Payload>>) -> Flux<Result<Payload>> {
        futures_util::stream::once(async {
            Err(anyhow::anyhow!("RSocket request-channel is unsupported"))
        })
        .boxed()
    }
}

fn is_instrument_list_request(payload: &Payload) -> bool {
    payload
        .data()
        .and_then(|data| std::str::from_utf8(data.as_ref()).ok())
        == Some("instruments:list")
}

