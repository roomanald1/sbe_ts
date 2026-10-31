fn covered_call(
    s: &mut Sleeve,
    x: &CallInput,
    o: &Observation,
    allowed: bool,
    active: bool,
) -> Result<(), String> {
    quote(&x.spot, o.observed_at_ms)?;
    ensure(
        label(&x.instrument) && pos(x.strike_usd) && nn(x.option_fee_usd_per_btc),
        "Invalid call contract or fee",
    )?;
    ensure(
        x.delta.is_finite() && (0.0..=1.0).contains(&x.delta),
        "Invalid call delta",
    )?;
    let existing = s
        .positions
        .iter()
        .next()
        .map(|(k, p)| (k.clone(), p.clone()));
    if let Some((
        key,
        Position::CoveredCall {
            quantity,
            spot_entry,
            premium_entry,
            strike,
            expiry_ms,
            ..
        },
    )) = existing
    {
        ensure(
            key == x.instrument && strike == x.strike_usd && expiry_ms == x.expiry_ms,
            "Must mark the held call",
        )?;
        let expired = o.observed_at_ms >= expiry_ms;
        let ask = if expired {
            let v = x
                .settlement
                .as_ref()
                .ok_or("Official option settlement required")?;
            ensure(
                pos(v.price_usd)
                    && label(&v.source)
                    && v.timestamp_ms >= expiry_ms
                    && v.timestamp_ms <= o.observed_at_ms,
                "Invalid option settlement",
            )?;
            (v.price_usd - strike).max(0.0)
        } else {
            quote(&x.call, o.observed_at_ms)?;
            x.call.ask
        };
        s.positions.insert(
            key.clone(),
            Position::CoveredCall {
                quantity,
                spot_entry,
                premium_entry,
                strike,
                expiry_ms,
                spot_bid: x.spot.bid,
                call_ask: ask,
            },
        );
        if expired || (active && (!s.config.enabled || x.delta >= 0.50)) {
            ensure(
                quantity <= x.spot.bid_quantity,
                "Insufficient BTC exit depth",
            )?;
            if !expired {
                ensure(
                    quantity <= x.call.ask_quantity,
                    "Insufficient call buyback depth",
                )?;
            }
            s.cash_usd += quantity * (x.spot.bid - ask);
            realize(
                s,
                quantity * (x.spot.bid - spot_entry + premium_entry - ask),
            );
            cost(
                s,
                quantity
                    * (x.spot.bid * s.config.spot_fee_bps / 10_000.0 + x.option_fee_usd_per_btc),
            );
            s.positions.remove(&key);
            s.status = "Covered call closed; waiting for eligible contract".into();
            event(s, o, "close", format!("{key}; both legs closed"));
        } else {
            s.status = "Covered BTC and short call marked at bid/ask".into();
        }
        return Ok(());
    }
    ensure(s.positions.is_empty(), "Unexpected covered-call position")?;
    quote(&x.call, o.observed_at_ms)?;
    ensure(
        [
            x.prior_close_usd,
            x.prior_sma50_usd,
            x.implied_volatility,
            x.prior_30d_mean_iv,
        ]
        .into_iter()
        .all(pos),
        "Missing IV/trend history",
    )?;
    ensure(
        x.indicators_at_ms < o.observed_at_ms && o.observed_at_ms - x.indicators_at_ms <= 2 * DAY,
        "Stale/future call indicators",
    )?;
    let dte = x.expiry_ms.saturating_sub(o.observed_at_ms) as f64 / DAY as f64;
    if !allowed
        || !(21.0..=45.0).contains(&dte)
        || !(0.10..=0.25).contains(&x.delta)
        || x.prior_close_usd > x.prior_sma50_usd
        || x.implied_volatility <= x.prior_30d_mean_iv
        || x.strike_usd <= x.spot.ask
        || x.call.bid <= 2.0 * x.option_fee_usd_per_btc
    {
        s.status =
            "Waiting for 21–45d call, 0.10–0.25 delta, elevated IV and non-bullish trend".into();
        return Ok(());
    }
    ensure(nn(x.minimum_quantity) && nn(x.quantity_step), "Invalid option lot size")?;
    let raw_quantity = s.config.max_trade_usd.min(s.cash_usd / 1.02) / x.spot.ask;
    let quantity = if x.quantity_step > 0.0 {
        (raw_quantity / x.quantity_step + 1e-12).floor() * x.quantity_step
    } else { raw_quantity };
    if quantity < x.minimum_quantity || !pos(quantity) {
        s.status = "Waiting: budget below published option minimum".into();
        return Ok(());
    }
    ensure(
        pos(quantity) && quantity <= x.spot.ask_quantity && quantity <= x.call.bid_quantity,
        "Insufficient covered-call depth",
    )?;
    s.cash_usd += quantity * (x.call.bid - x.spot.ask);
    cost(
        s,
        quantity * (x.spot.ask * s.config.spot_fee_bps / 10_000.0 + x.option_fee_usd_per_btc),
    );
    s.positions.insert(
        x.instrument.clone(),
        Position::CoveredCall {
            quantity,
            spot_entry: x.spot.ask,
            premium_entry: x.call.bid,
            strike: x.strike_usd,
            expiry_ms: x.expiry_ms,
            spot_bid: x.spot.bid,
            call_ask: x.call.ask,
        },
    );
    s.status = "Covered call open; full BTC inventory backs the call".into();
    event(
        s,
        o,
        "open",
        format!("{}; {quantity:.8} BTC covered", x.instrument),
    );
    Ok(())
}
