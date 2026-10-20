use anyhow::Result;
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt, TryStreamExt};
use reqwest::header::USER_AGENT;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::sync::{broadcast::Sender, mpsc};
use tokio::time::{Duration, Instant, sleep};
use tokio_tungstenite::{connect_async, tungstenite::Utf8Bytes};

use crate::encoders::{SYMBOL_LENGTH, encode_book_depth, encode_symbol_price, encode_trade_tick};

const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);
const STABLE_CONNECTION_DURATION: Duration = Duration::from_secs(60);
const VOLATILITY_HISTORY_MS: u64 = 300_000;
const OKX_IDLE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

#[derive(Default)]
pub(crate) struct RollingVolatility {
    samples: VecDeque<(u64, f64)>,
    minute_returns: VecDeque<(u64, f64)>,
    window_returns: VecDeque<(u64, f64)>,
    minute_squared_sum: f64,
    window_squared_sum: f64,
}

impl RollingVolatility {
    pub(crate) fn update(&mut self, timestamp_ms: u64, price: f64) -> (f64, f64) {
        if let Some(&(last_timestamp, _)) = self.samples.back() {
            if timestamp_ms < last_timestamp {
                return self.values(timestamp_ms);
            }
            if timestamp_ms == last_timestamp {
                if let Some(last_sample) = self.samples.back_mut() {
                    last_sample.1 = price;
                }
                if self.samples.len() > 1 {
                    let previous = self.samples[self.samples.len() - 2].1;
                    let squared = (price / previous).ln().powi(2);
                    self.replace_latest_return(timestamp_ms, squared);
                }
                return self.values(timestamp_ms);
            }
        }

        let previous_price = self.samples.back().map(|sample| sample.1);
        self.samples.push_back((timestamp_ms, price));
        if let Some(previous_price) = previous_price {
            let squared = (price / previous_price).ln().powi(2);
            self.prune_returns(timestamp_ms);
            self.minute_returns.push_back((timestamp_ms, squared));
            self.window_returns.push_back((timestamp_ms, squared));
            self.minute_squared_sum += squared;
            self.window_squared_sum += squared;
        }

        let oldest_to_keep = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
        while self.samples.len() > 2 && self.samples[1].0 < oldest_to_keep {
            self.samples.pop_front();
        }
        self.values(timestamp_ms)
    }

    fn prune_returns(&mut self, timestamp_ms: u64) {
        let minute_cutoff = timestamp_ms.saturating_sub(60_000);
        while self
            .minute_returns
            .front()
            .is_some_and(|(time, _)| *time <= minute_cutoff)
        {
            if let Some((_, squared)) = self.minute_returns.pop_front() {
                self.minute_squared_sum -= squared;
            }
        }
        let window_cutoff = timestamp_ms.saturating_sub(VOLATILITY_HISTORY_MS);
        while self
            .window_returns
            .front()
            .is_some_and(|(time, _)| *time <= window_cutoff)
        {
            if let Some((_, squared)) = self.window_returns.pop_front() {
                self.window_squared_sum -= squared;
            }
        }
    }

    fn replace_latest_return(&mut self, timestamp_ms: u64, squared: f64) {
        if let Some((time, previous)) = self.minute_returns.back_mut() {
            if *time == timestamp_ms {
                self.minute_squared_sum += squared - *previous;
                *previous = squared;
            }
        }
        if let Some((time, previous)) = self.window_returns.back_mut() {
            if *time == timestamp_ms {
                self.window_squared_sum += squared - *previous;
                *previous = squared;
            }
        }
    }

    fn values(&self, timestamp_ms: u64) -> (f64, f64) {
        if self
            .samples
            .back()
            .is_some_and(|(time, _)| *time == timestamp_ms)
        {
            return (
                volatility_from_sum(self.minute_squared_sum, self.minute_returns.len()),
                volatility_from_sum(self.window_squared_sum, self.window_returns.len()),
            );
        }
        (
            self.realized_volatility(timestamp_ms, 60_000),
            self.realized_volatility(timestamp_ms, VOLATILITY_HISTORY_MS),
        )
    }

