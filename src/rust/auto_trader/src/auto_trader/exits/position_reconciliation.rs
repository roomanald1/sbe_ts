async fn reconcile_auto_positions_with_ledger(
    client: &reqwest::Client,
    base: &str,
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    let response: serde_json::Value = client
        .get(format!("{base}/positions?source=auto"))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json()
        .await
        .map_err(upstream_error)?;
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
            ))
        })
        .filter(|(_, quantity, average_entry_price, entry_fees)| {
            quantity.is_finite() && average_entry_price.is_finite() && entry_fees.is_finite()
        })
        .map(|(symbol, quantity, average_entry_price, entry_fees)| {
            (symbol, (quantity, average_entry_price, entry_fees))
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
                .is_none_or(|(quantity, _, _)| quantity.abs() <= 1e-12)
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
        let Some((quantity, average_entry_price, entry_fees)) = ledger_positions.get(symbol) else {
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
    for (symbol, (ledger_quantity, ledger_average_price, ledger_fees)) in &ledger_positions {
        let Some(aggregate) = settings.positions.get(symbol).cloned() else { continue; };
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
                replacement.strategy = Some("live-insights".into());
                lots.insert("live-insights".into(), replacement);
                changed = true;
            }
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
        persist_settings(&state.settings_path, &settings)?;
    }
    Ok(())
}

