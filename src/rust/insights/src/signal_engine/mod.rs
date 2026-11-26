use anyhow::{Context, Result};
use axum::{Json, Router, http::StatusCode, routing::get};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio::sync::{Mutex, mpsc, oneshot};

const PRICE_WINDOW_MS: u64 = 3_610_000;
const TRADE_WINDOW_MS: u64 = 300_000;
const ABSORPTION_WINDOW_MS: u64 = 60_000;
const STRATEGY_REJECTION_AUDIT_RETENTION_MS: u64 = 60 * 60 * 1_000;
const MAX_BOOK_AGE_MS: u64 = 5_000;
const MAX_PRICE_LOOKBACK_GAP_MS: u64 = 5_000;
/// Heavy research capture is sampled independently from live signal generation.
// Persist one research/signal sample every ten seconds. Live signal snapshots
// are still computed on every feed event, while this cadence limits the
// historical database growth from snapshots, book data, and their outcomes.
pub const RESEARCH_SAMPLE_INTERVAL_MS: u64 = 10_000;
/// Recompute and publish live signals at most once per symbol every two seconds.
pub const LIVE_SIGNAL_SNAPSHOT_INTERVAL_MS: u64 = 2_000;
const CALIBRATION_SNAPSHOT_LIMIT: usize = 1_000;
const CALIBRATION_OUTCOME_LIMIT: usize = 5_000;
const MAX_DURABLE_SCOUT_PATH_REGISTRATIONS: i64 = 32_768;

include!("engine/signal_contracts.rs");
include!("engine/signal_engine_types.rs");

impl Default for SignalEngine {
    fn default() -> Self {
        Self {
            prices: VecDeque::with_capacity(512),
            snapshot_price_change_square_prefix: Vec::new(),
            pending: VecDeque::with_capacity(256),
            pending_scratch: VecDeque::with_capacity(256),
            trades: VecDeque::with_capacity(2_048),
            snapshot_trade_buy_prefix: Vec::new(),
            snapshot_trade_sell_prefix: Vec::new(),
            snapshot_trade_notional_prefix: Vec::new(),
            snapshot_trade_quantity_prefix: Vec::new(),
            snapshot_absorption_events: Vec::new(),
            absorption_trades: VecDeque::with_capacity(512),
            book_imbalance: None,
            best_quote: None,
            book_bid_levels: Vec::with_capacity(16),
            book_ask_levels: Vec::with_capacity(16),
            book_history: VecDeque::with_capacity(64),
            volatility_1m: None,
            last_research_sample_at: None,
            last_live_snapshot_at: None,
            completed_scout_ids: BTreeSet::new(),
            completed_scout_order: VecDeque::new(),
        }
    }
}