    fn realized_volatility(&self, timestamp_ms: u64, window_ms: u64) -> f64 {
        let cutoff = timestamp_ms.saturating_sub(window_ms);
        let mut squared_returns = 0.0;
        let mut return_count = 0;
        let mut samples = self.samples.iter();
        let Some(mut previous) = samples.next() else {
            return f64::NAN;
        };
        for current in samples {
            if current.0 > cutoff {
                let log_return = (current.1 / previous.1).ln();
                squared_returns += log_return * log_return;
                return_count += 1;
            }
            previous = current;
        }
        volatility_from_sum(squared_returns, return_count)
    }
}

fn volatility_from_sum(squared_returns: f64, return_count: usize) -> f64 {
    if return_count == 0 {
        f64::NAN
    } else {
        squared_returns.max(0.0).sqrt() * 100.0
    }
}

pub async fn get_instrument_data() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/market/tickers?instType=SPOT";
    crate::service_log!("INFO", "{}", request_url);

    let client = reqwest::Client::new();
    let response = client
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;

    let json: Value = serde_json::from_str(&response.text().await?)?;

    let tickers = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot tickers response is missing data"))?;
    select_top_instruments(tickers)
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct SpotInstrumentPrecision {
    pub symbol: String,
    #[serde(rename = "tickSz")]
    pub tick_size: String,
    #[serde(rename = "lotSz")]
    pub lot_size: String,
}

pub async fn get_spot_instrument_precisions() -> Result<Vec<SpotInstrumentPrecision>> {
    let response = reqwest::Client::new()
        .get("https://www.okx.com/api/v5/public/instruments?instType=SPOT")
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot instruments response is missing data"))?;
    let mut result = instruments
        .iter()
        .filter_map(|instrument| {
            let symbol = instrument.get("instId")?.as_str()?;
            let is_live = instrument.get("state").and_then(Value::as_str) == Some("live");
            let tick_size = instrument.get("tickSz")?.as_str()?;
            let lot_size = instrument.get("lotSz")?.as_str()?;
            (is_live && symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH).then(|| {
                SpotInstrumentPrecision {
                    symbol: symbol.to_owned(),
                    tick_size: tick_size.to_owned(),
                    lot_size: lot_size.to_owned(),
                }
            })
        })
        .collect::<Vec<_>>();
    result.sort_unstable_by(|a, b| a.symbol.cmp(&b.symbol));
    anyhow::ensure!(
        !result.is_empty(),
        "OKX returned no live spot instrument precisions"
    );
    Ok(result)
}

pub async fn get_all_spot_instruments() -> Result<Vec<String>> {
    let request_url = "https://www.okx.com/api/v5/public/instruments?instType=SPOT";
    let response = reqwest::Client::new()
        .get(request_url)
        .header(USER_AGENT, "rust-web-api-client")
        .send()
        .await?
        .error_for_status()?;
    let json: Value = serde_json::from_str(&response.text().await?)?;
    let instruments = json
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("OKX spot instruments response is missing data"))?;
    let mut symbols = instruments
        .iter()
        .filter_map(|instrument| {
            let symbol = instrument.get("instId")?.as_str()?;
            let is_live = instrument.get("state").and_then(Value::as_str) == Some("live");
            (is_live && symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH)
                .then(|| symbol.to_owned())
        })
        .collect::<Vec<_>>();
    symbols.sort_unstable();
    symbols.dedup();
    anyhow::ensure!(!symbols.is_empty(), "OKX returned no live spot instruments");
    Ok(symbols)
}

