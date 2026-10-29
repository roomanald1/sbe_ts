// Insights owns the Binance market data and emits paired carry signals to AutoTrader.
use std::collections::BTreeSet;

const CARRY_SIGNAL_DAY_MS: u64 = 86_400_000;
const CARRY_SIGNAL_COST_PERCENT: f64 = 0.5;

#[derive(Serialize)]
struct CarryEntryCandidate {
    spot_symbol: String,
    future_symbol: String,
    expiry_ms: u64,
    days_to_expiry: f64,
    quantity: f64,
    spot_ask: f64,
    future_bid: f64,
    basis_percent: f64,
    net_after_budget_percent: f64,
    eligible: bool,
    reason: String,
}

async fn carry_market_json(client: &reqwest::Client, url: &str) -> anyhow::Result<serde_json::Value> {
    Ok(client.get(url).send().await?.error_for_status()?.json().await?)
}

fn carry_book_levels(value: &serde_json::Value, side: &str) -> Vec<(f64, f64)> {
    value.get(side).and_then(serde_json::Value::as_array)
        .into_iter().flatten()
        .filter_map(|row| {
            let price = row.get(0)?.as_str()?.parse::<f64>().ok()?;
            let size = row.get(1)?.as_str()?.parse::<f64>().ok()?;
            (price.is_finite() && size.is_finite() && price > 0.0 && size > 0.0)
                .then_some((price, size))
        }).collect()
}

fn carry_book_walk(levels: &[(f64, f64)], quantity: f64) -> Option<f64> {
    if quantity <= 0.0 { return None; }
    let mut left = quantity;
    let mut value = 0.0;
    for &(price, size) in levels {
        let filled = left.min(size);
        value += filled * price;
        left -= filled;
        if left <= 1e-10 { return Some(value / quantity); }
    }
    None
}

fn carry_future_step(symbol: &serde_json::Value) -> f64 {
    symbol.get("filters").and_then(serde_json::Value::as_array)
        .into_iter().flatten()
        .find(|item| item.get("filterType").and_then(serde_json::Value::as_str) == Some("LOT_SIZE"))
        .and_then(|item| item.get("stepSize").and_then(serde_json::Value::as_str))
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|step| step.is_finite() && *step > 0.0)
        .unwrap_or(0.001).max(0.001)
}

async fn carry_paper_size(client: &reqwest::Client, base: &str) -> f64 {
    let url = format!("{base}/positions/auto-trader/dated-carry");
    carry_market_json(client, &url).await.ok()
        .and_then(|book| book.get("max_spot_notional_usdt").and_then(serde_json::Value::as_f64))
        .filter(|size| size.is_finite() && (100.0..=10_000.0).contains(size))
        .unwrap_or(1_000.0)
}

async fn carry_entry_signals(
    client: &reqwest::Client,
    base: &str,
    now: u64,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let size = carry_paper_size(client, base).await;
    let exchange = carry_market_json(client, "https://fapi.binance.com/fapi/v1/exchangeInfo").await?;
    let symbols = exchange.get("symbols").and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("Futures exchangeInfo symbols missing"))?;
    let mut signals = Vec::new();
    for symbol in symbols {
        let Some(future_symbol) = symbol.get("symbol").and_then(serde_json::Value::as_str) else { continue };
        let spot_symbol = if future_symbol.starts_with("BTCUSDT_") { "BTCUSDT" }
            else if future_symbol.starts_with("ETHUSDT_") { "ETHUSDT" }
            else { continue };
        if symbol.get("status").and_then(serde_json::Value::as_str) != Some("TRADING") { continue; }
        let Some(expiry_ms) = symbol.get("deliveryDate").and_then(serde_json::Value::as_u64) else { continue };
        if expiry_ms <= now { continue; }
        let days = (expiry_ms - now) as f64 / CARRY_SIGNAL_DAY_MS as f64;
        if !(58.0..=62.0).contains(&days) { continue; }
        let spot_url = format!("https://api.binance.com/api/v3/depth?symbol={spot_symbol}&limit=100");
        let future_url = format!("https://fapi.binance.com/fapi/v1/depth?symbol={future_symbol}&limit=100");
        let (spot_result, future_result) = tokio::join!(
            carry_market_json(client, &spot_url),
            carry_market_json(client, &future_url)
        );
        let spot_book = spot_result?;
        let future_book = future_result?;
        let asks = carry_book_levels(&spot_book, "asks");
        let bids = carry_book_levels(&future_book, "bids");
        let Some(best_ask) = asks.first().map(|item| item.0) else { continue };
        let step = carry_future_step(symbol);
        let quantity = ((size / (best_ask * 1.0005)) / step).floor() * step;
        let Some(spot_ask) = carry_book_walk(&asks, quantity) else { continue };
        let Some(future_bid) = carry_book_walk(&bids, quantity) else { continue };
        let basis_percent = (future_bid / spot_ask - 1.0) * 100.0;
        let net_after_budget_percent = basis_percent - CARRY_SIGNAL_COST_PERCENT;
        let eligible = net_after_budget_percent > 0.0;
        let candidate = CarryEntryCandidate {
            spot_symbol: spot_symbol.into(),
            future_symbol: future_symbol.into(),
            expiry_ms,
            days_to_expiry: days,
            quantity,
            spot_ask,
            future_bid,
            basis_percent,
            net_after_budget_percent,
            eligible,
            reason: if eligible { "premium exceeds 0.5% cost budget" }
                else { "premium does not cover 0.5% cost budget" }.into(),
        };
        signals.push(serde_json::json!({
            "kind": "entry", "emitted_at_ms": now_ms(), "candidate": candidate
        }));
    }
    Ok(signals)
}

