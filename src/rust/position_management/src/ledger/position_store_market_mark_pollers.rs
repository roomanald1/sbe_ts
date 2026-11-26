const MAX_MARKET_MARKS: usize = 256;

impl PositionStore {
    fn ensure_market_mark_polling(self: &Arc<Self>, symbol: &str) {
        let symbol = symbol.to_ascii_uppercase();
        let should_start = {
            let mut market_mark_pollers = self
                .market_mark_pollers
                .lock()
                .expect("market mark poller lock poisoned");
            if market_mark_pollers.contains(&symbol) {
                false
            } else if market_mark_pollers.len() >= MAX_MARKET_MARKS {
                crate::service_log!(
                    "WARN",
                    "Market mark cache is full; not subscribing to {symbol}"
                );
                false
            } else {
                market_mark_pollers.insert(symbol.clone())
            }
        };
        if !should_start {
            return;
        }
        let store = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let path = format!("/internal/market-mark?symbol={symbol}");
                match paper_exchange_client::paper_exchange_get(&path).await {
                    Ok(value) => {
                        if let Ok(mark) = serde_json::from_value::<MarketMark>(value) {
                            let is_new = store.market_mark(&symbol).is_none_or(|previous| {
                                previous.updated_at_ms != mark.updated_at_ms
                            });
                            store
                                .market_marks
                                .write()
                                .expect("market mark map poisoned")
                                .insert(symbol.clone(), mark);
                            if is_new {
                                if let Err(error) = store.match_resting(&symbol).await {
                                    crate::service_log!(
                                        "ERROR",
                                        "Unable to process resting orders for {symbol}: {error:#}"
                                    );
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
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
        // Post-only is checked when accepting the order. Once resting, a
        // later crossing book is a fill opportunity, not a reason to cancel.
        let orders = {
            let ledger = self.ledger.lock().await;
            ledger
                .orders
                .iter()
                .filter(|order| order.symbol == symbol)
                .cloned()
                .collect::<Vec<_>>()
        };
        if orders.is_empty() {
            return Ok(());
        }
        let mut matches = Vec::with_capacity(orders.len());
        let mut stale_reduce_only_orders = Vec::new();
        for order in orders {
            let match_quantity = if order.reduce_only {
                let ledger = self.ledger.lock().await;
                let current_quantity = indexed_position_summary(self, &ledger, Some(&order.source))
                    .positions
                    .into_iter()
                    .find(|position| position.symbol == symbol)
                    .map(|position| position.quantity)
                    .unwrap_or(0.0);
                let closes = (matches!(&order.side, TradeSide::Sell) && current_quantity > EPSILON)
                    || (matches!(&order.side, TradeSide::Buy) && current_quantity < -EPSILON);
                if !closes {
                    0.0
                } else {
                    order.remaining_quantity.min(current_quantity.abs())
                }
            } else {
                order.remaining_quantity
            };
            if match_quantity <= EPSILON {
                if order.reduce_only {
                    stale_reduce_only_orders.push(order.id);
                }
                continue;
            }
            let book_timestamp = self
                .market_mark(&order.symbol)
                .map(|mark| mark.updated_at_ms)
                .unwrap_or(now);
            let attempt_order_id = format!("{}:resting:{book_timestamp}", order.id);
            let parent_order_id = order
                .parent_order_id
                .as_deref()
                .or(order.client_order_id.as_deref());
            let mut response = paper_exchange_match(
                &order.symbol,
                &order.side,
                match_quantity,
                Some(order.limit_price),
                Some(&attempt_order_id),
                parent_order_id,
                "resting_limit",
                Some(order.limit_price),
            )
            .await
            .map_err(|(status, message)| {
                anyhow::anyhow!("paper exchange returned {status}: {message}")
            })?;
            response.source_reported_filled_quantity = Some(response.filled_quantity);
            for fill in &mut response.fills {
                fill.source_quantity = Some(fill.quantity);
                fill.source_price = Some(fill.price);
            }
            // This order was already resting: its maker fills execute at
            // its own limit, even if the next observed book gaps through it.
            for fill in &mut response.fills {
                fill.price = order.limit_price;
            }
            response.average_execution_price =
                (response.filled_quantity > EPSILON).then_some(order.limit_price);
            matches.push((order.id, match_quantity, response));
        }
        let mut ledger = self.ledger.lock().await;
        let original_order_count = ledger.orders.len();
        for order in &mut ledger.orders {
            if stale_reduce_only_orders.iter().any(|id| id == &order.id) {
                order.remaining_quantity = 0.0;
            }
        }
        let mut fills = Vec::new();
        for (id, requested_quantity, result) in matches {
            let reduce_allowance = ledger
                .orders
                .iter()
                .find(|order| order.id == id)
                .filter(|order| order.reduce_only)
                .map(|order| {
                    let current_quantity =
                        indexed_position_summary(self, &ledger, Some(&order.source))
                            .positions
                            .into_iter()
                            .find(|position| position.symbol == symbol)
                            .map(|position| position.quantity)
                            .unwrap_or(0.0);
                    let closes = (matches!(&order.side, TradeSide::Sell)
                        && current_quantity > EPSILON)
                        || (matches!(&order.side, TradeSide::Buy) && current_quantity < -EPSILON);
                    if closes { current_quantity.abs() } else { 0.0 }
                });
            if let Some(order) = ledger.orders.iter_mut().find(|order| order.id == id) {
                let mut result = result;
                if let Some(allowance) = reduce_allowance {
                    limit_match_quantity(&mut result, allowance.min(order.remaining_quantity));
                    if allowance <= EPSILON {
                        order.remaining_quantity = 0.0;
                    }
                }
                let prior_remaining = order.remaining_quantity;
                order.remaining_quantity = (prior_remaining - result.filled_quantity).max(0.0);
                order.estimated_fee =
                    order.remaining_quantity * order.limit_price * -order.maker_fee_rate;
                let fee_rate = if order.maker_fee_rate == 0.0 {
                    configured_fee_rates().maker
                } else {
                    order.maker_fee_rate
                };
                let source_evidence = paper_exchange_fill_evidence(
                    &result,
                    result.order_id.as_deref(),
                    requested_quantity,
                );
                fills.extend(
                    result
                        .fills
                        .into_iter()
                        .zip(source_evidence)
                        .map(|(fill, evidence)| {
                            (
                                order.id.clone(),
                                order.side.clone(),
                                fill.quantity,
                                fill.price,
                                fee_rate,
                                order.source.clone(),
                                order.parent_order_id.clone(),
                                order.client_order_id.clone(),
                                order.lifecycle.clone(),
                                evidence,
                            )
                        }),
                );
            }
        }
        ledger
            .orders
            .retain(|order| order.remaining_quantity > EPSILON);
        if !fills.is_empty() {
            let timestamp = now_ms()?;
            let offset = ledger.trades.len();
            let mut canonical_lifecycle_fill = false;
            for (
                index,
                (
                    order_id,
                    side,
                    quantity,
                    execution_price,
                    fee_rate,
                    source,
                    parent_order_id,
                    client_order_id,
                    lifecycle,
                    source_evidence,
                ),
            ) in fills.into_iter().enumerate()
            {
                canonical_lifecycle_fill |= lifecycle.is_some();
                let fill_id = allocate_fill_id(&mut ledger, timestamp);
                ledger.trades.push(PositionTrade {
                    id: format!("{timestamp}-{index}-{offset}"),
                    fill_id: Some(fill_id),
                    paper_exchange_schema_version: source_evidence.schema_version,
                    paper_exchange_source_epoch: source_evidence.source_epoch,
                    paper_exchange_source_cursor: source_evidence.source_cursor,
                    paper_exchange_request_fingerprint: source_evidence.request_fingerprint,
                    paper_exchange_execution_id: source_evidence.execution_id,
                    paper_exchange_source_quantity: source_evidence.source_quantity,
                    paper_exchange_source_price: source_evidence.source_price,
                    paper_exchange_source_complete: source_evidence.complete,
                    symbol: symbol.to_owned(),
                    side,
                    quantity,
                    execution_price,
                    reference_price: None,
                    timestamp,
                    source,
                    parent_order_id,
                    client_order_id,
                    lifecycle,
                    fee: -quantity * execution_price * fee_rate.abs(),
                    fee_known: true,
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
            if canonical_lifecycle_fill {
                // Do not let a maker fill advance its order state while its
                // canonical ledger row is still only queued for persistence.
                drop(ledger);
                self.persist_durable_snapshot(None).await?;
                return Ok(());
            }
            self.persist(&ledger).await?;
        } else if ledger.orders.len() != original_order_count {
            self.persist(&ledger).await?;
        }
        Ok(())
    }
}