fn select_top_instruments(tickers: &[Value]) -> Result<Vec<String>> {
    let mut instruments = tickers
        .iter()
        .filter_map(|ticker| {
            let instrument = ticker.get("instId")?.as_str()?;
            let volume = ticker.get("volCcy24h")?.as_str()?.parse::<f64>().ok()?;

            (instrument.is_ascii()
                && instrument.len() <= SYMBOL_LENGTH
                && volume.is_finite()
                && volume >= 0.0)
                .then(|| (instrument.to_owned(), volume))
        })
        .collect::<Vec<_>>();
    instruments.sort_by(|(left_id, left_volume), (right_id, right_volume)| {
        right_volume
            .total_cmp(left_volume)
            .then_with(|| left_id.cmp(right_id))
    });
    instruments.truncate(100);

    anyhow::ensure!(
        !instruments.is_empty(),
        "OKX returned no valid spot tickers"
    );

    Ok(instruments
        .into_iter()
        .map(|(instrument, _)| instrument)
        .collect())
}

#[derive(Deserialize)]
struct TickerMessage<'a> {
    #[serde(borrow)]
    data: Option<Vec<Ticker<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
}

#[derive(Deserialize)]
struct Ticker<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
    #[serde(borrow)]
    last: &'a str,
    #[serde(borrow)]
    ts: Option<&'a str>,
}

#[derive(Deserialize)]
struct BookMessage<'a> {
    #[serde(borrow)]
    arg: Option<BookArg<'a>>,
    #[serde(borrow)]
    data: Option<Vec<BookUpdate<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
    action: Option<&'a str>,
}

#[derive(Deserialize)]
struct BookArg<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
}

#[derive(Deserialize)]
struct BookUpdate<'a> {
    #[serde(borrow)]
    asks: Vec<Vec<&'a str>>,
    #[serde(borrow)]
    bids: Vec<Vec<&'a str>>,
    #[serde(borrow)]
    ts: &'a str,
}

const BOOK_DEPTH: usize = 7;

#[derive(Deserialize)]
struct TradeMessage<'a> {
    #[serde(borrow)]
    data: Option<Vec<Trade<'a>>>,
    event: Option<&'a str>,
    msg: Option<&'a str>,
}

#[derive(Deserialize)]
struct Trade<'a> {
    #[serde(rename = "instId", borrow)]
    instrument: &'a str,
    #[serde(rename = "tradeId", borrow)]
    trade_id: &'a str,
    #[serde(borrow)]
    px: &'a str,
    #[serde(borrow)]
    sz: &'a str,
    #[serde(borrow)]
    side: &'a str,
    #[serde(borrow)]
    ts: &'a str,
}

#[derive(Clone, Debug)]
pub struct TickerSample {
    pub instrument: String,
    pub price: f64,
    pub timestamp: u64,
}

pub async fn subscribe_ticker_samples(
    instruments: &[String],
    updates: mpsc::Sender<TickerSample>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| {
            anyhow::anyhow!("failed to connect to OKX hedge ticker stream: {error}")
        })?;
    let (mut write, mut read) = incoming_ws.split();
    for (batch_index, batch) in instruments.chunks(240).enumerate() {
        let args = batch
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument }))
            .collect::<Vec<_>>();
        let request = json!({
            "op": "subscribe",
            "id": (batch_index + 1).to_string(),
            "args": args,
        });
        write
            .send(tokio_tungstenite::tungstenite::Message::Text(
                Utf8Bytes::from(request.to_string()),
            ))
            .await?;
        if batch_index + 1 < instruments.chunks(240).len() {
            sleep(Duration::from_millis(500)).await;
        }
    }
    loop {
        tokio::select! {
            _ = updates.closed() => return Ok(()),
            message = read.next() => {
                let Some(message) = message else { anyhow::bail!("OKX hedge ticker stream ended") };
                let message = message?;
                if !message.is_text() { continue; }
                let update = match serde_json::from_str::<TickerMessage<'_>>(message.to_text()?) {
                    Ok(update) => update,
                    Err(error) => { crate::service_log!("ERROR", "Ignoring invalid OKX hedge ticker JSON: {error}"); continue; }
                };
                if update.event == Some("error") {
                    anyhow::bail!("OKX hedge ticker subscription error: {}", update.msg.unwrap_or("unknown error"));
                }
                for ticker in update.data.unwrap_or_default() {
                    let (Some(timestamp), Ok(price)) = (
                        ticker.ts.and_then(|value| value.parse::<u64>().ok()),
                        ticker.last.parse::<f64>(),
                    ) else { continue; };
                    if !price.is_finite() || price <= 0.0 { continue; }
                    updates.send(TickerSample {
                        instrument: ticker.instrument.to_owned(),
                        price,
                        timestamp,
                    }).await.map_err(|_| anyhow::anyhow!("hedge ticker sample receiver was dropped"))?;
                }
            }
        }
    }
}

