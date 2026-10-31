fn staking(s: &mut Sleeve, x: &StakingInput, o: &Observation, allowed: bool) -> Result<(), String> {
    match x {
        StakingInput::Open {
            opportunity_id,
            token,
            input_eth,
            token_quantity,
            output_eth,
            eth_usd,
            total_cost_usd,
            atomic,
            redeem_after_ms,
            chain_id,
            block_number,
        } => {
            ensure(
                label(opportunity_id) && label(token) && *chain_id > 0 && *block_number > 0,
                "Missing staking route provenance",
            )?;
            ensure(
                [*input_eth, *token_quantity, *output_eth, *eth_usd]
                    .into_iter()
                    .all(pos)
                    && nn(*total_cost_usd),
                "Invalid staking amounts",
            )?;
            let key = format!("route:{chain_id}:{opportunity_id}");
            if s.processed_ids.contains(&key) {
                s.status = "Staking route already processed".into();
                return Ok(());
            }
            if s.processed_ids.len() >= 100_000 {
                s.status = "Archive required before new staking entries".into();
                return Ok(());
            }
            let principal = input_eth * eth_usd;
            let gross = (output_eth - input_eth) * eth_usd;
            let eligible = allowed
                && principal <= s.config.max_trade_usd
                && principal + total_cost_usd <= s.cash_usd
                && gross - total_cost_usd > principal * s.config.min_edge_bps / 10_000.0;
            if !eligible {
                s.status = "Staking route rejected by net edge, cash or size gate".into();
                return Ok(());
            }
            if *atomic {
                s.cash_usd += gross;
                realize(s, gross);
                cost(s, *total_cost_usd);
                s.status = "Atomic staking route simulated".into();
            } else {
                ensure(
                    *redeem_after_ms > o.observed_at_ms,
                    "Delayed redemption requires future claim time",
                )?;
                s.cash_usd -= principal;
                cost(s, *total_cost_usd);
                // Never realize estimated proceeds while a redemption is pending.
                s.positions.insert(
                    key.clone(),
                    Position::Redemption {
                        token: token.clone(),
                        quantity: *token_quantity,
                        cost_usd: principal,
                        expected_eth: *output_eth,
                        redeem_after_ms: *redeem_after_ms,
                        mark_usd: principal,
                    },
                );
                s.status = "Redemption pending; carried at cost, pending claims unmarked".into();
            }
            s.processed_ids.insert(key);
            event(
                s,
                o,
                "staking-route",
                format!("{token}; expected net {:.4} USD", gross - total_cost_usd),
            );
        }
        StakingInput::Settle {
            opportunity_id,
            received_eth,
            eth_usd,
            total_cost_usd,
            chain_id,
            transaction_hash,
        } => {
            ensure(
                label(transaction_hash)
                    && *chain_id > 0
                    && pos(*received_eth)
                    && pos(*eth_usd)
                    && nn(*total_cost_usd),
                "Invalid redemption settlement",
            )?;
            let key = format!("route:{chain_id}:{opportunity_id}");
            let p = s
                .positions
                .get(&key)
                .ok_or("Unknown or already settled redemption")?;
            let Position::Redemption {
                cost_usd,
                redeem_after_ms,
                ..
            } = p
            else {
                return Err("Not a redemption".into());
            };
            ensure(
                o.observed_at_ms >= *redeem_after_ms,
                "Redemption has not matured",
            )?;
            let pnl = received_eth * eth_usd - cost_usd;
            s.cash_usd += received_eth * eth_usd;
            realize(s, pnl);
            cost(s, *total_cost_usd);
            s.positions.remove(&key);
            s.status = "Paper redemption settled from supplied claim proceeds".into();
            event(
                s,
                o,
                "settle",
                format!("{transaction_hash}; proceeds recorded"),
            );
        }
    }
    Ok(())
}
fn liquidate(
    s: &mut Sleeve,
    x: &LiquidationInput,
    o: &Observation,
    allowed: bool,
) -> Result<(), String> {
    ensure(
        label(&x.opportunity_id) && label(&x.protocol) && x.chain_id > 0 && x.block_number > 0,
        "Missing liquidation provenance",
    )?;
    ensure(
        pos(x.debt_repayment_usd)
            && pos(x.executable_collateral_sale_usd)
            && nn(x.total_cost_usd)
            && pos(x.health_factor),
        "Invalid liquidation amounts",
    )?;
    let key = format!("liquidation:{}:{}", x.chain_id, x.opportunity_id);
    if s.processed_ids.contains(&key) {
        s.status = "Liquidation already processed".into();
        return Ok(());
    }
    let gross = x.executable_collateral_sale_usd - x.debt_repayment_usd;
    let net = gross - x.total_cost_usd;
    if s.processed_ids.len() >= 100_000 {
        s.status = "Archive required before new liquidation entries".into();
        return Ok(());
    }
    if !allowed
        || !x.atomic
        || !x.simulation_succeeded
        || x.health_factor >= 1.0
        || x.debt_repayment_usd > s.config.max_trade_usd
        || x.debt_repayment_usd + x.total_cost_usd > s.cash_usd
        || net <= x.debt_repayment_usd * s.config.min_edge_bps / 10_000.0
    {
        s.status = "Liquidation rejected: eligibility, atomic simulation, cash or net edge".into();
        return Ok(());
    }
    s.cash_usd += gross;
    realize(s, gross);
    cost(s, x.total_cost_usd);
    s.processed_ids.insert(key);
    s.status = "Paper liquidation completed at supplied executable sale quote".into();
    event(
        s,
        o,
        "liquidate",
        format!("{}; net {net:.4} USD", x.protocol),
    );
    Ok(())
}
