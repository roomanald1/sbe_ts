// Daily strategy signals are calculated in Insights from closed FeedHandlers candles.
const DAILY_PAIRS: [(&str, &str); 7] = [
    ("BTC-USDT", "BTCUSDT"),
    ("ETH-USDT", "ETHUSDT"),
    ("BNB-USDT", "BNBUSDT"),
    ("XRP-USDT", "XRPUSDT"),
    ("SOL-USDT", "SOLUSDT"),
    ("ADA-USDT", "ADAUSDT"),
    ("DOGE-USDT", "DOGEUSDT"),
];
const TREND_PORTFOLIO_PAIRS: [(&str, &str); 16] = [
    ("BTC-USDT", "BTCUSDT"), ("ETH-USDT", "ETHUSDT"), ("BNB-USDT", "BNBUSDT"),
    ("XRP-USDT", "XRPUSDT"), ("ADA-USDT", "ADAUSDT"), ("SOL-USDT", "SOLUSDT"),
    ("DOGE-USDT", "DOGEUSDT"), ("LTC-USDT", "LTCUSDT"), ("TRX-USDT", "TRXUSDT"),
    ("LINK-USDT", "LINKUSDT"), ("BCH-USDT", "BCHUSDT"), ("XLM-USDT", "XLMUSDT"),
    ("ETC-USDT", "ETCUSDT"), ("FIL-USDT", "FILUSDT"), ("AVAX-USDT", "AVAXUSDT"),
    ("DOT-USDT", "DOTUSDT"),
];
const DAILY_STRATEGIES: [&str; 11] = [
    "donchian-55-btc200",
    "donchian-20-sma200",
    "donchian-20-btc200",
    "trend-sleeves-btc200",
    "trend-50-200",
    "relative-strength-90-sma200",
    "risk-adjusted-monthly-momentum",
    "rsi2-oversold-rebound",
    "donchian-20-btc200-hold3",
    "risk-adjusted-trend-portfolio",
    "donchian-10-btc200",
];

#[derive(Clone, Deserialize)]
struct InsightDailyBar {
    close_time_ms: u64,
    high: f64,
    close: f64,
}

#[derive(Clone)]
struct InsightBtcRegime {
    close: f64,
    sma: f64,
    window_days: usize,
    risk_on: bool,
}

struct InsightDailyEvaluation {
    primary_rule: String,
    primary_level: f64,
    primary_passed: bool,
    secondary_rule: Option<String>,
    secondary_level: Option<f64>,
    btc_regime: Option<InsightBtcRegime>,
    strategy_order_size_multiplier: Option<f64>,
    monthly_anchor_close_time_ms: Option<u64>,
}

fn insight_daily_sma(bars: &[InsightDailyBar], days: usize) -> anyhow::Result<f64> {
    if days == 0 || bars.len() < days {
        anyhow::bail!("Need {days} closed daily candles");
    }
    Ok(bars[bars.len() - days..]
        .iter()
        .map(|bar| bar.close)
        .sum::<f64>()
        / days as f64)
}

fn insight_prior_high(bars: &[InsightDailyBar], days: usize) -> anyhow::Result<f64> {
    if days == 0 || bars.len() <= days {
        anyhow::bail!("Need {days} prior daily candles");
    }
    Ok(bars[bars.len() - days - 1..bars.len() - 1]
        .iter()
        .map(|bar| bar.high)
        .fold(f64::NEG_INFINITY, f64::max))
}

fn insight_rsi(bars: &[InsightDailyBar], period: usize) -> f64 {
    let period = period.max(2);
    if bars.len() <= period {
        return 50.0;
    }
    let mut gains = 0.0;
    let mut losses = 0.0;
    for pair in bars[bars.len() - period - 1..].windows(2) {
        let delta = pair[1].close - pair[0].close;
        if delta > 0.0 {
            gains += delta;
        } else {
            losses -= delta;
        }
    }
    if losses == 0.0 {
        return 100.0;
    }
    100.0 - 100.0 / (1.0 + gains / losses)
}

fn insight_parameter(params: &BTreeMap<String, f64>, key: &str, default: f64) -> f64 {
    params
        .get(key)
        .copied()
        .filter(|value| value.is_finite())
        .unwrap_or(default)
}

fn insight_btc_regime(
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    days: usize,
) -> anyhow::Result<InsightBtcRegime> {
    let btc = candles
        .get("BTCUSDT")
        .ok_or_else(|| anyhow::anyhow!("BTC candles missing"))?;
    let close = btc
        .last()
        .ok_or_else(|| anyhow::anyhow!("BTC candles empty"))?
        .close;
    let sma = insight_daily_sma(btc, days)?;
    Ok(InsightBtcRegime {
        close,
        sma,
        window_days: days,
        risk_on: close > sma,
    })
}

const INSIGHT_DAY_MS: u64 = 86_400_000;
const SCOUT_PARAMETER_SAMPLE_INTERVAL_MS: u64 = 4 * 60 * 60 * 1_000;

