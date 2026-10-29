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
const DAILY_STRATEGIES: [&str; 8] = [
    "donchian-55-btc200",
    "donchian-20-sma200",
    "donchian-20-btc200",
    "trend-sleeves-btc200",
    "trend-50-200",
    "relative-strength-90-sma200",
    "risk-adjusted-monthly-momentum",
    "rsi2-oversold-rebound",
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
    active: bool,
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

struct InsightRiskAdjustedCandidate {
    symbol: &'static str,
    momentum: f64,
    daily_volatility: f64,
    score: f64,
    order_size_multiplier: f64,
    anchor_close_time_ms: u64,
}

struct InsightRiskAdjustedSelection {
    btc_regime: InsightBtcRegime,
    candidate: Option<InsightRiskAdjustedCandidate>,
}

fn insight_risk_adjusted_monthly_selection(
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    now: u64,
) -> anyhow::Result<InsightRiskAdjustedSelection> {
    let current_month = insight_utc_month_id(now);
    let btc = candles.get("BTCUSDT").ok_or_else(|| anyhow::anyhow!("BTC candles missing"))?;
    let btc_anchor_index = btc.iter().rposition(|bar| insight_utc_month_id(bar.close_time_ms) < current_month)
        .ok_or_else(|| anyhow::anyhow!("No completed daily candle before this month"))?;
    let btc_history = &btc[..=btc_anchor_index];
    let btc_close = btc_history.last().expect("anchor index exists").close;
    let btc_sma = insight_daily_sma(btc_history, 200)?;
    let btc_regime = InsightBtcRegime {
        close: btc_close,
        sma: btc_sma,
        window_days: 200,
        risk_on: btc_close > btc_sma,
    };
    if !btc_regime.risk_on {
        return Ok(InsightRiskAdjustedSelection { btc_regime, candidate: None });
    }

    let btc_anchor_time = btc_history.last().expect("anchor index exists").close_time_ms;
    let mut candidates = Vec::<InsightRiskAdjustedCandidate>::new();
    for &(symbol, pair) in &DAILY_PAIRS {
        let Some(bars) = candles.get(pair) else { continue };
        let Some(anchor_index) = bars.iter().rposition(|bar| insight_utc_month_id(bar.close_time_ms) < current_month) else { continue };
        let history = &bars[..=anchor_index];
        if history.last().is_none_or(|bar| bar.close_time_ms != btc_anchor_time) || history.len() < 201 {
            continue;
        }
        let close = history.last().expect("anchor index exists").close;
        let own_sma100 = insight_daily_sma(history, 100)?;
        if close <= own_sma100 {
            continue;
        }
        let momentum_reference = history[history.len() - 91].close;
        let momentum = close / momentum_reference - 1.0;
        if !momentum.is_finite() || momentum <= 0.0 {
            continue;
        }
        let Some(daily_volatility) = insight_realized_daily_volatility(history, 60) else { continue };
        let score = momentum / daily_volatility;
        let annualized_volatility = daily_volatility * 365.0_f64.sqrt();
        let order_size_multiplier = (0.40 / annualized_volatility).clamp(0.0, 1.0);
        if !score.is_finite() || !order_size_multiplier.is_finite() || order_size_multiplier <= 0.0 {
            continue;
        }
        candidates.push(InsightRiskAdjustedCandidate {
            symbol,
            momentum,
            daily_volatility,
            score,
            order_size_multiplier,
            anchor_close_time_ms: history.last().expect("anchor index exists").close_time_ms,
        });
    }
    let candidate = candidates.into_iter().max_by(|left, right| {
        left.score.total_cmp(&right.score).then_with(|| right.symbol.cmp(left.symbol))
    });
    Ok(InsightRiskAdjustedSelection { btc_regime, candidate })
}

fn insight_evaluate_daily(
    strategy: &str,
    symbol: &str,
    pair: &str,
    candles: &BTreeMap<String, Vec<InsightDailyBar>>,
    params: &BTreeMap<String, f64>,
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
                active: regime.risk_on && passed,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "donchian-20-btc200" => {
            // Fixed to the 20/200 parameters screened in research.
            let regime = insight_btc_regime(candles, 200)?;
            let level = insight_prior_high(bars, 20)?;
            let passed = close > level;
            Ok(InsightDailyEvaluation {
                primary_rule: "Prior 20-day high".into(),
                primary_level: level,
                primary_passed: passed,
                secondary_rule: None,
                secondary_level: None,
                active: regime.risk_on && passed,
                btc_regime: Some(regime),
                strategy_order_size_multiplier: None,
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
                active: passed && close > trend,
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
                active: regime.risk_on && passed,
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
                active: passed && close > slow,
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "risk-adjusted-monthly-momentum" => {
            let selection = insight_risk_adjusted_monthly_selection(candles, now)?;
            let own_sma = insight_daily_sma(bars, 100)?;
            let candidate = selection.candidate.as_ref();
            let active = candidate.is_some_and(|selected| selected.symbol == symbol);
            let primary_rule = candidate.map_or_else(
                || "Own 100-day SMA; no eligible monthly leader".to_owned(),
                |selected| format!(
                    "Own 100-day SMA; rank by 90-day return / 60-day volatility; selected score {:.3} (return {:.2}%, vol {:.2}%)",
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
                active,
                btc_regime: Some(selection.btc_regime),
                strategy_order_size_multiplier: candidate.filter(|selected| selected.symbol == symbol)
                    .map(|selected| selected.order_size_multiplier),
                monthly_anchor_close_time_ms: candidate.filter(|selected| selected.symbol == symbol)
                    .map(|selected| selected.anchor_close_time_ms),
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
                active: passed && close > trend,
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        "rsi2-oversold-rebound" => {
            let trend_days = days("trend_sma_days", 200.0);
            let rsi_days = days("rsi_period_days", 2.0);
            let max_hold = days("maximum_hold_days", 5.0);
            let entry_max = insight_parameter(params, "rsi_entry_max", 10.0);
            let exit_level = insight_parameter(params, "rsi_exit_level", 70.0);
            let trend = insight_daily_sma(bars, trend_days)?;
            let rsi = insight_rsi(bars, rsi_days);
            let hold_active = bars
                .iter()
                .enumerate()
                .rev()
                .take(max_hold)
                .any(|(end, bar)| {
                    let history = &bars[..=end];
                    bar.close_time_ms <= now
                        && insight_daily_sma(history, trend_days)
                            .ok()
                            .is_some_and(|sma| bar.close > sma)
                        && insight_rsi(history, rsi_days) <= entry_max
                });
            Ok(InsightDailyEvaluation {
                primary_rule: format!("RSI({rsi_days}) at or below {entry_max:.0}"),
                primary_level: entry_max,
                primary_passed: rsi <= entry_max,
                secondary_rule: Some(format!("Own {trend_days}-day SMA")),
                secondary_level: Some(trend),
                active: hold_active && rsi < exit_level && close > trend,
                btc_regime: None,
                strategy_order_size_multiplier: None,
                monthly_anchor_close_time_ms: None,
            })
        }
        _ => anyhow::bail!("Unknown daily strategy {strategy}"),
    }
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
) -> anyhow::Result<(serde_json::Value, Vec<serde_json::Value>)> {
    let mut active = Vec::<(String, String)>::new();
    let mut signals = Vec::<serde_json::Value>::new();
    let mut diagnostics = Vec::<serde_json::Value>::new();
    let mut audits = Vec::<serde_json::Value>::new();
    for strategy in strategies
        .iter()
        .filter(|name| DAILY_STRATEGIES.contains(&name.as_str()))
    {
        let mut diagnostic_symbols = Vec::new();
        let mut strategy_btc: Option<InsightBtcRegime> = None;
        for &(symbol, pair) in &DAILY_PAIRS {
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
            let parameters = insight_profile_parameters(profiles, symbol, strategy);
            let evaluation =
                insight_evaluate_daily(strategy, symbol, pair, candles, &parameters, now)?;
            let btc = evaluation.btc_regime.as_ref();
            if let Some(regime) = btc {
                strategy_btc = Some(regime.clone());
            }
            let primary_metric_value = (strategy == "rsi2-oversold-rebound").then(|| {
                let period = insight_parameter(&parameters, "rsi_period_days", 2.0).round().max(2.0) as usize;
                insight_rsi(bars, period)
            });
            let distance = if evaluation.primary_level > 0.0 {
                match primary_metric_value {
                    Some(value) => (value / evaluation.primary_level - 1.0) * 100.0,
                    None => (close / evaluation.primary_level - 1.0) * 100.0,
                }
            } else {
                0.0
            };
            let secondary_distance = evaluation
                .secondary_level
                .map(|level| (close / level - 1.0) * 100.0);
            let secondary_passed = evaluation.secondary_level.map(|level| close > level);
            let decision = serde_json::json!({
                "strategy": strategy,
                "symbol": symbol,
                "bar_close_timestamp": bars.last().map(|bar| bar.close_time_ms),
                "observed_close": close,
                "primary_rule": evaluation.primary_rule,
                "primary_level": evaluation.primary_level,
                "primary_distance_percent": distance,
                "primary_metric_value": primary_metric_value,
                "primary_metric_unit": primary_metric_value.map(|_| "RSI"),
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
                "active": evaluation.active,
                "strategy_parameters": parameters,
                "strategy_order_size_multiplier": evaluation.strategy_order_size_multiplier,
                "monthly_anchor_close_time_ms": evaluation.monthly_anchor_close_time_ms,
            });
            diagnostic_symbols.push(serde_json::json!({
                "symbol": symbol, "close": close, "eligible": true,
                "primary_rule": evaluation.primary_rule,
                "primary_level": evaluation.primary_level,
                "primary_distance_percent": distance,
                "primary_metric_value": primary_metric_value,
                "primary_metric_unit": primary_metric_value.map(|_| "RSI"),
                "primary_passed": evaluation.primary_passed,
                "secondary_rule": evaluation.secondary_rule,
                "secondary_level": evaluation.secondary_level,
                "secondary_distance_percent": secondary_distance,
                "secondary_passed": secondary_passed,
                "signal_active": evaluation.active,
            }));
            let audit_key = format!(
                "{strategy}:{symbol}:{}",
                bars.last().map_or(0, |bar| bar.close_time_ms)
            );
            audits.push(serde_json::json!({
                "record_schema_version": 1, "decision_id": audit_key,
                "stage": "strategy:evaluation", "timestamp": now,
                "symbol": symbol, "strategy_decision": decision,
                "status": if evaluation.active { "entry-qualified" } else { "entry-rejected" },
            }));
            if evaluation.active {
                active.push((strategy.clone(), symbol.into()));
                signals.push(serde_json::json!({
                    "symbol": symbol, "direction": "bullish", "strength": 1.0,
                    "timestamp": now, "stale": false, "score": 1.0,
                    "ranging_probability": 0.0, "components": {},
                    "strategy_candidate": true, "strategy": strategy,
                    "strategy_decision": decision,
                    "market_regime": btc.map(|regime| if regime.risk_on {
                        format!("btc-above-sma{}", regime.window_days)
                    } else { format!("btc-below-sma{}", regime.window_days) }),
                }));
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
            "active_strategy_symbols": active, "generated_signals": signals,
        },
        "diagnostics": diagnostics,
    });
    Ok((batch, audits))
}

async fn poll_daily_strategy_signals(store: SignalStore) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(40))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new());
    let auto = std::env::var("AUTO_TRADER_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".into());
    let feed =
        std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into());
    let mut audited = std::collections::BTreeSet::<String>::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(300));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let now = now_ms();
        let result = async {
            let (status_result, parameters_result, candles_result) = tokio::join!(
                client.get(format!("{auto}/positions/auto-trader")).send(),
                client.get(format!("{auto}/positions/auto-trader/parameters")).send(),
                client.get(format!("{feed}/market-data/daily-candles?symbols=BTCUSDT,ETHUSDT,BNBUSDT,XRPUSDT,SOLUSDT,ADAUSDT,DOGEUSDT")).send(),
            );
            let status = status_result?.error_for_status()?.json::<serde_json::Value>().await?;
            let parameters = parameters_result?.error_for_status()?.json::<serde_json::Value>().await?;
            let mut candles = candles_result?.error_for_status()?
                .json::<BTreeMap<String, Vec<InsightDailyBar>>>().await?;
            for bars in candles.values_mut() {
                bars.retain(|bar| bar.close_time_ms < now && bar.close.is_finite() && bar.close > 0.0
                    && bar.high.is_finite() && bar.high > 0.0);
                bars.sort_by_key(|bar| bar.close_time_ms);
            }
            let strategies = status.get("strategies").and_then(serde_json::Value::as_array)
                .into_iter().flatten().filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>();
            insight_daily_batch(&strategies, &parameters["strategy_profiles"], &candles, now)
        }.await;
        let (batch, audits) = match result {
            Ok(value) => value,
            Err(error) => {
                crate::service_log!("WARN", "Insights daily strategy scan failed: {error}");
                continue;
            }
        };
        let delivery = client
            .post(format!(
                "{auto}/internal/auto-trader/daily-strategy-signals"
            ))
            .json(&batch)
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        if let Err(error) = delivery {
            crate::service_log!(
                "WARN",
                "Insights daily strategy signal delivery failed: {error}"
            );
            continue;
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
        eth[55] = bar(120.0, 120.0);
        let mut candles = BTreeMap::from([
            ("BTCUSDT".into(), btc),
            ("ETHUSDT".into(), eth),
        ]);
        let parameters = BTreeMap::from([("breakout_lookback_days".into(), 55.0)]);
        let evaluate = |candles: &BTreeMap<String, Vec<InsightDailyBar>>| {
            insight_evaluate_daily(
                "donchian-20-btc200", "ETH-USDT", "ETHUSDT",
                candles, &parameters, 0,
            ).unwrap()
        };
        let active = evaluate(&candles);
        assert!(active.active);
        assert_eq!(active.primary_level, 100.0);
        assert!(active.btc_regime.unwrap().risk_on);

        candles.get_mut("BTCUSDT").unwrap()[200] = bar(90.0, 90.0);
        assert!(!evaluate(&candles).active);
        candles.get_mut("BTCUSDT").unwrap()[200] = bar(110.0, 110.0);
        candles.get_mut("ETHUSDT").unwrap()[55] = bar(99.0, 99.0);
        assert!(!evaluate(&candles).active);

        candles.get_mut("ETHUSDT").unwrap()[55] = bar(120.0, 120.0);
        for &(_, pair) in &DAILY_PAIRS {
            candles.entry(pair.into()).or_insert_with(|| vec![bar(100.0, 100.0); 56]);
        }
        let (batch, audits) = insight_daily_batch(
            &["donchian-20-btc200".into()],
            &serde_json::json!([]),
            &candles,
            42,
        ).unwrap();
        let generated = batch["snapshot"]["generated_signals"].as_array().unwrap();
        assert!(generated.iter().any(|signal| signal["strategy"] == "donchian-20-btc200"
            && signal["symbol"] == "ETH-USDT"));
        assert!(audits.iter().any(|audit| audit["status"] == "entry-qualified"
            && audit["symbol"] == "ETH-USDT"));
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
        let selection = insight_risk_adjusted_monthly_selection(&candles, 1_769_904_000_000).unwrap();
        let candidate = selection.candidate.unwrap();
        assert!(selection.btc_regime.risk_on);
        assert_eq!(candidate.anchor_close_time_ms, 1_769_903_999_999);
        assert!(candidate.order_size_multiplier > 0.0 && candidate.order_size_multiplier <= 1.0);
        let mut anchor_only = candles.clone();
        for bars in anchor_only.values_mut() {
            bars.retain(|bar| bar.close_time_ms <= 1_769_903_999_999);
        }
        let without_future = insight_risk_adjusted_monthly_selection(&anchor_only, 1_769_904_000_000)
            .unwrap().candidate.unwrap();
        assert_eq!(candidate.symbol, without_future.symbol);
        assert!((candidate.score - without_future.score).abs() < 1e-12);
        assert!((candidate.order_size_multiplier - without_future.order_size_multiplier).abs() < 1e-12);
    }
}
