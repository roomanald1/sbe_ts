fn residual_scores(x: &ResidualInput, now: u64) -> Result<BTreeMap<String, f64>, String> {
    ensure(
        (4..=32).contains(&x.assets.len()),
        "Residual portfolio needs 4–32 assets",
    )?;
    let anchor = x.assets[0]
        .closes
        .last()
        .ok_or("Missing closes")?
        .timestamp_ms;
    ensure(
        anchor < now && now - anchor <= 2 * DAY,
        "Stale or incomplete daily candle",
    )?;
    let n = 45;
    let mut symbols = BTreeSet::new();
    let mut timestamps = None;
    let mut returns = Vec::new();
    for a in &x.assets {
        ensure(
            label(&a.symbol) && symbols.insert(a.symbol.clone()),
            "Duplicate/invalid symbol",
        )?;
        quote(&a.quote, now)?;
        ensure(
            a.closes.len() >= n && a.closes.len() <= 400,
            "Need 45–400 daily closes",
        )?;
        let bars = &a.closes[a.closes.len() - n..];
        ensure(bars.iter().all(|b| pos(b.close)), "Invalid close")?;
        ensure(
            bars.windows(2)
                .all(|b| b[1].timestamp_ms.checked_sub(b[0].timestamp_ms) == Some(DAY)),
            "Noncontiguous daily history",
        )?;
        ensure(
            bars.last().unwrap().timestamp_ms == anchor,
            "Daily anchors differ",
        )?;
        let times = bars.iter().map(|b| b.timestamp_ms).collect::<Vec<_>>();
        if let Some(t) = &timestamps {
            ensure(*t == times, "Unsynchronized histories")?;
        } else {
            timestamps = Some(times);
        }
        returns.push(
            bars.windows(2)
                .map(|b| b[1].close / b[0].close - 1.0)
                .collect::<Vec<_>>(),
        );
    }
    let market = (0..n - 1)
        .map(|j| returns.iter().map(|r| r[j]).sum::<f64>() / returns.len() as f64)
        .collect::<Vec<_>>();
    let mut scores = BTreeMap::new();
    for (a, r) in x.assets.iter().zip(&returns) {
        let mut score = 0.0;
        for t in 30..n - 1 {
            // Each OLS beta uses only returns before the return being residualized.
            let mx = market[t - 30..t].iter().sum::<f64>() / 30.0;
            let my = r[t - 30..t].iter().sum::<f64>() / 30.0;
            let variance = market[t - 30..t]
                .iter()
                .map(|v| (v - mx).powi(2))
                .sum::<f64>();
            ensure(variance > 1e-16, "Zero market-factor variance")?;
            let covariance = (t - 30..t)
                .map(|j| (market[j] - mx) * (r[j] - my))
                .sum::<f64>();
            let beta = covariance / variance;
            score += r[t] - (my - beta * mx) - beta * market[t];
        }
        ensure(score.is_finite(), "Nonfinite residual score")?;
        scores.insert(a.symbol.clone(), score);
    }
    if let Some(insights) = &x.scores {
        ensure(insights.len() == scores.len() && scores.iter().all(|(k,v)|
            insights.get(k).is_some_and(|s|s.is_finite() && (s-v).abs()<1e-10)),
            "Insights residual scores disagree with causal history")?;
        return Ok(insights.clone());
    }
    Ok(scores)
}
fn residual(
    s: &mut Sleeve,
    x: &ResidualInput,
    o: &Observation,
    allowed: bool,
    active: bool,
) -> Result<(), String> {
    let scores = residual_scores(x, o.observed_at_ms)?;
    let quotes = x
        .assets
        .iter()
        .map(|a| (a.symbol.as_str(), &a.quote))
        .collect::<BTreeMap<_, _>>();
    for (symbol, p) in &mut s.positions {
        if let Position::Spot {
            quantity,
            mark_price,
            ..
        } = p
        {
            let q = quotes
                .get(symbol.as_str())
                .ok_or("Held residual asset missing")?;
            *mark_price = if *quantity >= 0.0 { q.bid } else { q.ask };
        }
    }
    if s.last_observation_ms > 0 {
        let years = (o.observed_at_ms - s.last_observation_ms) as f64 / (365.0 * DAY as f64);
        let borrowed = s
            .positions
            .values()
            .map(|p| match p {
                Position::Spot {
                    quantity,
                    mark_price,
                    ..
                } if *quantity < 0.0 => -quantity * mark_price,
                _ => 0.0,
            })
            .sum::<f64>();
        cost(s, borrowed * s.config.short_borrow_apr * years);
    }
    mark(s)?;
    if !active {
        return Ok(());
    }
    let day = x.assets[0].closes.last().unwrap().timestamp_ms / DAY;
    let rebalance = s
        .last_rebalance_day
        .is_none_or(|d| (d + 4) / 7 != (day + 4) / 7);
    let flatten =
        !s.config.enabled || !allowed || s.daily_pnl_usd <= -s.config.daily_loss_limit_usd;
    if !rebalance && !flatten {
        s.status = "Holding weekly residual portfolio".into();
        return Ok(());
    }
    let mut ranks = scores.into_iter().collect::<Vec<_>>();
    ranks.sort_by(|a, b| a.1.total_cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    let mut target = BTreeMap::<String, f64>::new();
    if !flatten {
        let long = ranks.iter().rev().take(2).collect::<Vec<_>>();
        let short = ranks
            .iter()
            .take(2)
            .filter(|(k, _)| x.assets.iter().any(|a| a.symbol == *k && a.short_available))
            .collect::<Vec<_>>();
        if long.len() == 2 && short.len() == 2 {
            let leg = s.config.max_trade_usd.min(
                s.equity_usd.max(0.0) / (4.0 * (1.0 + 4.0 * s.config.spot_fee_bps / 10_000.0)),
            );
            for (k, _) in long {
                target.insert(k.clone(), leg / quotes[k.as_str()].ask);
            }
            for (k, _) in short {
                target.insert(k.clone(), -leg / quotes[k.as_str()].bid);
            }
        }
    }
    let mut trades = Vec::new();
    let mut gross = 0.0;
    for a in &x.assets {
        let current = match s.positions.get(&a.symbol) {
            Some(Position::Spot { quantity, .. }) => *quantity,
            _ => 0.0,
        };
        let wanted = target.get(&a.symbol).copied().unwrap_or(0.0);
        let delta = wanted - current;
        let price = if delta >= 0.0 {
            a.quote.ask
        } else {
            a.quote.bid
        };
        ensure(
            delta.abs()
                <= if delta >= 0.0 {
                    a.quote.ask_quantity
                } else {
                    a.quote.bid_quantity
                },
            "Insufficient residual depth",
        )?;
        gross += wanted.abs() * price;
        trades.push((a.symbol.clone(), current, wanted, delta, price));
    }
    ensure(
        gross <= s.equity_usd.max(0.0) + 1e-8,
        "Gross exposure exceeds equity",
    )?;
    for (k, current, wanted, delta, price) in trades {
        if delta.abs() < 1e-12 {
            continue;
        }
        let entry = match s.positions.get(&k) {
            Some(Position::Spot { entry_price, .. }) => *entry_price,
            _ => price,
        };
        let closed = if current * delta < 0.0 {
            current.abs().min(delta.abs())
        } else {
            0.0
        };
        realize(s, closed * (price - entry) * current.signum());
        s.cash_usd -= delta * price;
        cost(s, delta.abs() * price * s.config.spot_fee_bps / 10_000.0);
        if wanted.abs() < 1e-12 {
            s.positions.remove(&k);
        } else {
            let entry_price = if current * wanted <= 0.0 {
                price
            } else if wanted.abs() > current.abs() {
                (current.abs() * entry + delta.abs() * price) / wanted.abs()
            } else {
                entry
            };
            s.positions.insert(
                k.clone(),
                Position::Spot {
                    quantity: wanted,
                    entry_price,
                    mark_price: if wanted >= 0.0 {
                        quotes[k.as_str()].bid
                    } else {
                        quotes[k.as_str()].ask
                    },
                },
            );
        }
        event(s, o, "rebalance", format!("{k}: {delta:+.8} at {price:.4}"));
    }
    s.last_rebalance_day = Some(day);
    s.status = if s.positions.is_empty() {
        "Cash; no balanced residual portfolio"
    } else {
        "Holding weekly residual portfolio"
    }
    .into();
    Ok(())
}
