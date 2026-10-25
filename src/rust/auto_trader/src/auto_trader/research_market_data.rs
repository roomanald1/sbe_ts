
const RESEARCH_SYMBOLS: [(&str, &str); 7] = [
    ("BTC-USDT", "BTCUSDT"),
    ("ETH-USDT", "ETHUSDT"),
    ("BNB-USDT", "BNBUSDT"),
    ("XRP-USDT", "XRPUSDT"),
    ("SOL-USDT", "SOLUSDT"),
    ("ADA-USDT", "ADAUSDT"),
    ("DOGE-USDT", "DOGEUSDT"),
];
const BTC_REGIME_SMA_DAYS: usize = 200;
const RESEARCH_DATA_TTL_MS: u64 = 5 * 60 * 1000;
const DONCHIAN_55_DAYS: usize = 55;
const DONCHIAN_20_DAYS: usize = 20;

#[derive(Clone, Debug)]
struct DailyCandle {
    close_time_ms: u64,
    high: f64,
    close: f64,
}

#[derive(Clone, Default)]
struct ResearchDailyDataCache {
    fetched_at_ms: u64,
    candles: BTreeMap<String, Vec<DailyCandle>>,
    rsi_hold_until_ms: BTreeMap<String, u64>,
}

#[derive(Clone, Serialize)]
struct ResearchStrategyDiagnostic {
    strategy: String,
    as_of_ms: u64,
    btc_close: f64,
    btc_sma200: f64,
    btc_distance_percent: f64,
    btc_gate_required: bool,
    btc_gate_passed: bool,
    symbols: Vec<ResearchSymbolDiagnostic>,
}

#[derive(Clone, Serialize)]
struct ResearchSymbolDiagnostic {
    symbol: String,
    close: f64,
    eligible: bool,
    primary_rule: String,
    primary_level: f64,
    primary_distance_percent: f64,
    primary_passed: bool,
    secondary_rule: Option<String>,
    secondary_level: Option<f64>,
    secondary_distance_percent: Option<f64>,
    secondary_passed: Option<bool>,
    signal_active: bool,
}

#[derive(Clone)]
struct ResearchRegimeSnapshot {
    btc_risk_on: bool,
    enabled_symbols: BTreeSet<String>,
    generated_signals: Vec<Signal>,
    strategies: Vec<ResearchStrategy>,
}

impl ResearchRegimeSnapshot {
    fn entry_enabled(&self, symbol: &str) -> bool {
        self.enabled_symbols.contains(symbol)
    }

    fn empty_signal_reason(&self) -> &'static str {
        if self.strategies.iter().any(|strategy| matches!(strategy, ResearchStrategy::Donchian55Btc200 | ResearchStrategy::TrendSleevesBtc200)) && !self.btc_risk_on {
            "BTC is below its 200-day SMA; BTC-gated strategies are in cash"
        } else {
            "No active signal from the enabled daily strategies"
        }
    }
}

#[derive(serde::Deserialize)]
struct FeedDailyCandle { close_time_ms: u64, high: f64, close: f64 }