pub async fn subscribe_trades_once(
    instrument: &str,
    updates: mpsc::Sender<Result<Bytes>>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX trades WebSocket: {error}"))?;
    let (mut write, mut read) = incoming_ws.split();
    let request =
        json!({"op": "subscribe", "args": [{ "channel": "trades", "instId": instrument }]});
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            Utf8Bytes::from(request.to_string()),
        ))
        .await?;
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    tokio::pin!(heartbeat);
    loop {
        tokio::select! {
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            _ = updates.closed() => {
                let request = json!({"op": "unsubscribe", "args": [{ "channel": "trades", "instId": instrument }]});
                let _ = write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from(request.to_string()))).await;
                return Ok(());
            }
            message = read.next() => {
                let Some(message) = message else { anyhow::bail!("OKX trades WebSocket stream ended") };
                let message = message?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                if !message.is_text() { continue; }
                let text = message.to_text()?;
                if text == "pong" { continue; }
                let update: TradeMessage<'_> = match serde_json::from_str(text) { Ok(update) => update, Err(error) => { crate::service_log!("ERROR", "Ignoring invalid OKX trades JSON message: {error}"); continue; } };
                if update.event == Some("error") { anyhow::bail!("OKX trades subscription error: {}", update.msg.unwrap_or("unknown error")); }
                let Some(trades) = update.data else { continue; };
                for trade in trades {
                    if trade.instrument != instrument {
                        continue;
                    }
                    let (Ok(price), Ok(size), Ok(timestamp)) = (
                        trade.px.parse::<f64>(),
                        trade.sz.parse::<f64>(),
                        trade.ts.parse::<u64>(),
                    ) else {
                        continue;
                    };
                    let encoded = encode_trade_tick(
                        instrument,
                        trade.trade_id,
                        timestamp,
                        price,
                        size,
                        trade.side,
                    )?;
                    updates
                        .send(Ok(encoded))
                        .await
                        .map_err(|_| anyhow::anyhow!("trades stream receiver was dropped"))?;
                }
            }
        }
    }
}

