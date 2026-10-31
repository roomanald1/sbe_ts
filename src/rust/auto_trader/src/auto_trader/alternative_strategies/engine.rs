use super::types::*;
use std::collections::{BTreeMap, BTreeSet};
fn ensure(ok: bool, msg: &str) -> Result<(), String> {
    if ok { Ok(()) } else { Err(msg.into()) }
}
fn pos(x: f64) -> bool {
    x.is_finite() && x > 0.0
}
fn nn(x: f64) -> bool {
    x.is_finite() && x >= 0.0
}
fn label(x: &str) -> bool {
    !x.is_empty() && x.len() <= 200 && !x.chars().any(char::is_control)
}
pub fn validate_config(c: &Config) -> Result<(), String> {
    ensure(
        pos(c.max_trade_usd) && c.max_trade_usd <= 1000.0,
        "Trade cap must be in (0, 1000] USD",
    )?;
    ensure(
        pos(c.daily_loss_limit_usd) && c.daily_loss_limit_usd <= 1000.0,
        "Invalid daily loss cap",
    )?;
    ensure(
        nn(c.spot_fee_bps) && c.spot_fee_bps <= 100.0,
        "Invalid spot fees",
    )?;
    ensure(
        nn(c.short_borrow_apr) && c.short_borrow_apr <= 5.0,
        "Invalid short borrow APR",
    )?;
    ensure(
        nn(c.min_edge_bps) && c.min_edge_bps <= 1000.0,
        "Invalid edge buffer",
    )
}
fn quote(q: &Quote, now: u64) -> Result<(), String> {
    ensure(
        pos(q.bid) && pos(q.ask) && q.bid <= q.ask,
        "Invalid or crossed quote",
    )?;
    ensure(
        nn(q.bid_quantity) && nn(q.ask_quantity),
        "Invalid quote depth",
    )?;
    ensure(
        q.timestamp_ms <= now && now - q.timestamp_ms <= 30_000,
        "Quote older than 30 seconds or from future",
    )
}
fn cost(s: &mut Sleeve, v: f64) {
    s.cash_usd -= v;
    s.fees_usd += v;
    realize(s, -v);
}
fn realize(s: &mut Sleeve, v: f64) {
    s.realized_pnl_usd += v;
    s.daily_pnl_usd += v;
}
fn event(s: &mut Sleeve, o: &Observation, action: &str, detail: String) {
    // The final mark in apply validates finiteness before the transaction commits.
    let _ = mark(s);
    s.events.push(Event {
        timestamp_ms: o.observed_at_ms,
        observation_id: o.id.clone(),
        action: action.into(),
        detail,
        cash_usd: s.cash_usd,
        equity_usd: s.equity_usd,
    });
    if s.events.len() > 1000 {
        s.events.drain(..s.events.len() - 1000);
    }
}
fn mark(s: &mut Sleeve) -> Result<(), String> {
    let assets = s
        .positions
        .values()
        .map(|p| match p {
            Position::Spot {
                quantity,
                mark_price,
                ..
            } => quantity * mark_price,
            Position::CoveredCall {
                quantity,
                spot_bid,
                call_ask,
                ..
            } => quantity * (spot_bid - call_ask),
            Position::Redemption { mark_usd, .. } => *mark_usd,
            Position::Liquidity {
                liquidity,
                lower_price,
                upper_price,
                price,
                token0_usd,
                token1_usd,
                accrued_fees_usd,
                exit_cost_usd,
                ..
            } => {
                let (a, b) = lp_amounts(*liquidity, *price, *lower_price, *upper_price);
                a * token0_usd + b * token1_usd + accrued_fees_usd - exit_cost_usd
            }
        })
        .sum::<f64>();
    s.equity_usd = s.cash_usd + assets;
    ensure(s.equity_usd.is_finite(), "Nonfinite equity")?;
    s.peak_equity_usd = s.peak_equity_usd.max(s.equity_usd);
    s.max_drawdown_percent = s
        .max_drawdown_percent
        .min(100.0 * (s.equity_usd / s.peak_equity_usd - 1.0));
    Ok(())
}
pub fn apply(s: &mut Sleeve, o: &Observation, active: bool) -> Result<(), String> {
    ensure(s.id == o.data.id(), "Strategy mismatch")?;
    ensure(
        label(&o.id) && label(&o.source),
        "Observation id/source required",
    )?;
    if s.events.iter().any(|event| event.observation_id == o.id) {
        return Ok(());
    }
    ensure(
        o.observed_at_ms > s.last_observation_ms,
        "Out-of-order observation",
    )?;
    let day = o.observed_at_ms / DAY;
    if day != s.utc_day {
        s.utc_day = day;
        s.daily_pnl_usd = 0.0;
    }
    let allowed = active
        && s.config.enabled
        && s.daily_pnl_usd > -s.config.daily_loss_limit_usd
        && s.equity_usd > 0.0;
    match &o.data {
        Input::ResidualMomentum(x) => residual(s, x, o, allowed, active)?,
        Input::FilteredCoveredCall(x) => covered_call(s, x, o, allowed, active)?,
        Input::StakingRedemptionArbitrage(x) => staking(s, x, o, allowed)?,
        Input::LiquidationKeeper(x) => liquidate(s, x, o, allowed)?,
        Input::AdaptiveStablecoinLiquidity(x) => liquidity(s, x, o, allowed, active)?,
    }
    mark(s)?;
    s.last_observation_ms = o.observed_at_ms;
    if !active {
        s.status = "AutoTrader paused; marks and settlements only".into();
    }
    event(s, o, "observation", s.status.clone());
    Ok(())
}
include!("residual.rs");
include!("covered_call.rs");
include!("onchain.rs");
include!("liquidity.rs");
