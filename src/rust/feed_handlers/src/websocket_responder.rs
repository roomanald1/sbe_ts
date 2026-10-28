struct ServerResponder {
    price_updates: tokio::sync::broadcast::Sender<Bytes>,
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    signal_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_price_feeds: Arc<DashMap<String, broadcast::Sender<Bytes>>>,
    signal_events: broadcast::Sender<Bytes>,
    usd_rate_snapshot: watch::Sender<Bytes>,
    news_snapshot: watch::Sender<Bytes>,
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
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), updates)),
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
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

        if payload.data().and_then(|data| std::str::from_utf8(data.as_ref()).ok()) == Some("news:*") {
            let stream = futures_util::stream::unfold(
                (self.news_snapshot.subscribe(), true),
                |(mut snapshot, initial)| async move {
                    if !initial && snapshot.changed().await.is_err() { return None; }
                    let data = snapshot.borrow_and_update().clone();
                    Some((Ok(Payload::new(Some(data), None)), (snapshot, false)))
                },
            ).boxed();
            return log_stream_end(stream, "Backend news snapshot stream disconnected".to_owned());
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
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), events)),
                            Err(broadcast::error::RecvError::Lagged(skipped)) => {
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
                            Ok(data) => break Some((Ok(Payload::new(Some(data), None)), receiver)),
                            Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
            futures_util::stream::unfold(
                (
                    0_u64,
                    0_usize,
                    (0..100)
                        .map(|_| RollingVolatility::default())
                        .collect::<Vec<_>>(),
                ),
                |(round, symbol_index, mut volatility_by_symbol)| async move {
                    if symbol_index == 0 {
                        sleep(Duration::from_millis(10)).await;
                    }

                    let (symbol, price) = synthetic_price_update(round, symbol_index);
                    let volatility = volatility_by_symbol[symbol_index]
                        .update(round.saturating_mul(1_000), price);
                    let result = Ok(Payload::builder()
                        .set_data(encode_symbol_price_bytes(
                            &symbol,
                            price,
                            volatility.0,
                            volatility.1,
                        ))
                        .build());
                    let next_state = if symbol_index == 99 {
                        (round.wrapping_add(1), 0, volatility_by_symbol)
                    } else {
                        (round, symbol_index + 1, volatility_by_symbol)
                    };
                    Some((result, next_state))
                },
            )
            .boxed()
        } else {
            futures_util::stream::unfold(self.price_updates.subscribe(), |mut updates| async move {
                loop {
                    match updates.recv().await {
                        Ok(data) => {
                            break Some((Ok(Payload::new(Some(data), None)), updates));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
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
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Fire and Forget interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
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
        todo!()
    }

    #[doc = " Request-Channel interaction model of RSocket."]
    fn request_channel(&self, reqs: Flux<Result<Payload>>) -> Flux<Result<Payload>> {
        todo!()
    }
}

