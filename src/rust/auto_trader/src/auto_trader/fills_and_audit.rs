fn tag_position_strategy(
    positions: &mut BTreeMap<String, TraderPosition>,
    symbol: &str,
    strategy: Option<&str>,
) {
    let Some(strategy) = strategy.filter(|strategy| *strategy != "generic-insight-signals") else {
        return;
    };
    if let Some(position) = positions.get_mut(symbol).filter(|position| position.quantity.abs() > 1e-12) {
        position.research_strategy = Some(strategy.to_owned());
    }
}

fn strategy_from_entry_context(context: &str) -> Option<String> {
    [
        "donchian-55-btc200",
        "donchian-20-sma200",
        "trend-sleeves-btc200",
        "trend-50-200",
        "relative-strength-90-sma200",
        "rsi2-oversold-rebound",
    ]
    .into_iter()
    .find(|strategy| context.contains(strategy))
    .map(str::to_owned)
}

fn apply_auto_fill(
    positions: &mut BTreeMap<String, TraderPosition>,
    symbol: &str,
    side: &str,
    quantity: f64,
    price: f64,
    fee: f64,
    order_type: &str,
    context: &str,
    count_entry: bool,
    horizon_take_profit_percent: Option<f64>,
) -> (f64, Option<String>) {
    let position = positions.entry(symbol.to_owned()).or_default();
    let old = position.quantity;
    let incoming = if side == "buy" { quantity } else { -quantity };
    if old == 0.0 || old.signum() == incoming.signum() {
        if old == 0.0 {
            position.take_profit_deferred_at_ms = None;
            position.entries = 1;
            position.entry_timestamp_ms = current_time_ms();
            position.price_path.clear();
        } else if count_entry {
            position.entries = position.entries.saturating_add(1);
        }
        let total = old.abs() + incoming.abs();
        position.average_entry_price =
            (old.abs() * position.average_entry_price + incoming.abs() * price) / total;
        position.quantity = old + incoming;
        position.entry_fees += fee;
        if let Some(target) = horizon_take_profit_percent {
            position.horizon_take_profit_percent = Some(
                position
                    .horizon_take_profit_percent
                    .map_or(target, |current| current.max(target)),
            );
        }
        if position.entry_order_type.is_empty() {
            position.entry_order_type = order_type.to_owned();
            position.entry_context = context.to_owned();
            position.scout_entry = context.starts_with("scout:");
        }
        return (0.0, None);
    }
    let entry_type = if position.entry_order_type.is_empty() {
        order_type.to_owned()
    } else {
        position.entry_order_type.clone()
    };
    let entry_context = if position.entry_context.is_empty() {
        context.to_owned()
    } else {
        position.entry_context.clone()
    };
    let closed = old.abs().min(incoming.abs());
    let entry_fee_closed = position.entry_fees * (closed / old.abs());
    let closing_fee = fee * (closed / incoming.abs());
    let pnl = closed
        * if old > 0.0 {
            price - position.average_entry_price
        } else {
            position.average_entry_price - price
        }
        - entry_fee_closed
        - closing_fee;
    position.realized_pnl += pnl;
    let net = old + incoming;
    if net.abs() < 1e-12 {
        *position = TraderPosition::default();
    } else if net.signum() != old.signum() {
        position.quantity = net;
        position.average_entry_price = price;
        position.entry_fees = fee - closing_fee;
        position.entry_order_type = order_type.to_owned();
        position.entry_context = context.to_owned();
        position.scout_entry = context.starts_with("scout:");
        position.entries = 1;
        position.horizon_take_profit_percent = horizon_take_profit_percent;
        position.take_profit_deferred_at_ms = None;
    } else {
        position.quantity = net;
        position.entry_fees -= entry_fee_closed;
    }
    (pnl, Some(format!("{entry_context}:{entry_type}")))
}

fn signal_decision_id(signal: &Signal) -> String {
    format!(
        "{}|{}|{}",
        signal.timestamp, signal.symbol, signal.direction
    )
}

fn signal_audit_payload(
    signal: &Signal,
    stage: &str,
    timestamp: u64,
    status: &str,
    reason: Option<&str>,
    exploratory: Option<bool>,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    evidence: Option<&WalkForward>,
) -> serde_json::Value {
    let evidence_metrics = evidence.map(|item| serde_json::json!({
        "horizon_seconds": item.horizon_seconds,
        "metrics": if signal.direction == "bullish" { &item.all_signal_long } else { &item.all_signal_short },
    }));
    serde_json::json!({
        "record_schema_version": 1,
        "decision_id": signal_decision_id(signal),
        "stage": stage,
        "timestamp": timestamp,
        "signal_timestamp": signal.timestamp,
        "symbol": signal.symbol,
        "direction": signal.direction,
        "score": signal.score,
        "strength": signal.strength,
        "ranging_probability": signal.ranging_probability,
        "market_context": market_context(signal),
        "research_strategy": signal.research_strategy,
        "market_regime": signal.market_regime,
        "signal_components": signal.components,
        "exploratory": exploratory,
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "walk_forward_evidence": evidence_metrics,
        "status": status,
        "reason": reason,
    })
}

fn queue_auto_trader_audit(state: &AutoTraderState, payload: serde_json::Value) {
    let is_execution = payload
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|stage| stage == "entry" || stage.starts_with("exit"));
    if is_execution {
        // Keep execution events separate so decision bursts cannot crowd them out
        // or delay position protection while the audit service is unavailable.
        if state.execution_audit_tx.send(payload).is_err() {
            crate::service_log!(
                "ERROR",
                "Auto-trader execution audit writer stopped; event could not be queued"
            );
        }
    } else if state.audit_tx.try_send(payload).is_err() {
        crate::service_log!(
            "WARN",
            "Auto-trader audit queue is full; dropping a decision event"
        );
    }
}

async fn store_auto_trader_audit(client: &reqwest::Client, payload: &serde_json::Value) -> bool {
    let base = std::env::var("INSIGHTS_URL").unwrap_or_else(|_| "http://127.0.0.1:3004".into());
    let url = format!("{base}/signals/auto-trader-audit");
    for attempt in 0..3 {
        match client
            .post(&url)
            .timeout(Duration::from_secs(2))
            .json(payload)
            .send()
            .await
        {
            Ok(response) if response.status().is_success() => return true,
            Ok(response) => {
                let status = response.status();
                if attempt == 2 {
                    crate::service_log!(
                        "ERROR",
                        "Auto-trader audit write failed after 3 attempts with status {}",
                        status
                    );
                }
            }
            Err(error) if attempt == 2 => {
                crate::service_log!(
                    "ERROR",
                    "Auto-trader audit write failed after 3 attempts: {}",
                    error
                );
            }
            Err(_) => {}
        }
        tokio::time::sleep(Duration::from_millis(150 * (attempt + 1))).await;
    }
    false
}

async fn store_execution_audit_until_saved(client: &reqwest::Client, payload: &serde_json::Value) {
    let mut retry_delay = Duration::from_secs(1);
    while !store_auto_trader_audit(client, payload).await {
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}