fn insight_utc_month_id(timestamp_ms: u64) -> i64 {
    let days = (timestamp_ms / INSIGHT_DAY_MS) as i64;
    let shifted = days + 719_468;
    let era = if shifted >= 0 { shifted } else { shifted - 146_096 } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era = (day_of_era - day_of_era / 1_460 + day_of_era / 36_524
        - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era
        - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    year * 12 + month - 1
}

fn insight_realized_daily_volatility(bars: &[InsightDailyBar], days: usize) -> Option<f64> {
    if days < 2 || bars.len() <= days {
        return None;
    }
    let returns = bars[bars.len() - days - 1..]
        .windows(2)
        .map(|pair| pair[1].close / pair[0].close - 1.0)
        .collect::<Vec<_>>();
    if returns.len() != days || returns.iter().any(|value| !value.is_finite()) {
        return None;
    }
    let mean = returns.iter().sum::<f64>() / returns.len() as f64;
    let variance = returns.iter().map(|value| (value - mean).powi(2)).sum::<f64>()
        / (returns.len() - 1) as f64;
    (variance.is_finite() && variance > 0.0).then(|| variance.sqrt())
}

fn insight_trend_portfolio_weights(
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    profiles: &serde_json::Value,
) -> anyhow::Result<(InsightBtcRegime, BTreeMap<String, f64>)> {
    let parameters = insight_profile_parameters(profiles, "BTC-USDT", "risk-adjusted-trend-portfolio");
    let regime_days = insight_parameter(&parameters, "trend_portfolio_btc_regime_days", 200.0).round().max(2.0) as usize;
    let lookback_days = insight_parameter(&parameters, "trend_portfolio_lookback_days", 160.0).round().max(16.0) as usize;
    let volatility_days = insight_parameter(&parameters, "trend_portfolio_volatility_days", 30.0).round().max(2.0) as usize;
    let weight_cap = insight_parameter(&parameters, "trend_portfolio_weight_cap", 0.25).clamp(0.05, 1.0);
    let regime = insight_btc_regime(candles, regime_days)?;
    let mut raw = BTreeMap::<String, f64>::new();
    if regime.risk_on {
        for &(_, pair) in &TREND_PORTFOLIO_PAIRS {
            let Some(bars) = candles.get(pair) else { continue };
            if bars.len() <= lookback_days { continue; }
            let close = bars.last().expect("nonempty history").close;
            let score = [1, 2, 4, 8, 16].iter()
                .map(|scale| (lookback_days / scale).max(2))
                .map(|days| close / bars[bars.len() - 1 - days].close - 1.0)
                .sum::<f64>() / 5.0;
            let Some(volatility) = insight_realized_daily_volatility(bars, volatility_days) else { continue };
            let weight = score / volatility;
            if score.is_finite() && score > 0.0 && weight.is_finite() && weight > 0.0 {
                raw.insert(pair.to_owned(), weight);
            }
        }
    }
    let mut weights = BTreeMap::<String, f64>::new();
    let mut remaining = 1.0;
    while !raw.is_empty() && remaining > 1e-12 {
        let total = raw.values().sum::<f64>();
        if !total.is_finite() || total <= 0.0 { break; }
        let capped = raw.iter().filter_map(|(pair, score)|
            (*score / total * remaining >= weight_cap).then_some(pair.clone())
        ).collect::<Vec<_>>();
        if capped.is_empty() {
            for (pair, score) in raw {
                weights.insert(pair, score / total * remaining);
            }
            break;
        }
        for pair in capped {
            weights.insert(pair.clone(), weight_cap);
            remaining -= weight_cap;
            raw.remove(&pair);
        }
    }
    Ok((regime, weights))
}

#[derive(Clone)]
struct InsightRiskAdjustedCandidate {
    symbol: &'static str,
    momentum: f64,
    daily_volatility: f64,
    score: f64,
    order_size_multiplier: f64,
}

struct InsightRiskAdjustedSelection {
    btc_regime: InsightBtcRegime,
    anchor_close_time_ms: u64,
    candidate: Option<InsightRiskAdjustedCandidate>,
    ranked_candidates: Vec<InsightRiskAdjustedCandidate>,
}

fn insight_risk_adjusted_monthly_selection(
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    profiles: &serde_json::Value,
    now: u64,
) -> anyhow::Result<InsightRiskAdjustedSelection> {
    let parameters = insight_profile_parameters(profiles, "BTC-USDT", "risk-adjusted-monthly-momentum");
    let regime_days = insight_parameter(&parameters, "btc_regime_sma_days", 200.0).round().max(2.0) as usize;
    let trend_days = insight_parameter(&parameters, "trend_sma_days", 100.0).round().max(2.0) as usize;
    let momentum_days = insight_parameter(&parameters, "monthly_momentum_lookback_days", 90.0).round().max(2.0) as usize;
    let volatility_days = insight_parameter(&parameters, "monthly_momentum_volatility_days", 60.0).round().max(2.0) as usize;
    let volatility_target = insight_parameter(&parameters, "monthly_momentum_volatility_target", 0.40).clamp(0.05, 1.0);
    let current_month = insight_utc_month_id(now);
    let btc = candles.get("BTCUSDT").ok_or_else(|| anyhow::anyhow!("BTC candles missing"))?;
    let btc_anchor_index = btc.iter().rposition(|bar| insight_utc_month_id(bar.close_time_ms) < current_month)
        .ok_or_else(|| anyhow::anyhow!("No completed daily candle before this month"))?;
    let btc_history = &btc[..=btc_anchor_index];
    let btc_close = btc_history.last().expect("anchor index exists").close;
    let btc_sma = insight_daily_sma(btc_history, regime_days)?;
    let btc_anchor_time = btc_history.last().expect("anchor index exists").close_time_ms;
    let btc_regime = InsightBtcRegime {
        close: btc_close,
        sma: btc_sma,
        window_days: regime_days,
        risk_on: btc_close > btc_sma,
    };
    if !btc_regime.risk_on {
        return Ok(InsightRiskAdjustedSelection {
            btc_regime,
            anchor_close_time_ms: btc_anchor_time,
            candidate: None,
            ranked_candidates: Vec::new(),
        });
    }


    let mut candidates = Vec::<InsightRiskAdjustedCandidate>::new();
    for &(symbol, pair) in &DAILY_PAIRS {
        let Some(bars) = candles.get(pair) else { continue };
        let Some(anchor_index) = bars.iter().rposition(|bar| insight_utc_month_id(bar.close_time_ms) < current_month) else { continue };
        let history = &bars[..=anchor_index];
        if history.last().is_none_or(|bar| bar.close_time_ms != btc_anchor_time) || history.len() < trend_days.max(momentum_days + 1) {
            continue;
        }
        let close = history.last().expect("anchor index exists").close;
        let own_sma100 = insight_daily_sma(history, trend_days)?;
        if close <= own_sma100 {
            continue;
        }
        let momentum_reference = history[history.len() - momentum_days - 1].close;
        let momentum = close / momentum_reference - 1.0;
        if !momentum.is_finite() || momentum <= 0.0 {
            continue;
        }
        let Some(daily_volatility) = insight_realized_daily_volatility(history, volatility_days) else { continue };
        let score = momentum / daily_volatility;
        let annualized_volatility = daily_volatility * 365.0_f64.sqrt();
        let order_size_multiplier = (volatility_target / annualized_volatility).clamp(0.0, 1.0);
        if !score.is_finite() || !order_size_multiplier.is_finite() || order_size_multiplier <= 0.0 {
            continue;
        }
        candidates.push(InsightRiskAdjustedCandidate {
            symbol,
            momentum,
            daily_volatility,
            score,
            order_size_multiplier,
        });
    }
    candidates.sort_by(|left, right| right.score.total_cmp(&left.score).then_with(|| right.symbol.cmp(left.symbol)));
    let candidate = candidates.first().cloned();
    Ok(InsightRiskAdjustedSelection {
        btc_regime,
        anchor_close_time_ms: btc_anchor_time,
        candidate,
        ranked_candidates: candidates,
    })
}

fn insight_relative_strength_ranking(
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    profiles: &serde_json::Value,
) -> Vec<(String, f64)> {
    let mut ranking = DAILY_PAIRS.iter().filter_map(|(symbol, pair)| {
        let bars = candles.get(*pair)?;
        let parameters = insight_profile_parameters(profiles, symbol, "relative-strength-90-sma200");
        let rotation_days = insight_parameter(&parameters, "relative_strength_days", 90.0).round().max(1.0) as usize;
        let trend_days = insight_parameter(&parameters, "trend_sma_days", 200.0).round().max(1.0) as usize;
        if bars.len() < trend_days.max(rotation_days + 1) { return None; }
        let close = bars.last()?.close;
        if close <= insight_daily_sma(bars, trend_days).ok()? { return None; }
        let reference = bars.get(bars.len().checked_sub(rotation_days + 1)?)?.close;
        Some(((*symbol).to_owned(), close / reference - 1.0))
    }).collect::<Vec<_>>();
    ranking.sort_by(|left, right| right.1.total_cmp(&left.1).then_with(|| right.0.cmp(&left.0)));
    ranking
}

fn insight_evaluate_daily(
    strategy: &str,
    symbol: &str,
    pair: &str,
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    params: &BTreeMap<String, f64>,
    profiles: &serde_json::Value,
    now: u64,
) -> anyhow::Result<InsightDailyEvaluation> {
    let bars = candles
        .get(pair)
        .ok_or_else(|| anyhow::anyhow!("{pair} candles missing"))?;
    let close = bars
        .last()
        .ok_or_else(|| anyhow::anyhow!("{pair} candles empty"))?
        .close;
    let days =
        |key, default: f64| insight_parameter(params, key, default).round().max(1.0) as usize;
    match strategy {
        "donchian-55-btc200" => {
            let lookback = days("breakout_lookback_days", 55.0);
            let regime = insight_btc_regime(candles, days("btc_trend_days", 200.0))?;
            let level = insight_prior_high(bars, lookback)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Prior {lookback}-day high"),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "donchian-20-btc200" => {
            let lookback = days("breakout_lookback_days", 20.0);
            let regime = insight_btc_regime(candles, days("btc_trend_days", 200.0))?;
            let level = insight_prior_high(bars, lookback)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Prior {lookback}-day high"),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "donchian-20-btc200-hold3" => {
            let lookback = days("breakout_lookback_days", 20.0);
            let hold_days = days("minimum_hold_days", 3.0);
            let regime = insight_btc_regime(candles, days("btc_trend_days", 200.0))?;
            let level = insight_prior_high(bars, lookback)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Prior {lookback}-day high; minimum {hold_days}-day hold"),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: Some(0.25),
                monthly_anchor_close_time_ms: None,
            })
        }
        "donchian-10-btc200" => {
            let lookback = days("breakout_lookback_days", 10.0);
            let regime = insight_btc_regime(candles, days("btc_trend_days", 200.0))?;
            let level = insight_prior_high(bars, lookback)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Prior {lookback}-day high"),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "risk-adjusted-trend-portfolio" => {
            let parameters = insight_profile_parameters(profiles, "BTC-USDT", strategy);
            let weight_cap = insight_parameter(&parameters, "trend_portfolio_weight_cap", 0.25).clamp(0.05, 1.0);
            let (regime, weights) = insight_trend_portfolio_weights(candles, profiles)?;
            let weight = weights.get(pair).copied();
            let active = weight.is_some();
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Positive trend return score over configurable windows, inverse volatility weighted; per asset cap {weight_cap:.2}"),
                primary_level: weight.unwrap_or(0.0),
                primary_passed: active,
                secondary_rule: Some(format!("BTC above {}-day SMA", regime.window_days)),
                secondary_level: Some(regime.sma),
                btc_regime: Some(regime),
                strategy_order_size_multiplier: weight,
                monthly_anchor_close_time_ms: None,
            })
        }
        "donchian-20-sma200" => {
            let lookback = days("breakout_lookback_days", 20.0);
            let trend_days = days("trend_sma_days", 200.0);
            let level = insight_prior_high(bars, lookback)?;
            let trend = insight_daily_sma(bars, trend_days)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Prior {lookback}-day high"),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: Some(format!("Own {trend_days}-day SMA")),
                secondary_level: Some(trend),
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "trend-sleeves-btc200" => {
            let regime = insight_btc_regime(candles, days("btc_regime_sma_days", 200.0))?;
            let trend_days = if symbol == "BNB-USDT" {
                days("bnb_sma_days", 100.0)
            } else {
                days("btc_eth_sma_days", 50.0)
            };
            let trend = insight_daily_sma(bars, trend_days)?;
            let passed = close > trend;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Own {trend_days}-day SMA"),
                primary_level: trend,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "trend-50-200" => {
            let fast_days = days("fast_sma_days", 50.0);
            let slow_days = days("slow_sma_days", 200.0);
            let fast = insight_daily_sma(bars, fast_days)?;
            let slow = insight_daily_sma(bars, slow_days)?;
            let passed = close > fast;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("Own {fast_days}-day SMA"),
                primary_level: fast,
                primary_passed: passed,
                secondary_rule: Some(format!("Own {slow_days}-day SMA")),
                secondary_level: Some(slow),
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "risk-adjusted-monthly-momentum" => {
            let parameters = insight_profile_parameters(profiles, "BTC-USDT", strategy);
            let selection = insight_risk_adjusted_monthly_selection(candles, profiles, now)?;
            let trend_days = insight_parameter(&parameters, "trend_sma_days", 100.0).round().max(2.0) as usize;
            let own_sma = insight_daily_sma(bars, trend_days)?;
            let candidate = selection.candidate.as_ref();
            let own_candidate = selection.ranked_candidates.iter().find(|item| item.symbol == symbol);
            let active = candidate.is_some_and(|selected| selected.symbol == symbol);
            let momentum_days = insight_parameter(&parameters, "monthly_momentum_lookback_days", 90.0).round().max(2.0) as usize;
            let volatility_days = insight_parameter(&parameters, "monthly_momentum_volatility_days", 60.0).round().max(2.0) as usize;
            let primary_rule = candidate.map_or_else(
                || format!("Own {trend_days}-day SMA; no eligible monthly leader"),
                |selected| format!(
                    "Own {trend_days}-day SMA; rank by {momentum_days}-day return / {volatility_days}-day volatility; selected score {:.3} (return {:.2}%, vol {:.2}%)",
                    selected.score,
                    selected.momentum * 100.0,
                    selected.daily_volatility * 365.0_f64.sqrt() * 100.0,
                ),
            );
            Ok(InsightDailyEvaluation {
                primary_rule,
                primary_level: own_sma,
                primary_passed: active,
                secondary_rule: None,
                secondary_level: None,
                btc_regime: Some(selection.btc_regime),
                strategy_order_size_multiplier: own_candidate.map(|selected| selected.order_size_multiplier),
                monthly_anchor_close_time_ms: Some(selection.anchor_close_time_ms),
            })
        }
        "relative-strength-90-sma200" => {
            let rotation_days = days("relative_strength_days", 90.0);
            let trend_days = days("trend_sma_days", 200.0);
            let leader = DAILY_PAIRS
                .iter()
                .filter_map(|(candidate, candidate_pair)| {
                    let history = candles.get(*candidate_pair)?;
                    if history.len() < trend_days.max(rotation_days + 1) {
                        return None;
                    }
                    let asset_close = history.last()?.close;
                    if asset_close <= insight_daily_sma(history, trend_days).ok()? {
                        return None;
                    }
                    Some((
                        asset_close / history[history.len() - rotation_days - 1].close - 1.0,
                        *candidate,
                    ))
                })
                .max_by(|left, right| left.0.total_cmp(&right.0))
                .map(|(_, symbol)| symbol);
            let reference = bars
                .get(bars.len().saturating_sub(rotation_days + 1))
                .ok_or_else(|| anyhow::anyhow!("Not enough candles for rotation lookback"))?
                .close;
            let trend = insight_daily_sma(bars, trend_days)?;
            let passed = leader == Some(symbol);
            Ok(InsightDailyEvaluation {
                primary_rule: format!("{rotation_days}-day return vs eligible assets"),
                primary_level: reference,
                primary_passed: passed,
                secondary_rule: Some(format!("Own {trend_days}-day SMA")),
                secondary_level: Some(trend),
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "rsi2-oversold-rebound" => {
            let trend_days = days("trend_sma_days", 200.0);
            let rsi_days = days("rsi_period_days", 2.0);
            let entry_max = insight_parameter(params, "rsi_entry_max", 10.0);
            let trend = insight_daily_sma(bars, trend_days)?;
            let rsi = insight_rsi(bars, rsi_days);
            // The maximum hold window belongs to exits, not delayed entries after RSI rebounds.
            let entry_passed = rsi <= entry_max;
            Ok(InsightDailyEvaluation {
                primary_rule: format!("RSI({rsi_days}) at or below {entry_max:.0}"),
                primary_level: entry_max,
                primary_passed: entry_passed,
                secondary_rule: Some(format!("Own {trend_days}-day SMA")),
                secondary_level: Some(trend),
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        _ => anyhow::bail!("Unknown daily strategy {strategy}"),
    }
}

#[cfg(test)]
fn insight_parameter_probe(
    profiles: &serde_json::Value,
    symbol: &str,
    strategy: &str,
    day: u64,
) -> Option<(String, f64, BTreeMap<String, f64>, serde_json::Value)> {
    let profile_symbol = if matches!(strategy, "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") {
        "BTC-USDT"
    } else {
        symbol
    };
    let profile = profiles.as_array()?.iter().find(|profile| {
        profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
            && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
    })?;
    let parameter_values = profile.get("parameters")?.as_array()?;
    let candidates = parameter_values.iter().filter_map(|parameter| {
        let key = parameter.get("key")?.as_str()?;
        let value = parameter.get("value")?.as_f64()?;
        let min = parameter.get("min")?.as_f64()?;
        let max = parameter.get("max")?.as_f64()?;
        let step = parameter.get("step")?.as_f64()?;
        (key != "learning_rate" && value.is_finite() && min.is_finite()
            && max.is_finite() && step.is_finite() && min < max && step > 0.0)
            .then_some((key.to_owned(), value, min, max, step))
    }).collect::<Vec<_>>();
    if candidates.is_empty() {
        return None;
    }
    // Give Donchian strategies a clean, meaningfully different lookback arm
    // every other day. Their normal multi-parameter probes change two values
    // together by one step, which cannot tell us whether a shorter breakout
    // window alone would have caught a move.
    if day % 2 == 0 {
        if let Some((key, current, min, max, step)) = candidates
            .iter()
            .find(|(key, _, _, _, _)| key == "breakout_lookback_days")
        {
            let delta = (((max - min) / 10.0 / step).round().max(1.0)) * step;
            let preferred = current - delta;
            let value = if preferred >= *min && preferred <= *max {
                preferred
            } else {
                current + delta
            };
            if value >= *min && value <= *max && (value - current).abs() >= step * 0.5 {
                let mut probe_profiles = profiles.clone();
                let matching_profile = probe_profiles.as_array_mut()?.iter_mut().find(|profile| {
                    profile.get("symbol").and_then(serde_json::Value::as_str)
                        == Some(profile_symbol)
                        && profile.get("strategy").and_then(serde_json::Value::as_str)
                            == Some(strategy)
                })?;
                let matching_parameter = matching_profile
                    .get_mut("parameters")?
                    .as_array_mut()?
                    .iter_mut()
                    .find(|parameter| {
                        parameter.get("key").and_then(serde_json::Value::as_str)
                            == Some(key.as_str())
                    })?;
                matching_parameter["value"] = serde_json::json!(value);
                let parameters = insight_profile_parameters(&probe_profiles, profile_symbol, strategy);
                return Some((key.clone(), value, parameters, probe_profiles));
            }
        }
    }
    let stable_hash = profile_symbol.bytes().chain(strategy.bytes()).fold(2_166_136_261_u64, |hash, byte| {
        hash.wrapping_mul(16_777_619) ^ u64::from(byte)
    });
    let direction = if (day >> 1) & 1 == 0 { 1.0 } else { -1.0 };
    if candidates.len() == 1 {
        let (key, current, min, max, step) = &candidates[0];
        let single_direction = if day % 2 == 0 { 1.0 } else { -1.0 };
        let preferred = current + single_direction * step;
        let value = if preferred >= *min && preferred <= *max { preferred } else { current - single_direction * step };
        if value < *min || value > *max || (value-current).abs() < step * 0.5 { return None; }
        let mut probe_profiles = profiles.clone();
        let matching_profile = probe_profiles.as_array_mut()?.iter_mut().find(|profile| {
            profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
                && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
        })?;
        let matching_parameter = matching_profile.get_mut("parameters")?.as_array_mut()?.iter_mut().find(|parameter| {
            parameter.get("key").and_then(serde_json::Value::as_str) == Some(key.as_str())
        })?;
        matching_parameter["value"] = serde_json::json!(value);
        let parameters = insight_profile_parameters(&probe_profiles, profile_symbol, strategy);
        return Some((key.clone(), value, parameters, probe_profiles));
    }
    let pair_count = candidates.len() * (candidates.len() - 1) / 2;
    let wanted = (day.wrapping_add(stable_hash) % pair_count as u64) as usize;
    let mut pair_index = 0;
    let mut selected = None;
    for left in 0..candidates.len() {
        for right in left + 1..candidates.len() {
            if pair_index == wanted { selected = Some((left, right)); break; }
            pair_index += 1;
        }
        if selected.is_some() { break; }
    }
    let (left, right) = selected?;
    let mut combination = BTreeMap::new();
    for index in [left, right] {
        let (key, current, min, max, step) = &candidates[index];
        let preferred = current + direction * step;
        let value = if preferred >= *min && preferred <= *max { preferred } else { current - direction * step };
        if value < *min || value > *max || (value-current).abs() < step * 0.5 { return None; }
        combination.insert(key.clone(), value);
    }
    let mut probe_profiles = profiles.clone();
    let matching_profile = probe_profiles.as_array_mut()?.iter_mut().find(|profile| {
        profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
            && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
    })?;
    let matching_parameters = matching_profile.get_mut("parameters")?.as_array_mut()?;
    for (key, value) in &combination {
        let matching_parameter = matching_parameters.iter_mut().find(|parameter| {
            parameter.get("key").and_then(serde_json::Value::as_str) == Some(key.as_str())
        })?;
        matching_parameter["value"] = serde_json::json!(value);
    }
    let parameters = insight_profile_parameters(&probe_profiles, profile_symbol, strategy);
    let pair = combination.iter().map(|(key, value)| format!("{key}={value:.8}")).collect::<Vec<_>>().join(",");
    let mut context_hash = 14_695_981_039_346_656_037_u64;
    for (key, value) in parameters.iter().filter(|(key, _)| !combination.contains_key(*key)) {
        for byte in format!("{key}={value:.8};").bytes() {
            context_hash = (context_hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    let probe_key = format!("combo:{pair}|ctx:{context_hash:016x}");
    Some((probe_key, 0.0, parameters, probe_profiles))
}

fn insight_parameter_probe_variants(
    profiles: &serde_json::Value,
    symbol: &str,
    strategy: &str,
    day: u64,
) -> Option<Vec<(String, f64, BTreeMap<String, f64>, BTreeMap<String, f64>)>> {
    let profile_symbol = if matches!(strategy, "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") {
        "BTC-USDT"
    } else {
        symbol
    };
    let profile = profiles.as_array()?.iter().find(|profile| {
        profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
            && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
    })?;
    let parameter_values = profile.get("parameters")?.as_array()?;
    let baseline = insight_profile_parameters(profiles, profile_symbol, strategy);
    let specs = parameter_values.iter().filter_map(|parameter| {
        let key = parameter.get("key")?.as_str()?.to_owned();
        let current = parameter.get("value")?.as_f64()?;
        let min = parameter.get("min")?.as_f64()?;
        let max = parameter.get("max")?.as_f64()?;
        let step = parameter.get("step")?.as_f64()?;
        (key != "learning_rate" && current.is_finite() && min.is_finite()
            && max.is_finite() && step.is_finite() && min < max && step > 0.0)
            .then_some(scouts::ParameterSpec {
                key,
                label: parameter.get("label").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                default: current,
                min,
                max,
                step,
                unit: parameter.get("unit").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                learning_direction: 1.0,
            })
    }).collect::<Vec<_>>();
    let variants = scouts::single_parameter_variants(&baseline, &specs, day)?;
    Some(variants.into_iter().filter_map(|changes| {
        let (key, value) = if changes.len() == 1 {
            let (key, value) = changes.iter().next()?;
            (key.clone(), *value)
        } else {
            (scouts::combination_key(&changes, &baseline)?, 0.0)
        };
        let mut parameters = baseline.clone();
        parameters.extend(changes.iter().map(|(key, value)| (key.clone(), *value)));
        Some((key, value, parameters, changes))
    }).collect())
}

fn insight_parameter_variant_request(
    profiles: &serde_json::Value,
    symbol: &str,
    strategy: &str,
    day: u64,
) -> Option<(scouts::ParameterVariantsRequest, BTreeMap<String, f64>)> {
    let profile_symbol = if matches!(strategy, "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") {
        "BTC-USDT"
    } else {
        symbol
    };
    let profile = profiles.as_array()?.iter().find(|profile| {
        profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
            && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
    })?;
    let baseline = insight_profile_parameters(profiles, profile_symbol, strategy);
    let mut rejected_values = BTreeMap::new();
    let specs = profile.get("parameters").and_then(serde_json::Value::as_array)
        .into_iter().flatten().filter_map(|parameter| {
            let key = parameter.get("key")?.as_str()?.to_owned();
            let current = parameter.get("value")?.as_f64()?;
            let min = parameter.get("min")?.as_f64()?;
            let max = parameter.get("max")?.as_f64()?;
            let step = parameter.get("step")?.as_f64()?;
            let rejected = parameter.get("scout_variants").and_then(serde_json::Value::as_array)
                .into_iter().flatten().filter(|variant| variant.get("rejected").and_then(serde_json::Value::as_bool) == Some(true))
                .filter_map(|variant| variant.get("candidate_value").and_then(serde_json::Value::as_f64)).collect::<Vec<_>>();
            if !rejected.is_empty() { rejected_values.insert(key.clone(), rejected); }
            (key != "learning_rate" && current.is_finite() && min.is_finite()
                && max.is_finite() && step.is_finite() && min < max && step > 0.0)
                .then_some(scouts::ParameterSpec {
                    key,
                    label: parameter.get("label").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                    default: current,
                    min,
                    max,
                    step,
                    unit: parameter.get("unit").and_then(serde_json::Value::as_str).unwrap_or_default().to_owned(),
                    learning_direction: 1.0,
                })
        }).collect::<Vec<_>>();
    Some((scouts::ParameterVariantsRequest { parameters: baseline.clone(), specs, day, rejected_values }, baseline))
}

fn request_insight_parameter_probe_variant_batch(
    jobs: Vec<((String, String), scouts::ParameterVariantsRequest, BTreeMap<String, f64>)>,
) -> Result<BTreeMap<(String, String), Vec<(String, f64, BTreeMap<String, f64>, BTreeMap<String, f64>)>>, String> {
    if jobs.is_empty() { return Ok(BTreeMap::new()); }
    Ok(jobs.into_iter().filter_map(|(key, request, baseline)| {
        let variants = scouts::single_parameter_variants(
            &request.parameters,
            &request.specs,
            request.day,
        )?.into_iter().filter(|changes| changes.iter().all(|(parameter, value)| {
            !request.rejected_values.get(parameter).is_some_and(|rejected| rejected.iter().any(|candidate| (candidate - value).abs() <= 1e-8))
        })).filter_map(|changes| {
            let (parameter, value) = changes.iter().next()?;
            let mut parameters = baseline.clone();
            parameters.extend(changes.iter().map(|(key, value)| (key.clone(), *value)));
            Some((parameter.clone(), *value, parameters, changes))
        }).collect::<Vec<_>>();
        (!variants.is_empty()).then_some((key, variants))
    }).collect())
}

const MAX_DAILY_SCOUT_VARIANTS_PER_BATCH: usize = 3;

type InsightProbeVariant = (
    String,
    f64,
    BTreeMap<String, f64>,
    BTreeMap<String, f64>,
);

async fn auto_trader_rpc(
    url: &str,
    request_id: String,
    method: strategies::ScoutWorkerHttpMethod,
    path: &str,
    payload: serde_json::Value,
) -> anyhow::Result<serde_json::Value> {
    let response = strategies::request_scout_service_route(
        url,
        &strategies::ScoutCoordinatorRouteCommand {
            request_id,
            strategy: None,
            method,
            path: path.into(),
            payload,
        },
    )
    .await?;
    if response.status >= 400 {
        anyhow::bail!("AutoTrader RSocket route {path} returned {}", response.status);
    }
    Ok(response.body)
}

async fn learning_scout_rpc(
    url: &str,
    request_id: String,
    path: &str,
    payload: serde_json::Value,
) -> anyhow::Result<()> {
    let response = strategies::request_scout_worker_route(
        url,
        &strategies::ScoutCoordinatorRouteCommand {
            request_id,
            strategy: None,
            method: strategies::ScoutWorkerHttpMethod::Post,
            path: path.into(),
            payload,
        },
    )
    .await?;
    if response.status >= 400 {
        anyhow::bail!("Learning Scout route {path} returned {}", response.status);
    }
    Ok(())
}

fn prior_paired_lift(
    profiles: &serde_json::Value,
    profile_symbol: &str,
    strategy: &str,
    parameter_key: &str,
    candidate_value: f64,
) -> Option<(f64, bool)> {
    let parameter = profiles.as_array()?.iter().find(|profile| {
        profile.get("symbol").and_then(serde_json::Value::as_str) == Some(profile_symbol)
            && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
    })?.get("parameters")?.as_array()?.iter().find(|parameter| {
        parameter.get("key").and_then(serde_json::Value::as_str) == Some(parameter_key)
    })?;
    parameter.get("scout_variants")?.as_array()?.iter().find_map(|variant| {
        let value = variant.get("candidate_value")?.as_f64()?;
        ((value - candidate_value).abs() <= 1e-8).then(|| {
            Some((
                variant.get("average_net_improvement_percent")?.as_f64()?,
                variant.get("rejected").and_then(serde_json::Value::as_bool) == Some(true),
            ))
        })?
    })
}

fn prioritize_daily_probe_batch(
    probes: &[InsightProbeVariant],
    profiles: &serde_json::Value,
    profile_symbol: &str,
    strategy: &str,
    day: u64,
) -> Vec<InsightProbeVariant> {
    let mut known = Vec::<(f64, InsightProbeVariant)>::new();
    let mut unexplored = Vec::<InsightProbeVariant>::new();
    for probe in probes {
        if let Some((score, rejected)) = prior_paired_lift(
            profiles,
            profile_symbol,
            strategy,
            &probe.0,
            probe.1,
        ) {
            if !rejected && score.is_finite() {
                known.push((score, probe.clone()));
            }
        } else {
            unexplored.push(probe.clone());
        }
    }
    known.sort_by(|left, right| {
        right.0.total_cmp(&left.0).then_with(|| left.1.0.cmp(&right.1.0))
    });
    unexplored.sort_by(|left, right| left.0.cmp(&right.0));
    if !unexplored.is_empty() {
        let offset = day as usize % unexplored.len();
        unexplored.rotate_left(offset);
    }

    // Reserve one slot for a rotating untested candidate while ranking the
    // other slots by observed paired average lift.
    let known_count = known
        .len()
        .min(MAX_DAILY_SCOUT_VARIANTS_PER_BATCH.saturating_sub(1));
    let mut selected = known
        .iter()
        .take(known_count)
        .map(|(_, probe)| probe.clone())
        .collect::<Vec<_>>();
    selected.extend(
        unexplored
            .iter()
            .take(MAX_DAILY_SCOUT_VARIANTS_PER_BATCH - selected.len())
            .cloned(),
    );
    if selected.len() < MAX_DAILY_SCOUT_VARIANTS_PER_BATCH {
        selected.extend(
            known
                .iter()
                .skip(known_count)
                .take(MAX_DAILY_SCOUT_VARIANTS_PER_BATCH - selected.len())
                .map(|(_, probe)| probe.clone()),
        );
    }
    selected.truncate(MAX_DAILY_SCOUT_VARIANTS_PER_BATCH);
    selected
}

#[cfg(test)]
mod daily_probe_batch_tests {
    use super::*;

    fn probe(key: &str, value: f64) -> InsightProbeVariant {
        (
            key.to_owned(),
            value,
            BTreeMap::from([(key.to_owned(), value)]),
            BTreeMap::from([(key.to_owned(), value)]),
        )
    }

    #[test]
    fn batch_ranks_paired_average_lift_and_keeps_one_exploration_slot() {
        let profiles = serde_json::json!([{
            "symbol": "BTC-USDT",
            "strategy": "test-strategy",
            "parameters": [
                {"key":"a", "scout_variants":[{"candidate_value": 2.0, "average_net_improvement_percent": 4.0, "rejected": false}]},
                {"key":"b", "scout_variants":[{"candidate_value": 2.0, "average_net_improvement_percent": 9.0, "rejected": false}]},
                {"key":"c", "scout_variants":[{"candidate_value": 2.0, "average_net_improvement_percent": 2.0, "rejected": false}]}
            ]
        }]);
        let probes = vec![probe("a", 2.0), probe("b", 2.0), probe("c", 2.0), probe("d", 2.0)];
        let selected = prioritize_daily_probe_batch(&probes, &profiles, "BTC-USDT", "test-strategy", 0);
        assert_eq!(selected.len(), 3);
        assert_eq!(selected[0].0, "b");
        assert_eq!(selected[1].0, "a");
        assert_eq!(selected[2].0, "d");
    }
}

fn daily_probe_group_key(signal: &serde_json::Value) -> String {
    let strategy = signal.get("strategy").and_then(serde_json::Value::as_str).unwrap_or_default();
    let symbol = signal.get("symbol").and_then(serde_json::Value::as_str).unwrap_or_default();
    let decision = signal.get("strategy_decision");
    let anchor = decision
        .and_then(|value| value.get("bar_close_timestamp"))
        .or_else(|| decision.and_then(|value| value.get("monthly_anchor_close_time_ms")))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(|| signal.get("timestamp").and_then(serde_json::Value::as_u64).unwrap_or_default());
    let baseline_parameters = decision
        .and_then(|value| value.get("scout_parameter_baseline_decision"))
        .and_then(|value| value.get("strategy_parameters"))
        .or_else(|| decision.and_then(|value| value.get("strategy_parameters")))
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    format!("{strategy}:{symbol}:{anchor}:{}", scouts::probe_id(&baseline_parameters.to_string()))
}

fn insight_profile_parameters(
    profiles: &serde_json::Value,
    symbol: &str,
    strategy: &str,
) -> BTreeMap<String, f64> {
    profiles
        .as_array()
        .into_iter()
        .flatten()
        .find(|profile| {
            profile.get("symbol").and_then(serde_json::Value::as_str) == Some(symbol)
                && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
        })
        .and_then(|profile| {
            profile
                .get("parameters")
                .and_then(serde_json::Value::as_array)
        })
        .into_iter()
        .flatten()
        .filter_map(|parameter| {
            Some((
                parameter.get("key")?.as_str()?.to_owned(),
                parameter.get("value")?.as_f64()?,
            ))
        })
        .collect()
}

fn insight_daily_batch(
    strategies: &[String],
    profiles: &serde_json::Value,
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    now: u64,
    remote_scout_variants: Option<&BTreeMap<(String, String), Vec<(String, f64, BTreeMap<String, f64>, BTreeMap<String, f64>)>>>,
) -> anyhow::Result<(serde_json::Value, Vec<serde_json::Value>)> {
    let mut signals = Vec::<serde_json::Value>::new();
    let mut diagnostics = Vec::<serde_json::Value>::new();
    let mut audits = Vec::<serde_json::Value>::new();
    for strategy in strategies
        .iter()
        .filter(|name| DAILY_STRATEGIES.contains(&name.as_str()))
    {
        let mut diagnostic_symbols = Vec::new();
        let mut strategy_btc: Option<InsightBtcRegime> = None;
        let monthly_selection = if strategy == "risk-adjusted-monthly-momentum" {
            Some(insight_risk_adjusted_monthly_selection(candles, profiles, now)?)
        } else { None };
        let relative_strength_ranking = if strategy == "relative-strength-90-sma200" {
            insight_relative_strength_ranking(candles, profiles)
        } else { Vec::new() };
        let pairs = if strategy == "risk-adjusted-trend-portfolio" {
            &TREND_PORTFOLIO_PAIRS[..]
        } else {
            &DAILY_PAIRS[..]
        };
        for &(symbol, pair) in pairs {
            if strategy == "trend-sleeves-btc200"
                && !matches!(symbol, "BTC-USDT" | "ETH-USDT" | "BNB-USDT")
            {
                continue;
            }
            let bars = candles
                .get(pair)
                .ok_or_else(|| anyhow::anyhow!("{pair} candles missing"))?;
            let close = bars
                .last()
                .ok_or_else(|| anyhow::anyhow!("{pair} candles empty"))?
                .close;
            let profile_symbol = if matches!(strategy.as_str(), "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") {
                "BTC-USDT"
            } else {
                symbol
            };
            let parameters = insight_profile_parameters(profiles, profile_symbol, strategy);
            let evaluation =
                insight_evaluate_daily(strategy, symbol, pair, candles, &parameters, profiles, now)?;
            let btc = evaluation.btc_regime.as_ref();
            if let Some(regime) = btc {
                strategy_btc = Some(regime.clone());
            }
            let primary_metric_value = if strategy == "rsi2-oversold-rebound" {
                let period = insight_parameter(&parameters, "rsi_period_days", 2.0).round().max(2.0) as usize;
                Some(insight_rsi(bars, period))
            } else if strategy == "risk-adjusted-trend-portfolio" {
                evaluation.strategy_order_size_multiplier
            } else { None };
            let (cross_sectional_metric_value, cross_sectional_threshold) = match strategy.as_str() {
                "risk-adjusted-monthly-momentum" => (
                    monthly_selection.as_ref().and_then(|selection| selection.ranked_candidates.iter()
                        .find(|candidate| candidate.symbol == symbol).map(|candidate| candidate.score)),
                    monthly_selection.as_ref().and_then(|selection| selection.candidate.as_ref().map(|candidate| candidate.score)),
                ),
                "relative-strength-90-sma200" => (
                    relative_strength_ranking.iter().find(|(candidate, _)| candidate == symbol).map(|(_, score)| *score),
                    relative_strength_ranking.first().map(|(_, score)| *score),
                ),
                _ => (None, None),
            };
            let primary_metric_unit = if strategy == "rsi2-oversold-rebound" {
                Some("RSI")
            } else if strategy == "risk-adjusted-trend-portfolio" {
                Some("target weight")
            } else { None };
            let distance = if evaluation.primary_level > 0.0 {
                match primary_metric_value {
                    Some(value) => (value / evaluation.primary_level - 1.0) * 100.0,
                    None => (close / evaluation.primary_level - 1.0) * 100.0,
                }
            } else {
                0.0
            };
            let (secondary_distance, secondary_passed) = match (
                strategy.as_str(),
                evaluation.secondary_level,
                btc,
            ) {
                ("risk-adjusted-trend-portfolio", Some(level), Some(regime)) => (
                    Some((regime.close / level - 1.0) * 100.0),
                    Some(regime.risk_on),
                ),
                (_, Some(level), _) => (
                    Some((close / level - 1.0) * 100.0),
                    Some(close > level),
                ),
                (_, None, _) => (None, None),
            };
            let trend_return_windows = if strategy == "risk-adjusted-trend-portfolio" {
                let portfolio_parameters = insight_profile_parameters(profiles, "BTC-USDT", strategy);
                let lookback = insight_parameter(&portfolio_parameters, "trend_portfolio_lookback_days", 160.0).round().max(16.0) as usize;
                [1_usize, 2, 4, 8, 16].iter().map(|scale| (lookback / scale).max(2)).collect::<Vec<_>>().iter().filter_map(|days| {
                    bars.len().checked_sub(1 + *days).map(|index| {
                        let reference_close = bars[index].close;
                        serde_json::json!({
                            "days": days,
                            "reference_close": reference_close,
                            "return_percent": (close / reference_close - 1.0) * 100.0,
                        })
                    })
                }).collect::<Vec<_>>()
            } else { Vec::new() };
            let decision = serde_json::json!({
                "strategy": strategy,
                "symbol": symbol,
                "bar_close_timestamp": bars.last().map(|bar| bar.close_time_ms),
                "observed_close": close,
                "primary_rule": evaluation.primary_rule,
                "primary_level": evaluation.primary_level,
                "primary_distance_percent": distance,
                "primary_metric_value": primary_metric_value,
                "primary_metric_unit": primary_metric_unit,
                "primary_passed": evaluation.primary_passed,
                "secondary_rule": evaluation.secondary_rule,
                "secondary_level": evaluation.secondary_level,
                "secondary_distance_percent": secondary_distance,
                "secondary_passed": secondary_passed,
                "btc_close": btc.map(|regime| regime.close),
                "btc_sma": btc.map(|regime| regime.sma),
                "btc_window_days": btc.map(|regime| regime.window_days),
                "btc_gate_required": btc.is_some(),
                "btc_gate_passed": btc.map(|regime| regime.risk_on),
                "trend_return_windows": if trend_return_windows.is_empty() { None } else { Some(trend_return_windows.clone()) },
                "strategy_parameters": parameters,
                "strategy_order_size_multiplier": evaluation.strategy_order_size_multiplier,
                "monthly_anchor_close_time_ms": evaluation.monthly_anchor_close_time_ms,
                "cross_sectional_metric_value": cross_sectional_metric_value,
                "cross_sectional_threshold": cross_sectional_threshold,
            });
            let leader_gap_percent = monthly_selection.as_ref().and_then(|selection| {
                let leader = selection.ranked_candidates.first()?;
                if leader.symbol != symbol { return None; }
                let runner_up = selection.ranked_candidates.get(1)?;
                (leader.score > 0.0).then_some((leader.score - runner_up.score) / leader.score * 100.0)
            });
            let relative_strength_gap_pp = relative_strength_ranking.first().and_then(|(leader, score)| {
                if leader != symbol { return None; }
                let runner_up = relative_strength_ranking.get(1)?;
                Some((score - runner_up.1) * 100.0)
            });
            diagnostic_symbols.push(serde_json::json!({
                "symbol": symbol, "close": close, "eligible": true,
                "leader_gap_percent": leader_gap_percent,
                "relative_strength_gap_pp": relative_strength_gap_pp,
                "rsi_exit_level": (strategy == "rsi2-oversold-rebound").then(|| insight_parameter(&parameters, "rsi_exit_level", 70.0)),
                "maximum_hold_days": (strategy == "rsi2-oversold-rebound").then(|| insight_parameter(&parameters, "maximum_hold_days", 5.0)),
                "minimum_hold_days": (strategy == "donchian-20-btc200-hold3").then(|| insight_parameter(&parameters, "minimum_hold_days", 3.0)),
                "primary_rule": evaluation.primary_rule,
                "primary_level": evaluation.primary_level,
                "primary_distance_percent": distance,
                "primary_metric_value": primary_metric_value,
                "primary_metric_unit": primary_metric_unit,
                "primary_passed": evaluation.primary_passed,
                "secondary_rule": evaluation.secondary_rule,
                "secondary_level": evaluation.secondary_level,
                "secondary_distance_percent": secondary_distance,
                "secondary_passed": secondary_passed,
                "trend_return_windows": if trend_return_windows.is_empty() { None } else { Some(trend_return_windows) },
            }));
            let audit_key = format!(
                "{strategy}:{symbol}:{}",
                bars.last().map_or(0, |bar| bar.close_time_ms)
            );
            audits.push(serde_json::json!({
                "record_schema_version": 1, "decision_id": audit_key,
                "stage": "strategy:evaluation", "timestamp": now,
                "symbol": symbol, "strategy_decision": decision,
                "status": "signal-published",
            }));
            let base_signal = serde_json::json!({
                "symbol": symbol, "direction": "bullish", "strength": 1.0,
                "timestamp": now, "stale": false, "score": 1.0,
                "ranging_probability": 0.0, "components": {},
                "strategy_candidate": true, "strategy": strategy,
                "strategy_decision": decision,
                "market_regime": btc.map(|regime| if regime.risk_on {
                    format!("btc-above-sma{}", regime.window_days)
                } else { format!("btc-below-sma{}", regime.window_days) }),
            });
            signals.push(base_signal.clone());
            let local_probes;
            let probes = if let Some(remote) = remote_scout_variants {
                remote
                    .get(&(strategy.clone(), symbol.to_owned()))
                    .map(Vec::as_slice)
                    .unwrap_or_default()
            } else {
                local_probes = insight_parameter_probe_variants(
                    profiles,
                    symbol,
                    strategy,
                    now / INSIGHT_DAY_MS,
                )
                .unwrap_or_default();
                local_probes.as_slice()
            };
            let profile_symbol = if matches!(strategy.as_str(), "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") {
                "BTC-USDT"
            } else {
                symbol
            };
            let selected_probes = prioritize_daily_probe_batch(
                probes,
                profiles,
                profile_symbol,
                strategy,
                now / INSIGHT_DAY_MS,
            );
            for (probe_key, probe_value, probe_parameters, changes) in &selected_probes
            {
                let mut probe_profiles = profiles.clone();
                let Some(profile_list) = probe_profiles.as_array_mut() else { continue; };
                let Some(matching_profile) = profile_list.iter_mut().find(|profile| {
                    profile.get("symbol").and_then(serde_json::Value::as_str) == Some(
                        if matches!(strategy.as_str(), "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") { "BTC-USDT" } else { symbol }
                    ) && profile.get("strategy").and_then(serde_json::Value::as_str) == Some(strategy)
                }) else { continue; };
                let Some(matching_parameters) = matching_profile.get_mut("parameters").and_then(serde_json::Value::as_array_mut) else { continue; };
                let mut profile_valid = true;
                for (key, value) in changes {
                    let Some(parameter) = matching_parameters.iter_mut().find(|parameter| {
                        parameter.get("key").and_then(serde_json::Value::as_str) == Some(key.as_str())
                    }) else {
                        profile_valid = false;
                        break;
                    };
                    parameter["value"] = serde_json::json!(value);
                }
                if !profile_valid { continue; }
                let probe_evaluation = insight_evaluate_daily(
                    strategy, symbol, pair, candles, &probe_parameters, &probe_profiles, now,
                )?;
                let probe_metric = if strategy == "rsi2-oversold-rebound" {
                    let period = insight_parameter(&probe_parameters, "rsi_period_days", 2.0).round().max(2.0) as usize;
                    Some(insight_rsi(bars, period))
                } else if strategy == "risk-adjusted-trend-portfolio" {
                    probe_evaluation.strategy_order_size_multiplier
                } else { None };
                let probe_btc = probe_evaluation.btc_regime.as_ref();
                let (probe_cross_metric, probe_cross_threshold) = match strategy.as_str() {
                    "risk-adjusted-monthly-momentum" => {
                        let selection = insight_risk_adjusted_monthly_selection(candles, &probe_profiles, now)?;
                        (
                            selection.ranked_candidates.iter().find(|candidate| candidate.symbol == symbol)
                                .map(|candidate| candidate.score),
                            selection.candidate.map(|candidate| candidate.score),
                        )
                    }
                    "relative-strength-90-sma200" => {
                        let ranking = insight_relative_strength_ranking(candles, &probe_profiles);
                        (
                            ranking.iter().find(|(candidate, _)| candidate == symbol).map(|(_, score)| *score),
                            ranking.first().map(|(_, score)| *score),
                        )
                    }
                    _ => (None, None),
                };
                let probe_secondary_passed = match (probe_evaluation.secondary_level, probe_btc) {
                    (Some(level), Some(regime)) if strategy == "risk-adjusted-trend-portfolio" => Some(regime.risk_on && regime.close > level),
                    (Some(level), _) => Some(close > level),
                    _ => None,
                };
                let mut probe_signal = base_signal.clone();
                let mut probe_decision = decision.clone();
                probe_decision["primary_rule"] = serde_json::json!(probe_evaluation.primary_rule);
                probe_decision["primary_level"] = serde_json::json!(probe_evaluation.primary_level);
                probe_decision["primary_passed"] = serde_json::json!(probe_evaluation.primary_passed);
                probe_decision["secondary_rule"] = serde_json::json!(probe_evaluation.secondary_rule);
                probe_decision["secondary_level"] = serde_json::json!(probe_evaluation.secondary_level);
                probe_decision["secondary_passed"] = serde_json::json!(probe_secondary_passed);
                probe_decision["cross_sectional_metric_value"] = serde_json::json!(probe_cross_metric);
                probe_decision["cross_sectional_threshold"] = serde_json::json!(probe_cross_threshold);
                probe_decision["primary_metric_value"] = serde_json::json!(probe_metric);
                probe_decision["strategy_order_size_multiplier"] = serde_json::json!(probe_evaluation.strategy_order_size_multiplier);
                probe_decision["monthly_anchor_close_time_ms"] = serde_json::json!(probe_evaluation.monthly_anchor_close_time_ms);
                probe_decision["strategy_parameters"] = serde_json::to_value(probe_parameters).unwrap_or_default();
                probe_decision["scout_parameter_probe"] = serde_json::json!(true);
                probe_decision["scout_parameter_profile_symbol"] = serde_json::json!(
                    if matches!(strategy.as_str(), "risk-adjusted-trend-portfolio" | "risk-adjusted-monthly-momentum") { "BTC-USDT" } else { symbol }
                );
                // Preserve the unmodified decision so AutoTrader can simulate
                // this probe beside its baseline on the same candle snapshot.
                probe_decision["scout_parameter_baseline_decision"] = decision.clone();
                probe_decision["scout_parameter_key"] = serde_json::json!(probe_key);
                probe_decision["scout_parameter_value"] = serde_json::json!(probe_value);
                probe_decision["btc_close"] = serde_json::json!(probe_btc.map(|regime| regime.close));
                probe_decision["btc_sma"] = serde_json::json!(probe_btc.map(|regime| regime.sma));
                probe_decision["btc_window_days"] = serde_json::json!(probe_btc.map(|regime| regime.window_days));
                probe_decision["btc_gate_passed"] = serde_json::json!(probe_btc.map(|regime| regime.risk_on));
                probe_signal["strategy_decision"] = probe_decision;
                if let Some(regime) = probe_btc {
                    probe_signal["market_regime"] = serde_json::json!(if regime.risk_on {
                        format!("btc-above-sma{}", regime.window_days)
                    } else { format!("btc-below-sma{}", regime.window_days) });
                }
                signals.push(probe_signal);
            }
        }
        let btc = strategy_btc.as_ref();
        diagnostics.push(serde_json::json!({
            "strategy": strategy, "as_of_ms": now,
            "btc_close": btc.map(|regime| regime.close),
            "btc_sma": btc.map(|regime| regime.sma),
            "btc_distance_percent": btc.map(|regime| (regime.close/regime.sma-1.0)*100.0),
            "btc_window_days": btc.map(|regime| regime.window_days),
            "btc_gate_required": btc.is_some(),
            "btc_gate_passed": btc.map(|regime| regime.risk_on),
            "symbols": diagnostic_symbols,
        }));
    }
    let batch = serde_json::json!({
        "snapshot": {
            "as_of_ms": now, "empty_signal_reasons": [],
            "active_strategy_symbols": [], "generated_signals": signals,
        },
        "diagnostics": diagnostics,
    });
    Ok((batch, audits))
}

async fn poll_daily_strategy_signals(store: SignalStore) {
    let auto = std::env::var("AUTO_TRADER_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10006".into());
    let learning = std::env::var("LEARNING_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10010".into());
    let mut audited = std::collections::BTreeSet::<String>::new();
    let mut last_sweep_signature: Option<String> = None;
    let mut completed_sweep_groups = std::collections::BTreeSet::<String>::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(30));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let result: anyhow::Result<(serde_json::Value, Vec<serde_json::Value>, String)> = async {
            let (status, parameters, configuration) = tokio::try_join!(
                auto_trader_rpc(&auto, format!("insights-daily:{}", now_ms()), strategies::ScoutWorkerHttpMethod::Get, "/positions/auto-trader", serde_json::Value::Null),
                auto_trader_rpc(&auto, format!("insights-daily:{}", now_ms()), strategies::ScoutWorkerHttpMethod::Get, "/positions/auto-trader/parameters", serde_json::Value::Null),
                async {
                    let response = strategies::request_scout_worker_route(&learning, &strategies::ScoutCoordinatorRouteCommand {
                        request_id: format!("insights-daily:{}", now_ms()), strategy: None,
                        method: strategies::ScoutWorkerHttpMethod::Get, path: "/configuration".into(),
                        payload: serde_json::Value::Null,
                    }).await?;
                    if response.status >= 400 { anyhow::bail!("Learning Scout configuration route returned {}", response.status); }
                    Ok::<_, anyhow::Error>(response.body)
                }
            )?;
            let mut strategies = status.get("strategies").and_then(serde_json::Value::as_array)
                .into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            strategies.extend(configuration.get("strategies").and_then(serde_json::Value::as_array)
                .into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned)));
            strategies.sort();
            strategies.dedup();
            let symbols = if strategies.iter().any(|strategy| strategy == "risk-adjusted-trend-portfolio") {
                "BTCUSDT,ETHUSDT,BNBUSDT,XRPUSDT,ADAUSDT,SOLUSDT,DOGEUSDT,LTCUSDT,TRXUSDT,LINKUSDT,BCHUSDT,XLMUSDT,ETCUSDT,FILUSDT,AVAXUSDT,DOTUSDT"
            } else {
                "BTCUSDT,ETHUSDT,BNBUSDT,XRPUSDT,SOLUSDT,ADAUSDT,DOGEUSDT"
            };
            let mut candles = binance_client::daily_candles(symbols).await?;
            let candle_snapshot_at = now_ms();
            for bars in candles.values_mut() {
                bars.retain(|bar| bar.close_time_ms < candle_snapshot_at && bar.close.is_finite() && bar.close > 0.0
                    && bar.high.is_finite() && bar.high > 0.0);
                bars.sort_by_key(|bar| bar.close_time_ms);
            }
            let batch_as_of = now_ms();
            let mut scout_variant_jobs = Vec::new();
            for strategy in strategies.iter().filter(|strategy| DAILY_STRATEGIES.contains(&strategy.as_str())) {
                let pairs = if strategy == "risk-adjusted-trend-portfolio" {
                    &TREND_PORTFOLIO_PAIRS[..]
                } else {
                    &DAILY_PAIRS[..]
                };
                for &(symbol, _) in pairs {
                    if let Some((request, baseline)) = insight_parameter_variant_request(
                        &parameters["strategy_profiles"],
                        symbol,
                        strategy,
                        batch_as_of / INSIGHT_DAY_MS,
                    ) {
                        scout_variant_jobs.push(((strategy.clone(), symbol.to_owned()), request, baseline));
                    }
                }
            }
            let remote_scout_variants = request_insight_parameter_probe_variant_batch(scout_variant_jobs)
                .unwrap_or_default();
            let (batch, audits) = insight_daily_batch(
                &strategies,
                &parameters["strategy_profiles"],
                &candles,
                batch_as_of,
                Some(&remote_scout_variants),
            )?;
            let candle_anchor = candles
                .values()
                .filter_map(|bars| bars.last().map(|bar| bar.close_time_ms))
                .max()
                .unwrap_or_default();
            let sweep_signature = format!("{candle_anchor}:{}", strategies.join(","));
            Ok((batch, audits, sweep_signature))
        }.await;
        let (batch, audits, sweep_signature) = match result {
            Ok(value) => value,
            Err(error) => {
                crate::service_log!("WARN", "Insights daily strategy scan failed: {error}");
                continue;
            }
        };
        if last_sweep_signature.as_deref() != Some(sweep_signature.as_str()) {
            last_sweep_signature = Some(sweep_signature.clone());
            completed_sweep_groups.clear();
        }
        let snapshot = batch.get("snapshot").cloned().unwrap_or_else(|| serde_json::json!({}));
        let generated = snapshot.get("generated_signals").and_then(serde_json::Value::as_array)
            .cloned().unwrap_or_default();
        let diagnostics = batch.get("diagnostics").cloned().unwrap_or_else(|| serde_json::json!([]));
        if !generated.is_empty() {
            let now = snapshot.get("as_of_ms").and_then(serde_json::Value::as_u64).unwrap_or_else(now_ms);
            let sweep_id = scouts::probe_id(&sweep_signature);
            let probe_window = now / SCOUT_PARAMETER_SAMPLE_INTERVAL_MS;
            let scout_request = serde_json::json!({
                "request_id": format!("daily-sweep-paired-v2:{sweep_id}:{probe_window}"),
                "now_ms": now,
                "signals": generated.clone(),
            });
            if let Err(error) = learning_scout_rpc(&learning, scout_request["request_id"].as_str().unwrap_or_default().to_owned(), "/daily-inputs", scout_request).await {
                crate::service_log!("WARN", "Insights daily Scout delivery failed: {error}");
            }
        }
        let mut grouped = BTreeMap::<String, Vec<serde_json::Value>>::new();
        for signal in generated {
            grouped.entry(daily_probe_group_key(&signal)).or_default().push(signal);
        }
        let groups = grouped.into_iter().collect::<Vec<_>>();
        let mut group_failed = false;
        for (group_key, signals) in groups {
            if completed_sweep_groups.contains(&group_key) {
                continue;
            }
            for chunk in signals.chunks(128) {
                let payload = serde_json::json!({
                    "snapshot": {
                        "as_of_ms": snapshot.get("as_of_ms"),
                    "sweep_id": sweep_signature,
                        "empty_signal_reasons": snapshot.get("empty_signal_reasons"),
                        "active_strategy_symbols": snapshot.get("active_strategy_symbols"),
                        "generated_signals": chunk,
                    },
                    "diagnostics": diagnostics.clone(),
                });
                if let Err(error) = auto_trader_rpc(&auto, format!("insights-daily:{}", now_ms()), strategies::ScoutWorkerHttpMethod::Post, "/internal/auto-trader/daily-strategy-signals", payload).await {
                    crate::service_log!("WARN", "Insights daily strategy sweep delivery failed: {error}");
                    group_failed = true;
                    break;
                }
            }
            if group_failed {
                break;
            }
            completed_sweep_groups.insert(group_key);
        }
        {
            let heartbeat = serde_json::json!({
                "snapshot": {
                    "as_of_ms": snapshot.get("as_of_ms"),
                    "sweep_id": sweep_signature,
                    "empty_signal_reasons": snapshot.get("empty_signal_reasons"),
                    "active_strategy_symbols": [],
                    "generated_signals": [],
                },
                "diagnostics": diagnostics,
                "refresh_only": true,
            });
            if let Err(error) = auto_trader_rpc(&auto, format!("insights-daily:{}", now_ms()), strategies::ScoutWorkerHttpMethod::Post, "/internal/auto-trader/daily-strategy-signals", heartbeat).await {
                crate::service_log!("WARN", "Insights daily strategy snapshot refresh failed: {error}");
            }
        }
        for audit in audits {
            let Some(key) = audit.get("decision_id").and_then(serde_json::Value::as_str) else {
                continue;
            };
            if !audited.contains(key) {
                let key = key.to_owned();
                match store.save_auto_trader_audit(audit).await {
                    Ok(()) => {
                        audited.insert(key);
                    }
                    Err(error) => {
                        crate::service_log!("WARN", "Insights daily strategy audit failed: {error}")
                    }
                }
            }
        }
        let now = now_ms();
        audited.retain(|key| {
            key.rsplit(':')
                .next()
                .and_then(|value| value.parse::<u64>().ok())
                .is_some_and(|time| now.saturating_sub(time) <= 30 * 86_400_000)
        });
    }
}


#[cfg(test)]
mod donchian_20_btc200_tests {
    use super::*;

    fn bar(high: f64, close: f64) -> InsightDailyBar {
        InsightDailyBar { close_time_ms: 0, high, close }
    }

    #[test]
    fn fixed_twenty_day_breakout_requires_btc_regime() {
        let mut btc = vec![bar(100.0, 100.0); 201];
        btc[200] = bar(110.0, 110.0);
        let mut eth = vec![bar(100.0, 100.0); 56];
        eth[20].high = 150.0; // Outside the prior 20, inside the prior 55.
        eth[34].high = 110.0; // Included by the 21-day probe, excluded by baseline.
        eth[55] = bar(120.0, 120.0);
        let mut candles = BTreeMap::from([
            ("BTCUSDT".into(), btc),
            ("ETHUSDT".into(), eth),
        ]);
        let parameters = BTreeMap::from([("breakout_lookback_days".into(), 20.0)]);
        let evaluate = |candles: &BTreeMap<String, Vec<InsightDailyBar>>| {
            insight_evaluate_daily(
                "donchian-20-btc200", "ETH-USDT", "ETHUSDT",
                candles, &parameters, &serde_json::json!([]), 0,
            ).unwrap()
        };
        let active = evaluate(&candles);
        assert!(active.primary_passed);
        assert_eq!(active.primary_level, 100.0);
        assert!(active.btc_regime.unwrap().risk_on);

        candles.get_mut("BTCUSDT").unwrap()[200] = bar(90.0, 90.0);
        assert!(!evaluate(&candles).btc_regime.unwrap().risk_on);
        candles.get_mut("BTCUSDT").unwrap()[200] = bar(110.0, 110.0);
        candles.get_mut("ETHUSDT").unwrap()[55] = bar(99.0, 99.0);
        assert!(!evaluate(&candles).primary_passed);

        candles.get_mut("ETHUSDT").unwrap()[55] = bar(120.0, 120.0);
        for &(_, pair) in &DAILY_PAIRS {
            candles.entry(pair.into()).or_insert_with(|| vec![bar(100.0, 100.0); 56]);
        }
        let probe_profiles = serde_json::json!([{
            "symbol": "ETH-USDT", "strategy": "donchian-20-btc200",
            "parameters": [{"key":"breakout_lookback_days", "value":20.0,
                "min":5.0, "max":60.0, "step":1.0}]
        }]);
        let (batch, audits) = insight_daily_batch(
            &["donchian-20-btc200".into()],
            &probe_profiles,
            &candles,
            42,
            None,
        ).unwrap();
        let generated = batch["snapshot"]["generated_signals"].as_array().unwrap();
        assert!(generated.len() >= DAILY_PAIRS.len() + 1);
        let probe = generated.iter().find(|signal| signal["strategy_decision"]["scout_parameter_probe"] == true).unwrap();
        assert_eq!(probe["strategy_decision"]["scout_parameter_key"], "breakout_lookback_days");
        assert_eq!(probe["strategy_decision"]["scout_parameter_value"], 21.0);
        assert_eq!(probe["strategy_decision"]["scout_parameter_baseline_decision"]["strategy_parameters"]["breakout_lookback_days"], 20.0);
        assert_eq!(probe["strategy_decision"]["primary_level"], 110.0);
        assert!(generated.iter().all(|signal| signal["strategy_candidate"] == true));
        assert!(batch["snapshot"]["active_strategy_symbols"].as_array().unwrap().is_empty());
        assert!(generated.iter().any(|signal| signal["strategy"] == "donchian-20-btc200"
            && signal["symbol"] == "ETH-USDT"));
        assert!(audits.iter().any(|audit| audit["status"] == "signal-published"
            && audit["symbol"] == "ETH-USDT"));
    }

    #[test]
    fn ten_parameter_profile_generates_bounded_single_parameter_probes() {
        let parameters = (0..10)
            .map(|index| {
                serde_json::json!({
                    "key": format!("parameter_{index}"),
                    "value": 5.0,
                    "default_value": 5.0,
                    "min": 0.0,
                    "max": 10.0,
                    "step": 1.0,
                    "unit": "units"
                })
            })
            .collect::<Vec<_>>();
        let profiles = serde_json::json!([{
            "symbol": "BTC-USDT",
            "strategy": "test-strategy",
            "parameters": parameters,
        }]);

        let variants = insight_parameter_probe_variants(
            &profiles,
            "BTC-USDT",
            "test-strategy",
            0,
        )
        .unwrap();
        assert_eq!(variants.len(), 20);
        assert!(variants
            .iter()
            .all(|(_, _, _, changes)| changes.len() == 1));
    }

    #[test]
    fn all_parameter_variants_share_their_baseline_sweep_batch() {
        let baseline_parameters = serde_json::json!({"threshold_a": 0.5, "threshold_b": 0.5});
        let baseline = serde_json::json!({
            "strategy": "test-strategy",
            "symbol": "BTC-USDT",
            "timestamp": 123,
            "strategy_decision": {
                "bar_close_timestamp": 100,
                "strategy_parameters": baseline_parameters,
            }
        });
        let mut variant = baseline.clone();
        variant["strategy_decision"]["strategy_parameters"]["threshold_a"] = serde_json::json!(0.6);
        variant["strategy_decision"]["scout_parameter_baseline_decision"] = baseline["strategy_decision"].clone();
        variant["strategy_decision"]["scout_parameter_probe"] = serde_json::json!(true);
        assert_eq!(daily_probe_group_key(&baseline), daily_probe_group_key(&variant));
    }
}


#[cfg(test)]
mod risk_adjusted_monthly_momentum_tests {
    use super::*;

    fn bar(close_time_ms: u64, close: f64) -> InsightDailyBar {
        InsightDailyBar { close_time_ms, high: close, close }
    }

    fn trending_candles(anchor_ms: u64) -> BTreeMap<String, Vec<InsightDailyBar>> {
        let slopes = [
            ("BTCUSDT", 1.0), ("ETHUSDT", 2.5), ("BNBUSDT", 1.8),
            ("XRPUSDT", 0.7), ("SOLUSDT", 2.0), ("ADAUSDT", 0.9), ("DOGEUSDT", 0.8),
        ];
        slopes.into_iter().map(|(symbol, slope)| {
            let mut bars = (0..260).map(|index| {
                let close = 100.0 + slope * index as f64;
                let timestamp = anchor_ms - ((259 - index) as u64 * INSIGHT_DAY_MS);
                bar(timestamp, close)
            }).collect::<Vec<_>>();
            // Future current-month candles must not leak into the current month's anchor.
            bars.push(bar(176_999_039_9999, 100_000.0));
            bars.push(bar(177_007_679_9999, 1.0));
            (symbol.to_owned(), bars)
        }).collect()
    }

    #[test]
    fn utc_month_id_changes_at_month_boundary() {
        assert_eq!(
            insight_utc_month_id(1_769_903_999_999),
            insight_utc_month_id(1_767_225_600_000),
        );
        assert_ne!(
            insight_utc_month_id(1_769_903_999_999),
            insight_utc_month_id(1_769_904_000_000),
        );
    }

    #[test]
    fn realized_volatility_requires_non_constant_returns() {
        let flat = (0..61).map(|index| bar(index, 100.0)).collect::<Vec<_>>();
        assert_eq!(insight_realized_daily_volatility(&flat, 60), None);
        let varying = (0..61).map(|index| {
            bar(index, if index % 2 == 0 { 100.0 } else { 101.0 })
        }).collect::<Vec<_>>();
        assert!(insight_realized_daily_volatility(&varying, 60).unwrap() > 0.0);
    }

    #[test]
    fn monthly_selection_uses_previous_month_close_not_future_bars() {
        let candles = trending_candles(1_769_903_999_999);
        let profiles = serde_json::json!([]);
        let selection = insight_risk_adjusted_monthly_selection(&candles, &profiles, 1_769_904_000_000).unwrap();
        let candidate = selection.candidate.unwrap();
        assert!(selection.btc_regime.risk_on);
        assert!(candidate.order_size_multiplier > 0.0 && candidate.order_size_multiplier <= 1.0);
        let mut anchor_only = candles.clone();
        for bars in anchor_only.values_mut() {
            bars.retain(|bar| bar.close_time_ms <= 1_769_903_999_999);
        }
        let without_future = insight_risk_adjusted_monthly_selection(&anchor_only, &profiles, 1_769_904_000_000)
            .unwrap().candidate.unwrap();
        assert_eq!(candidate.symbol, without_future.symbol);
        assert!((candidate.score - without_future.score).abs() < 1e-12);
        assert!((candidate.order_size_multiplier - without_future.order_size_multiplier).abs() < 1e-12);
    }
}

#[cfg(test)]
mod strategy_parameter_probe_tests {
    use super::*;

    #[test]
    fn probes_a_shorter_breakout_lookback_without_changing_other_parameters() {
        let profiles = serde_json::json!([{
            "symbol": "ETH-USDT",
            "strategy": "donchian-20-sma200",
            "parameters": [
                {"key": "breakout_lookback_days", "value": 20.0, "min": 5.0, "max": 30.0, "step": 1.0},
                {"key": "breakout_entry_distance_percent", "value": 0.5, "min": 0.0, "max": 2.0, "step": 0.1}
            ]
        }]);
        let probe = insight_parameter_probe(&profiles, "ETH-USDT", "donchian-20-sma200", 2).unwrap();
        assert_eq!(probe.0, "breakout_lookback_days");
        assert_eq!(probe.1, 17.0);
        let changed = probe.3[0]["parameters"].as_array().unwrap().iter()
            .filter(|parameter| parameter["value"] != profiles[0]["parameters"][
                if parameter["key"] == "breakout_lookback_days" { 0 } else { 1 }
            ]["value"])
            .count();
        assert_eq!(changed, 1);
        assert_eq!(probe.2["breakout_lookback_days"], 17.0);
        assert_eq!(probe.2["breakout_entry_distance_percent"], 0.5);
    }

    #[test]
    fn continues_testing_other_parameter_combinations_between_lookback_arms() {
        let profiles = serde_json::json!([{
            "symbol": "ETH-USDT",
            "strategy": "donchian-20-sma200",
            "parameters": [
                {"key": "breakout_lookback_days", "value": 20.0, "min": 5.0, "max": 30.0, "step": 1.0},
                {"key": "breakout_entry_distance_percent", "value": 0.5, "min": 0.0, "max": 2.0, "step": 0.1}
            ]
        }]);
        let probe = insight_parameter_probe(&profiles, "ETH-USDT", "donchian-20-sma200", 3).unwrap();
        assert!(probe.0.starts_with("combo:"));
        assert_eq!(probe.1, 0.0);
        assert_eq!(probe.2.len(), 2);
    }

    #[test]
    fn rotates_parameter_values_within_profile_bounds() {
        let profiles = serde_json::json!([{
            "symbol": "ETH-USDT",
            "strategy": "donchian-20-sma200",
            "parameters": [{
                "key": "breakout_lookback_days", "value": 20.0,
                "min": 5.0, "max": 30.0, "step": 1.0
            }]
        }]);
        let up = insight_parameter_probe(&profiles, "ETH-USDT", "donchian-20-sma200", 2).unwrap();
        let down = insight_parameter_probe(&profiles, "ETH-USDT", "donchian-20-sma200", 3).unwrap();
        assert_eq!(up.0, "breakout_lookback_days");
        assert_eq!(up.1, 17.0);
        assert_eq!(down.1, 19.0);
        assert_eq!(up.2["breakout_lookback_days"], 17.0);
        assert_eq!(up.3[0]["parameters"][0]["value"], 17.0);
    }
}
