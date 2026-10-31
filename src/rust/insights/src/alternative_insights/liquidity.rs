use super::*;
use chain::*;
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct LpTracker {
    key: String,
    growth0: String,
    growth1: String,
    cumulative_usd: f64,
    last_block: u64,
}
fn inside(
    global: Word,
    lower_outside: Word,
    upper_outside: Word,
    current: i32,
    lower: i32,
    upper: i32,
) -> Word {
    let below = if current >= lower {
        lower_outside
    } else {
        global.sub(lower_outside)
    };
    let above = if current < upper {
        upper_outside
    } else {
        global.sub(upper_outside)
    };
    global.sub(below).sub(above)
}
fn realized_vol(rows: &Value, now: u64) -> anyhow::Result<(f64, u64)> {
    let mut closes = BTreeMap::new();
    for row in rows
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("Stablecoin hourly history missing"))?
    {
        let Some(time) = row[6].as_u64().filter(|t| *t < now) else {
            continue;
        };
        let price = row[4]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .filter(|v| v.is_finite() && *v > 0.0)
            .ok_or_else(|| anyhow::anyhow!("Invalid stablecoin close"))?;
        closes.insert(time, price);
    }
    let mut points = closes.into_iter().rev().take(25).collect::<Vec<_>>();
    points.reverse();
    anyhow::ensure!(
        points.len() == 25 && points.windows(2).all(|w| w[1].0 - w[0].0 == 3_600_000),
        "Need 25 contiguous closed hourly prices"
    );
    let time = points.last().unwrap().0;
    anyhow::ensure!(now - time <= 2 * 3_600_000, "Stablecoin history stale");
    let returns = points
        .windows(2)
        .map(|p| (p[1].1 / p[0].1).ln())
        .collect::<Vec<_>>();
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance =
        returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
    Ok((variance.sqrt() * 24f64.sqrt(), time))
}
async fn initialized_range(
    feed: &FeedClient,
    ctx: &Context,
    lower: i32,
    upper: i32,
    spacing: i32,
) -> anyhow::Result<(i32, i32)> {
    let lo_word = lower.div_euclid(spacing).div_euclid(256);
    let hi_word = upper.div_euclid(spacing).div_euclid(256);
    let indices = [
        lo_word - 1,
        lo_word,
        lo_word + 1,
        hi_word - 1,
        hi_word,
        hi_word + 1,
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    let mut ticks = BTreeSet::new();
    let requests = indices.iter().map(|word| async {
        Ok::<_, anyhow::Error>((
            *word,
            feed.call(
                LP_POOL,
                "tickBitmap(int16)",
                &[signed(*word as i64)],
                &ctx.tag,
            )
            .await?,
        ))
    });
    for result in futures_util::future::join_all(requests).await {
        let (word, values) = result?;
        let bitmap = Word::parse(field(&values, 0)?)?;
        for bit in 0..256 {
            if bitmap.bit(bit) {
                ticks.insert((word * 256 + bit as i32) * spacing);
            }
        }
    }
    let lower = *ticks
        .range(..=lower)
        .next_back()
        .ok_or_else(|| anyhow::anyhow!("No initialized lower tick near target"))?;
    let upper = *ticks
        .range(upper..)
        .next()
        .ok_or_else(|| anyhow::anyhow!("No initialized upper tick near target"))?;
    Ok((lower, upper))
}
pub(super) async fn build(
    feed: &FeedClient,
    ctx: &Context,
    sleeve: &Value,
    tracker: &mut Option<LpTracker>,
) -> anyhow::Result<Output> {
    let history_params = [("interval", "1h".into()), ("limit", "49".into())];
    let (slot, spacing, token0, token1, history) = tokio::join!(
        feed.call(LP_POOL, "slot0()", &[], &ctx.tag),
        feed.call(LP_POOL, "tickSpacing()", &[], &ctx.tag),
        feed.call(LP_POOL, "token0()", &[], &ctx.tag),
        feed.call(LP_POOL, "token1()", &[], &ctx.tag),
        feed.binance("klines", "USDCUSDT", &history_params)
    );
    anyhow::ensure!(
        word_address(field(&token0?, 0)?)? == USDC && word_address(field(&token1?, 0)?)? == USDT,
        "Pool token order changed"
    );
    let (dec0, dec1) = tokio::join!(
        feed.call(USDC, "decimals()", &[], &ctx.tag),
        feed.call(USDT, "decimals()", &[], &ctx.tag)
    );
    anyhow::ensure!(
        number(field(&dec0?, 0)?)? == 6.0 && number(field(&dec1?, 0)?)? == 6.0,
        "This pool adapter requires six-decimal stablecoins"
    );
    let slot = slot?;
    let sqrt = number(field(&slot, 0)?)? / 79228162514264337593543950336.0;
    let price = sqrt * sqrt;
    let current = Word::parse(field(&slot, 1)?)?.i24();
    anyhow::ensure!(price.is_finite() && price > 0.0, "Invalid pool price");
    let spacing = number(field(&spacing?, 0)?)? as i32;
    anyhow::ensure!(spacing > 0 && spacing <= 200, "Invalid tick spacing");
    let (vol, indicators_at) = realized_vol(&history?.data, now_ms())?;
    let key = format!("1:{LP_POOL}");
    let held = sleeve["positions"]
        .get(&key)
        .filter(|p| p["kind"] == "liquidity");
    let (lower, upper) = if let Some(position) = held {
        let lower = position["tick_lower"]
            .as_i64()
            .map(|v| v as i32)
            .unwrap_or_else(|| {
                (position["lower_price"].as_f64().unwrap_or(0.0).ln() / 1.0001f64.ln()).round()
                    as i32
            });
        let upper = position["tick_upper"]
            .as_i64()
            .map(|v| v as i32)
            .unwrap_or_else(|| {
                (position["upper_price"].as_f64().unwrap_or(0.0).ln() / 1.0001f64.ln()).round()
                    as i32
            });
        (lower, upper)
    } else {
        let width = (2.0 * vol).clamp(0.0005, 0.02);
        let lower = ((price * (1.0 - width)).ln() / 1.0001f64.ln() / spacing as f64).floor() as i32
            * spacing;
        let upper = ((price * (1.0 + width)).ln() / 1.0001f64.ln() / spacing as f64).ceil() as i32
            * spacing;
        initialized_range(feed, ctx, lower, upper, spacing).await?
    };
    anyhow::ensure!(
        lower < upper && lower % spacing == 0 && upper % spacing == 0,
        "Invalid LP tick range"
    );
    let lower_price = 1.0001f64.powi(lower);
    let upper_price = 1.0001f64.powi(upper);
    if held.is_none() {
        anyhow::ensure!(
            lower_price < price
                && price < upper_price
                && lower_price >= price * 0.95
                && upper_price <= price * 1.05,
            "Initialized range too wide"
        );
    }
    let lower_args = [signed(lower as i64)];
    let upper_args = [signed(upper as i64)];
    let (lo, hi, g0, g1) = tokio::join!(
        feed.call(LP_POOL, "ticks(int24)", &lower_args, &ctx.tag),
        feed.call(LP_POOL, "ticks(int24)", &upper_args, &ctx.tag),
        feed.call(LP_POOL, "feeGrowthGlobal0X128()", &[], &ctx.tag),
        feed.call(LP_POOL, "feeGrowthGlobal1X128()", &[], &ctx.tag)
    );
    let lo = lo?;
    let hi = hi?;
    anyhow::ensure!(
        number(field(&lo, 7)?)? == 1.0 && number(field(&hi, 7)?)? == 1.0,
        "Range boundary no longer initialized; refusing fabricated LP fees"
    );
    let growth0 = inside(
        Word::parse(field(&g0?, 0)?)?,
        Word::parse(field(&lo, 2)?)?,
        Word::parse(field(&hi, 2)?)?,
        current,
        lower,
        upper,
    );
    let growth1 = inside(
        Word::parse(field(&g1?, 0)?)?,
        Word::parse(field(&lo, 3)?)?,
        Word::parse(field(&hi, 3)?)?,
        current,
        lower,
        upper,
    );
    let range_key = format!("{key}:{lower}:{upper}");
    let mut baseline_gap = false;
    let cumulative = if held.is_some() && tracker.as_ref().is_some_and(|t| t.key == range_key) {
        let old = tracker.as_ref().unwrap();
        anyhow::ensure!(
            ctx.number >= old.last_block,
            "LP chain head moved backwards"
        );
        let d0 = growth0.sub(Word::parse(&old.growth0)?).float() / 2f64.powi(128);
        let d1 = growth1.sub(Word::parse(&old.growth1)?).float() / 2f64.powi(128);
        anyhow::ensure!(
            d0.is_finite() && d1.is_finite() && d0 < 1.0 && d1 < 1.0,
            "LP fee counter discontinuity"
        );
        old.cumulative_usd + d0 * ctx.usdc_usd + d1 * ctx.usdt_usd
    } else {
        baseline_gap = held.is_some();
        held.and_then(|p| p["last_fee_growth"].as_f64())
            .unwrap_or(0.0)
    };
    *tracker = Some(LpTracker {
        key: range_key,
        growth0: growth0.hex(),
        growth1: growth1.hex(),
        cumulative_usd: cumulative,
        last_block: ctx.number,
    });
    let enter_cost = ctx.gas_usd(480_000.0);
    let exit_cost = ctx.gas_usd(265_000.0);
    let cap = sleeve["config"]["max_trade_usd"].as_f64().unwrap_or(100.0);
    let cost_gate = enter_cost + exit_cost <= cap * 0.01;
    let depeg = !(0.98..=1.02).contains(&ctx.usdc_usd) || !(0.98..=1.02).contains(&ctx.usdt_usd);
    let time = now_ms();
    let data = json!({"chain_id":1,"pool":LP_POOL,"block_number":ctx.number,"price":price,
        "token0_usd":ctx.usdc_usd,"token1_usd":ctx.usdt_usd,"fee_growth_inside_usd":cumulative,
        "fee_lower_price":lower_price,"fee_upper_price":upper_price,"prior_daily_volatility":vol,
        "indicators_at_ms":indicators_at,"enter_cost_usd":enter_cost,"exit_cost_usd":exit_cost,
        "range_lower_price":lower_price,"range_upper_price":upper_price,"tick_lower":lower,"tick_upper":upper,"tick_spacing":spacing});
    Ok(Output::ready(
        IDS[4],
        ctx.timestamp,
        if held.is_some() {
            "held-range"
        } else if cost_gate && !depeg {
            "qualified"
        } else {
            "waiting-for-costs-or-peg"
        },
        format!(
            "USDC/USDT initialized ticks {lower}–{upper}; round-trip gas estimate {:.4} USD",
            enter_cost + exit_cost
        ),
        json!({"block":ctx.number,"block_hash":ctx.hash,"pool":LP_POOL,"tick":current,"tick_lower":lower,"tick_upper":upper,
            "prior_daily_volatility":vol,"token0_usd":ctx.usdc_usd,"token1_usd":ctx.usdt_usd,
            "cumulative_fee_growth_usd":cumulative,"fee_baseline_gap":baseline_gap,
            "fee_source":"Exact modulo-U256 feeGrowthInside for initialized boundaries; USD conversion of increments only",
            "gas_model":"Live gas price; conservative mint/burn/approval gas budgets"}),
        vec![observation(
            IDS[4],
            "FeedHandlers pinned Uniswap V3 state, Chainlink marks, closed stablecoin history; Insights adaptive range/fee growth",
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
    fn fee_growth_inside_handles_all_tick_sides() {
        let n = Word::from_u64;
        assert_eq!(inside(n(100), n(20), n(30), 0, -10, 10), n(50));
        assert_eq!(inside(n(100), n(80), n(30), -11, -10, 10), n(50));
        assert_eq!(inside(n(100), n(20), n(70), 11, -10, 10), n(50));
    }
    #[test]
    fn stablecoin_volatility_uses_closed_hours_only() {
        let now = 100 * DAY + 1000;
        let mut rows = (0..25)
            .map(|i| json!([0, 0, 0, 0, "1.0", 0, now - 3_600_000 * (25 - i)]))
            .collect::<Vec<_>>();
        rows.push(json!([0, 0, 0, 0, "100.0", 0, now + 3_600_000]));
        let (vol, time) = realized_vol(&json!(rows), now).unwrap();
        assert_eq!(vol, 0.0);
        assert!(time < now);
    }
}