fn carry_civil_from_days(day: i64) -> (i32, u32, u32) {
    let z = day + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let mut year = (yoe + era * 400) as i32;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let date_day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (mp + if mp < 10 { 3 } else { -9 }) as u32;
    if month <= 2 { year += 1; }
    (year, month, date_day)
}

fn carry_month_days(year: i32, month: u32) -> u32 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

fn carry_recent_expiries(now: u64) -> Vec<(String, String, u64)> {
    let today = (now / CARRY_SIGNAL_DAY_MS) as i64;
    let mut result = Vec::new();
    for offset in 0..=120 {
        let day = today - offset;
        let (year, month, date_day) = carry_civil_from_days(day);
        if !matches!(month, 3 | 6 | 9 | 12)
            || (day + 3).rem_euclid(7) != 4
            || date_day + 7 <= carry_month_days(year, month) { continue; }
        let expiry_ms = day as u64 * CARRY_SIGNAL_DAY_MS + 8 * 3_600_000;
        if expiry_ms + 60_000 > now { continue; }
        for coin in ["BTC", "ETH"] {
            let spot = format!("{coin}USDT");
            let future = format!("{spot}_{:02}{month:02}{date_day:02}", year % 100);
            result.push((spot, future, expiry_ms));
        }
    }
    result
}

async fn carry_spot_twap(
    client: &reqwest::Client, symbol: &str, expiry_ms: u64
) -> anyhow::Result<f64> {
    let start = expiry_ms - 30 * 60_000;
    let url = format!(
        "https://api.binance.com/api/v3/klines?symbol={symbol}&interval=1m&startTime={start}&endTime={}&limit=30",
        expiry_ms - 1
    );
    let value = carry_market_json(client, &url).await?;
    let rows = value.as_array().ok_or_else(|| anyhow::anyhow!("Spot exit candles missing"))?;
    if rows.len() != 30 { anyhow::bail!("Expected 30 spot exit candles, got {}", rows.len()); }
    let mut sum = 0.0;
    for (index, row) in rows.iter().enumerate() {
        let time = row.get(0).and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("Spot exit candle timestamp missing"))?;
        if time != start + index as u64 * 60_000 { anyhow::bail!("Spot exit candle gap"); }
        let open = row.get(1).and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Spot exit candle open missing"))?.parse::<f64>()?;
        if !open.is_finite() || open <= 0.0 { anyhow::bail!("Spot exit candle invalid"); }
        sum += open;
    }
    Ok(sum / 30.0)
}

async fn carry_settlement_price(
    client: &reqwest::Client, symbol: &str, expiry_ms: u64
) -> anyhow::Result<(f64, &'static str)> {
    let url = format!("https://fapi.binance.com/futures/data/delivery-price?pair={symbol}");
    let deliveries = carry_market_json(client, &url).await?;
    if let Some(price) = deliveries.as_array().into_iter().flatten()
        .find(|item| item.get("deliveryTime").and_then(serde_json::Value::as_u64) == Some(expiry_ms))
        .and_then(|item| item.get("deliveryPrice").and_then(serde_json::Value::as_f64))
        .filter(|price| price.is_finite() && *price > 0.0)
    {
        return Ok((price, "official"));
    }
    let start = expiry_ms - 30 * 60_000;
    let url = format!(
        "https://fapi.binance.com/fapi/v1/indexPriceKlines?pair={symbol}&interval=1m&startTime={start}&endTime={}&limit=30",
        expiry_ms - 1
    );
    let value = carry_market_json(client, &url).await?;
    let rows = value.as_array().ok_or_else(|| anyhow::anyhow!("Settlement index candles missing"))?;
    if rows.len() != 30 { anyhow::bail!("Expected 30 settlement index candles"); }
    let mut sum = 0.0;
    for (index, row) in rows.iter().enumerate() {
        let time = row.get(0).and_then(serde_json::Value::as_u64)
            .ok_or_else(|| anyhow::anyhow!("Settlement index timestamp missing"))?;
        if time != start + index as u64 * 60_000 { anyhow::bail!("Settlement index gap"); }
        let open = row.get(1).and_then(serde_json::Value::as_str)
            .ok_or_else(|| anyhow::anyhow!("Settlement index open missing"))?.parse::<f64>()?;
        if !open.is_finite() || open <= 0.0 { anyhow::bail!("Settlement index open invalid"); }
        sum += open;
    }
    Ok((sum / 30.0, "index_1m_estimate"))
}

