impl PositionStore {
    fn ensure_subscription(self: &Arc<Self>, symbol: &str) {
        let mut subscriptions = self
            .subscriptions
            .lock()
            .expect("subscription lock poisoned");
        if subscriptions.contains_key(symbol) {
            return;
        }
        let (sender, mut receiver) = mpsc::channel(64);
        subscriptions.insert(symbol.to_owned(), sender.clone());
        let store = Arc::clone(self);
        let symbol = symbol.to_owned();
        let subscription_symbol = symbol.clone();
        let log_symbol = symbol.clone();
        let book_slot = self.book_slot(&symbol);
        tokio::spawn(async move {
            crate::service_log!(
                "INFO",
                "Paper order manager subscribing to OKX books for {symbol}"
            );
            while let Some(result) = receiver.recv().await {
                match result {
                    Ok(bytes) => {
                        if let Some(book) = decode_book(&bytes) {
                            let mid = match (book.bids.first(), book.asks.first()) {
                                (Some(bid), Some(ask)) => Some((bid.price + ask.price) / 2.0),
                                _ => None,
                            };
                            book_slot.store(Some(Arc::new(book)));
                            if let (Some(mid), Ok(now)) = (mid, now_ms()) {
                                let mut histories = store.price_samples.lock().await;
                                let samples = histories.entry(symbol.clone()).or_default();
                                if samples
                                    .last()
                                    .map_or(true, |(time, _)| now.saturating_sub(*time) >= 10_000)
                                {
                                    samples.push((now, mid));
                                    samples.retain(|(time, _)| {
                                        now.saturating_sub(*time) <= 30 * 60_000
                                    });
                                }
                            }
                            store.book_changed.notify_waiters();
                            if let Err(error) = store.match_resting(&symbol).await {
                                crate::service_log!(
                                    "ERROR",
                                    "Unable to match resting orders for {symbol}: {error:#}"
                                );
                            }
                        }
                    }
                    Err(error) => {
                        crate::service_log!(
                            "ERROR",
                            "Paper order book stream for {symbol} failed: {error:#}"
                        )
                    }
                }
            }
        });
        tokio::spawn(async move {
            loop {
                if let Err(error) = async {
                    let mut stream =
                        crate::feed_client::stream(format!("book:{subscription_symbol}")).await?;
                    while let Some(item) = stream.next().await {
                        sender
                            .send(item)
                            .await
                            .map_err(|_| anyhow::anyhow!("book consumer stopped"))?;
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .await
                {
                    crate::service_log!("ERROR", 
                        "Paper OKX book subscription for {log_symbol} disconnected: {error:#}; retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
    }
    async fn match_resting(&self, symbol: &str) -> anyhow::Result<()> {
        let now = now_ms()?;
        {
            let mut ledger = self.ledger.lock().await;
            let old_len = ledger.orders.len();
            ledger
                .orders
                .retain(|order| order.expires_at.is_none_or(|expires| expires > now));
            if ledger.orders.len() != old_len {
                self.persist(&ledger).await?;
            }
        }
        let Some(book) = self.book(symbol) else {
            return Ok(());
        };
        {
            let mut ledger = self.ledger.lock().await;
            let old_len = ledger.orders.len();
            ledger.orders.retain(|order| {
                if !order.post_only || order.symbol != symbol {
                    return true;
                }
                match order.side {
                    TradeSide::Buy => !book
                        .asks
                        .first()
                        .is_some_and(|ask| order.limit_price >= ask.price),
                    TradeSide::Sell => !book
                        .bids
                        .first()
                        .is_some_and(|bid| order.limit_price <= bid.price),
                }
            });
            if ledger.orders.len() != old_len {
                self.persist(&ledger).await?;
            }
        }
        let orders = self
            .ledger
            .lock()
            .await
            .orders
            .iter()
            .filter(|order| order.symbol == symbol)
            .cloned()
            .collect::<Vec<_>>();
        if orders.is_empty() {
            return Ok(());
        }
        let matches = orders
            .into_iter()
            .map(|order| {
                let response = match_book(
                    &order.side,
                    order.remaining_quantity,
                    Some(order.limit_price),
                    &book,
                );
                (order.id, response)
            })
            .collect::<Vec<_>>();
        let mut ledger = self.ledger.lock().await;
        let mut fills = Vec::new();
        for (id, result) in matches {
            if let Some(order) = ledger.orders.iter_mut().find(|order| order.id == id) {
                order.remaining_quantity = result.remaining_quantity.max(0.0);
                order.estimated_fee =
                    order.remaining_quantity * order.limit_price * -order.maker_fee_rate;
                let fee_rate = if order.maker_fee_rate == 0.0 {
                    configured_fee_rates().maker
                } else {
                    order.maker_fee_rate
                };
                fills.extend(result.fills.into_iter().map(|fill| {
                    (
                        order.id.clone(),
                        order.side.clone(),
                        fill.quantity,
                        fill.price,
                        fee_rate,
                        order.source.clone(),
                        order.parent_order_id.clone(),
                    )
                }));
            }
        }
        ledger
            .orders
            .retain(|order| order.remaining_quantity > EPSILON);
        if !fills.is_empty() {
            let timestamp = now_ms()?;
            let offset = ledger.trades.len();
            for (
                index,
                (order_id, side, quantity, execution_price, fee_rate, source, parent_order_id),
            ) in fills.into_iter().enumerate()
            {
                ledger.trades.push(PositionTrade {
                    id: format!("{timestamp}-{index}-{offset}"),
                    symbol: symbol.to_owned(),
                    side,
                    quantity,
                    execution_price,
                    reference_price: None,
                    timestamp,
                    source,
                    parent_order_id,
                    fee: -quantity * execution_price * fee_rate.abs(),
                    fee_currency: symbol
                        .split_once('-')
                        .map(|(_, quote)| quote.to_owned())
                        .unwrap_or_default(),
                    fee_rate,
                    fee_liquidity: "maker".to_owned(),
                    resting_order_id: Some(order_id),
                    decision_timestamp: None,
                    decision_price: None,
                    decision_mid_price: None,
                    slippage: None,
                    slippage_bps: None,
                });
            }
            self.persist(&ledger).await?;
        }
        Ok(())
    }
}