fn breakout_watch(
    components: &SignalComponents,
    range_bounds: Option<(f64, f64)>,
    range_probability: Option<f64>,
) -> Option<(&'static str, f64)> {
    if range_probability? < 0.65 {
        return None;
    }
    let (lower, upper) = range_bounds?;
    let factors = [
        (
            components
                .trend_1m_percent
                .map(|value| (value / 0.25).clamp(-1.0, 1.0)),
            0.15,
        ),
        (
            components
                .momentum_10s_percent
                .map(|value| (value / 0.12).clamp(-1.0, 1.0)),
            0.20,
        ),
        (components.trade_flow_imbalance, 0.25),
        (components.book_imbalance, 0.25),
        (
            components.pressure_without_price_move.map(|value| -value),
            0.15,
        ),
    ];
    let (weighted_bias, available_weight) = factors
        .iter()
        .filter_map(|(value, weight)| value.map(|value| (value * weight, *weight)))
        .fold((0.0, 0.0), |(bias, weight_sum), (value, weight)| {
            (bias + value, weight_sum + weight)
        });
    if available_weight == 0.0 {
        return None;
    }
    let bias = weighted_bias / available_weight;
    if bias.abs() < 0.3 {
        return None;
    }

    let midpoint = lower + (upper - lower) * 0.5;
    let width_percent = if midpoint > 0.0 {
        (upper - lower) / midpoint * 100.0
    } else {
        f64::INFINITY
    };
    let expected_move = components.volatility_1m_percent.unwrap_or_default().abs() * 5.0_f64.sqrt();
    let compression = if expected_move > 0.0 {
        (1.0 - width_percent / (expected_move * 3.0)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let pressure = (bias.abs() * (0.7 + 0.3 * compression)).clamp(0.0, 1.0);
    (pressure >= 0.35).then_some((if bias > 0.0 { "UP" } else { "DOWN" }, pressure))
}

include!("engine/signal_outcome_models.rs");

include!("engine/signal_engine_outcomes.rs");
include!("engine/signal_engine_research.rs");
include!("engine/signal_engine_market_inputs.rs");
include!("engine/signal_engine_snapshot.rs");
include!("engine/signal_snapshot_diagnostics.rs");
include!("engine/signal_engine_state.rs");

include!("store/signal_store_types.rs");

include!("store/signal_record_codec.rs");

include!("store/signal_store_database.rs");
include!("store/signal_store_audit.rs");
include!("store/signal_store_execution_quality.rs");
const DECISION_QUALITY_MIN_COHORT_SAMPLES: usize = 10;
include!("store/signal_store_decision_quality.rs");
include!("store/signal_store_routes.rs");
include!("store/close_insights_api.rs");
include!("store/signal_store_market_data.rs");
include!("store/signal_store_research_history.rs");
include!("store/signal_store_snapshot_history.rs");
include!("store/signal_store_dashboard.rs");
include!("store/signal_store_performance.rs");
include!("store/signal_store_record_writes.rs");

#[path = "store/browser_rsocket.rs"]
mod browser_rsocket;
pub use browser_rsocket::serve_insights_browser_rsocket;

fn run_database_writer(mut connection: Connection, mut receiver: mpsc::Receiver<WriteRequest>) {
    const MAX_BATCH_REQUESTS: usize = 128;
    while let Some(first) = receiver.blocking_recv() {
        let mut batch = vec![first];
        while batch.len() < MAX_BATCH_REQUESTS {
            match receiver.try_recv() {
                Ok(request) => batch.push(request),
                Err(_) => break,
            }
        }
        let mut scout_path_validation_error = None;
        let result = (|| -> Result<()> {
            // Do serialization and compression before taking SQLite's write lock.
            let mut signal_rows = Vec::new();
            let mut audit_rows = Vec::new();
            let mut scout_registrations = Vec::new();
            let mut scout_path_samples = Vec::new();
            let mut failed_scout_paths = Vec::new();
            let mut completed_scout_paths = Vec::new();
            for request in &batch {
                match &request.operation {
                    WriteOperation::Records(records) => {
                        for record in records {
                            signal_rows.push((
                                record_timestamp(record) as i64,
                                record_kind(record),
                                compress_record(record)?,
                            ));
                        }
                    }
                    WriteOperation::Audit { decision_id, stage, timestamp, symbol, payload } => {
                        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
                        encoder.write_all(payload)?;
                        audit_rows.push((decision_id, stage, timestamp, symbol, encoder.finish()?));
                    }
                    WriteOperation::ScoutRegistration { decision_id, symbol, signal_timestamp, direction, entry_price, entry_bid, entry_ask, entry_quote_timestamp } => {
                        scout_registrations.push((decision_id, symbol, signal_timestamp, direction, entry_price, entry_bid, entry_ask, entry_quote_timestamp));
                    }
                    WriteOperation::ScoutPathSamples(samples) => scout_path_samples.extend(samples.iter()),
                    WriteOperation::ScoutPathFailure(decision_ids) => failed_scout_paths.extend(decision_ids.iter()),
                    WriteOperation::ScoutPathComplete(decision_id) => completed_scout_paths.push(decision_id),
                }
            }
            let has_scout_path_operations = !scout_registrations.is_empty()
                || !scout_path_samples.is_empty()
                || !failed_scout_paths.is_empty()
                || !completed_scout_paths.is_empty();
            let transaction = connection.transaction()?;
            if !signal_rows.is_empty() {
                let mut insert = transaction.prepare(
                    "INSERT INTO signal_records(timestamp, kind, payload) VALUES (?1, ?2, ?3)",
                )?;
                for (timestamp, kind, payload) in signal_rows {
                    insert.execute(params![timestamp, kind, payload])?;
                }
            }
            if !audit_rows.is_empty() {
                let rejection_audit_cutoff = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis()
                    .saturating_sub(STRATEGY_REJECTION_AUDIT_RETENTION_MS as u128) as i64;
                transaction.execute(
                    "DELETE FROM auto_trader_audit WHERE stage = 'strategy_rejection' AND timestamp < ?1",
                    [rejection_audit_cutoff],
                )?;
                let mut insert = transaction.prepare(
                    "INSERT INTO auto_trader_audit(decision_id, stage, timestamp, symbol, payload) VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(decision_id, stage) DO UPDATE SET timestamp=excluded.timestamp, symbol=excluded.symbol, payload=excluded.payload",
                )?;
                for (decision_id, stage, timestamp, symbol, payload) in audit_rows {
                    if stage == "strategy_rejection" && *timestamp < rejection_audit_cutoff {
                        continue;
                    }
                    insert.execute(params![decision_id, stage, timestamp, symbol, payload])?;
                }
            }
            if !scout_registrations.is_empty() {
                const SCOUT_PATH_RETENTION_MS: u64 = 7 * 86_400_000;
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;
                let cutoff = now.saturating_sub(SCOUT_PATH_RETENTION_MS) as i64;
                transaction.execute(
                    "DELETE FROM scout_path_samples WHERE decision_id IN (SELECT decision_id FROM scout_path_registrations WHERE signal_timestamp < ?1)",
                    [cutoff],
                )?;
                transaction.execute(
                    "DELETE FROM scout_path_registrations WHERE signal_timestamp < ?1",
                    [cutoff],
                )?;
                let mut insert = transaction.prepare(
                    "INSERT INTO scout_path_registrations(decision_id, symbol, signal_timestamp, direction, entry_price, entry_bid, entry_ask, entry_quote_timestamp) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(decision_id) DO NOTHING",
                )?;
                for (decision_id, symbol, timestamp, direction, entry_price, entry_bid, entry_ask, quote_timestamp) in scout_registrations {
                    insert.execute(params![decision_id, symbol, *timestamp as i64, direction, entry_price, entry_bid, entry_ask, quote_timestamp.map(|value| value as i64)])?;
                    transaction.execute(
                        "UPDATE scout_path_registrations SET capture_complete=0 WHERE decision_id=?1 AND (symbol<>?2 OR signal_timestamp<>?3 OR direction<>?4 OR abs(entry_price-?5)>1e-12 OR (?6 IS NOT NULL AND entry_bid IS NOT ?6) OR (?7 IS NOT NULL AND entry_ask IS NOT ?7) OR (?8 IS NOT NULL AND entry_quote_timestamp IS NOT ?8))",
                        params![decision_id, symbol, *timestamp as i64, direction, entry_price, entry_bid, entry_ask, quote_timestamp.map(|value| value as i64)],
                    )?;
                }
            }
            if !scout_path_samples.is_empty() {
                let mut insert = transaction.prepare(
                    "INSERT OR IGNORE INTO scout_path_samples(decision_id, path_epoch, sequence, timestamp, bid, ask, freshness_ms, gap_ms, first_source_timestamp, maximum_gap_ms, source_timestamps_monotonic, maximum_favorable_excursion_percent, maximum_adverse_excursion_percent, sample_count, capture_complete) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                )?;
                let mut touched_scout_paths = HashSet::new();
                let mut inserted_per_path = HashMap::<&str, u64>::new();
                for sample in scout_path_samples {
                    let registration: Option<(String, i64)> = transaction
                        .query_row(
                            "SELECT symbol, active FROM scout_path_registrations WHERE decision_id=?1",
                            [&sample.decision_id],
                            |row| Ok((row.get(0)?, row.get(1)?)),
                        )
                        .optional()?;
                    if !registration.as_ref().is_some_and(|(symbol, active)| {
                        symbol == &sample.symbol && *active != 0
                    }) {
                        if registration.is_some() {
                            transaction.execute(
                                "UPDATE scout_path_registrations SET capture_complete=0 WHERE decision_id=?1",
                                [&sample.decision_id],
                            )?;
                        }
                        scout_path_validation_error.get_or_insert_with(|| {
                            format!(
                                "Scout path sample identity is not an active registration: {}",
                                sample.decision_id
                            )
                        });
                        continue;
                    }
                    let inserted = insert.execute(params![sample.decision_id, sample.path_epoch as i64, sample.sequence as i64, sample.timestamp as i64, sample.bid, sample.ask, sample.freshness_ms as i64, sample.gap_ms as i64, sample.first_source_timestamp.map(|value| value as i64), sample.maximum_gap_ms as i64, sample.source_timestamps_monotonic, sample.maximum_favorable_excursion_percent, sample.maximum_adverse_excursion_percent, sample.sample_count as i64, sample.capture_complete])?;
                    touched_scout_paths.insert(sample.decision_id.as_str());
                    if inserted == 0 {
                        let existing: (i64, f64, f64, i64, i64, Option<i64>, i64, bool, Option<f64>, Option<f64>, i64, bool) = transaction.query_row(
                            "SELECT timestamp, bid, ask, freshness_ms, gap_ms, first_source_timestamp, maximum_gap_ms, source_timestamps_monotonic, maximum_favorable_excursion_percent, maximum_adverse_excursion_percent, sample_count, capture_complete FROM scout_path_samples WHERE decision_id=?1 AND path_epoch=?2 AND sequence=?3",
                            params![sample.decision_id, sample.path_epoch as i64, sample.sequence as i64],
                            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?, row.get(6)?, row.get(7)?, row.get(8)?, row.get(9)?, row.get(10)?, row.get(11)?)),
                        )?;
                        let duplicate_matches = existing.0 as u64 == sample.timestamp
                            && existing.1.to_bits() == sample.bid.to_bits()
                            && existing.2.to_bits() == sample.ask.to_bits()
                            && existing.3 as u64 == sample.freshness_ms
                            && existing.4 as u64 == sample.gap_ms
                            && existing.5.map(|value| value as u64) == sample.first_source_timestamp
                            && existing.6 as u64 == sample.maximum_gap_ms
                            && existing.7 == sample.source_timestamps_monotonic
                            && existing.8 == sample.maximum_favorable_excursion_percent
                            && existing.9 == sample.maximum_adverse_excursion_percent
                            && existing.10 as u64 == sample.sample_count
                            && existing.11 == sample.capture_complete;
                        if !duplicate_matches {
                            transaction.execute(
                                "UPDATE scout_path_registrations SET capture_complete=0 WHERE decision_id=?1",
                                [&sample.decision_id],
                            )?;
                            scout_path_validation_error.get_or_insert_with(|| {
                                format!(
                                    "conflicting Scout path sample replay for {} epoch {} sequence {}",
                                    sample.decision_id, sample.path_epoch, sample.sequence
                                )
                            });
                        }
                    }
                    if !sample.capture_complete {
                        transaction.execute(
                            "UPDATE scout_path_registrations SET capture_complete=0 WHERE decision_id=?1",
                            [&sample.decision_id],
                        )?;
                    }
                    if inserted != 0 {
                        *inserted_per_path.entry(sample.decision_id.as_str()).or_default() += 1;
                    }
                }
                drop(insert);
                for decision_id in touched_scout_paths {
                    let inserted_count = inserted_per_path.get(decision_id).copied().unwrap_or(0);
                    if inserted_count > 0 {
                        transaction.execute(
                            "UPDATE scout_path_registrations SET sample_count=sample_count+?2 WHERE decision_id=?1",
                            params![decision_id, inserted_count as i64],
                        )?;
                    }
                    let count: i64 = transaction.query_row(
                        "SELECT COUNT(*) FROM scout_path_samples WHERE decision_id=?1",
                        [decision_id],
                        |row| row.get(0),
                    )?;
                    if count > 4_096 {
                        transaction.execute("UPDATE scout_path_registrations SET truncated=1 WHERE decision_id=?1", [decision_id])?;
                        transaction.execute(
                            "DELETE FROM scout_path_samples WHERE rowid IN (SELECT rowid FROM scout_path_samples WHERE decision_id=?1 ORDER BY path_epoch DESC, sequence DESC LIMIT -1 OFFSET 4096)",
                            [decision_id],
                        )?;
                    }
                }
                const MAX_DURABLE_SCOUT_PATH_ROWS: i64 = 262_144;
                let total_rows: i64 = transaction.query_row(
                    "SELECT COUNT(*) FROM scout_path_samples",
                    [],
                    |row| row.get(0),
                )?;
                let excess = total_rows.saturating_sub(MAX_DURABLE_SCOUT_PATH_ROWS);
                if excess > 0 {
                    transaction.execute(
                        "UPDATE scout_path_registrations SET truncated=1 WHERE decision_id IN (SELECT decision_id FROM scout_path_samples ORDER BY timestamp, path_epoch, sequence LIMIT ?1)",
                        [excess],
                    )?;
                    transaction.execute(
                        "DELETE FROM scout_path_samples WHERE rowid IN (SELECT rowid FROM scout_path_samples ORDER BY timestamp, path_epoch, sequence LIMIT ?1)",
                        [excess],
                    )?;
                }
            }
            for decision_id in completed_scout_paths {
                transaction.execute("UPDATE scout_path_registrations SET active=0 WHERE decision_id=?1", [&decision_id])?;
            }
            if has_scout_path_operations {
                enforce_scout_path_registration_limit(
                    &transaction,
                    MAX_DURABLE_SCOUT_PATH_REGISTRATIONS,
                )?;
            }
            for decision_id in failed_scout_paths {
                transaction.execute(
                    "UPDATE scout_path_registrations SET capture_complete=0 WHERE decision_id=?1",
                    [&decision_id],
                )?;
            }
            transaction.commit()?;
            if let Some(error) = scout_path_validation_error.take() {
                anyhow::bail!("{error}");
            }
            Ok(())
        })()
        .map_err(|error| error.to_string());
        for request in batch {
            let _ = request.ack.send(result.clone());
        }
    }
}

fn enforce_scout_path_registration_limit(
    transaction: &rusqlite::Transaction<'_>,
    max_rows: i64,
) -> Result<()> {
    if max_rows < 0 {
        anyhow::bail!("Scout path registration limit must be non-negative");
    }
    let total: i64 =
        transaction.query_row("SELECT COUNT(*) FROM scout_path_registrations", [], |row| {
            row.get(0)
        })?;
    let excess = total.saturating_sub(max_rows);
    if excess == 0 {
        return Ok(());
    }
    let inactive_ids = {
        let mut statement = transaction.prepare(
            "SELECT decision_id FROM scout_path_registrations WHERE active=0 ORDER BY signal_timestamp ASC, decision_id ASC LIMIT ?1",
        )?;
        statement
            .query_map([excess], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for decision_id in inactive_ids {
        transaction.execute(
            "DELETE FROM scout_path_samples WHERE decision_id=?1",
            [&decision_id],
        )?;
        transaction.execute(
            "DELETE FROM scout_path_registrations WHERE decision_id=?1 AND active=0",
            [&decision_id],
        )?;
    }
    let remaining: i64 =
        transaction.query_row("SELECT COUNT(*) FROM scout_path_registrations", [], |row| {
            row.get(0)
        })?;
    if remaining > max_rows {
        anyhow::bail!(
            "active Scout path registrations exceed the durable bound ({remaining}>{max_rows})"
        );
    }
    Ok(())
}

#[cfg(test)]
mod scout_path_registration_retention_tests {
    use super::*;

    fn insert_registration(
        transaction: &rusqlite::Transaction<'_>,
        id: &str,
        time: i64,
        active: bool,
    ) {
        transaction
            .execute(
                "INSERT INTO scout_path_registrations(decision_id,symbol,signal_timestamp,direction,entry_price,active) VALUES (?1,'BTC-USDT',?2,'bullish',100.0,?3)",
                params![id, time, active],
            )
            .unwrap();
        transaction
            .execute(
                "INSERT INTO scout_path_samples(decision_id,path_epoch,sequence,timestamp,bid,ask,freshness_ms,gap_ms,first_source_timestamp,maximum_gap_ms,source_timestamps_monotonic,maximum_favorable_excursion_percent,maximum_adverse_excursion_percent,sample_count,capture_complete) VALUES (?1,1,1,?2,99.9,100.1,1,0,?2,0,1,0.0,0.0,1,1)",
                params![id, time],
            )
            .unwrap();
    }

    #[test]
    fn registration_bound_prunes_old_inactive_rows_and_their_samples() {
        let path = std::env::temp_dir().join(format!(
            "insights-scout-path-retention-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut connection = SignalStore::open_database_at(&path).unwrap();
        {
            let transaction = connection.transaction().unwrap();
            insert_registration(&transaction, "inactive-old", 1, false);
            insert_registration(&transaction, "inactive-new", 2, false);
            insert_registration(&transaction, "active", 3, true);
            enforce_scout_path_registration_limit(&transaction, 2).unwrap();
            transaction.commit().unwrap();
        }
        let registrations: i64 = connection
            .query_row("SELECT COUNT(*) FROM scout_path_registrations", [], |row| {
                row.get(0)
            })
            .unwrap();
        let samples: i64 = connection
            .query_row("SELECT COUNT(*) FROM scout_path_samples", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(registrations, 2);
        assert_eq!(samples, 2);
        assert_eq!(
            connection
                .query_row(
                    "SELECT COUNT(*) FROM scout_path_registrations WHERE decision_id='inactive-old'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn registration_bound_fails_closed_instead_of_evicting_active_paths() {
        let path = std::env::temp_dir().join(format!(
            "insights-scout-path-active-limit-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let mut connection = SignalStore::open_database_at(&path).unwrap();
        {
            let transaction = connection.transaction().unwrap();
            insert_registration(&transaction, "active-one", 1, true);
            insert_registration(&transaction, "active-two", 2, true);
            transaction.commit().unwrap();
        }
        {
            let transaction = connection.transaction().unwrap();
            assert!(enforce_scout_path_registration_limit(&transaction, 1).is_err());
            transaction.rollback().unwrap();
        }
        let active: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM scout_path_registrations WHERE active=1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(active, 2);
        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}

#[cfg(test)]
mod coverage_tests {
    use super::*;

    #[test]
    fn breakout_watch_requires_confidence_range_and_directional_pressure() {
        let bullish = SignalComponents {
            trend_1m_percent: Some(0.25),
            momentum_10s_percent: Some(0.12),
            trade_flow_imbalance: Some(1.0),
            book_imbalance: Some(1.0),
            volatility_1m_percent: Some(0.2),
            trend_5m_percent: None,
            trend_5m_volatility_units: None,
            mispricing_vs_vwap_bps: None,
            pressure_without_price_move: None,
            aggressor_cost_bps: None,
            ..SignalComponents::default()
        };
        assert_eq!(
            breakout_watch(&bullish, Some((99.0, 101.0)), Some(0.64)),
            None
        );
        assert_eq!(breakout_watch(&bullish, None, Some(0.9)), None);
        let (direction, pressure) =
            breakout_watch(&bullish, Some((99.0, 101.0)), Some(0.9)).unwrap();
        assert_eq!(direction, "UP");
        assert!(pressure >= 0.35);

        let bearish = SignalComponents {
            trend_1m_percent: None,
            momentum_10s_percent: None,
            trade_flow_imbalance: Some(-1.0),
            book_imbalance: Some(-1.0),
            volatility_1m_percent: None,
            trend_5m_percent: None,
            trend_5m_volatility_units: None,
            mispricing_vs_vwap_bps: None,
            pressure_without_price_move: None,
            aggressor_cost_bps: None,
            ..SignalComponents::default()
        };
        assert_eq!(
            breakout_watch(&bearish, Some((99.0, 101.0)), Some(0.9))
                .unwrap()
                .0,
            "DOWN"
        );
        assert_eq!(
            breakout_watch(
                &SignalComponents {
                    trend_1m_percent: None,
                    momentum_10s_percent: None,
                    trade_flow_imbalance: None,
                    book_imbalance: None,
                    volatility_1m_percent: None,
                    trend_5m_percent: None,
                    trend_5m_volatility_units: None,
                    mispricing_vs_vwap_bps: None,
                    pressure_without_price_move: None,
                    aggressor_cost_bps: None,
                    ..SignalComponents::default()
                },
                Some((99.0, 101.0)),
                Some(0.9)
            ),
            None
        );
    }
}

#[cfg(test)]
mod granular_live_feature_tests {
    use super::*;

    #[test]
    fn snapshots_publish_window_and_depth_variants_for_live_scouts() {
        let mut engine = SignalEngine::default();
        for index in 0..=120_u64 {
            let timestamp = 1_000 + index * 1_000;
            let price = 100.0 + index as f64 * 0.01;
            engine.on_price(timestamp, price, 0.0);
            if index % 5 == 0 {
                engine.on_book(
                    timestamp,
                    price - 0.01,
                    price + 0.01,
                    &[1.0, 2.0, 3.0, 4.0, 5.0],
                    &[5.0, 4.0, 3.0, 2.0, 1.0],
                    &[
                        (price - 0.01, 1.0),
                        (price - 0.02, 2.0),
                        (price - 0.03, 3.0),
                        (price - 0.04, 4.0),
                        (price - 0.05, 5.0),
                    ],
                    &[
                        (price + 0.01, 5.0),
                        (price + 0.02, 4.0),
                        (price + 0.03, 3.0),
                        (price + 0.04, 2.0),
                        (price + 0.05, 1.0),
                    ],
                );
                engine.on_trade(timestamp, price + 0.01, 1.0, true);
            }
        }

        let snapshot = engine.snapshot("ETH-USDT", 121_000).unwrap();
        let components = snapshot.components;
        assert!(
            components
                .momentum_by_window
                .iter()
                .any(|feature| { feature.window_seconds == 10 && feature.value.is_some() })
        );
        assert!(components.trade_flow_by_window.iter().any(|feature| {
            feature.window_seconds == 60 && feature.value.is_some() && feature.sample_count > 0
        }));
        assert!(
            components
                .book_imbalance_by_depth
                .iter()
                .any(|feature| { feature.depth_levels == 5 && feature.value.is_some() })
        );
        assert!(components.vwap_dislocation_by_window.iter().any(|feature| {
            feature.window_seconds == 60 && feature.value.is_some() && feature.sample_count >= 5
        }));
        assert!(
            components
                .absorption_pressure_by_window
                .iter()
                .any(|feature| {
                    feature.window_seconds == 30
                        && feature.replenishment_window_ms == 2_000
                        && feature.touch_tolerance_bps == 2.0
                })
        );
    }
}
