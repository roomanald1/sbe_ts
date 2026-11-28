//! Source-preserving Binance USDⓈ-M dated-contract depth updates.
//!
//! This adapter intentionally does not publish into the existing spot `book:`
//! channel. The event contains incremental depth only; callers must establish
//! a synchronized snapshot before treating it as an executable book.

use anyhow::{Context, Result, anyhow, ensure};
use futures_util::{SinkExt, Stream, StreamExt};
use serde_json::Value;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const BINANCE_USDM_WS: &str = "wss://fstream.binance.com/ws";
const BINANCE_USDM_REST: &str = "https://fapi.binance.com/fapi/v1/depth";
const SNAPSHOT_LIMIT: &str = "1000";
static CONNECTION_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// One Binance source event for a specific dated USDⓈ-M futures contract.
///
/// Prices and quantities stay as decimal strings so downstream accounting does
/// not introduce binary floating-point rounding while retaining source data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatedFutureDepthUpdate {
    pub venue: &'static str,
    pub contract_symbol: String,
    pub connection_epoch: String,
    pub source_event_time_ms: u64,
    pub first_update_id: u64,
    pub final_update_id: u64,
    pub previous_final_update_id: u64,
    pub bids: Vec<[String; 2]>,
    pub asks: Vec<[String; 2]>,
}

/// Stream starts with a sequence-synchronized snapshot and continues with
/// cursor-contiguous diffs. Snapshot levels are the REST baseline; apply the
/// attached bridge diff before treating the result as a source-timestamped
/// executable book.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DatedFutureDepthEvent {
    SynchronizedSnapshot {
        venue: &'static str,
        contract_symbol: String,
        connection_epoch: String,
        source_event_time_ms: u64,
        snapshot_update_id: u64,
        bridge: DatedFutureDepthUpdate,
        bids: Vec<[String; 2]>,
        asks: Vec<[String; 2]>,
    },
    Update(DatedFutureDepthUpdate),
}

/// Connect to the exact dated contract's Binance USDⓈ-M diff-depth stream.
///
/// Reconnect by calling this function again; each returned stream has a new
/// local connection epoch. A stream boundary is therefore visible to callers
/// and cannot be mistaken for continuous book history.
pub async fn subscribe_dated_future_depth(
    contract_symbol: &str,
) -> Result<impl Stream<Item = Result<DatedFutureDepthEvent>>> {
    ensure!(
        is_dated_usdm_contract(contract_symbol),
        "expected exact Binance dated USDⓈ-M contract symbol, got {contract_symbol}"
    );
    let stream_name = format!("{}@depth@100ms", contract_symbol.to_ascii_lowercase());
    let url = format!("{BINANCE_USDM_WS}/{stream_name}");
    let (socket, _) = connect_async(&url)
        .await
        .with_context(|| format!("connecting to Binance dated-contract stream {stream_name}"))?;
    let epoch = new_connection_epoch()?;
    let expected_symbol = contract_symbol.to_owned();
    let snapshot = fetch_snapshot(&expected_symbol).await?;

    Ok(async_stream::try_stream! {
        let mut socket = socket;
        let mut buffered = VecDeque::new();
        let mut last_update_id = snapshot.last_update_id;
        let mut synchronized = false;

        while let Some(message) = socket.next().await {
            match message? {
                Message::Text(text) => {
                    let update = parse_depth_update(text.as_bytes(), &expected_symbol, &epoch)?;
                    if !synchronized {
                        buffered.push_back(update);
                        while let Some(candidate) = buffered.pop_front() {
                            if candidate.final_update_id <= snapshot.last_update_id {
                                continue;
                            }
                            let next_id = snapshot.last_update_id.checked_add(1)
                                .ok_or_else(|| anyhow!("Binance snapshot update ID overflow"))?;
                            ensure!(
                                candidate.first_update_id <= next_id
                                    && candidate.final_update_id >= next_id,
                                "Binance snapshot/diff gap for {}: snapshot={}, event=[{},{}]",
                                expected_symbol,
                                snapshot.last_update_id,
                                candidate.first_update_id,
                                candidate.final_update_id,
                            );
                            last_update_id = candidate.final_update_id;
                            let source_event_time_ms = candidate.source_event_time_ms;
                            yield DatedFutureDepthEvent::SynchronizedSnapshot {
                                venue: "BINANCE_USDM",
                                contract_symbol: expected_symbol.clone(),
                                connection_epoch: epoch.clone(),
                                source_event_time_ms,
                                snapshot_update_id: snapshot.last_update_id,
                                bridge: candidate,
                                bids: snapshot.bids.clone(),
                                asks: snapshot.asks.clone(),
                            };
                            synchronized = true;
                            break;
                        }
                    } else {
                        ensure!(
                            update.previous_final_update_id == last_update_id,
                            "Binance diff cursor gap for {} in epoch {}: expected pu={}, got {}",
                            expected_symbol,
                            epoch,
                            last_update_id,
                            update.previous_final_update_id,
                        );
                        ensure!(
                            update.final_update_id > last_update_id,
                            "Binance diff cursor did not advance for {}",
                            expected_symbol,
                        );
                        last_update_id = update.final_update_id;
                        yield DatedFutureDepthEvent::Update(update);
                    }
                }
                Message::Ping(payload) => socket.send(Message::Pong(payload)).await?,
                Message::Close(_) => break,
                Message::Binary(_) | Message::Pong(_) | Message::Frame(_) => {}
            }
        }
    })
}