async fn carry_publish(client: &reqwest::Client, base: &str, signal: &serde_json::Value) -> anyhow::Result<()> {
    client.post(format!("{base}/internal/auto-trader/dated-carry-signal"))
        .json(signal).send().await?.error_for_status()?;
    Ok(())
}

async fn run_dated_carry_signal_worker(store: SignalStore) {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build().unwrap_or_else(|_| reqwest::Client::new());
    let base = std::env::var("AUTO_TRADER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3005".into());
    let mut sent_settlements = BTreeSet::new();
    let mut audited_entries = BTreeSet::new();
    let mut ticker = tokio::time::interval(Duration::from_secs(300));
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticker.tick().await;
        let now = now_ms();
        match carry_entry_signals(&client, &base, now).await {
            Ok(signals) => {
                let candidates = signals.iter().filter_map(|signal| signal.get("candidate").cloned()).collect::<Vec<_>>();
                let scan = serde_json::json!({
                    "kind": "scan", "emitted_at_ms": now, "candidates": candidates, "last_error": null
                });
                if let Err(error) = carry_publish(&client, &base, &scan).await {
                    crate::service_log!("WARN", "Dated carry scan status delivery failed: {error}");
                }
                for signal in signals {
                    let future = signal["candidate"]["future_symbol"].as_str().unwrap_or("");
                    let spot = signal["candidate"]["spot_symbol"].as_str().unwrap_or("");
                    let eligible = signal["candidate"]["eligible"].as_bool().unwrap_or(false);
                    let audit_key = format!("{future}:{eligible}");
                    if !future.is_empty() && audited_entries.insert(audit_key.clone()) {
                        let audit = serde_json::json!({
                            "decision_id": format!("dated-carry:{audit_key}"),
                            "stage": "dated_carry_signal_entry",
                            "timestamp": now_ms(),
                            "symbol": spot,
                            "signal": signal,
                        });
                        if let Err(error) = store.save_auto_trader_audit(audit).await {
                            crate::service_log!("WARN", "Dated carry signal audit failed: {error}");
                        }
                    }
                    if let Err(error) = carry_publish(&client, &base, &signal).await {
                        crate::service_log!("WARN", "Dated carry entry signal delivery failed: {error}");
                    }
                }
            }
            Err(error) => {
                crate::service_log!("WARN", "Dated carry signal scan failed: {error}");
                let scan = serde_json::json!({
                    "kind": "scan", "emitted_at_ms": now, "candidates": [], "last_error": error.to_string()
                });
                if let Err(delivery_error) = carry_publish(&client, &base, &scan).await {
                    crate::service_log!("WARN", "Dated carry scan error delivery failed: {delivery_error}");
                }
            }
        }
        for (spot, future, expiry_ms) in carry_recent_expiries(now) {
            if sent_settlements.contains(&future) { continue; }
            let (twap, settlement) = tokio::join!(
                carry_spot_twap(&client, &spot, expiry_ms),
                carry_settlement_price(&client, &spot, expiry_ms)
            );
            let (Ok(spot_exit_twap), Ok((settlement_price, settlement_source))) = (twap, settlement)
                else { continue };
            let signal = serde_json::json!({
                "kind": "settlement",
                "emitted_at_ms": now_ms(),
                "spot_symbol": spot,
                "future_symbol": future,
                "expiry_ms": expiry_ms,
                "spot_exit_twap": spot_exit_twap,
                "settlement_price": settlement_price,
                "settlement_source": settlement_source,
            });
            match carry_publish(&client, &base, &signal).await {
                Ok(()) => {
                    let audit = serde_json::json!({
                        "decision_id": format!("dated-carry:{future}"),
                        "stage": "dated_carry_signal_settlement",
                        "timestamp": now_ms(),
                        "symbol": spot,
                        "signal": signal,
                    });
                    if let Err(error) = store.save_auto_trader_audit(audit).await {
                        crate::service_log!("WARN", "Dated carry settlement audit failed: {error}");
                    }
                    sent_settlements.insert(future);
                }
                Err(error) => crate::service_log!("WARN", "Dated carry settlement signal delivery failed: {error}"),
            }
        }
    }
}
