use super::*;
use chain::*;
#[derive(Serialize, Deserialize, Default)]
pub(super) struct AaveIndex {
    indexed_to: u64,
    indexed_hash: String,
    first_block: u64,
    borrowers: Vec<String>,
    next_account: usize,
}
async fn reserve_tokens(
    feed: &FeedClient,
    asset: &str,
    block: &str,
) -> anyhow::Result<(String, String)> {
    let words = feed
        .call(AAVE, "getReserveData(address)", &[addr_word(asset)?], block)
        .await?;
    Ok((
        word_address(field(&words, 8)?)?,
        word_address(field(&words, 10)?)?,
    ))
}
async fn index(feed: &FeedClient, ctx: &Context, s: &mut AaveIndex) -> anyhow::Result<()> {
    if s.indexed_to > 0 {
        let old = feed
            .rpc(
                "eth_getBlockByNumber",
                json!([format!("0x{:x}", s.indexed_to), false]),
            )
            .await?;
        if old["hash"].as_str() != Some(&s.indexed_hash) {
            s.indexed_to = s.indexed_to.saturating_sub(32);
        }
    }
    let start = if s.indexed_to == 0 {
        ctx.number.saturating_sub(8192)
    } else {
        s.indexed_to + 1
    };
    if s.first_block == 0 {
        s.first_block = start;
    }
    let topic = feed
        .hash(&format!(
            "0x{}",
            "Borrow(address,address,address,uint256,uint8,uint256,uint16)"
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        ))
        .await?;
    let mut unique = s.borrowers.iter().cloned().collect::<BTreeSet<_>>();
    if let Ok(seeds) = std::env::var("AAVE_PAPER_WATCHLIST") {
        for user in seeds.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            addr_word(user)?;
            unique.insert(user.to_lowercase());
        }
    }
    let mut from = start;
    // Bounded catch-up; state exposes exactly which block interval is indexed.
    let end = ctx.number.min(start.saturating_add(8191));
    while from <= end {
        let to = end.min(from + 2047);
        let logs = feed
            .rpc(
                "eth_getLogs",
                json!([{"address":AAVE,"fromBlock":format!("0x{from:x}"),
            "toBlock":format!("0x{to:x}"),"topics":[topic]}]),
            )
            .await?;
        for log in logs
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Borrow logs missing"))?
        {
            if log["removed"] == true {
                continue;
            }
            let user = log["topics"][2]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Borrow debtor missing"))?;
            let user = word_address(user)?;
            if unique.insert(user.clone()) {
                s.borrowers.push(user);
            }
        }
        from = to + 1;
    }
    for seed in unique {
        if !s.borrowers.contains(&seed) {
            s.borrowers.push(seed);
        }
    }
    if s.borrowers.len() > 5000 {
        s.borrowers.drain(..s.borrowers.len() - 5000);
    }
    if end >= start {
        let header = feed
            .rpc("eth_getBlockByNumber", json!([format!("0x{end:x}"), false]))
            .await?;
        s.indexed_to = end;
        s.indexed_hash = header["hash"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Indexed block hash missing"))?
            .into();
    }
    Ok(())
}
pub(super) async fn build(
    feed: &FeedClient,
    ctx: &Context,
    sleeve: &Value,
    s: &mut AaveIndex,
) -> anyhow::Result<Output> {
    feed.get(
        "/market-data/alternative/simulation-capability",
        &[],
        30_000,
    )
    .await?;
    index(feed, ctx, s).await?;
    let (_, debt_token) = reserve_tokens(feed, WETH, &ctx.tag).await?;
    let (collateral_token, _) = reserve_tokens(feed, USDC, &ctx.tag).await?;
    anyhow::ensure!(
        debt_token != "0x0000000000000000000000000000000000000000"
            && collateral_token != "0x0000000000000000000000000000000000000000",
        "Aave reserve metadata unavailable"
    );
    let mut selected = Vec::new();
    for _ in 0..32.min(s.borrowers.len()) {
        let user = s.borrowers[s.next_account % s.borrowers.len()].clone();
        s.next_account = (s.next_account + 1) % s.borrowers.len();
        selected.push(user);
    }
    let selector = feed.selector("getUserAccountData(address)").await?;
    let calls=selected.iter().map(|user|Ok(("eth_call",json!([{"to":AAVE,"data":format!("0x{selector}{}",addr_word(user)?),"gas":"0x1e8480"},ctx.tag])))).collect::<anyhow::Result<Vec<_>>>()?;
    let values = feed.batch(calls).await?;
    let mut observations = Vec::new();
    let mut unhealthy = 0;
    let mut simulations = 0;
    let mut reverted = 0;
    let cap = sleeve["config"]["max_trade_usd"].as_f64().unwrap_or(100.0);
    let edge = sleeve["config"]["min_edge_bps"].as_f64().unwrap_or(10.0);
    let mut best_net: Option<f64> = None;
    for (user, value) in selected.iter().zip(values) {
        let value = value.map_err(|e| anyhow::anyhow!("Borrower state error: {e}"))?;
        let account_words = words(
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Borrower result not hex"))?,
        )?;
        let health = number(field(&account_words, 5)?)? / 1e18;
        if !(health > 0.0 && health < 1.0) {
            continue;
        }
        unhealthy += 1;
        let user_args = [addr_word(user)?];
        let (debt, collateral) = tokio::join!(
            feed.call(&debt_token, "balanceOf(address)", &user_args, &ctx.tag),
            feed.call(
                &collateral_token,
                "balanceOf(address)",
                &user_args,
                &ctx.tag
            )
        );
        let debt = number(field(&debt?, 0)?)?;
        let collateral = number(field(&collateral?, 0)?)?;
        if debt <= 0.0 || collateral <= 0.0 {
            continue;
        }
        let budget = (0.9 * cap / ctx.eth_usd * 1e18).floor();
        let repayment = if debt <= budget {
            debt
        } else {
            budget.min(debt * 0.5)
        }
        .floor() as u128;
        if repayment == 0 {
            continue;
        }
        simulations += 1;
        let raw = feed
            .post(
                "/market-data/alternative/liquidation-simulation",
                json!({"user":user,"repayment_wei":format!("0x{repayment:x}"),"block":ctx.tag}),
            )
            .await?;
        if !raw.data["error"].is_null() {
            reverted += 1;
            continue;
        }
        let result = words(
            raw.data["result"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Liquidation simulation result missing"))?,
        )?;
        let output = number(field(&result, 0)?)? / 1e18;
        let gas = number(field(&result, 1)?)?;
        let paid = number(field(&result, 2)?)? / 1e18;
        anyhow::ensure!(
            paid > 0.0 && output > 0.0 && gas > 20_000.0,
            "Invalid measured liquidation execution"
        );
        let principal = paid * ctx.eth_usd;
        let proceeds = output * ctx.eth_usd;
        let conversion = 0.0015 * (principal + proceeds);
        let costs = ctx.gas_usd(gas) + conversion;
        let net = proceeds - principal - costs;
        best_net = Some(best_net.map_or(net, |n| n.max(net)));
        let time = now_ms();
        let data = json!({"opportunity_id":format!("aave-usdc-weth:{}:{user}:{repayment}",ctx.hash),
            "chain_id":1,"block_number":ctx.number,"protocol":"Aave V3 Ethereum USDC collateral / WETH debt",
            "health_factor":health,"debt_repayment_usd":principal,"executable_collateral_sale_usd":proceeds,
            "total_cost_usd":costs,"simulation_succeeded":true,"atomic":true});
        observations.push(observation(IDS[3],"FeedHandlers pinned Aave liquidation + Uniswap collateral sale eth_call; Insights net-edge assessment",time,data,&format!("{}:{user}",ctx.hash)));
        if observations.len() >= 4 {
            break;
        }
    }
    let qualifies = best_net.is_some_and(|v| v > cap * edge / 10_000.0);
    Ok(Output::ready(
        IDS[3],
        ctx.timestamp,
        if qualifies {
            "qualified"
        } else {
            "waiting-for-liquidation"
        },
        format!(
            "Aave USDC/WETH: {} indexed borrowers; {} checked, {unhealthy} unhealthy; {simulations} atomic simulations",
            s.borrowers.len(),
            selected.len()
        ),
        json!({"block":ctx.number,"block_hash":ctx.hash,"watchlist_size":s.borrowers.len(),
            "checked_accounts":selected.len(),"unhealthy_accounts":unhealthy,"atomic_simulations":simulations,"reverted_simulations":reverted,
            "indexed_from_block":s.first_block,"indexed_to_block":s.indexed_to,"best_net_usd":best_net,
            "coverage":"Recent borrow-event universe, continuously indexed; up to 32 accounts per scan",
            "funding":"paper WETH balance supplied only through eth_call state override"}),
        observations,
    ))
}