struct DatedFutureDepthSnapshot {
    last_update_id: u64,
    bids: Vec<[String; 2]>,
    asks: Vec<[String; 2]>,
}

async fn fetch_snapshot(contract_symbol: &str) -> Result<DatedFutureDepthSnapshot> {
    let response: Value = reqwest::Client::new()
        .get(BINANCE_USDM_REST)
        .query(&[("symbol", contract_symbol), ("limit", SNAPSHOT_LIMIT)])
        .send()
        .await
        .context("requesting Binance USDⓈ-M dated-contract depth snapshot")?
        .error_for_status()
        .context("Binance dated-contract depth snapshot returned an error status")?
        .json()
        .await
        .context("decoding Binance dated-contract depth snapshot")?;
    let last_update_id = required_u64(&response, "lastUpdateId")?;
    let bids = required_levels(&response, "bids")?;
    let asks = required_levels(&response, "asks")?;
    ensure!(last_update_id > 0, "Binance snapshot update ID must be positive");
    ensure!(!bids.is_empty() && !asks.is_empty(), "Binance snapshot has an empty side");
    Ok(DatedFutureDepthSnapshot {
        last_update_id,
        bids,
        asks,
    })
}

fn new_connection_epoch() -> Result<String> {
    let time_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock precedes Unix epoch")?
        .as_millis();
    let sequence = CONNECTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("binance-usdm-ws:{time_ms}:{sequence}"))
}

fn is_dated_usdm_contract(symbol: &str) -> bool {
    let Some((pair, expiry)) = symbol.split_once('_') else {
        return false;
    };
    !pair.is_empty()
        && pair
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
        && expiry.len() == 6
        && expiry.bytes().all(|byte| byte.is_ascii_digit())
}

fn parse_depth_update(
    payload: &[u8],
    expected_symbol: &str,
    connection_epoch: &str,
) -> Result<DatedFutureDepthUpdate> {
    let event: Value = serde_json::from_slice(payload).context("parsing Binance depth event")?;
    ensure!(
        event.get("e").and_then(Value::as_str) == Some("depthUpdate"),
        "unexpected Binance market event type"
    );
    let contract_symbol = required_string(&event, "s")?;
    ensure!(
        contract_symbol == expected_symbol,
        "Binance contract mismatch: expected {expected_symbol}, got {contract_symbol}"
    );
    ensure!(
        is_dated_usdm_contract(contract_symbol),
        "Binance event is not a dated USDⓈ-M contract"
    );

    let source_event_time_ms = required_u64(&event, "E")?;
    let first_update_id = required_u64(&event, "U")?;
    let final_update_id = required_u64(&event, "u")?;
    ensure!(
        source_event_time_ms > 0,
        "Binance event timestamp must be positive"
    );
    ensure!(
        first_update_id > 0 && final_update_id >= first_update_id,
        "invalid Binance depth update ID range"
    );
    let previous_final_update_id = required_u64(&event, "pu")?;
    let bids = required_levels(&event, "b")?;
    let asks = required_levels(&event, "a")?;

    Ok(DatedFutureDepthUpdate {
        venue: "BINANCE_USDM",
        contract_symbol: contract_symbol.to_owned(),
        connection_epoch: connection_epoch.to_owned(),
        source_event_time_ms,
        first_update_id,
        final_update_id,
        previous_final_update_id,
        bids,
        asks,
    })
}

