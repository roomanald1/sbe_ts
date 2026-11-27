fn tag_position_strategy(
    positions: &mut BTreeMap<String, TraderPosition>,
    symbol: &str,
    strategy: Option<&str>,
) {
    let Some(strategy) = strategy.filter(|strategy| *strategy != "generic-insight-signals") else {
        return;
    };
    if let Some(position) = positions
        .get_mut(symbol)
        .filter(|position| position.quantity.abs() > 1e-12)
    {
        position.strategy = Some(strategy.to_owned());
    }
}

fn strategy_from_entry_context(context: &str) -> Option<String> {
    [
        "donchian-55-btc200",
        "donchian-20-sma200",
        "donchian-20-btc200",
        "donchian-10-btc200",
        "donchian-20-btc200-hold3",
        "risk-adjusted-trend-portfolio",
        "trend-sleeves-btc200",
        "trend-50-200",
        "relative-strength-90-sma200",
        "risk-adjusted-monthly-momentum",
        "rsi2-oversold-rebound",
    ]
    .into_iter()
    .find(|strategy| context.contains(strategy))
    .map(str::to_owned)
}

fn append_entry_fill_evidence(
    position: &mut TraderPosition,
    response: &serde_json::Value,
    request_body: &serde_json::Value,
) {
    const MAX_ENTRY_FILL_LINKS: usize = 4096;
    let expected_order_id = request_body["client_order_id"].as_str();
    let expected_symbol = request_body["symbol"].as_str();
    let expected_side = request_body["side"].as_str();
    let Some(fills) = response["fill_evidence"]
        .as_array()
        .filter(|fills| !fills.is_empty())
    else {
        position.entry_fill_costs_complete = false;
        return;
    };
    let mut valid = expected_order_id.is_some()
        && expected_symbol.is_some()
        && matches!(expected_side, Some("buy" | "sell"))
        && response["fee_complete"].as_bool() == Some(true);
    let mut page_ids = std::collections::BTreeSet::new();
    let mut response_quantity = 0.0;
    let mut known_fee_total = 0.0;
    let mut page_fee_currencies = std::collections::BTreeSet::new();
    for fill in fills {
        let fill_id = fill["fill_id"].as_str();
        let stable_fill_id = fill_id.is_some_and(|value| {
            value
                .strip_prefix("pm-fill:v1:")
                .and_then(|rest| rest.split_once(':'))
                .is_some_and(|(epoch, sequence)| {
                    !epoch.trim().is_empty() && sequence.parse::<u64>().is_ok()
                })
        });
        let quantity = fill["quantity"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        let execution_price = fill["execution_price"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        let identity_matches = fill["parent_order_id"].as_str() == expected_order_id
            && fill["client_order_id"].as_str() == expected_order_id
            && fill["symbol"].as_str() == expected_symbol
            && fill["side"].as_str() == expected_side;
        if let Some(fill_id) = fill_id.filter(|_| stable_fill_id) {
            if !position
                .entry_fill_ids
                .iter()
                .any(|existing| existing == fill_id)
            {
                position.entry_fill_ids.push(fill_id.to_owned());
            }
        } else {
            valid = false;
        }
        let unique_in_page = fill_id.is_some_and(|fill_id| page_ids.insert(fill_id.to_owned()));
        if let Some(quantity) = quantity {
            response_quantity += quantity;
        }
        if !stable_fill_id
            || !unique_in_page
            || !identity_matches
            || quantity.is_none()
            || execution_price.is_none()
        {
            valid = false;
            continue;
        }
        let fee_amount = fill["fee"].as_f64().filter(|value| value.is_finite());
        let fee_known = fill["fee_known"].as_bool() == Some(true) && fee_amount.is_some();
        let fee_currency = fill["fee_currency"]
            .as_str()
            .filter(|currency| !currency.trim().is_empty())
            .map(str::to_owned);
        if let Some(currency) = fee_currency.as_ref() {
            page_fee_currencies.insert(currency.clone());
            if !position
                .entry_fee_currencies
                .iter()
                .any(|existing| existing == currency)
            {
                position.entry_fee_currencies.push(currency.clone());
            }
        } else {
            valid = false;
        }
        if !fee_known {
            valid = false;
        } else {
            known_fee_total += fee_amount.unwrap().abs();
        }
        let link = PositionEntryFillLink {
            fill_id: fill_id.unwrap().to_owned(),
            client_order_id: expected_order_id.unwrap().to_owned(),
            symbol: expected_symbol.unwrap().to_owned(),
            side: expected_side.unwrap().to_owned(),
            quantity: quantity.unwrap(),
            execution_price: execution_price.unwrap(),
            timestamp_ms: fill["timestamp"]
                .as_u64()
                .filter(|timestamp| *timestamp > 0),
            fee_known,
            fee_amount,
            fee_currency,
        };
        if let Some(existing) = position
            .entry_fill_links
            .iter()
            .find(|existing| existing.fill_id == link.fill_id)
        {
            if existing.client_order_id != link.client_order_id
                || existing.symbol != link.symbol
                || existing.side != link.side
                || (existing.quantity - link.quantity).abs() > 1e-12
                || (existing.execution_price - link.execution_price).abs() > 1e-9
                || existing.timestamp_ms != link.timestamp_ms
                || existing.fee_amount != link.fee_amount
                || existing.fee_currency != link.fee_currency
            {
                valid = false;
            }
        } else if position.entry_fill_links.len() < MAX_ENTRY_FILL_LINKS {
            position.entry_fill_links.push(link);
        } else {
            valid = false;
        }
    }
    let quantity_matches = response["filled_quantity"]
        .as_f64()
        .is_some_and(|reported| {
            reported.is_finite()
                && reported >= 0.0
                && (reported - response_quantity).abs()
                    <= 1e-9 * response_quantity.abs().max(reported.abs()).max(1.0)
        });
    let reported_fee = response["estimated_fee"]
        .as_f64()
        .filter(|fee| fee.is_finite())
        .map(f64::abs);
    let fee_total_matches = page_fee_currencies.len() == 1
        && reported_fee.is_some_and(|reported| {
            (reported - known_fee_total).abs()
                <= 1e-9 * known_fee_total.abs().max(reported.abs()).max(1.0)
        });
    position.entry_fill_costs_complete &= valid && quantity_matches && fee_total_matches;
}

fn update_initial_risk_reference(
    position: &mut TraderPosition,
    response: &serde_json::Value,
    request_body: &serde_json::Value,
    stop_loss_percent: f64,
) {
    const MAX_RISK_FILL_IDS: usize = 4096;

    let expected_client_order_id = request_body["client_order_id"].as_str();
    let expected_symbol = request_body["symbol"].as_str();
    let expected_side = request_body["side"].as_str();
    let quote_currency = expected_symbol
        .and_then(|symbol| symbol.split_once('-').map(|(_, quote)| quote))
        .filter(|quote| !quote.trim().is_empty());
    let existing_reference = position.initial_risk_reference.take();
    let had_reference = existing_reference.is_some();
    let mut reference = existing_reference.unwrap_or_else(|| InitialRiskReference {
        schema_version: 1,
        quote_currency: quote_currency.map(str::to_owned),
        stop_loss_percent: (stop_loss_percent.is_finite() && stop_loss_percent > 0.0)
            .then_some(stop_loss_percent),
        quote_amount: Some(0.0),
        fill_ids: Vec::new(),
        status: "incomplete".into(),
    });

    let mut valid = expected_client_order_id.is_some()
        && expected_symbol.is_some()
        && matches!(expected_side, Some("buy" | "sell"))
        && quote_currency.is_some()
        && stop_loss_percent.is_finite()
        && stop_loss_percent > 0.0
        && position
            .entry_fill_ids
            .iter()
            .all(|fill_id| reference.fill_ids.contains(fill_id));
    if had_reference && reference.status != "complete" {
        valid = false;
    }
    if reference.schema_version != 1
        || reference.quote_currency.as_deref() != quote_currency
        || reference.stop_loss_percent.is_none_or(|baseline| {
            (baseline - stop_loss_percent).abs() > 1e-9 * baseline.abs().max(1.0)
        })
    {
        valid = false;
    }
    let fills = response["fill_evidence"].as_array();
    let reported_filled = response["filled_quantity"].as_f64();
    let mut request_fill_ids = std::collections::BTreeSet::new();
    let existing_ids = reference
        .fill_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let mut response_quantity = 0.0;
    let mut response_risk = 0.0;
    let Some(fills) = fills.filter(|fills| !fills.is_empty()) else {
        valid = false;
        reference.status = "incomplete".into();
        position.initial_risk_reference = Some(reference);
        return;
    };
    for fill in fills {
        let fill_id = fill["fill_id"].as_str();
        let stable_fill_id = fill_id.is_some_and(|value| {
            value
                .strip_prefix("pm-fill:v1:")
                .and_then(|rest| rest.split_once(':'))
                .is_some_and(|(epoch, sequence)| {
                    !epoch.trim().is_empty() && sequence.parse::<u64>().is_ok()
                })
        });
        let quantity = fill["quantity"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        let execution_price = fill["execution_price"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        let identity_matches = fill["parent_order_id"].as_str() == expected_client_order_id
            && fill["client_order_id"].as_str() == expected_client_order_id
            && fill["symbol"].as_str() == expected_symbol
            && fill["side"].as_str() == expected_side;
        let unique = fill_id.is_some_and(|id| request_fill_ids.insert(id.to_owned()));
        if !stable_fill_id
            || !identity_matches
            || !unique
            || quantity.is_none()
            || execution_price.is_none()
        {
            valid = false;
            continue;
        }
        let (quantity, execution_price) = (quantity.unwrap(), execution_price.unwrap());
        response_quantity += quantity;
        if !existing_ids.contains(fill_id.unwrap()) {
            response_risk += quantity * execution_price * stop_loss_percent / 100.0;
        }
    }
    let quantity_matches = reported_filled.is_some_and(|reported| {
        reported.is_finite()
            && reported >= 0.0
            && (reported - response_quantity).abs()
                <= 1e-9 * response_quantity.abs().max(reported.abs()).max(1.0)
    });
    valid &= quantity_matches && response_risk.is_finite() && response_risk >= 0.0;

    if valid {
        let new_ids = request_fill_ids
            .into_iter()
            .filter(|fill_id| !existing_ids.contains(fill_id))
            .collect::<Vec<_>>();
        if reference.fill_ids.len().saturating_add(new_ids.len()) > MAX_RISK_FILL_IDS {
            valid = false;
        } else {
            // Replayed pages are idempotent: only risk rows not already linked
            // to this position are added to the persisted baseline.
            for fill_id in new_ids {
                reference.fill_ids.push(fill_id);
            }
            let existing_risk = reference.quote_amount.unwrap_or(0.0);
            if existing_risk.is_finite() && existing_risk >= 0.0 {
                reference.quote_amount = Some(existing_risk + response_risk);
            } else {
                valid = false;
            }
        }
    }
    reference.status = if valid { "complete" } else { "incomplete" }.into();
    position.initial_risk_reference = Some(reference);
}

fn attach_authoritative_outcome_lineage(
    mut context: strategy::TradeReplayContext,
    position: &TraderPosition,
    response: &serde_json::Value,
    identity: Option<&CanonicalLifecycleIdentity>,
    outcome_key: &str,
) -> strategy::TradeReplayContext {
    let Some(identity) = identity else {
        return context;
    };
    let exit_fills = response
        .get("fill_evidence")
        .and_then(serde_json::Value::as_array);
    let mut exit_fill_ids = Vec::new();
    let mut currencies = position
        .entry_fee_currencies
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut fees_complete = position.entry_fill_costs_complete
        && !position.entry_fill_ids.is_empty()
        && response
            .get("fee_complete")
            .and_then(serde_json::Value::as_bool)
            == Some(true);
    if let Some(fills) = exit_fills {
        if fills.is_empty() {
            fees_complete = false;
        }
        for fill in fills {
            if let Some(fill_id) = fill
                .get("fill_id")
                .and_then(serde_json::Value::as_str)
                .filter(|fill_id| !fill_id.trim().is_empty())
            {
                exit_fill_ids.push(fill_id.to_owned());
            } else {
                fees_complete = false;
            }
            if fill.get("fee_known").and_then(serde_json::Value::as_bool) != Some(true) {
                fees_complete = false;
            }
            if let Some(currency) = fill
                .get("fee_currency")
                .and_then(serde_json::Value::as_str)
                .filter(|currency| !currency.trim().is_empty())
            {
                currencies.insert(currency.to_owned());
            } else {
                fees_complete = false;
            }
        }
    } else {
        fees_complete = false;
    }
    let fee_currency = (currencies.len() == 1).then(|| currencies.into_iter().next().unwrap());
    fees_complete &= fee_currency.is_some();
    let source_event_id = position
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("live_worker_event_id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned);
    context.lineage = Some(strategy::TradeOutcomeLineage {
        outcome_event_id: format!("trade:{outcome_key}"),
        source_event_id,
        episode_id: Some(identity.episode_id.clone()),
        opportunity_id: Some(identity.opportunity_id.clone()),
        trade_id: Some(identity.trade_id.clone()),
        profile_hash: identity.profile_hash.clone(),
        profile_version: identity.profile_version,
        execution_mode: Some(identity.execution_mode.clone()),
        entry_policy: Some(identity.entry_policy.clone()),
        source: Some(identity.source.clone()),
        entry_fill_ids: position.entry_fill_ids.clone(),
        exit_fill_ids,
        fee_currency,
        fees_complete,
        evidence_gain_correlation: position
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("evidence_gain_correlation_id"))
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    });
    context
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
        if position.entry_order_types.is_empty() && !position.entry_order_type.is_empty() {
            position.entry_order_types.push(
                position
                    .entry_order_type
                    .split(" · ")
                    .next()
                    .unwrap_or(&position.entry_order_type)
                    .to_owned(),
            );
        }
        let entry_kind = order_type
            .split(" · ")
            .next()
            .unwrap_or(order_type)
            .to_owned();
        if !position
            .entry_order_types
            .iter()
            .any(|kind| kind == &entry_kind)
        {
            position.entry_order_types.push(entry_kind);
        }
        // A changed entry basis starts a fresh profit-protection trail.
        position.profit_protection_peak_return_percent = None;
        position.thesis_failure_confirmation_count = 0;
        position.thesis_failure_last_signal_timestamp_ms = 0;
        if old == 0.0 {
            position.take_profit_deferred_at_ms = None;
            position.entries = 1;
            position.entry_timestamp_ms = current_time_ms();
            position.price_path.clear();
            position.price_path_total_samples = 0;
            position.price_path_truncated = false;
            position.price_path_restart_interrupted = false;
            position.initial_entry_notional = incoming.abs() * price;
            position.total_fees_paid = fee;
            position.total_entry_fees_paid = fee;
            position.entry_fill_ids.clear();
            position.entry_fill_links.clear();
            position.entry_fee_currencies.clear();
            position.entry_fill_costs_complete = true;
            position.initial_risk_reference = None;
            position.price_path_epoch = None;
            position.exit_parameter_provenance = None;
        } else {
            if position.initial_entry_notional <= 0.0 {
                position.initial_entry_notional = old.abs() * position.average_entry_price;
            }
            position.initial_entry_notional += incoming.abs() * price;
            position.total_fees_paid += fee;
            position.total_entry_fees_paid += fee;
            if count_entry {
                position.entries = position.entries.saturating_add(1);
            }
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
        } else if !context.starts_with("scout:") {
            // The aggregate position is real as soon as any real strategy adds to it.
            position.scout_entry = false;
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
        position.profit_protection_peak_return_percent = None;
        position.quantity = net;
        position.average_entry_price = price;
        position.entry_fees = fee - closing_fee;
        position.realized_pnl = 0.0;
        position.initial_entry_notional = net.abs() * price;
        position.total_fees_paid = fee - closing_fee;
        position.total_entry_fees_paid = fee - closing_fee;
        position.entry_order_type = order_type.to_owned();
        position.entry_order_types = vec![
            order_type
                .split(" · ")
                .next()
                .unwrap_or(order_type)
                .to_owned(),
        ];
        position.entry_context = context.to_owned();
        position.scout_entry = context.starts_with("scout:");
        position.entries = 1;
        position.horizon_take_profit_percent = horizon_take_profit_percent;
        position.take_profit_deferred_at_ms = None;
        position.entry_fill_ids.clear();
        position.entry_fill_links.clear();
        position.entry_fee_currencies.clear();
        position.entry_fill_costs_complete = true;
        position.initial_risk_reference = None;
        position.price_path.clear();
        position.price_path_total_samples = 0;
        position.price_path_truncated = false;
        position.price_path_restart_interrupted = false;
        position.price_path_epoch = None;
        position.entry_timestamp_ms = current_time_ms();
        position.entry_decision_id = None;
        position.trade_id = None;
        position.lifecycle_identity = None;
        position.strategy_decision = None;
        position.exit_parameter_provenance = None;
        position.stop_loss_percent = None;
        position.take_profit_percent = None;
        position.protective_stops.clear();
    } else {
        position.quantity = net;
        position.entry_fees -= entry_fee_closed;
        position.total_fees_paid += fee;
    }
    (pnl, Some(format!("{entry_context}:{entry_type}")))
}

fn apply_strategy_position_fill(
    strategy_positions: &mut BTreeMap<String, BTreeMap<String, TraderPosition>>,
    aggregate_positions: &mut BTreeMap<String, TraderPosition>,
    symbol: &str,
    strategy: &str,
    side: &str,
    quantity: f64,
    price: f64,
    fee: f64,
    order_type: &str,
    context: &str,
    count_entry: bool,
    horizon_take_profit_percent: Option<f64>,
    decision_id: Option<&str>,
    strategy_decision: Option<&serde_json::Value>,
    stop_loss_percent: Option<f64>,
    take_profit_percent: Option<f64>,
) -> (f64, Option<String>) {
    let reversed_position = strategy_positions
        .get(symbol)
        .and_then(|lots| lots.get(strategy))
        .is_some_and(|position| {
            position.quantity.abs() > 1e-12
                && position.quantity.signum() != if side == "buy" { 1.0 } else { -1.0 }
        });
    let mut one_position = BTreeMap::new();
    if let Some(position) = strategy_positions
        .get(symbol)
        .and_then(|lots| lots.get(strategy))
    {
        one_position.insert(strategy.to_owned(), position.clone());
    }
    let result = apply_auto_fill(
        &mut one_position,
        strategy,
        side,
        quantity,
        price,
        fee,
        order_type,
        context,
        count_entry,
        horizon_take_profit_percent,
    );
    let updated = one_position.remove(strategy).unwrap_or_default();
    let active = updated.quantity.abs() > 1e-12;
    let lots = strategy_positions.entry(symbol.to_owned()).or_default();
    if active {
        let mut updated = updated;
        updated.strategy = Some(strategy.to_owned());
        // One strategy/symbol lot can receive add-on fills. Keep its original
        // trade key so every later exit stays attached to the same position.
        if updated.entry_decision_id.is_none() {
            updated.entry_decision_id = decision_id.map(str::to_owned);
        }
        if updated.trade_id.is_none() {
            updated.trade_id = decision_id.map(str::to_owned);
        }
        if updated.strategy_decision.is_none() {
            updated.strategy_decision = strategy_decision.cloned();
        }
        if updated.stop_loss_percent.is_none() {
            updated.stop_loss_percent = stop_loss_percent;
        }
        if updated.take_profit_percent.is_none() {
            updated.take_profit_percent = take_profit_percent;
        }
        if reversed_position || updated.price_path_epoch.is_none() {
            updated.price_path_epoch = updated.trade_id.as_ref().map(|trade_id| {
                format!(
                    "auto-trader-position-path:v1:{}:{trade_id}",
                    updated.entry_timestamp_ms
                )
            });
        }
        lots.insert(strategy.to_owned(), updated);
    } else {
        lots.remove(strategy);
    }
    let category_key = strategy_category_key(strategy_category(Some(strategy))).to_owned();
    let category_entries = lots
        .iter()
        .filter(|(name, _)| {
            strategy_category(Some(name.as_str())) == strategy_category(Some(strategy))
        })
        .map(|(_, position)| position.entries)
        .sum::<u32>();
    if let Some(aggregate) = aggregate_positions.get_mut(symbol) {
        if category_entries > 0 {
            aggregate
                .entries_by_category
                .insert(category_key, category_entries);
        } else {
            aggregate.entries_by_category.remove(&category_key);
        }
    }
    if lots.is_empty() {
        strategy_positions.remove(symbol);
    }
    result
}

fn signal_decision_id(signal: &Signal) -> String {
    let worker_event_id = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("live_worker_event_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let profile_id = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| {
            decision
                .get("learning_profile_id")
                .or_else(|| decision.get("scout_parameter_profile_id"))
        })
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let cycle_id = signal
        .strategy_decision
        .as_ref()
        .and_then(|decision| decision.get("entry_cycle_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if worker_event_id.is_empty() && profile_id.is_empty() && cycle_id.is_empty() {
        return format!(
            "{}|{}|{}",
            signal.timestamp, signal.symbol, signal.direction
        );
    }
    format!(
        "{}|{}|{}|{}|{}|{}|{}",
        cycle_id,
        signal.timestamp,
        signal.symbol,
        signal.direction,
        signal.strategy.as_deref().unwrap_or_default(),
        profile_id,
        worker_event_id,
    )
}

fn next_unique_order_timestamp(now: u64) -> u64 {
    static LAST_TIMESTAMP: AtomicU64 = AtomicU64::new(0);
    loop {
        let previous = LAST_TIMESTAMP.load(Ordering::Relaxed);
        let next = now.max(previous.saturating_add(1));
        if LAST_TIMESTAMP
            .compare_exchange(previous, next, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
        {
            return next;
        }
    }
}

fn record_strategy_order_realized_pnl(order: &mut TraderOrder, realized_delta: f64) {
    order.strategy_realized_pnl =
        Some(order.strategy_realized_pnl.unwrap_or_default() + realized_delta);
    let today = utc_day();
    if order.strategy_realized_pnl_day.as_deref() != Some(today.as_str()) {
        order.strategy_realized_pnl_day = Some(today);
        order.strategy_realized_pnl_today = Some(0.0);
    }
    order.strategy_realized_pnl_today =
        Some(order.strategy_realized_pnl_today.unwrap_or_default() + realized_delta);
}

fn effective_audit_entry_threshold(
    parameters: &serde_json::Map<String, serde_json::Value>,
    key: StrategyParameterKey,
    market_regime: Option<&str>,
) -> Option<f64> {
    market_regime
        .and_then(|regime| key.regime_threshold(regime))
        .and_then(|regime_key| parameters.get(regime_key.as_str()))
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite())
        .or_else(|| {
            parameters
                .get(key.as_str())
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite())
        })
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
) -> serde_json::Value {
    let decision_id = signal_decision_id(signal);
    let mut audited_components = serde_json::to_value(&signal.components).unwrap_or_default();
    if let (Some(component_values), Some(parameters)) = (
        audited_components.as_object_mut(),
        signal
            .strategy_decision
            .as_ref()
            .and_then(|decision| decision.get("strategy_parameters"))
            .and_then(serde_json::Value::as_object),
    ) {
        for key in [
            StrategyParameterKey::MinimumMomentum,
            StrategyParameterKey::MinimumFlowImbalance,
            StrategyParameterKey::MinimumBookImbalance,
            StrategyParameterKey::MinimumVwapDislocationBps,
            StrategyParameterKey::MinimumAbsorptionPressure,
        ] {
            if let Some(value) =
                effective_audit_entry_threshold(parameters, key, signal.market_regime.as_deref())
            {
                component_values
                    .insert(format!("{}_floor", key.as_str()), serde_json::json!(value));
            }
        }
    }
    let mut payload = serde_json::json!({
        "record_schema_version": 3,
        "decision_id": decision_id,
        "stage": stage,
        "timestamp": timestamp,
        "signal_timestamp": signal.timestamp,
        "symbol": signal.symbol,
        "direction": signal.direction,
        "score": signal.score,
        "strength": signal.strength,
        "signal_strategy_candidate": signal.strategy_candidate,
        "daily_cold_start_candidate_eligible": is_daily_cold_start_exploration_candidate(signal),
        "strength_size_multiplier": signal_confidence_size_multiplier(signal),
        "confidence_size_multiplier": signal_confidence_size_multiplier(signal),
        "ranging_probability": signal.ranging_probability,
        "market_context": market_context(signal),
        // Keep execution audit strategy aligned with the entry order fallback.
        "strategy": signal.strategy.as_deref().unwrap_or("unattributed"),
        "strategy_decision": signal.strategy_decision,
        "market_regime": signal.market_regime,
        "news_attribution": signal.news_attribution,
        "entry_gate": signal.entry_gate,
        "signal_components": audited_components,
        "exploratory": exploratory,
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "status": status,
        "reason": reason,
    });
    if let Some(decision) = signal.strategy_decision.as_ref() {
        payload["learning_profile_id"] = decision
            .get("learning_profile_id")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        payload["learning_profile_version"] = decision
            .get("learning_profile_version")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        payload["effective_exit_protection_parameters"] = decision
            .get("effective_exit_protection_parameters")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
        payload["learned_exit_protection_policy"] = decision
            .get("learned_exit_protection_policy")
            .cloned()
            .unwrap_or(serde_json::Value::Null);
    }
    if stage == "entry" || (stage == "decision" && matches!(status, "selected" | "scouted")) {
        payload["trade_id"] = serde_json::json!(decision_id);
    }
    payload
}

fn strategy_rejection_audit_payload(
    signal: &Signal,
    strategy: &str,
    reason: &str,
    timestamp_ms: u64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    phase: &str,
) -> serde_json::Value {
    let mut payload = signal_audit_payload(
        signal,
        "strategy_rejection",
        timestamp_ms,
        "rejected",
        Some(reason),
        None,
        stop_loss_percent,
        take_profit_percent,
    );
    payload["decision_id"] = serde_json::json!(format!(
        "{}|strategy:{strategy}|phase:{phase}|audit:{timestamp_ms}",
        signal_decision_id(signal),
    ));
    let mut entry_gate = signal
        .entry_gate
        .clone()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    entry_gate["selection_mode"] = serde_json::json!("strategy_rejection");
    entry_gate["rejection_reasons"] = serde_json::json!([reason]);
    payload["entry_gate"] = entry_gate;
    if phase == "daily_execution" || phase == "async_submission" {
        let gate_id = if phase == "daily_execution" {
            "daily_strategy_execution"
        } else {
            "async_order_submission"
        };
        payload["execution_gate"] = serde_json::json!({
            "gate_id": gate_id,
            "stage": "post_selection_execution",
            "status": "blocked",
            "result_status": "rejected",
            "reason": reason,
            "evaluated": true,
            "order_submitted": false,
            "short_circuited": true,
            "subsequent_checks": "not_evaluated_after_block",
        });
        payload["gate_path"] = serde_json::json!([
            {
                "stage": "candidate_selection",
                "status": "selected",
                "selection_mode": signal.entry_gate.as_ref().and_then(|gate| gate.get("selection_mode")),
                "gate": signal.entry_gate.as_ref(),
            },
            {
                "stage": "execution",
                "status": "blocked",
                "gate_id": gate_id,
                "reason": reason,
                "evaluated": true,
                "short_circuited": true,
                "subsequent_checks": "not_evaluated_after_block",
            }
        ]);
    }
    payload
}

fn daily_execution_blocker_audit_payload(
    signal: &Signal,
    strategy: &str,
    reason: &str,
    result_status: &str,
    timestamp_ms: u64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    exploratory: bool,
) -> serde_json::Value {
    let mut payload = strategy_rejection_audit_payload(
        signal,
        strategy,
        reason,
        timestamp_ms,
        stop_loss_percent,
        take_profit_percent,
        "daily_execution",
    );
    payload["exploratory"] = serde_json::json!(exploratory);
    payload["actual_order_path_status"] = serde_json::json!(result_status);
    payload["execution_gate"]["result_status"] = serde_json::json!(result_status);
    payload["gate_path"][1]["result_status"] = serde_json::json!(result_status);
    let mut entry_gate = signal
        .entry_gate
        .clone()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    entry_gate["rejection_reasons"] = serde_json::json!([reason]);
    entry_gate["execution_result_status"] = serde_json::json!(result_status);
    payload["entry_gate"] = entry_gate;
    payload
}

fn queue_strategy_rejection_audit(
    state: &AutoTraderState,
    signal: &Signal,
    strategy: &str,
    reason: &str,
    timestamp_ms: u64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    phase: &str,
) {
    enqueue_strategy_rejection_audit(
        &state.audit_tx,
        signal,
        strategy,
        reason,
        timestamp_ms,
        stop_loss_percent,
        take_profit_percent,
        phase,
    );
}

fn enqueue_strategy_rejection_audit(
    queue: &AuditQueue,
    signal: &Signal,
    strategy: &str,
    reason: &str,
    timestamp_ms: u64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    phase: &str,
) -> AuditEnqueueResult {
    queue.enqueue(strategy_rejection_audit_payload(
        signal,
        strategy,
        reason,
        timestamp_ms,
        stop_loss_percent,
        take_profit_percent,
        phase,
    ))
}

fn queue_auto_trader_audit(state: &AutoTraderState, payload: serde_json::Value) {
    let is_execution = payload
        .get("stage")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|stage| {
            stage == "entry"
                || stage == "entry_fill"
                || stage == "dated_carry_entry"
                || stage.starts_with("exit")
        });
    let queue = if is_execution {
        &state.execution_audit_tx
    } else {
        &state.audit_tx
    };
    match queue.enqueue(payload) {
        // Decision rejections/skips are explicitly replaceable under pressure.
        // Evicting one is an accounted loss, but it must not pause trading:
        // critical decisions and all execution records remain queued. The
        // execution queue never evicts; an admission failure is always fatal.
        AuditEnqueueResult::Queued
        | AuditEnqueueResult::Coalesced
        | AuditEnqueueResult::EvictedOld => {}
        AuditEnqueueResult::Dropped => {
            let bit = if is_execution { 2 } else { 1 };
            state.audit_health.fetch_and(!bit, Ordering::Relaxed);
            if !state.audit_failure_latched.swap(true, Ordering::Relaxed) {
                crate::service_log!(
                    "ERROR",
                    "Auto-trader audit queue reached capacity; an audit event was dropped and new signal entries are paused until restart"
                );
            }
        }
    }
}

async fn store_auto_trader_audit(payload: &serde_json::Value) -> bool {
    match crate::service_rpc::insights_browser_request(
        "POST",
        "/signals/auto-trader-audit",
        Some(payload.clone()),
    )
    .await
    {
        Ok(response) if (200..300).contains(&response.status) => true,
        Ok(response) => {
            crate::service_log!(
                "ERROR",
                "Auto-trader audit write failed with status {}",
                response.status
            );
            false
        }
        Err(error) => {
            crate::service_log!("ERROR", "Auto-trader audit write failed: {error}");
            false
        }
    }
}

async fn store_execution_audit_until_saved(
    payload: &serde_json::Value,
    audit_health: &AtomicU64,
    healthy_bit: u64,
) {
    let mut retry_delay = Duration::from_secs(1);
    while !store_auto_trader_audit(payload).await {
        if audit_health.fetch_and(!healthy_bit, Ordering::Relaxed) & healthy_bit != 0 {
            crate::service_log!(
                "ERROR",
                "Auto-trader audit persistence unavailable; new signal entries are paused while queued events retry"
            );
        }
        tokio::time::sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
    }
}

#[cfg(test)]
mod strategy_lot_pnl_tests {
    use super::*;

    fn fill_response(
        client_order_id: &str,
        fill_id: &str,
        quantity: f64,
        price: f64,
    ) -> serde_json::Value {
        serde_json::json!({
            "filled_quantity": quantity,
            "estimated_fee": 0.01,
            "fee_complete": true,
            "fill_evidence": [{
                "id": format!("pm-row-{fill_id}"),
                "fill_id": fill_id,
                "parent_order_id": client_order_id,
                "client_order_id": client_order_id,
                "symbol": "BTC-USDT",
                "side": "buy",
                "quantity": quantity,
                "execution_price": price,
                "timestamp": 1_500,
                "fee": 0.01,
                "fee_known": true,
                "fee_currency": "USDT"
            }]
        })
    }

    #[test]
    fn authoritative_initial_risk_links_partial_fill_pages_idempotently() {
        let request = serde_json::json!({
            "client_order_id": "entry-order-1",
            "symbol": "BTC-USDT",
            "side": "buy"
        });
        let first = fill_response("entry-order-1", "pm-fill:v1:paper:11", 0.5, 100.0);
        let second = fill_response("entry-order-1", "pm-fill:v1:paper:12", 0.5, 102.0);
        let mut position = TraderPosition {
            entry_fill_costs_complete: true,
            ..TraderPosition::default()
        };

        update_initial_risk_reference(&mut position, &first, &request, 0.5);
        append_entry_fill_evidence(&mut position, &first, &request);
        update_initial_risk_reference(&mut position, &second, &request, 0.5);
        append_entry_fill_evidence(&mut position, &second, &request);
        update_initial_risk_reference(&mut position, &second, &request, 0.5);

        let risk = position.initial_risk_reference.as_ref().unwrap();
        assert_eq!(risk.status, "complete");
        assert_eq!(risk.quote_currency.as_deref(), Some("USDT"));
        assert_eq!(risk.fill_ids.len(), 2);
        assert!((risk.quote_amount.unwrap() - 0.505).abs() < 1e-12);
        assert!(position.entry_fill_costs_complete);
        assert_eq!(position.entry_fill_links.len(), 2);
        assert!(
            position
                .entry_fill_links
                .iter()
                .all(|link| link.client_order_id == "entry-order-1")
        );
        assert!(
            position
                .entry_fill_links
                .iter()
                .all(|link| link.timestamp_ms == Some(1_500))
        );
    }

    #[test]
    fn mismatched_or_missing_execution_identity_keeps_initial_risk_unavailable() {
        let request = serde_json::json!({
            "client_order_id": "entry-order-1",
            "symbol": "BTC-USDT",
            "side": "buy"
        });
        let mut mismatched = fill_response("entry-order-2", "pm-fill:v1:paper:11", 1.0, 100.0);
        mismatched["fill_evidence"][0]["parent_order_id"] = serde_json::json!("entry-order-1");
        let mut position = TraderPosition::default();
        update_initial_risk_reference(&mut position, &mismatched, &request, 0.5);
        assert_eq!(
            position.initial_risk_reference.as_ref().unwrap().status,
            "incomplete"
        );
        position.entry_fill_costs_complete = true;
        append_entry_fill_evidence(&mut position, &mismatched, &request);
        assert!(!position.entry_fill_costs_complete);
        assert!(position.entry_fill_links.is_empty());

        let mut legacy = TraderPosition {
            entry_fill_ids: vec!["legacy-fill-without-execution-fields".into()],
            ..TraderPosition::default()
        };
        let complete_future_fill =
            fill_response("entry-order-1", "pm-fill:v1:paper:12", 1.0, 100.0);
        update_initial_risk_reference(&mut legacy, &complete_future_fill, &request, 0.5);
        assert_eq!(legacy.initial_risk_reference.unwrap().status, "incomplete");
    }

    #[test]
    fn add_on_fill_keeps_the_original_entry_exit_policy_and_profile_trace() {
        let mut lots = BTreeMap::new();
        let mut aggregate = BTreeMap::new();
        let captured = serde_json::json!({
            "strategy_parameters": {
                "profit_protection_activation_percent": 0.20,
                "profit_protection_giveback_percent": 0.15,
                "profit_protection_minimum_lock_percent": 0.10
            },
            "learning_profile_id": "entry-profile",
            "learning_profile_version": 17,
            "learned_exit_protection_policy": { "as_of_ms": 1234 }
        });
        apply_strategy_position_fill(
            &mut lots,
            &mut aggregate,
            "BTC-USDT",
            "live-momentum",
            "buy",
            1.0,
            100.0,
            0.0,
            "Limit",
            "live-momentum:strict",
            true,
            None,
            Some("entry-1"),
            Some(&captured),
            Some(0.5),
            Some(0.8),
        );
        let refreshed = serde_json::json!({
            "strategy_parameters": { "profit_protection_activation_percent": 0.50 },
            "learning_profile_id": "later-profile",
            "learning_profile_version": 18
        });
        apply_strategy_position_fill(
            &mut lots,
            &mut aggregate,
            "BTC-USDT",
            "live-momentum",
            "buy",
            0.5,
            101.0,
            0.0,
            "Limit",
            "live-momentum:strict",
            true,
            None,
            Some("entry-2"),
            Some(&refreshed),
            Some(0.7),
            Some(1.2),
        );
        let position = &lots["BTC-USDT"]["live-momentum"];
        assert_eq!(position.strategy_decision.as_ref(), Some(&captured));
        assert_eq!(position.stop_loss_percent, Some(0.5));
        assert_eq!(position.take_profit_percent, Some(0.8));
    }

    #[test]
    fn real_fill_turns_scout_only_aggregate_into_real_position() {
        let mut positions = BTreeMap::new();
        apply_auto_fill(
            &mut positions,
            "ETH-USDT",
            "buy",
            1.0,
            100.0,
            0.0,
            "Limit",
            "scout:scout",
            true,
            None,
        );
        assert!(positions["ETH-USDT"].scout_entry);

        apply_auto_fill(
            &mut positions,
            "ETH-USDT",
            "buy",
            1.0,
            101.0,
            0.0,
            "Limit",
            "live-momentum:strict",
            true,
            None,
        );
        assert!(!positions["ETH-USDT"].scout_entry);
    }

    #[test]
    fn direction_reversal_starts_a_new_path_and_identity_lifetime() {
        let mut lots = BTreeMap::new();
        let mut aggregate = BTreeMap::new();
        apply_strategy_position_fill(
            &mut lots,
            &mut aggregate,
            "BTC-USDT",
            "live-momentum",
            "buy",
            1.0,
            100.0,
            0.0,
            "Limit",
            "live-momentum:strict",
            true,
            None,
            Some("trade-old"),
            None,
            Some(0.5),
            Some(0.8),
        );
        {
            let position = lots
                .get_mut("BTC-USDT")
                .unwrap()
                .get_mut("live-momentum")
                .unwrap();
            position.price_path.push(TradePriceSample {
                timestamp: 1,
                bid: 99.0,
                ask: 101.0,
                sequence: 1,
                epoch: Some("old-path".into()),
                ..TradePriceSample::default()
            });
            position.initial_risk_reference = Some(InitialRiskReference {
                schema_version: 1,
                status: "complete".into(),
                ..InitialRiskReference::default()
            });
        }
        apply_strategy_position_fill(
            &mut lots,
            &mut aggregate,
            "BTC-USDT",
            "live-momentum",
            "sell",
            2.0,
            99.0,
            0.0,
            "Limit",
            "live-momentum:strict",
            true,
            None,
            Some("trade-new"),
            None,
            Some(0.6),
            Some(0.9),
        );

        let position = &lots["BTC-USDT"]["live-momentum"];
        assert_eq!(position.quantity, -1.0);
        assert_eq!(position.trade_id.as_deref(), Some("trade-new"));
        assert_eq!(position.entry_decision_id.as_deref(), Some("trade-new"));
        assert!(position.price_path.is_empty());
        assert_eq!(position.price_path_total_samples, 0);
        assert!(position.initial_risk_reference.is_none());
        assert_eq!(position.stop_loss_percent, Some(0.6));
    }

    #[test]
    fn closing_return_uses_the_strategy_lot_basis() {
        let symbol = "BTC-USDT";
        let strategy = "live-trade-flow";
        let mut aggregate = BTreeMap::from([(
            symbol.to_owned(),
            TraderPosition {
                quantity: 0.01,
                average_entry_price: 120.0,
                ..TraderPosition::default()
            },
        )]);
        let mut lots = BTreeMap::from([(
            symbol.to_owned(),
            BTreeMap::from([(
                strategy.to_owned(),
                TraderPosition {
                    quantity: 1.0,
                    average_entry_price: 100.0,
                    entry_fees: 0.1,
                    ..TraderPosition::default()
                },
            )]),
        )]);
        let (lot_pnl, _) = apply_strategy_position_fill(
            &mut lots,
            &mut aggregate,
            symbol,
            strategy,
            "sell",
            1.0,
            101.0,
            0.1,
            "Ioc",
            "take-profit",
            false,
            None,
            None,
            None,
            None,
            None,
        );
        assert!((lot_pnl - 0.8).abs() < 1e-12);
        assert!(!lots.contains_key(symbol));
    }
}
