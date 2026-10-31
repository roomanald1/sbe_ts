use super::{engine, types::*};
fn obs(id: &str, data: Input, time: u64) -> Observation {
    Observation {
        id: id.into(),
        source: "test fixture".into(),
        observed_at_ms: time,
        data,
    }
}
fn account(id: &str) -> Sleeve {
    let mut s = Sleeve::new(id);
    s.config.enabled = true;
    s.config.max_trade_usd = 100.0;
    s
}
fn q(bid: f64, ask: f64, time: u64) -> Quote {
    Quote {
        bid,
        ask,
        bid_quantity: 100.0,
        ask_quantity: 100.0,
        timestamp_ms: time,
    }
}
fn liquidation() -> LiquidationInput {
    LiquidationInput {
        opportunity_id: "loan-1".into(),
        chain_id: 1,
        block_number: 20,
        protocol: "aave".into(),
        health_factor: 0.9,
        debt_repayment_usd: 90.0,
        executable_collateral_sale_usd: 99.0,
        total_cost_usd: 2.0,
        simulation_succeeded: true,
        atomic: true,
    }
}
fn call(time: u64) -> CallInput {
    CallInput {
        minimum_quantity: 0.0,
        quantity_step: 0.0,
        instrument: "BTC-test-C".into(),
        spot: q(99.0, 100.0, time),
        call: q(3.0, 4.0, time),
        strike_usd: 110.0,
        expiry_ms: time + 30 * DAY,
        delta: 0.20,
        implied_volatility: 70.0,
        prior_30d_mean_iv: 60.0,
        prior_close_usd: 99.0,
        prior_sma50_usd: 105.0,
        indicators_at_ms: time - 1,
        option_fee_usd_per_btc: 0.1,
        settlement: None,
    }
}
fn pool(time: u64) -> LiquidityInput {
    LiquidityInput {
        range_lower_price: None,
        range_upper_price: None,
        tick_lower: None,
        tick_upper: None,
        tick_spacing: None,
        chain_id: 1,
        pool: "USDC-USDT".into(),
        block_number: 1,
        price: 1.0,
        token0_usd: 1.0,
        token1_usd: 1.0,
        fee_growth_inside_usd: 0.0,
        fee_lower_price: Some(0.99),
        fee_upper_price: Some(1.01),
        prior_daily_volatility: 0.005,
        indicators_at_ms: time - 1,
        enter_cost_usd: 0.1,
        exit_cost_usd: 0.1,
    }
}
#[test]
fn default_accounts_disabled_and_independent() {
    let b = Book::initialized();
    assert_eq!(b.sleeves.len(), 5);
    assert!(
        b.sleeves
            .values()
            .all(|s| !s.config.enabled && s.cash_usd == 1000.0)
    );
}
#[test]
fn observation_wire_roundtrip() {
    let o = obs("one", Input::LiquidationKeeper(liquidation()), DAY);
    let text = serde_json::to_string(&o).unwrap();
    let decoded: Observation = serde_json::from_str(&text).unwrap();
    assert_eq!(decoded.data.id(), IDS[3]);
}
#[test]
fn liquidation_net_costs_and_duplicate_opportunity() {
    let mut s = account(IDS[3]);
    engine::apply(
        &mut s,
        &obs("one", Input::LiquidationKeeper(liquidation()), DAY),
        true,
    )
    .unwrap();
    assert_eq!(s.cash_usd, 1007.0);
    assert_eq!(s.realized_pnl_usd, 7.0);
    assert_eq!(s.fees_usd, 2.0);
    engine::apply(
        &mut s,
        &obs("two", Input::LiquidationKeeper(liquidation()), DAY + 1),
        true,
    )
    .unwrap();
    assert_eq!(s.cash_usd, 1007.0);
}
#[test]
fn disabled_paused_or_unsimulated_cannot_open() {
    for scenario in 0..3 {
        let mut s = account(IDS[3]);
        let mut x = liquidation();
        if scenario == 0 {
            s.config.enabled = false;
        }
        if scenario == 2 {
            x.simulation_succeeded = false;
        }
        engine::apply(
            &mut s,
            &obs("one", Input::LiquidationKeeper(x), DAY),
            scenario != 1,
        )
        .unwrap();
        assert_eq!(s.cash_usd, 1000.0);
    }
}
#[test]
fn loss_limit_resets_only_on_next_utc_day() {
    let mut s = account(IDS[3]);
    s.utc_day = 1;
    s.daily_pnl_usd = -26.0;
    engine::apply(
        &mut s,
        &obs("one", Input::LiquidationKeeper(liquidation()), DAY + 1),
        true,
    )
    .unwrap();
    assert_eq!(s.cash_usd, 1000.0);
    engine::apply(
        &mut s,
        &obs("two", Input::LiquidationKeeper(liquidation()), 2 * DAY),
        true,
    )
    .unwrap();
    assert_eq!(s.cash_usd, 1007.0);
}
#[test]
fn delayed_redemption_does_not_realize_projected_profit() {
    let mut s = account(IDS[2]);
    let t = DAY;
    let x = StakingInput::Open {
        opportunity_id: "r1".into(),
        token: "stETH".into(),
        input_eth: 0.03,
        token_quantity: 0.031,
        output_eth: 0.031,
        eth_usd: 3000.0,
        total_cost_usd: 1.0,
        atomic: false,
        redeem_after_ms: t + DAY,
        chain_id: 1,
        block_number: 1,
    };
    engine::apply(
        &mut s,
        &obs("one", Input::StakingRedemptionArbitrage(x), t),
        true,
    )
    .unwrap();
    assert_eq!(s.realized_pnl_usd, -1.0);
    assert_eq!(s.equity_usd, 999.0);
    assert_eq!(s.positions.len(), 1);
    let settle = StakingInput::Settle {
        opportunity_id: "r1".into(),
        received_eth: 0.0305,
        eth_usd: 3000.0,
        total_cost_usd: 0.5,
        chain_id: 1,
        transaction_hash: "0xobserved".into(),
    };
    engine::apply(
        &mut s,
        &obs("two", Input::StakingRedemptionArbitrage(settle), t + DAY),
        false,
    )
    .unwrap();
    assert!((s.equity_usd - 1000.0).abs() < 1e-8);
    assert!(s.positions.is_empty());
}
#[test]
fn covered_call_cash_equity_and_expiry_payoff() {
    let mut s = account(IDS[1]);
    s.config.spot_fee_bps = 0.0;
    let t = DAY;
    let mut x = call(t);
    engine::apply(
        &mut s,
        &obs("one", Input::FilteredCoveredCall(x.clone()), t),
        true,
    )
    .unwrap();
    assert!((s.cash_usd - 902.9).abs() < 1e-8);
    assert!((s.equity_usd - 997.9).abs() < 1e-8);
    x.spot = q(120.0, 121.0, t + 30 * DAY);
    x.settlement = Some(Settlement {
        price_usd: 120.0,
        timestamp_ms: t + 30 * DAY,
        source: "official".into(),
    });
    engine::apply(
        &mut s,
        &obs("two", Input::FilteredCoveredCall(x), t + 30 * DAY),
        false,
    )
    .unwrap();
    assert!(s.positions.is_empty());
    assert!((s.cash_usd - 1012.8).abs() < 1e-8);
}
#[test]
fn held_options_require_exact_contract_and_official_settlement() {
    let mut s = account(IDS[1]);
    let t = DAY;
    let mut x = call(t);
    engine::apply(
        &mut s,
        &obs("one", Input::FilteredCoveredCall(x.clone()), t),
        true,
    )
    .unwrap();
    x.instrument = "other".into();
    x.spot.timestamp_ms = t + 1;
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("two", Input::FilteredCoveredCall(x.clone()), t + 1),
            true
        )
        .is_err()
    );
    x.instrument = "BTC-test-C".into();
    x.spot.timestamp_ms = t + 30 * DAY;
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("two", Input::FilteredCoveredCall(x), t + 30 * DAY),
            true
        )
        .is_err()
    );
}
#[test]
fn lp_amounts_outside_range_are_one_sided() {
    let (a, b) = engine::lp_amounts(1.0, 0.8, 0.99, 1.01);
    assert!(a > 0.0);
    assert_eq!(b, 0.0);
    let (a, b) = engine::lp_amounts(1.0, 1.2, 0.99, 1.01);
    assert_eq!(a, 0.0);
    assert!(b > 0.0);
}
#[test]
fn liquidity_fees_accrue_once_and_exit_preserves_costs() {
    let mut s = account(IDS[4]);
    let t = DAY;
    let mut x = pool(t);
    engine::apply(
        &mut s,
        &obs("one", Input::AdaptiveStablecoinLiquidity(x.clone()), t),
        true,
    )
    .unwrap();
    let l = match s.positions.values().next().unwrap() {
        Position::Liquidity { liquidity, .. } => *liquidity,
        _ => panic!(),
    };
    x.fee_growth_inside_usd = 1.0 / l;
    engine::apply(
        &mut s,
        &obs("two", Input::AdaptiveStablecoinLiquidity(x.clone()), t + 1),
        true,
    )
    .unwrap();
    let eq = s.equity_usd;
    engine::apply(
        &mut s,
        &obs(
            "three",
            Input::AdaptiveStablecoinLiquidity(x.clone()),
            t + 2,
        ),
        true,
    )
    .unwrap();
    assert!((s.equity_usd - eq).abs() < 1e-8);
    s.config.enabled = false;
    engine::apply(
        &mut s,
        &obs("four", Input::AdaptiveStablecoinLiquidity(x), t + 3),
        true,
    )
    .unwrap();
    assert!(s.positions.is_empty());
    assert!((s.cash_usd - 1000.8).abs() < 1e-8);
}
#[test]
fn liquidity_rejects_fee_growth_from_another_range() {
    let mut s = account(IDS[4]);
    let t = DAY;
    let mut x = pool(t);
    engine::apply(
        &mut s,
        &obs("one", Input::AdaptiveStablecoinLiquidity(x.clone()), t),
        true,
    )
    .unwrap();
    x.fee_lower_price = Some(0.98);
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("two", Input::AdaptiveStablecoinLiquidity(x), t + 1),
            true
        )
        .is_err()
    );
}
#[test]
fn stale_quotes_and_out_of_order_events_are_rejected() {
    let mut s = account(IDS[1]);
    let t = DAY;
    let x = call(t);
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("one", Input::FilteredCoveredCall(x.clone()), t + 30_001),
            true
        )
        .is_err()
    );
    engine::apply(
        &mut s,
        &obs("one", Input::FilteredCoveredCall(x.clone()), t),
        true,
    )
    .unwrap();
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("two", Input::FilteredCoveredCall(x), t - 1),
            true
        )
        .is_err()
    );
}
fn residual_input(t: u64) -> ResidualInput {
    let assets = (0..4)
        .map(|i| {
            let mut price = 100.0;
            let closes = (0..52)
                .map(|d| {
                    let market = 0.01 * (d as f64 * 0.7).sin();
                    let specific = if d > 30 {
                        (i as f64 - 1.5) * 0.002
                    } else {
                        0.0
                    };
                    price *= 1.0 + market + specific;
                    Close {
                        timestamp_ms: (48 + d) * DAY,
                        close: price,
                    }
                })
                .collect::<Vec<_>>();
            ResidualAsset {
                symbol: format!("asset-{i}"),
                closes,
                quote: q(price * 0.999, price * 1.001, t),
                short_available: true,
            }
        })
        .collect();
    ResidualInput {
        scores: None,
        assets,
    }
}
#[test]
fn residual_portfolio_balances_and_borrow_is_charged_without_rebalance() {
    let t = 100 * DAY;
    let mut s = account(IDS[0]);
    let mut x = residual_input(t);
    engine::apply(
        &mut s,
        &obs("one", Input::ResidualMomentum(x.clone()), t),
        true,
    )
    .unwrap();
    assert_eq!(s.positions.len(), 4);
    let quantities = s
        .positions
        .values()
        .filter_map(|p| match p {
            Position::Spot { quantity, .. } => Some(*quantity),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(quantities.iter().filter(|q| **q < 0.0).count(), 2);
    let fees = s.fees_usd;
    for a in &mut x.assets {
        a.quote.timestamp_ms = t + 3_600_000;
    }
    engine::apply(
        &mut s,
        &obs("two", Input::ResidualMomentum(x), t + 3_600_000),
        true,
    )
    .unwrap();
    assert!(s.fees_usd > fees);
    assert_eq!(s.positions.len(), 4);
}
#[test]
fn residual_rejects_unsynchronized_or_future_history() {
    let t = 100 * DAY;
    let s = account(IDS[0]);
    let mut x = residual_input(t);
    x.assets[1].closes[51].timestamp_ms += DAY;
    assert!(
        engine::apply(
            &mut s.clone(),
            &obs("one", Input::ResidualMomentum(x), t),
            true
        )
        .is_err()
    );
}

#[test]
fn fixture_replays_all_five_accounts_without_starting_services() {
    let input = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../research/autotrader/fixtures/alternative_observations.jsonl"
    ));
    let output = std::env::temp_dir().join(format!(
        "alternative-replay-{}-{}.json",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    super::replay::run(input, &output).unwrap();
    let book: Book = serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    assert_eq!(book.sleeves[IDS[0]].positions.len(), 4);
    for id in &IDS[1..] {
        assert!(book.sleeves[*id].positions.is_empty(), "{id}");
    }
    assert_eq!(book.sleeves[IDS[3]].realized_pnl_usd, 7.0);
    assert!(
        super::replay::run(input, &output).is_err(),
        "Existing report must not be overwritten"
    );
    std::fs::remove_file(output).unwrap();
}

#[test]
fn covered_call_enforces_published_minimum_and_quantity_step() {
    let mut s = account(IDS[1]);
    s.config.max_trade_usd = 250.0;
    let time = DAY;
    let mut input = call(time);
    input.minimum_quantity = 1.0;
    input.quantity_step = 1.0;
    engine::apply(
        &mut s,
        &obs("lot", Input::FilteredCoveredCall(input.clone()), time),
        true,
    )
    .unwrap();
    let Some(Position::CoveredCall { quantity, .. }) = s.positions.values().next() else {
        panic!("Expected covered call")
    };
    assert_eq!(*quantity, 2.0);
    let mut s = account(IDS[1]);
    s.config.max_trade_usd = 90.0;
    engine::apply(
        &mut s,
        &obs("too-small", Input::FilteredCoveredCall(input), time),
        true,
    )
    .unwrap();
    assert!(s.positions.is_empty());
    assert!(s.status.contains("minimum"));
}
#[test]
fn liquidity_uses_exact_initialized_tick_bounds_and_rejects_inconsistent_ticks() {
    let time = DAY;
    let mut input = pool(time);
    input.tick_lower = Some(-10);
    input.tick_upper = Some(10);
    input.tick_spacing = Some(1);
    input.range_lower_price = Some(1.0001f64.powi(-10));
    input.range_upper_price = Some(1.0001f64.powi(10));
    input.fee_lower_price = input.range_lower_price;
    input.fee_upper_price = input.range_upper_price;
    let mut s = account(IDS[4]);
    engine::apply(
        &mut s,
        &obs(
            "ticks",
            Input::AdaptiveStablecoinLiquidity(input.clone()),
            time,
        ),
        true,
    )
    .unwrap();
    assert!(matches!(
        s.positions.values().next(),
        Some(Position::Liquidity {
            tick_lower: Some(-10),
            tick_upper: Some(10),
            ..
        })
    ));
    let mut s = account(IDS[4]);
    input.tick_upper = Some(11);
    assert!(
        engine::apply(
            &mut s,
            &obs(
                "wrong-ticks",
                Input::AdaptiveStablecoinLiquidity(input),
                time
            ),
            true
        )
        .is_err()
    );
}
#[test]
fn residual_rejects_scores_that_disagree_with_causal_history() {
    let time = 100 * DAY;
    let mut input = residual_input(time);
    input.scores = Some(
        input
            .assets
            .iter()
            .map(|a| (a.symbol.clone(), 0.12345))
            .collect(),
    );
    let mut s = account(IDS[0]);
    assert!(
        engine::apply(
            &mut s,
            &obs("wrong-factors", Input::ResidualMomentum(input), time),
            true
        )
        .is_err()
    );
}
