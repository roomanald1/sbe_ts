use super::*;
fn number_field(v: &Value, key: &str) -> anyhow::Result<f64> {
    let n = v[key]
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("Option {key} missing"))?;
    anyhow::ensure!(n.is_finite(), "Nonfinite option {key}");
    Ok(n)
}
fn utc_date(timestamp: u64) -> String {
    let z = (timestamp / DAY) as i64 + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    if m <= 2 {
        y += 1;
    }
    format!("{y:04}-{m:02}-{d:02}")
}
fn volatility_gate(rows: &Value, now: u64) -> anyhow::Result<(f64, f64)> {
    let rows = rows["data"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("DVOL history missing"))?;
    let mut closed = BTreeMap::new();
    let mut current = None;
    for r in rows {
        let Some(t) = r[0].as_u64() else { continue };
        let Some(v) = r[4].as_f64().filter(|v| v.is_finite() && *v > 0.0) else {
            continue;
        };
        if t + DAY <= now {
            closed.insert(t, v);
        } else if t <= now {
            current = Some((t, v));
        }
    }
    let values = closed.iter().rev().take(30).collect::<Vec<_>>();
    anyhow::ensure!(values.len() == 30, "Need 30 completed DVOL days");
    anyhow::ensure!(
        values.windows(2).all(|w| *w[0].0 - *w[1].0 == DAY),
        "DVOL days contain gaps"
    );
    let latest = *values[0].1;
    let value = current.map(|(_, v)| v).unwrap_or(latest);
    anyhow::ensure!(now - *values[0].0 <= 2 * DAY, "DVOL history stale");
    Ok((value, values.iter().map(|(_, v)| **v).sum::<f64>() / 30.0))
}
pub(super) async fn build(feed: &FeedClient, sleeve: &Value) -> anyhow::Result<Output> {
    let held = sleeve["positions"]
        .as_object()
        .and_then(|p| p.iter().next());
    let inverse = held.is_some_and(|(name, _)| name.starts_with("BTC-"));
    let symbol = if inverse { "BTCUSDT" } else { "BTCUSDC" };
    let depth_params = [("limit", "100".into())];
    let vol_params = [
        ("currency", "BTC".into()),
        (
            "start_timestamp",
            now_ms().saturating_sub(35 * DAY).to_string(),
        ),
        ("end_timestamp", now_ms().to_string()),
        ("resolution", "86400".into()),
    ];
    let (spot, vol, candles) = tokio::join!(
        feed.binance("depth", symbol, &depth_params),
        feed.deribit("get_volatility_index_data", &vol_params),
        feed.http
            .get(format!("{}/market-data/daily-candles", feed.base))
            .query(&[("symbols", "BTCUSDT")])
            .send()
    );
    let spot = spot?;
    let vol = vol?;
    let (iv, mean_iv) = volatility_gate(&vol.data, now_ms())?;
    let candles = candles?
        .error_for_status()?
        .json::<BTreeMap<String, Vec<InsightDailyBar>>>()
        .await?;
    let bars = candles
        .get("BTCUSDT")
        .ok_or_else(|| anyhow::anyhow!("BTC daily history missing"))?;
    let bars = bars
        .iter()
        .filter(|b| b.close_time_ms < now_ms())
        .collect::<Vec<_>>();
    anyhow::ensure!(bars.len() >= 50, "Need 50 completed BTC candles");
    let prior = bars[bars.len() - 1];
    anyhow::ensure!(
        now_ms() - prior.close_time_ms <= 2 * DAY,
        "BTC trend history stale"
    );
    let sma = bars[bars.len() - 50..].iter().map(|b| b.close).sum::<f64>() / 50.0;
    let asks = levels(&spot.data, "asks")?;
    let spot_ask = asks
        .first()
        .ok_or_else(|| anyhow::anyhow!("BTC spot asks missing"))?
        .0;
    let cap = sleeve["config"]["max_trade_usd"].as_f64().unwrap_or(1000.0);
    let (instrument, metadata, book) = if let Some((name, _)) = held {
        let meta_params = [("instrument_name", name.to_owned())];
        let book_params = [
            ("instrument_name", name.to_owned()),
            ("depth", "100".into()),
        ];
        let (meta, book) = tokio::join!(
            feed.deribit("get_instrument", &meta_params),
            feed.deribit("get_order_book", &book_params)
        );
        let meta = meta?;
        let expired = meta.data["expiration_timestamp"]
            .as_u64()
            .is_some_and(|e| e <= now_ms());
        let book = if expired {
            book.ok().map(|r| r.data).unwrap_or_else(|| json!({}))
        } else {
            book?.data
        };
        (name.to_owned(), meta.data, book)
    } else {
        let instruments = feed
            .deribit(
                "get_instruments",
                &[
                    ("currency", "USDC".into()),
                    ("kind", "option".into()),
                    ("expired", "false".into()),
                ],
            )
            .await?;
        let rows = instruments
            .data
            .as_array()
            .ok_or_else(|| anyhow::anyhow!("Option instrument list missing"))?;
        let now = now_ms();
        let mut candidates = rows
            .iter()
            .filter(|r| {
                let days = r["expiration_timestamp"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(now) as f64
                    / DAY as f64;
                r["base_currency"] == "BTC"
                    && r["option_type"] == "call"
                    && r["instrument_type"] == "linear"
                    && (21.0..=45.0).contains(&days)
                    && r["strike"]
                        .as_f64()
                        .is_some_and(|k| k > spot_ask && k < spot_ask * 1.5)
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|a, b| {
            let score = |r: &Value| {
                let d = r["expiration_timestamp"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(now) as f64
                    / DAY as f64;
                (d - 30.0).abs()
                    + 100.0
                        * (r["strike"].as_f64().unwrap_or(spot_ask) / (spot_ask * 1.10) - 1.0).abs()
            };
            score(a).total_cmp(&score(b))
        });
        let candidates = candidates.into_iter().take(12).collect::<Vec<_>>();
        anyhow::ensure!(!candidates.is_empty(), "No active 21–45d BTC/USDC calls");
        let mut chosen = None;
        let requests = candidates.iter().map(|meta| async {
            let name = meta["instrument_name"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("Option name missing"))?;
            Ok::<_, anyhow::Error>((
                *meta,
                feed.deribit(
                    "get_order_book",
                    &[("instrument_name", name.into()), ("depth", "100".into())],
                )
                .await?
                .data,
            ))
        });
        for result in futures_util::future::join_all(requests).await {
            let Ok((meta, book)) = result else { continue };
            let delta = book["greeks"]["delta"].as_f64().unwrap_or(-1.0);
            let bid = levels(&book, "bids").ok().and_then(|x| x.first().copied());
            let ask = levels(&book, "asks").ok().and_then(|x| x.first().copied());
            if !(0.10..=0.25).contains(&delta) || bid.is_none() || ask.is_none() {
                continue;
            }
            let score = (delta - 0.20).abs();
            if chosen
                .as_ref()
                .is_none_or(|(_, _, _, best): &(String, Value, Value, f64)| score < *best)
            {
                chosen = Some((
                    meta["instrument_name"].as_str().unwrap().to_owned(),
                    meta.clone(),
                    book,
                    score,
                ));
            }
        }
        let (name, meta, book, _) =
            chosen.ok_or_else(|| anyhow::anyhow!("No two-sided 0.10–0.25 delta BTC/USDC call"))?;
        (name, meta, book)
    };
    let expiry = metadata["expiration_timestamp"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("Expiry missing"))?;
    let strike = number_field(&metadata, "strike")?;
    let minimum = number_field(&metadata, "min_trade_amount")?;
    let multiplier = if inverse { spot_ask } else { 1.0 };
    let held_quantity = held.and_then(|(_, p)| p["quantity"].as_f64());
    let quantity =
        held_quantity.unwrap_or(((cap / spot_ask / minimum).floor() * minimum).max(minimum));
    let spot_quote = quote(&spot.data, quantity, spot.requested_at_ms)?;
    let mut settlement = Value::Null;
    let (call_quote, delta) = if expiry <= now_ms() {
        let index = metadata["price_index"].as_str().unwrap_or(if inverse {
            "btc_usd"
        } else {
            "btc_usdc"
        });
        let deliveries = feed
            .deribit(
                "get_delivery_prices",
                &[("index_name", index.into()), ("count", "1000".into())],
            )
            .await?;
        let target = utc_date(expiry);
        let price = deliveries.data["data"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|r| r["date"] == target)
            .and_then(|r| r["delivery_price"].as_f64())
            .ok_or_else(|| anyhow::anyhow!("Official {target} settlement not published"))?;
        settlement = json!({"price_usd":price,"timestamp_ms":expiry,"source":format!("Deribit official {index} delivery {target}")});
        (
            json!({"bid":0.0,"ask":0.0,"bid_quantity":0.0,"ask_quantity":0.0,"timestamp_ms":now_ms()}),
            0.0,
        )
    } else {
        let mut q = quote(
            &book,
            quantity,
            book["timestamp"]
                .as_u64()
                .ok_or_else(|| anyhow::anyhow!("Option source timestamp missing"))?,
        )?;
        let bid = q["bid"].as_f64().unwrap() * multiplier;
        let ask = q["ask"].as_f64().unwrap() * multiplier;
        q["bid"] = json!(bid);
        q["ask"] = json!(ask);
        (
            q,
            book["greeks"]["delta"]
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("Call delta missing"))?,
        )
    };
    let rate = metadata["taker_commission"]
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("Published option fee missing"))?;
    let premium = call_quote["bid"]
        .as_f64()
        .unwrap_or(0.0)
        .max(call_quote["ask"].as_f64().unwrap_or(0.0));
    let fee = if expiry <= now_ms() {
        let settlement_price = settlement["price_usd"]
            .as_f64()
            .ok_or_else(|| anyhow::anyhow!("Settlement price missing"))?;
        (0.00015 * settlement_price).min(0.125 * (settlement_price - strike).max(0.0))
    } else {
        (rate * spot_ask).min(0.125 * premium)
    };
    let time = now_ms();
    let qualifies =
        held.is_some() || (prior.close <= sma && iv > mean_iv && cap >= minimum * spot_ask);
    let data = json!({
        "instrument":instrument,"spot":spot_quote,"call":call_quote,"strike_usd":strike,"expiry_ms":expiry,
        "delta":delta,"implied_volatility":iv,"prior_30d_mean_iv":mean_iv,
        "prior_close_usd":prior.close,"prior_sma50_usd":sma,"indicators_at_ms":prior.close_time_ms,
        "option_fee_usd_per_btc":fee,"settlement":settlement,"minimum_quantity":minimum,"quantity_step":minimum
    });
    let source_time = spot
        .requested_at_ms
        .min(data["call"]["timestamp_ms"].as_u64().unwrap_or(time));
    Ok(Output::ready(
        IDS[1],
        source_time,
        if qualifies {
            "qualified"
        } else {
            "waiting-for-regime"
        },
        format!("{instrument}; DVOL {iv:.2} vs prior mean {mean_iv:.2}; minimum {minimum} BTC"),
        json!({"instrument":instrument,"dvol":iv,"prior_30d_mean_dvol":mean_iv,"btc_sma50":sma,
            "delta":delta,"minimum_spot_notional_usdc":minimum*spot_ask,"numeraire":"USDC","published_taker_rate":rate}),
        vec![observation(
            IDS[1],
            "FeedHandlers Deribit linear BTC/USDC books, DVOL and official settlements; Insights regime/contract selection",
            time,
            data,
            &time.to_string(),
        )],
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dvol_excludes_partial_current_day_and_requires_contiguous_history() {
        let now = 100 * DAY + 1000;
        let mut rows = (69..100)
            .map(|d| json!([d * DAY, 0, 0, 0, 40.0]))
            .collect::<Vec<_>>();
        rows.push(json!([100 * DAY, 0, 0, 0, 80.0]));
        let (current, mean) = volatility_gate(&json!({"data":rows}), now).unwrap();
        assert_eq!(current, 80.0);
        assert_eq!(mean, 40.0);
        rows.remove(29);
        assert!(volatility_gate(&json!({"data":rows}), now).is_err());
        assert_eq!(utc_date(1795737600000), "2026-11-27");
    }
}