pub async fn subscribe(instrument_data: &[Arc<str>], price_updates: Sender<Bytes>) -> Result<()> {
    let instrument_lookup = Arc::new(instrument_data.iter().cloned().collect::<HashSet<_>>());
    let mut retry_delay = INITIAL_RECONNECT_DELAY;

    loop {
        let attempt_started = Instant::now();
        let result = subscribe_once(
            instrument_data,
            Arc::clone(&instrument_lookup),
            price_updates.clone(),
        )
        .await;

        retry_delay = retry_delay_after_failure(retry_delay, attempt_started.elapsed());

        match result {
            Ok(()) => crate::service_log!("ERROR", 
                "OKX WebSocket stream ended; reconnecting in {}s",
                retry_delay.as_secs()
            ),
            Err(error) => crate::service_log!("ERROR", 
                "OKX WebSocket subscription failed: {error:#}; reconnecting in {}s",
                retry_delay.as_secs()
            ),
        }

        sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

pub async fn subscribe_book_once(
    instrument: &str,
    updates: mpsc::Sender<Result<Bytes>>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX book WebSocket: {error}"))?;
    let (mut write, mut read) = incoming_ws.split();
    let request = json!({
        "op": "subscribe",
        "args": [{ "channel": "books", "instId": instrument }]
    });
    write
        .send(tokio_tungstenite::tungstenite::Message::Text(
            Utf8Bytes::from(request.to_string()),
        ))
        .await?;
    let heartbeat = tokio::time::sleep(OKX_IDLE_HEARTBEAT_INTERVAL);
    tokio::pin!(heartbeat);

    let mut bid_book = Vec::new();
    let mut ask_book = Vec::new();
    loop {
        tokio::select! {
            _ = &mut heartbeat => {
                write.send(tokio_tungstenite::tungstenite::Message::Text(Utf8Bytes::from("ping"))).await?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
            }
            _ = updates.closed() => {
                crate::service_log!("INFO", "UI unsubscribed from OKX books for {instrument}");
                let request = json!({
                    "op": "unsubscribe",
                    "args": [{ "channel": "books", "instId": instrument }]
                });
                write
                    .send(tokio_tungstenite::tungstenite::Message::Text(
                        Utf8Bytes::from(request.to_string()),
                    ))
                    .await
                    .map_err(|error| anyhow::anyhow!("failed to send OKX books unsubscribe for {instrument}: {error}"))?;
                crate::service_log!("INFO", "Sent OKX books unsubscribe for {instrument}");
                return Ok(());
            },
            message = read.next() => {
                let Some(message) = message else {
                    anyhow::bail!("OKX book WebSocket stream ended");
                };
                let message = message?;
                heartbeat.as_mut().reset(tokio::time::Instant::now() + OKX_IDLE_HEARTBEAT_INTERVAL);
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring non-text OKX book message: {error}");
                        continue;
                    }
                };
                if text == "pong" { continue; }
                let update = match serde_json::from_str::<BookMessage<'_>>(text) {
                    Ok(update) => update,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring invalid OKX book JSON message: {error}");
                        continue;
                    }
                };

                if update.event == Some("error") {
                    anyhow::bail!(
                        "OKX book subscription error: {}",
                        update.msg.unwrap_or("unknown subscription error")
                    );
                }

                let Some(instrument_arg) = update.arg else {
                    continue;
                };
                if instrument_arg.instrument != instrument {
                    continue;
                }
                let Some(items) = update.data else {
                    continue;
                };
                for item in items {
                    if update.action == Some("snapshot") {
                        bid_book.clear();
                        ask_book.clear();
                    }
                    apply_book_side(&mut bid_book, &item.bids)?;
                    apply_book_side(&mut ask_book, &item.asks)?;
                    let bids = top_book_side(&bid_book, true);
                    let asks = top_book_side(&ask_book, false);
                    let timestamp = item
                        .ts
                        .parse::<u64>()
                        .map_err(|error| anyhow::anyhow!("invalid OKX book timestamp: {error}"))?;
                    let encoded = encode_book_depth(instrument, timestamp, &bids, &asks)?;
                    updates
                        .send(Ok(encoded))
                        .await
                        .map_err(|_| anyhow::anyhow!("book stream receiver was dropped"))?;
                }
            }
        }
    }
}

fn apply_book_side(book: &mut Vec<(f64, f64)>, rows: &[Vec<&str>]) -> Result<()> {
    for (row_index, row) in rows.iter().enumerate() {
        anyhow::ensure!(
            row.len() >= 2,
            "book row {row_index} must contain price and size"
        );
        let price = row[0]
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid price in book row {row_index}: {error}"))?;
        let size = row[1]
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid size in book row {row_index}: {error}"))?;
        anyhow::ensure!(
            price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0,
            "book row {row_index} contains an invalid price or size"
        );
        if let Some(index) = book.iter().position(|(book_price, _)| *book_price == price) {
            if size == 0.0 {
                book.remove(index);
            } else {
                book[index].1 = size;
            }
        } else if size > 0.0 {
            book.push((price, size));
        }
    }
    Ok(())
}

