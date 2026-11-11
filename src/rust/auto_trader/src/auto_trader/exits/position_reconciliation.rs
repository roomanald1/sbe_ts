async fn reconcile_auto_positions_with_ledger(
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let response = crate::service_rpc::position_get("/positions?source=auto")
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let ledger_positions = response["positions"]
        .as_array()
        .ok_or((
            StatusCode::BAD_GATEWAY,
            "Position manager response did not contain positions".into(),
        ))?
        .iter()
        .filter_map(|position| {
            Some((
                position["symbol"].as_str()?.to_owned(),
                position["quantity"].as_f64()?,
                position["average_entry_price"].as_f64()?,
                position["entry_fees"].as_f64().unwrap_or_default(),
                position["last_trade_timestamp"].as_u64().unwrap_or_default(),
            ))
        })
        .filter(|(_, quantity, average_entry_price, entry_fees, _)| {
            quantity.is_finite() && average_entry_price.is_finite() && entry_fees.is_finite()
        })
        .map(|(symbol, quantity, average_entry_price, entry_fees, last_trade_timestamp)| {
            (symbol, (quantity, average_entry_price, entry_fees, last_trade_timestamp))
        })
        .collect::<BTreeMap<_, _>>();

    let mut settings = state.settings.lock().await;
    let flat_symbols = settings
        .positions
        .keys()
        .chain(settings.strategy_positions.keys())
        .chain(
            settings
                .orders
                .iter()
                .filter(|order| order.purpose == "stop-loss" && order.status == "unfilled")
                .map(|order| &order.symbol),
        )
        .filter(|symbol| {
            ledger_positions
                .get(*symbol)
                .is_none_or(|(quantity, _, _, _)| quantity.abs() <= 1e-12)
        })
        .cloned()
        .collect::<BTreeSet<_>>();
    let mut changed = false;

    for (symbol, position) in &mut settings.positions {
        if flat_symbols.contains(symbol) {
            if position.quantity.abs() > 1e-12 || !position.protective_stops.is_empty() {
                crate::service_log!(
                    "INFO",
                    "Reconciling stale AutoTrader position for {symbol}: {} -> 0; the position manager ledger is flat",
                    position.quantity
                );
                *position = TraderPosition::default();
                changed = true;
            }
            continue;
        }
        let Some((quantity, average_entry_price, entry_fees, last_trade_timestamp)) = ledger_positions.get(symbol) else {
            continue;
        };
        let quantity_changed = (position.quantity - quantity).abs() > 1e-12;
        let normalized_entry_fees = entry_fees.abs();
        if quantity_changed
            || (position.average_entry_price - average_entry_price).abs() > 1e-12
            || (position.entry_fees - normalized_entry_fees).abs() > 1e-12
        {
            if quantity_changed {
                crate::service_log!(
                    "INFO",
                    "Reconciling AutoTrader quantity for {symbol}: {} -> {} from the position manager ledger",
                    position.quantity,
                    quantity
                );
            }
            position.quantity = *quantity;
            position.average_entry_price = *average_entry_price;
            // Position manager stores signed cash flows; AutoTrader stores costs as positive values.
            position.entry_fees = normalized_entry_fees;
            if position.entry_timestamp_ms == 0 {
                position.entry_timestamp_ms = *last_trade_timestamp;
            }
            if quantity_changed {
                position.protective_stops.clear();
            }
            changed = true;
        }
    }

    for symbol in &flat_symbols {
        if settings.strategy_positions.remove(symbol).is_some() { changed = true; }
    }

    // Preserve independent strategy lots while matching their combined size to the authoritative ledger.
    for (symbol, (ledger_quantity, ledger_average_price, ledger_fees, last_trade_timestamp)) in &ledger_positions {
        // The position manager ledger is authoritative. AutoTrader settings may
        // have been reset or may predate this position, so recreate the missing
        // aggregate before rebuilding strategy lots.
        let aggregate = settings.positions.entry(symbol.clone()).or_insert_with(|| {
            changed = true;
            let mut position = TraderPosition {
                quantity: *ledger_quantity,
                average_entry_price: *ledger_average_price,
                entry_fees: ledger_fees.abs(),
                entries: 1,
                entry_timestamp_ms: *last_trade_timestamp,
                strategy: Some("unattributed".into()),
                ..TraderPosition::default()
            };
            position.trade_id = position.entry_decision_id.clone();
            position
        }).clone();
        let aggregate_quantity = aggregate.quantity;
        if aggregate_quantity.abs() <= 1e-12 { continue; }
        if let Some(lots) = settings.strategy_positions.get_mut(symbol) {
            let lot_quantity = lots.values().map(|lot| lot.quantity).sum::<f64>();
            let same_direction = lot_quantity.abs() > 1e-12 && lot_quantity.signum() == ledger_quantity.signum();
            if same_direction && (lot_quantity - ledger_quantity).abs() > 1e-12 {
                let scale = ledger_quantity.abs() / lot_quantity.abs();
                let fee_total = lots.values().map(|lot| lot.entry_fees).sum::<f64>();
                for lot in lots.values_mut() {
                    let old_quantity = lot.quantity.abs();
                    lot.quantity *= scale;
                    lot.entry_fees = if fee_total > 1e-12 { *ledger_fees * (lot.entry_fees / fee_total) } else { *ledger_fees * old_quantity / lot_quantity.abs() };
                    lot.protective_stops.clear();
                }
                changed = true;
            } else if !same_direction {
                lots.clear();
                let mut replacement = aggregate.clone();
                replacement.quantity = *ledger_quantity;
                replacement.average_entry_price = *ledger_average_price;
                replacement.entry_fees = *ledger_fees;
                replacement.strategy = Some("unattributed".into());
                lots.insert("unattributed".into(), replacement);
                changed = true;
            }
        } else {
            let mut restored = aggregate;
            restored.quantity = *ledger_quantity;
            restored.average_entry_price = *ledger_average_price;
            restored.entry_fees = ledger_fees.abs();
            settings.strategy_positions.entry(symbol.clone()).or_default()
                .insert(restored.strategy.clone().unwrap_or_else(|| "unattributed".into()), restored);
            changed = true;
        }
    }

    for order in &mut settings.orders {
        if order.purpose == "stop-loss"
            && order.status == "unfilled"
            && flat_symbols.contains(&order.symbol)
        {
            order.status = "cancelled".into();
            order.failure_reason = None;
            changed = true;
        }
    }
    if changed {
        request_settings_persist(state)?;
    }
    Ok(())
}