fn required_string<'a>(event: &'a Value, field: &str) -> Result<&'a str> {
    event
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| anyhow!("Binance depth event is missing string field {field}"))
}

fn required_u64(event: &Value, field: &str) -> Result<u64> {
    event
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| anyhow!("Binance depth event is missing unsigned integer field {field}"))
}

fn required_levels(event: &Value, field: &str) -> Result<Vec<[String; 2]>> {
    let levels = event
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Binance depth event is missing level array {field}"))?;
    levels
        .iter()
        .map(|level| {
            let pair = level
                .as_array()
                .filter(|pair| pair.len() == 2)
                .ok_or_else(|| anyhow!("Binance depth level must contain price and quantity"))?;
            let price = pair[0]
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Binance depth price must be a decimal string"))?;
            let quantity = pair[1]
                .as_str()
                .filter(|value| !value.is_empty())
                .ok_or_else(|| anyhow!("Binance depth quantity must be a decimal string"))?;
            ensure!(
                is_decimal(price) && decimal_is_positive(price),
                "Binance depth price must be a positive decimal"
            );
            ensure!(
                is_decimal(quantity),
                "Binance depth quantity must be a non-negative decimal"
            );
            Ok([price.to_owned(), quantity.to_owned()])
        })
        .collect()
}

fn is_decimal(value: &str) -> bool {
    let mut parts = value.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    !whole.is_empty()
        && whole.bytes().all(|byte| byte.is_ascii_digit())
        && fraction.is_none_or(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
        && parts.next().is_none()
}

fn decimal_is_positive(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| byte.is_ascii_digit() && byte != b'0')
}

#[cfg(test)]
mod tests {
    use super::{is_dated_usdm_contract, parse_depth_update};

    const EVENT: &str = r#"{
        "e":"depthUpdate","E":1700000000123,"T":1700000000120,
        "s":"BTCUSDT_240628","U":41,"u":43,"pu":40,
        "b":[["65000.10","0.25"]],"a":[["65000.20","0.50"]]
    }"#;

    #[test]
    fn parses_exact_contract_source_time_cursor_and_decimal_levels() {
        let update = parse_depth_update(EVENT.as_bytes(), "BTCUSDT_240628", "epoch-1").unwrap();
        assert_eq!(update.venue, "BINANCE_USDM");
        assert_eq!(update.contract_symbol, "BTCUSDT_240628");
        assert_eq!(update.connection_epoch, "epoch-1");
        assert_eq!(update.source_event_time_ms, 1_700_000_000_123);
        assert_eq!((update.first_update_id, update.final_update_id), (41, 43));
        assert_eq!(update.previous_final_update_id, 40);
        assert_eq!(update.bids, [["65000.10", "0.25"]]);
        assert_eq!(update.asks, [["65000.20", "0.50"]]);
    }

    #[test]
    fn rejects_wrong_contract_or_missing_provenance() {
        assert!(parse_depth_update(EVENT.as_bytes(), "BTCUSDT", "epoch-1").is_err());
        let missing_event_time = EVENT.replace("\"E\":1700000000123,", "");
        assert!(
            parse_depth_update(missing_event_time.as_bytes(), "BTCUSDT_240628", "epoch-1").is_err()
        );
        let invalid_cursor = EVENT.replace("\"u\":43", "\"u\":40");
        assert!(
            parse_depth_update(invalid_cursor.as_bytes(), "BTCUSDT_240628", "epoch-1").is_err()
        );
    }

    #[test]
    fn only_accepts_dated_contract_symbol_shape() {
        assert!(is_dated_usdm_contract("BTCUSDT_240628"));
        assert!(!is_dated_usdm_contract("BTCUSDT"));
        assert!(!is_dated_usdm_contract("BTCUSDT_2406A8"));
        assert!(!is_dated_usdm_contract("btcUSDT_240628"));
    }
}