async fn fetch_research_daily_candles(
    client: &reqwest::Client,
    pair: &'static str,
    now_ms: u64,
) -> Result<Vec<DailyCandle>, String> {
    let feed_url = std::env::var("FEED_HANDLERS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
    let url = format!("{}/market-data/daily-candles?symbols={pair}", feed_url.trim_end_matches('/'));
    let response = client.get(url).send().await.map_err(|error| format!("FeedHandlers daily candle request failed: {error}"))?
        .error_for_status().map_err(|error| format!("FeedHandlers daily candle endpoint returned an error: {error}"))?;
    let mut by_symbol = response.json::<BTreeMap<String, Vec<FeedDailyCandle>>>().await
        .map_err(|error| format!("FeedHandlers daily candle response was invalid: {error}"))?;
    let candles = by_symbol.remove(pair).ok_or_else(|| format!("FeedHandlers returned no daily candles for {pair}"))?;
    let mut candles = candles.into_iter().filter(|bar| bar.close_time_ms < now_ms && bar.high.is_finite() && bar.high > 0.0 && bar.close.is_finite() && bar.close > 0.0)
        .map(|bar| DailyCandle { close_time_ms: bar.close_time_ms, high: bar.high, close: bar.close }).collect::<Vec<_>>();
    candles.sort_by_key(|candle| candle.close_time_ms);
    Ok(candles)
}

async fn load_research_regime_snapshot(
    state: &AutoTraderState,
    now_ms: u64,
) -> Result<ResearchRegimeSnapshot, String> {
    let existing = state.research_daily_data.lock().await.clone();
    let is_fresh = existing.fetched_at_ms > 0
        && now_ms.saturating_sub(existing.fetched_at_ms) <= RESEARCH_DATA_TTL_MS
        && existing.candles.len() == RESEARCH_SYMBOLS.len()
        && existing
            .candles
            .values()
            .all(|candles| candles.len() >= BTC_REGIME_SMA_DAYS + 1);
    let cache = if is_fresh {
        existing
    } else {
        let mut tasks = JoinSet::new();
        for (_, pair) in RESEARCH_SYMBOLS {
            let client = state.client.clone();
            tasks.spawn(async move {
                let candles = fetch_research_daily_candles(&client, pair, now_ms).await?;
                Ok::<_, String>((pair.to_owned(), candles))
            });
        }
        let mut candles = BTreeMap::new();
        while let Some(result) = tasks.join_next().await {
            let (pair, bars) = result
                .map_err(|error| format!("daily candle task failed: {error}"))??;
            if bars.len() < BTC_REGIME_SMA_DAYS + 1 {
                return Err(format!("not enough closed daily candles for {pair}"));
            }
            candles.insert(pair, bars);
        }
        let refreshed = ResearchDailyDataCache {
            fetched_at_ms: now_ms,
            candles,
            rsi_hold_until_ms: existing.rsi_hold_until_ms.clone(),
        };
        *state.research_daily_data.lock().await = refreshed.clone();
        refreshed
    };

    let mut cache = cache;
    if strategies_rsi_enabled(state).await {
        for (symbol, pair) in RESEARCH_SYMBOLS {
            let bars = cache.candles.get(pair).ok_or_else(|| format!("{pair} daily candles unavailable"))?;
            for end in bars.len().saturating_sub(5)..bars.len() {
                if end < 200 { continue; }
                let close = bars[end - 1].close;
                let sma200 = bars[end - 200..end].iter().map(|bar| bar.close).sum::<f64>() / 200.0;
                let changes = [bars[end - 2].close - bars[end - 3].close, bars[end - 1].close - bars[end - 2].close];
                let gains = changes.iter().map(|change| change.max(0.0)).sum::<f64>() / 2.0;
                let losses = changes.iter().map(|change| (-change).max(0.0)).sum::<f64>() / 2.0;
                let rsi2 = if losses == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gains / losses) };
                if close > sma200 && rsi2 <= 10.0 {
                    let expiry = bars[end - 1].close_time_ms.saturating_add(5 * 86_400_000);
                    if expiry > now_ms { cache.rsi_hold_until_ms.entry(symbol.to_owned()).or_insert(expiry); }
                }
            }
        }
        cache.rsi_hold_until_ms.retain(|_, expiry| *expiry > now_ms);
    }
    *state.research_daily_data.lock().await = cache.clone();
    let btc = cache
        .candles
        .get("BTCUSDT")
        .ok_or("BTC daily candles unavailable")?;
    let btc_close = latest_close(btc)?;
    let btc_sma200 = trailing_sma(btc, BTC_REGIME_SMA_DAYS)?;
    let btc_risk_on = btc_close > btc_sma200;
    let strategies = state.settings.lock().await.research_strategies.clone();
    let mut enabled_symbols = BTreeSet::new();
    let mut generated_signals = Vec::new();
    let mut primary_diagnostic = None;
    let rotation_leader = if strategies.contains(&ResearchStrategy::RelativeStrength90Sma200) {
        RESEARCH_SYMBOLS.iter().filter_map(|(symbol, pair)| {
            let bars = cache.candles.get(*pair)?;
            if bars.len() < 200 { return None; }
            let close = latest_close(bars).ok()?;
            let sma = trailing_sma(bars, 200).ok()?;
            if close <= sma { return None; }
            let momentum = close / bars[bars.len() - 91].close - 1.0;
            Some((momentum, *symbol))
        }).max_by(|left, right| left.0.total_cmp(&right.0)).map(|(_, symbol)| symbol)
    } else { None };

    for strategy in &strategies {
        let mut diagnostic_symbols = Vec::new();
        for (symbol, pair) in RESEARCH_SYMBOLS {
            let bars = cache.candles.get(pair).ok_or_else(|| format!("{pair} daily candles unavailable"))?;
            let close = latest_close(bars)?;
            let (eligible, primary_rule, primary_level, secondary_rule, secondary_level) = match strategy {
                ResearchStrategy::Donchian55Btc200 => (true, "Prior 55-day high".to_owned(), prior_high(bars, DONCHIAN_55_DAYS)?, None, None),
                ResearchStrategy::Donchian20Sma200 => (true, "Prior 20-day high".to_owned(), prior_high(bars, DONCHIAN_20_DAYS)?, Some("Own 200-day SMA".to_owned()), Some(trailing_sma(bars, BTC_REGIME_SMA_DAYS)?)),
                ResearchStrategy::TrendSleevesBtc200 => match symbol {
                    "BTC-USDT" | "ETH-USDT" => (true, "Own 50-day SMA".to_owned(), trailing_sma(bars, 50)?, None, None),
                    "BNB-USDT" => (true, "Own 100-day SMA".to_owned(), trailing_sma(bars, 100)?, None, None),
                    _ => (false, "Not included in this strategy".to_owned(), 0.0, None, None),
                },
                ResearchStrategy::Trend50200 => (true, "Own 50-day SMA".to_owned(), trailing_sma(bars, 50)?, Some("Own 200-day SMA".to_owned()), Some(trailing_sma(bars, 200)?)),
                ResearchStrategy::RelativeStrength90Sma200 => (true, "90-day return vs eligible assets".to_owned(), bars[bars.len() - 91].close, Some("Own 200-day SMA".to_owned()), Some(trailing_sma(bars, 200)?)),
                ResearchStrategy::Rsi2OversoldRebound => (true, "Two-day RSI(2) at or below 10".to_owned(), 10.0, Some("Own 200-day SMA".to_owned()), Some(trailing_sma(bars, 200)?)),
            };
            if !eligible { continue; }
            let primary_passed = match strategy {
                ResearchStrategy::RelativeStrength90Sma200 => rotation_leader == Some(symbol),
                ResearchStrategy::Rsi2OversoldRebound => rsi2_value(bars) <= 10.0,
                _ => close > primary_level,
            };
            let primary_distance_percent = if primary_level > 0.0 { (close / primary_level - 1.0) * 100.0 } else { 0.0 };
            let secondary_passed = secondary_level.map(|level| close > level);
            let secondary_distance_percent = secondary_level.map(|level| (close / level - 1.0) * 100.0);
            let active = match strategy {
                ResearchStrategy::Donchian55Btc200 | ResearchStrategy::TrendSleevesBtc200 => btc_risk_on && primary_passed,
                ResearchStrategy::Donchian20Sma200 => primary_passed && secondary_passed.unwrap_or(false),
                ResearchStrategy::Trend50200 => primary_passed && secondary_passed.unwrap_or(false),
                ResearchStrategy::RelativeStrength90Sma200 => rotation_leader == Some(symbol) && secondary_passed.unwrap_or(false),
                ResearchStrategy::Rsi2OversoldRebound => cache.rsi_hold_until_ms.get(symbol).is_some_and(|expiry| *expiry > now_ms) && rsi2_value(bars) < 70.0 && secondary_passed.unwrap_or(false),
            };
            diagnostic_symbols.push(ResearchSymbolDiagnostic { symbol: symbol.to_owned(), close, eligible, primary_rule, primary_level, primary_distance_percent, primary_passed, secondary_rule, secondary_level, secondary_distance_percent, secondary_passed, signal_active: active });
            if active {
                enabled_symbols.insert(symbol.to_owned());
                if generated_signals.iter().any(|signal: &Signal| signal.symbol == symbol) { continue; }
                generated_signals.push(Signal {
                    symbol: symbol.to_owned(), direction: "bullish".into(), strength: 1.0, timestamp: now_ms, stale: false, score: 1.0,
                    ranging_probability: Some(0.0), components: SignalComponents::default(), research_strategy_candidate: true,
                    research_strategy: Some(strategy.as_str().into()),
                    market_regime: Some(if btc_risk_on { "btc-above-sma200".into() } else { "btc-below-sma200".into() }),
                });
            }
        }
        if primary_diagnostic.is_none() {
            let btc_gate_required = matches!(strategy, ResearchStrategy::Donchian55Btc200 | ResearchStrategy::TrendSleevesBtc200);
            primary_diagnostic = Some(ResearchStrategyDiagnostic { strategy: strategy.as_str().to_owned(), as_of_ms: now_ms, btc_close, btc_sma200, btc_distance_percent: (btc_close / btc_sma200 - 1.0) * 100.0, btc_gate_required, btc_gate_passed: btc_risk_on, symbols: diagnostic_symbols });
        }
    }
    if let Ok(mut current) = state.research_strategy_diagnostic.lock() { *current = primary_diagnostic; }

    Ok(ResearchRegimeSnapshot {
        btc_risk_on,
        enabled_symbols,
        generated_signals,
        strategies,
    })
}