fn top_book_side(book: &[(f64, f64)], bids: bool) -> [(f64, f64); BOOK_DEPTH] {
    let mut levels = book.to_vec();
    levels.sort_by(|left, right| {
        if bids {
            right.0.total_cmp(&left.0)
        } else {
            left.0.total_cmp(&right.0)
        }
    });
    let mut top = [(0.0, 0.0); BOOK_DEPTH];
    for (index, level) in levels.into_iter().take(BOOK_DEPTH).enumerate() {
        top[index] = level;
    }
    top
}

async fn subscribe_once(
    instrument_data: &[Arc<str>],
    instrument_lookup: Arc<HashSet<Arc<str>>>,
    price_updates: Sender<Bytes>,
) -> Result<()> {
    let (incoming_ws, _) = connect_async("wss://ws.okx.com:8443/ws/v5/public")
        .await
        .map_err(|error| anyhow::anyhow!("failed to connect to OKX WebSocket: {error}"))?;

    crate::service_log!("INFO", "OKX WebSocket handshake has been successfully completed");
    let (mut write, read) = incoming_ws.split();
    let volatility_by_instrument =
        Arc::new(Mutex::new(HashMap::<String, RollingVolatility>::new()));
    let ws = {
        let read = read.map(|message| message.map_err(anyhow::Error::from));
        read.try_for_each(move |message| {
            let instrument_lookup = instrument_lookup.clone();
            let price_updates = price_updates.clone();
            let volatility_by_instrument = Arc::clone(&volatility_by_instrument);
            async move {
                let text = match message.to_text() {
                    Ok(text) => text,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring non-text OKX WebSocket message: {error}");
                        return Ok(());
                    }
                };
                let update = match serde_json::from_str::<TickerMessage<'_>>(text) {
                    Ok(update) => update,
                    Err(error) => {
                        crate::service_log!("ERROR", "Ignoring invalid OKX JSON message: {error}");
                        return Ok(());
                    }
                };

                if update.event == Some("error") {
                    anyhow::bail!(
                        "OKX subscription error: {}",
                        update.msg.unwrap_or("unknown subscription error")
                    );
                }

                if let Some(items) = update.data {
                    for item in items {
                        let Some(instrument) = instrument_lookup.get(item.instrument) else {
                            continue;
                        };
                        let Ok(price) = item.last.parse::<f64>() else {
                            continue;
                        };
                        if price.is_finite() && price > 0.0 {
                            let timestamp_ms = item
                                .ts
                                .and_then(|value| value.parse::<u64>().ok())
                                .unwrap_or_else(|| {
                                    SystemTime::now()
                                        .duration_since(UNIX_EPOCH)
                                        .unwrap_or_default()
                                        .as_millis() as u64
                                });
                            let (volatility_1m, volatility_5m) = volatility_by_instrument
                                .lock()
                                .expect("volatility state lock poisoned")
                                .entry(instrument.to_string())
                                .or_default()
                                .update(timestamp_ms, price);
                            let encoded = encode_symbol_price(
                                instrument,
                                price,
                                volatility_1m,
                                volatility_5m,
                            )?;
                            let _ = price_updates.send(encoded);
                        }
                    }
                }
                Ok(())
            }
        })
    };

    for (batch_index, instruments) in instrument_data.chunks(240).enumerate() {
        let args = instruments
            .iter()
            .map(|instrument| json!({ "channel": "tickers", "instId": instrument.as_ref() }))
            .collect::<Vec<_>>();
        let request = json!({
            "op": "subscribe",
            "id": (batch_index + 1).to_string(),
            "args": args
        });
        crate::service_log!("INFO", "Subscribing to {}", request);
        let request = Utf8Bytes::from(request.to_string());

        write
            .send(tokio_tungstenite::tungstenite::Message::Text(request))
            .await?;
    }

    ws.await?;
    anyhow::bail!("OKX WebSocket stream ended")
}

