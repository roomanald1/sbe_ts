/// Canonical Uniswap v3 amounts for human-normalized liquidity and prices.
pub fn lp_amounts(l: f64, p: f64, lower: f64, upper: f64) -> (f64, f64) {
    let a = lower.sqrt();
    let b = upper.sqrt();
    let r = p.sqrt().clamp(a, b);
    (l * (b - r) / (r * b), l * (r - a))
}
fn liquidity(
    s: &mut Sleeve,
    x: &LiquidityInput,
    o: &Observation,
    allowed: bool,
    active: bool,
) -> Result<(), String> {
    ensure(
        x.chain_id > 0 && x.block_number > 0 && label(&x.pool),
        "Missing pool provenance",
    )?;
    ensure(
        [x.price, x.token0_usd, x.token1_usd].into_iter().all(pos)
            && nn(x.fee_growth_inside_usd)
            && nn(x.enter_cost_usd)
            && nn(x.exit_cost_usd),
        "Invalid pool observation",
    )?;
    let prices_consistent = (x.price / (x.token0_usd / x.token1_usd) - 1.0).abs() < 0.005;
    ensure(
        nn(x.prior_daily_volatility)
            && x.prior_daily_volatility <= 1.0
            && x.indicators_at_ms < o.observed_at_ms
            && o.observed_at_ms - x.indicators_at_ms <= DAY,
        "Invalid or noncausal LP range",
    )?;
    let depeg = !(0.98..=1.02).contains(&x.token0_usd) || !(0.98..=1.02).contains(&x.token1_usd);
    let key = format!("{}:{}", x.chain_id, x.pool);
    let existing = s.positions.get(&key).cloned();
    ensure(
        s.positions.is_empty() || existing.is_some(),
        "Must mark held pool",
    )?;
    if let Some(Position::Liquidity {
        liquidity,
        tick_lower,
        tick_upper,
        lower_price,
        upper_price,
        cost_usd,
        last_fee_growth,
        accrued_fees_usd,
        ..
    }) = existing
    {
        ensure(
            x.fee_lower_price == Some(lower_price) && x.fee_upper_price == Some(upper_price),
            "Fee growth must match held range",
        )?;
        ensure(
            x.fee_growth_inside_usd >= last_fee_growth,
            "Inside-range fee growth moved backwards",
        )?;
        let fees = accrued_fees_usd + liquidity * (x.fee_growth_inside_usd - last_fee_growth);
        s.positions.insert(
            key.clone(),
            Position::Liquidity {
                liquidity,
                tick_lower,
                tick_upper,
                lower_price,
                upper_price,
                cost_usd,
                last_fee_growth: x.fee_growth_inside_usd,
                token0_usd: x.token0_usd,
                token1_usd: x.token1_usd,
                price: x.price,
                accrued_fees_usd: fees,
                exit_cost_usd: x.exit_cost_usd,
            },
        );
        if active
            && (x.price <= lower_price
                || x.price >= upper_price
                || !s.config.enabled
                || !allowed
                || depeg
                || !prices_consistent)
        {
            let (a, b) = lp_amounts(liquidity, x.price, lower_price, upper_price);
            let proceeds = a * x.token0_usd + b * x.token1_usd + fees;
            s.cash_usd += proceeds;
            realize(s, proceeds - cost_usd);
            cost(s, x.exit_cost_usd);
            s.positions.remove(&key);
            s.status = "LP closed; wait for a fresh range-specific fee baseline".into();
            event(
                s,
                o,
                "burn",
                format!("{key}; {a:.8}/{b:.8} tokens, {fees:.4} USD fees"),
            );
        } else {
            s.status = "LP principal and observed inside-range fees marked".into();
        }
        return Ok(());
    }
    if !allowed || depeg || !prices_consistent {
        s.status = "LP entry blocked: disabled, loss limit or depeg".into();
        return Ok(());
    }
    let half_width = (2.0 * x.prior_daily_volatility).clamp(0.0005, 0.02);
    let lower = x.range_lower_price.unwrap_or(x.price * (1.0 - half_width));
    let upper = x.range_upper_price.unwrap_or(x.price * (1.0 + half_width));
    ensure(pos(lower) && lower < x.price && upper > x.price
        && (lower / x.price - 1.0).abs() <= 0.05 && (upper / x.price - 1.0).abs() <= 0.05,
        "Invalid proposed LP range")?;
    if x.range_lower_price.is_some() || x.range_upper_price.is_some() {
        let spacing=x.tick_spacing.ok_or("Missing tick spacing")?;
        let lo=x.tick_lower.ok_or("Missing lower tick")?;
        let hi=x.tick_upper.ok_or("Missing upper tick")?;
        ensure(spacing > 0 && lo < hi && lo % spacing == 0 && hi % spacing == 0
            && (1.0001f64.powi(lo) / lower - 1.0).abs() < 1e-10
            && (1.0001f64.powi(hi) / upper - 1.0).abs() < 1e-10, "LP range must match aligned ticks")?;
    }
    ensure(
        x.fee_lower_price == Some(lower) && x.fee_upper_price == Some(upper),
        "Initial fee baseline must match proposed range",
    )?;
    let budget = s
        .config
        .max_trade_usd
        .min(s.cash_usd - x.enter_cost_usd - x.exit_cost_usd)
        .min(s.equity_usd.max(0.0) * 0.25);
    if budget <= 0.0 || x.enter_cost_usd + x.exit_cost_usd > budget * 0.01 {
        s.status = "LP entry blocked: costs exceed 1% of capital".into();
        return Ok(());
    }
    let (a, b) = lp_amounts(1.0, x.price, lower, upper);
    let units = budget / (a * x.token0_usd + b * x.token1_usd);
    ensure(pos(units), "Invalid LP size")?;
    s.cash_usd -= budget;
    cost(s, x.enter_cost_usd);
    s.positions.insert(
        key.clone(),
        Position::Liquidity {
            liquidity: units,
            tick_lower: x.tick_lower,
            tick_upper: x.tick_upper,
            lower_price: lower,
            upper_price: upper,
            cost_usd: budget,
            last_fee_growth: x.fee_growth_inside_usd,
            token0_usd: x.token0_usd,
            token1_usd: x.token1_usd,
            price: x.price,
            accrued_fees_usd: 0.0,
            exit_cost_usd: x.exit_cost_usd,
        },
    );
    s.status = "Adaptive stablecoin liquidity open".into();
    event(s, o, "mint", format!("{key}; range {lower:.8}–{upper:.8}"));
    Ok(())
}