fn rsi2_value(candles: &[DailyCandle]) -> f64 {
    if candles.len() < 3 { return 100.0; }
    let changes = [candles[candles.len() - 2].close - candles[candles.len() - 3].close, candles[candles.len() - 1].close - candles[candles.len() - 2].close];
    let gains = changes.iter().map(|change| change.max(0.0)).sum::<f64>() / 2.0;
    let losses = changes.iter().map(|change| (-change).max(0.0)).sum::<f64>() / 2.0;
    if losses == 0.0 { 100.0 } else { 100.0 - 100.0 / (1.0 + gains / losses) }
}

async fn strategies_rsi_enabled(state: &AutoTraderState) -> bool {
    state.settings.lock().await.research_strategies.contains(&ResearchStrategy::Rsi2OversoldRebound)
}

fn latest_close(candles: &[DailyCandle]) -> Result<f64, String> {
    candles
        .last()
        .map(|candle| candle.close)
        .ok_or_else(|| "daily candle history is empty".into())
}

fn trailing_sma(candles: &[DailyCandle], days: usize) -> Result<f64, String> {
    if candles.len() < days {
        return Err(format!("need {days} closed daily candles"));
    }
    Ok(candles[candles.len() - days..]
        .iter()
        .map(|candle| candle.close)
        .sum::<f64>()
        / days as f64)
}

fn prior_high(candles: &[DailyCandle], days: usize) -> Result<f64, String> {
    if candles.len() <= days {
        return Err(format!("need {days} prior closed daily candles"));
    }
    let prior = &candles[candles.len() - days - 1..candles.len() - 1];
    prior
        .iter()
        .map(|candle| candle.high)
        .reduce(f64::max)
        .ok_or_else(|| "prior daily high history is empty".into())
}