fn retry_delay_after_failure(current_delay: Duration, connection_duration: Duration) -> Duration {
    if connection_duration >= STABLE_CONNECTION_DURATION {
        INITIAL_RECONNECT_DELAY
    } else {
        current_delay
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BOOK_DEPTH, BookMessage, INITIAL_RECONNECT_DELAY, MAX_RECONNECT_DELAY,
        STABLE_CONNECTION_DURATION, TickerMessage, apply_book_side, retry_delay_after_failure,
        select_top_instruments, top_book_side,
    };
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn reconnect_delay_resets_after_a_stable_connection() {
        assert_eq!(
            retry_delay_after_failure(
                MAX_RECONNECT_DELAY,
                STABLE_CONNECTION_DURATION + Duration::from_secs(1)
            ),
            INITIAL_RECONNECT_DELAY
        );
    }

    #[test]
    fn reconnect_delay_is_retained_after_a_short_connection() {
        assert_eq!(
            retry_delay_after_failure(Duration::from_secs(8), Duration::from_secs(5)),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn selects_top_spot_instruments_by_24_hour_quote_volume() {
        let tickers = vec![
            json!({"instId": "LOW-USDT", "volCcy24h": "10"}),
            json!({"instId": "HIGH-USDT", "volCcy24h": "1000"}),
            json!({"instId": "MID-USDT", "volCcy24h": "100"}),
        ];

        assert_eq!(
            select_top_instruments(&tickers).unwrap(),
            ["HIGH-USDT", "MID-USDT", "LOW-USDT"]
        );
    }

    #[test]
    fn selects_no_more_than_the_top_100_instruments() {
        let tickers = (0..110)
            .map(|index| {
                json!({
                    "instId": format!("COIN{index:03}-USDT"),
                    "volCcy24h": index.to_string()
                })
            })
            .collect::<Vec<_>>();

        let selected = select_top_instruments(&tickers).unwrap();
        assert_eq!(selected.len(), 100);
        assert_eq!(selected[0], "COIN109-USDT");
        assert_eq!(selected[99], "COIN010-USDT");
    }

    #[test]
    fn ticker_updates_borrow_instrument_and_price_from_json() {
        let text =
            r#"{"arg":{"channel":"tickers"},"data":[{"instId":"BTC-USDT","last":"84000.5"}]}"#;
        let message: TickerMessage<'_> = serde_json::from_str(text).unwrap();
        let ticker = &message.data.unwrap()[0];

        assert_eq!(ticker.instrument, "BTC-USDT");
        assert_eq!(ticker.last, "84000.5");
    }

    #[test]
    fn parses_okx_book_snapshot_and_keeps_seven_best_levels() {
        let text = r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":[["101","1"],["102","2"]],"bids":[["99","3"],["98","4"]],"ts":"1700000000000"}]}"#;
        let message: BookMessage<'_> = serde_json::from_str(text).unwrap();
        let update = &message.data.unwrap()[0];
        let mut bid_book = Vec::new();
        let mut ask_book = Vec::new();
        apply_book_side(&mut bid_book, &update.bids).unwrap();
        apply_book_side(&mut ask_book, &update.asks).unwrap();
        let bids = top_book_side(&bid_book, true);
        let asks = top_book_side(&ask_book, false);

        assert_eq!(message.arg.unwrap().instrument, "BTC-USDT");
        assert_eq!(update.ts, "1700000000000");
        assert_eq!(bids[0], (99.0, 3.0));
        assert_eq!(bids[1], (98.0, 4.0));
        assert_eq!(bids[2..], [(0.0, 0.0); BOOK_DEPTH - 2]);
        assert_eq!(asks[0], (101.0, 1.0));
        assert_eq!(asks[1], (102.0, 2.0));
    }

    #[test]
    fn rejects_invalid_okx_book_levels() {
        assert!(apply_book_side(&mut Vec::new(), &[vec!["NaN", "1"]]).is_err());
        assert!(apply_book_side(&mut Vec::new(), &[vec!["100", "-1"]]).is_err());
        assert!(apply_book_side(&mut Vec::new(), &[vec!["100"]]).is_err());
    }
}
