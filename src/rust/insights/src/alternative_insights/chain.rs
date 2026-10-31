use super::*;
pub(super) const USDC: &str = "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48";
pub(super) const USDT: &str = "0xdac17f958d2ee523a2206206994597c13d831ec7";
pub(super) const WETH: &str = "0xc02aaa39b223fe8d0a0e5c4f27ead9083c756cc2";
pub(super) const AAVE: &str = "0x87870bca3f3fd6335c3f4ce8392d69350b4fa4e2";
pub(super) const LP_POOL: &str = "0x3416cf6c708da44db2624d63ea0aaef7113527c6";
#[derive(Clone)]
pub(super) struct Context {
    pub number: u64,
    pub tag: String,
    pub hash: String,
    pub timestamp: u64,
    pub gas_price: f64,
    pub eth_usd: f64,
    pub usdc_usd: f64,
    pub usdt_usd: f64,
}
impl Context {
    pub async fn load(feed: &FeedClient) -> anyhow::Result<Self> {
        let rows = feed
            .batch(vec![
                ("eth_chainId", json!([])),
                ("eth_getBlockByNumber", json!(["latest", false])),
                ("eth_gasPrice", json!([])),
            ])
            .await?;
        let chain = rows[0].as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        anyhow::ensure!(
            chain.as_str() == Some("0x1"),
            "Native strategies require Ethereum mainnet"
        );
        let header = rows[1].as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let tag = header["number"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Block number missing"))?
            .to_owned();
        let number = hex_u64(&tag)?;
        let timestamp = hex_u64(
            header["timestamp"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Block timestamp missing"))?,
        )? * 1000;
        anyhow::ensure!(
            timestamp <= now_ms() && now_ms() - timestamp < 60_000,
            "Ethereum head stale or future-dated"
        );
        let gas = rows[2].as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
        let gas_price = hex_u64(
            gas.as_str()
                .ok_or_else(|| anyhow::anyhow!("Gas price missing"))?,
        )? as f64;
        let (eth, usdc, usdt) = tokio::join!(
            oracle(
                feed,
                "0x5f4ec3df9cbd43714fe2740f5e3616155c5b8419",
                &tag,
                2 * 3_600_000
            ),
            oracle(
                feed,
                "0x8fffffd4afb6115b954bd326cbe7b4ba576818f6",
                &tag,
                25 * 3_600_000
            ),
            oracle(
                feed,
                "0x3e7d1eab13ad0104d2750b8863b489d65364e32d",
                &tag,
                25 * 3_600_000
            )
        );
        Ok(Self {
            number,
            tag,
            hash: header["hash"].as_str().unwrap_or_default().into(),
            timestamp,
            gas_price,
            eth_usd: eth?,
            usdc_usd: usdc?,
            usdt_usd: usdt?,
        })
    }
    pub fn gas_usd(&self, execution_gas: f64) -> f64 {
        // Includes an intrinsic/calldata allowance and a 25% inclusion buffer.
        (execution_gas + 30_000.0) * 1.25 * self.gas_price / 1e18 * self.eth_usd
    }
}
async fn oracle(
    feed: &FeedClient,
    address: &str,
    block: &str,
    max_age: u64,
) -> anyhow::Result<f64> {
    let (round, decimals) = tokio::join!(
        feed.call(address, "latestRoundData()", &[], block),
        feed.call(address, "decimals()", &[], block)
    );
    let round = round?;
    let decimals = decimals?;
    let updated = number(field(&round, 3)?)? as u64 * 1000;
    let now = now_ms();
    anyhow::ensure!(
        updated <= now && now - updated <= max_age,
        "Oracle {address} stale"
    );
    anyhow::ensure!(
        Word::parse(field(&round, 4)?)?.ge(Word::parse(field(&round, 0)?)?),
        "Oracle round incomplete"
    );
    let raw = Word::parse(field(&round, 1)?)?;
    anyhow::ensure!(!raw.bit(255), "Oracle answer negative");
    let decimals = number(field(&decimals, 0)?)?;
    anyhow::ensure!(
        (0.0..=18.0).contains(&decimals),
        "Unsupported oracle decimals"
    );
    let answer = raw.float() / 10f64.powf(decimals);
    anyhow::ensure!(answer.is_finite() && answer > 0.0, "Oracle answer invalid");
    Ok(answer)
}
pub(super) async fn staking(
    feed: &FeedClient,
    ctx: &Context,
    sleeve: &Value,
) -> anyhow::Result<Output> {
    feed.get(
        "/market-data/alternative/simulation-capability",
        &[],
        30_000,
    )
    .await?;
    let cap = sleeve["config"]["max_trade_usd"].as_f64().unwrap_or(100.0);
    let input_wei = (0.95 * cap / ctx.eth_usd * 1e18).floor() as u128;
    anyhow::ensure!(input_wei > 0, "Staking trade cap too small");
    let raw = feed
        .post(
            "/market-data/alternative/staking-simulation",
            json!({"input_wei":format!("0x{input_wei:x}"),"block":ctx.tag}),
        )
        .await?;
    if !raw.data["error"].is_null() {
        return Ok(Output::ready(
            IDS[2],
            ctx.timestamp,
            "waiting-for-route",
            "Lido/Curve route currently reverts; no paper entry".into(),
            json!({"block":ctx.number,"block_hash":ctx.hash,"route_error":raw.data["error"]}),
            vec![],
        ));
    }
    let result = words(
        raw.data["result"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("Staking simulation result missing"))?,
    )?;
    let output_eth = number(field(&result, 0)?)? / 1e18;
    let gas = number(field(&result, 1)?)?;
    let minted = number(field(&result, 2)?)? / 1e18;
    let input_eth = input_wei as f64 / 1e18;
    anyhow::ensure!(
        output_eth > 0.0 && minted > 0.0 && gas > 20_000.0,
        "Invalid measured staking route"
    );
    let gas_cost = ctx.gas_usd(gas);
    let conversion_cost = 0.0015 * (input_eth + output_eth) * ctx.eth_usd;
    let costs = gas_cost + conversion_cost;
    let net = (output_eth - input_eth) * ctx.eth_usd - costs;
    let buffer = sleeve["config"]["min_edge_bps"].as_f64().unwrap_or(10.0);
    let qualified = net > input_eth * ctx.eth_usd * buffer / 10_000.0;
    let time = now_ms();
    let data = json!({"action":"open","opportunity_id":format!("lido-curve:{}:{input_wei}",ctx.hash),
        "token":"stETH","input_eth":input_eth,"token_quantity":minted,"output_eth":output_eth,"eth_usd":ctx.eth_usd,
        "total_cost_usd":costs,"atomic":true,"redeem_after_ms":time,"chain_id":1,"block_number":ctx.number});
    Ok(Output::ready(
        IDS[2],
        ctx.timestamp,
        if qualified {
            "qualified"
        } else {
            "waiting-for-net-edge"
        },
        format!("Atomic Lido mint → Curve sale; net {net:.4} USD after measured gas"),
        json!({"block":ctx.number,"block_hash":ctx.hash,"input_eth":input_eth,"output_eth":output_eth,"execution_gas":gas,
            "cost_usd":costs,"gas_cost_usd":gas_cost,"conversion_cost_usd":conversion_cost,"expected_net_usd":net,"route":"atomic-mint-sale","simulation":"eth_call state override; no transaction broadcast"}),
        vec![observation(
            IDS[2],
            "FeedHandlers pinned Ethereum EVM simulation; Insights staking route/net-edge assessment",
            time,
            data,
            &ctx.hash,
        )],
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gas_buffer_includes_intrinsic_cost_and_does_not_assume_free_execution() {
        let c = Context {
            number: 1,
            tag: "0x1".into(),
            hash: "x".into(),
            timestamp: 1,
            gas_price: 1e9,
            eth_usd: 2000.0,
            usdc_usd: 1.0,
            usdt_usd: 1.0,
        };
        assert!((c.gas_usd(100_000.0) - 0.325).abs() < 1e-10);
    }
}
