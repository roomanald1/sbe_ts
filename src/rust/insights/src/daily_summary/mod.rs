use crate::signals::StoredSignalRecord;
use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
use flate2::{Compression, read::MultiGzDecoder, write::GzEncoder};
use rusqlite::{Connection, MAIN_DB, params};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DAY_MS: i64 = 86_400_000;
const RAW_RETENTION_DAYS: i64 = 1;
const DETAIL_RETENTION_DAYS: i64 = 90;
const MAINTENANCE_HEARTBEAT_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct MaintenanceStatus {
    pub heartbeat_at_ms: i64,
    pub last_attempt_at_ms: Option<i64>,
    pub last_success_at_ms: Option<i64>,
    pub last_run_success: Option<bool>,
    pub last_run_archived_records: i64,
    pub last_run_purged_signal_records: i64,
    pub last_run_compacted_audit_records: i64,
    pub last_run_purged_audit_records: i64,
    pub last_run_expired_training_archives: i64,
    pub consecutive_failures: u64,
    pub last_error: Option<String>,
}

#[derive(Default)]
struct MaintenanceRunStats {
    archived_records: i64,
    purged_signal_records: i64,
    compacted_audit_records: i64,
    purged_audit_records: i64,
    expired_training_archives: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DailySummary {
    day_utc: String,
    record_type: String,
    record_count: i64,
    compressed_payload_bytes: i64,
    first_record_timestamp: i64,
    last_record_timestamp: i64,
    training_archive_bytes: i64,
}

#[derive(Deserialize)]
struct SummaryQuery {
    days: Option<usize>,
}

pub fn router() -> Router {
    Router::new().route("/signals/daily-summary", get(get_summaries))
}

async fn get_summaries(
    Query(query): Query<SummaryQuery>,
) -> Result<Json<Vec<DailySummary>>, (StatusCode, String)> {
    let days = query.days.unwrap_or(30).clamp(1, 3650) as i64;
    let path = database_path();
    tokio::task::spawn_blocking(move || read_summaries(&path, days))
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
        .map(Json)
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))
}

pub async fn run_maintenance_service() -> anyhow::Result<()> {
    let status_path = maintenance_status_path();
    let mut status = read_maintenance_status(&status_path);
    loop {
        status.last_attempt_at_ms = Some(now_ms());
        status.heartbeat_at_ms = now_ms();
        status.last_run_success = None;
        status.last_run_archived_records = 0;
        status.last_run_purged_signal_records = 0;
        status.last_run_compacted_audit_records = 0;
        status.last_run_purged_audit_records = 0;
        status.last_run_expired_training_archives = 0;
        persist_maintenance_status(&status_path, &status);
        match summarize_missing_days().await {
            Ok(stats) => {
                status.last_success_at_ms = Some(now_ms());
                status.last_run_success = Some(true);
                status.last_run_archived_records = stats.archived_records;
                status.last_run_purged_signal_records = stats.purged_signal_records;
                status.last_run_compacted_audit_records = stats.compacted_audit_records;
                status.last_run_purged_audit_records = stats.purged_audit_records;
                status.last_run_expired_training_archives = stats.expired_training_archives;
                status.consecutive_failures = 0;
                status.last_error = None;
            }
            Err(error) => {
                eprintln!("Insights database maintenance failed: {error:#}");
                status.last_run_success = Some(false);
                status.consecutive_failures = status.consecutive_failures.saturating_add(1);
                status.last_error = Some(format!("{error:#}"));
            }
        }
        status.heartbeat_at_ms = now_ms();
        persist_maintenance_status(&status_path, &status);
        let now = status.heartbeat_at_ms;
        let next_midnight = (now.div_euclid(DAY_MS) + 1) * DAY_MS;
        while status.heartbeat_at_ms < next_midnight {
            let remaining = (next_midnight - status.heartbeat_at_ms).max(1) as u64;
            tokio::time::sleep(
                MAINTENANCE_HEARTBEAT_INTERVAL.min(Duration::from_millis(remaining)),
            )
            .await;
            status.heartbeat_at_ms = now_ms();
            persist_maintenance_status(&status_path, &status);
        }
    }
}

async fn summarize_missing_days() -> anyhow::Result<MaintenanceRunStats> {
    let path = database_path();
    tokio::task::spawn_blocking(move || summarize_missing_days_sync(&path))
        .await
        .map_err(|error| anyhow::anyhow!("daily summary task failed: {error}"))?
}

fn summarize_missing_days_sync(path: &PathBuf) -> anyhow::Result<MaintenanceRunStats> {
    let connection = open_database(path)?;
    let mut stats = MaintenanceRunStats::default();
    let today_start = now_ms().div_euclid(DAY_MS) * DAY_MS;
    let retention_start = today_start - RAW_RETENTION_DAYS * DAY_MS;
    let first_and_last_record = connection.query_row(
        "SELECT (SELECT timestamp FROM signal_records INDEXED BY signal_records_timestamp ORDER BY timestamp ASC LIMIT 1), (SELECT timestamp FROM signal_records INDEXED BY signal_records_timestamp ORDER BY timestamp DESC LIMIT 1)",
        [],
        |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
    )?;
    let day_starts = if let (Some(first_record), Some(last_record)) = first_and_last_record {
        let first_day = first_record.div_euclid(DAY_MS) * DAY_MS;
        let last_day = (last_record.div_euclid(DAY_MS) * DAY_MS).min(today_start - DAY_MS);
        let mut missing = Vec::new();
        let mut day_start = first_day;
        while day_start <= last_day {
            let has_records = connection.query_row(
                "SELECT EXISTS(SELECT 1 FROM signal_records INDEXED BY signal_records_timestamp WHERE timestamp >= ?1 AND timestamp < ?2)",
                params![day_start, day_start + DAY_MS],
                |row| row.get::<_, bool>(0),
            )?;
            if has_records {
                missing.push(day_start);
            }
            day_start += DAY_MS;
        }
        missing
    } else {
        Vec::new()
    };
    for day_start in day_starts {
        let aggregates = {
            let mut statement = connection.prepare(
                r#"SELECT kind, COUNT(*), COALESCE(SUM(length(payload)), 0), MIN(timestamp), MAX(timestamp)
                   FROM signal_records
                   WHERE timestamp >= ?1 AND timestamp < ?2
                   GROUP BY kind"#,
            )?;
            statement
                .query_map(params![day_start, day_start + DAY_MS], |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                    ))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        let existing_summary_kinds = {
            let mut statement = connection.prepare(
                "SELECT kind, training_payload IS NOT NULL FROM signal_daily_summary WHERE day_start_ms = ?1",
            )?;
            statement
                .query_map([day_start], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, bool>(1)?))
                })?
                .collect::<rusqlite::Result<std::collections::HashMap<_, _>>>()?
        };
        let training_archives = if day_start < retention_start {
            aggregates
                .iter()
                .filter(|(kind, ..)| !existing_summary_kinds.get(kind).copied().unwrap_or(false))
                .map(|(kind, count, ..)| {
                    write_training_archive(&connection, day_start, *kind, *count)
                })
                .collect::<anyhow::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        stats.archived_records += training_archives
            .iter()
            .map(|archive| archive.record_count)
            .sum::<i64>();
        // Decode and recompress historical rows before taking SQLite's write
        // lock. A backfill can be substantial and must not block live signal
        // writes for the duration of the compression pass.
        let transaction = connection.unchecked_transaction()?;
        for (kind, count, bytes, first, last) in &aggregates {
            transaction.execute(
                r#"INSERT OR IGNORE INTO signal_daily_summary
                   (day_start_ms, kind, record_count, compressed_payload_bytes, first_timestamp, last_timestamp, training_payload)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)"#,
                params![day_start, kind, count, bytes, first, last],
            )?;
        }
        for archive in training_archives {
            transaction.execute(
                "UPDATE signal_daily_summary SET training_payload = zeroblob(?3) WHERE day_start_ms = ?1 AND kind = ?2",
                params![day_start, archive.kind, archive.size as i64],
            )?;
            let row_id = transaction.query_row(
                "SELECT rowid FROM signal_daily_summary WHERE day_start_ms = ?1 AND kind = ?2",
                params![day_start, archive.kind],
                |row| row.get::<_, i64>(0),
            )?;
            let mut source = File::open(&archive.path)?;
            let mut destination = transaction.blob_open(
                MAIN_DB,
                "signal_daily_summary",
                "training_payload",
                row_id,
                false,
            )?;
            std::io::copy(&mut source, &mut destination)?;
            drop(destination);
        }
        if day_start < retention_start {
            stats.purged_signal_records += transaction.execute(
                "DELETE FROM signal_records WHERE timestamp >= ?1 AND timestamp < ?2",
                params![day_start, day_start + DAY_MS],
            )? as i64;
        }
        transaction.execute(
            "INSERT OR REPLACE INTO signal_daily_summary_runs(day_start_ms, summarized_at_ms) VALUES (?1, ?2)",
            params![day_start, now_ms()],
        )?;
        transaction.commit()?;
    }
    let retention_stats = apply_retention(&connection, today_start)?;
    stats.compacted_audit_records = retention_stats.compacted_audit_records;
    stats.purged_audit_records = retention_stats.purged_audit_records;
    stats.expired_training_archives = retention_stats.expired_training_archives;
    // Do not VACUUM after retention cleanup: rebuilding a multi-gigabyte
    // database takes an exclusive lock and blocks the live signal writer.
    // SQLite can reuse the freed pages for future inserts.
    Ok(stats)
}

#[derive(Default)]
struct RetentionStats {
    compacted_audit_records: i64,
    purged_audit_records: i64,
    expired_training_archives: i64,
}

fn apply_retention(connection: &Connection, today_start: i64) -> anyhow::Result<RetentionStats> {
    let detail_cutoff = today_start - DETAIL_RETENTION_DAYS * DAY_MS;
    let transaction = connection.unchecked_transaction()?;
    let expired_training_archives = transaction.execute(
        "UPDATE signal_daily_summary SET training_payload = NULL WHERE day_start_ms < ?1 AND training_payload IS NOT NULL",
        [detail_cutoff],
    )? as i64;
    let compacted_audit_records = transaction.query_row(
        "SELECT COALESCE(SUM(1), 0) FROM auto_trader_audit WHERE timestamp < ?1",
        [detail_cutoff],
        |row| row.get::<_, i64>(0),
    )?;
    transaction.execute(
        r#"INSERT INTO auto_trader_audit_daily_summary (day_start_ms, stage, symbol, record_count)
           SELECT (timestamp / ?1) * ?1, stage, COALESCE(symbol, ''), COUNT(*)
           FROM auto_trader_audit
           WHERE timestamp < ?2
           GROUP BY (timestamp / ?1) * ?1, stage, COALESCE(symbol, '')
           ON CONFLICT(day_start_ms, stage, symbol) DO UPDATE
           SET record_count = record_count + excluded.record_count"#,
        params![DAY_MS, detail_cutoff],
    )?;
    let purged_audit_records = transaction.execute(
        "DELETE FROM auto_trader_audit WHERE timestamp < ?1",
        [detail_cutoff],
    )? as i64;
    transaction.commit()?;
    Ok(RetentionStats {
        compacted_audit_records,
        purged_audit_records,
        expired_training_archives,
    })
}

fn maintenance_status_path() -> PathBuf {
    database_path().with_file_name("maintenance_status.json")
}

fn read_maintenance_status(path: &PathBuf) -> MaintenanceStatus {
    match std::fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(status) => status,
            Err(error) => {
                eprintln!("Could not parse Insights maintenance status: {error}");
                MaintenanceStatus::default()
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => MaintenanceStatus::default(),
        Err(error) => {
            eprintln!("Could not read Insights maintenance status: {error}");
            MaintenanceStatus::default()
        }
    }
}

fn persist_maintenance_status(path: &PathBuf, status: &MaintenanceStatus) {
    let result = (|| -> anyhow::Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent)?;
        }
        let temporary_path = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(status)?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary_path)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        std::fs::rename(temporary_path, path)?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("Could not persist Insights maintenance status: {error:#}");
    }
}

struct TrainingArchiveFile {
    kind: i64,
    record_count: i64,
    path: PathBuf,
    size: u64,
}

impl Drop for TrainingArchiveFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn write_training_archive(
    connection: &Connection,
    day_start: i64,
    kind: i64,
    record_count: i64,
) -> anyhow::Result<TrainingArchiveFile> {
    let path = std::env::temp_dir().join(format!(
        "insights-training-{}-{day_start}-{kind}-{}.gz",
        std::process::id(),
        now_ms()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)?;
    let result = (|| {
        let mut encoder = GzEncoder::new(file, Compression::fast());
        bincode::serialize_into(&mut encoder, &(record_count as u64))?;
        let mut statement = connection.prepare(
            "SELECT payload FROM signal_records WHERE kind = ?1 AND timestamp >= ?2 AND timestamp < ?3 ORDER BY timestamp, id",
        )?;
        let rows = statement.query_map(params![kind, day_start, day_start + DAY_MS], |row| {
            row.get::<_, Vec<u8>>(0)
        })?;
        for row in rows {
            let compressed = row?;
            let mut payload = Vec::new();
            MultiGzDecoder::new(compressed.as_slice()).read_to_end(&mut payload)?;
            let record = crate::signals::deserialize_record(kind, &payload)?;
            bincode::serialize_into(&mut encoder, &record)?;
        }
        let file = encoder.finish()?;
        file.sync_all()?;
        let size = file.metadata()?.len();
        Ok(TrainingArchiveFile {
            kind,
            record_count,
            path: path.clone(),
            size,
        })
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
    result
}

#[cfg(test)]
mod training_archive_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn archives_records_as_a_bincode_vector_without_materializing_the_day() {
        let connection = Connection::open_in_memory().unwrap();
        connection
            .execute(
                "CREATE TABLE signal_records (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, kind INTEGER NOT NULL, payload BLOB NOT NULL)",
                [],
            )
            .unwrap();
        for timestamp in 1..=3 {
            let record =
                StoredSignalRecord::ExternalMarket(crate::signals::ExternalMarketFeatures {
                    underlying: "BTC".to_owned(),
                    timestamp,
                    swap_instrument: "BTC-USDT-SWAP".to_owned(),
                    funding_rate: None,
                    next_funding_rate: None,
                    open_interest_contracts: None,
                    open_interest_currency: None,
                    open_interest_change_percent: None,
                    mark_price: None,
                    index_price: None,
                    perp_basis_bps: None,
                    options_atm_iv: None,
                    options_25d_skew: None,
                    options_expiry_timestamp: None,
                });
            let payload = bincode::serialize(&record).unwrap();
            let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&payload).unwrap();
            connection
                .execute(
                    "INSERT INTO signal_records (timestamp, kind, payload) VALUES (?1, 5, ?2)",
                    params![timestamp as i64, encoder.finish().unwrap()],
                )
                .unwrap();
        }

        let archive = write_training_archive(&connection, 0, 5, 3).unwrap();
        let mut decoder = MultiGzDecoder::new(File::open(&archive.path).unwrap());
        let mut payload = Vec::new();
        decoder.read_to_end(&mut payload).unwrap();
        let archived: Vec<StoredSignalRecord> = bincode::deserialize(&payload).unwrap();
        assert_eq!(archived.len(), 3);
        assert_eq!(archive.kind, 5);
        assert_eq!(
            archive.size as usize,
            std::fs::metadata(&archive.path).unwrap().len() as usize
        );
    }

    #[test]
    fn retention_drops_expired_training_payloads_and_compacts_audit_rows() {
        let connection = Connection::open_in_memory().unwrap();
        initialize_schema(&connection).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage));",
            )
            .unwrap();
        let old_day = DAY_MS;
        let recent_day = 40 * DAY_MS;
        connection
            .execute(
                "INSERT INTO signal_daily_summary (day_start_ms, kind, record_count, compressed_payload_bytes, first_timestamp, last_timestamp, training_payload) VALUES (?1, 0, 1, 10, ?1, ?1, X'0102'), (?2, 0, 1, 10, ?2, ?2, X'0304')",
                params![old_day, recent_day],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO auto_trader_audit (decision_id, stage, timestamp, symbol, payload) VALUES ('old-1', 'decision', ?1, 'BTC', X'01'), ('old-2', 'decision', ?1, 'BTC', X'02'), ('recent', 'decision', ?2, 'BTC', X'03')",
                params![old_day + 1, recent_day + 1],
            )
            .unwrap();

        apply_retention(&connection, 120 * DAY_MS).unwrap();

        assert_eq!(
            connection
                .query_row(
                    "SELECT training_payload IS NULL FROM signal_daily_summary WHERE day_start_ms = ?1",
                    [old_day],
                    |row| row.get::<_, bool>(0),
                )
                .unwrap(),
            true
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT length(training_payload) FROM signal_daily_summary WHERE day_start_ms = ?1",
                    [recent_day],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM auto_trader_audit", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT record_count FROM auto_trader_audit_daily_summary WHERE day_start_ms = ?1 AND stage = 'decision' AND symbol = 'BTC'",
                    [old_day],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
    }

    #[test]
    fn revisits_summarized_days_when_raw_rows_were_not_removed() {
        let path = std::env::temp_dir().join(format!(
            "insights-retention-recovery-{}-{}.sqlite",
            std::process::id(),
            now_ms()
        ));
        let connection = Connection::open(&path).unwrap();
        initialize_schema(&connection).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE signal_records (id INTEGER PRIMARY KEY, timestamp INTEGER NOT NULL, kind INTEGER NOT NULL, payload BLOB NOT NULL); CREATE INDEX signal_records_timestamp ON signal_records(timestamp); CREATE TABLE auto_trader_audit (decision_id TEXT NOT NULL, stage TEXT NOT NULL, timestamp INTEGER NOT NULL, symbol TEXT, payload BLOB NOT NULL, PRIMARY KEY(decision_id, stage));",
            )
            .unwrap();
        let timestamp = now_ms() - 2 * DAY_MS;
        let day_start = timestamp.div_euclid(DAY_MS) * DAY_MS;
        let record = StoredSignalRecord::ExternalMarket(crate::signals::ExternalMarketFeatures {
            underlying: "BTC".to_owned(),
            timestamp: timestamp as u64,
            swap_instrument: "BTC-USDT-SWAP".to_owned(),
            funding_rate: None,
            next_funding_rate: None,
            open_interest_contracts: None,
            open_interest_currency: None,
            open_interest_change_percent: None,
            mark_price: None,
            index_price: None,
            perp_basis_bps: None,
            options_atm_iv: None,
            options_25d_skew: None,
            options_expiry_timestamp: None,
        });
        let payload = bincode::serialize(&record).unwrap();
        let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&payload).unwrap();
        connection
            .execute(
                "INSERT INTO signal_records (timestamp, kind, payload) VALUES (?1, 5, ?2)",
                params![timestamp, encoder.finish().unwrap()],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO signal_daily_summary (day_start_ms, kind, record_count, compressed_payload_bytes, first_timestamp, last_timestamp) VALUES (?1, 5, 1, 1, ?2, ?2)",
                params![day_start, timestamp],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO signal_daily_summary_runs (day_start_ms, summarized_at_ms) VALUES (?1, ?2)",
                params![day_start, timestamp],
            )
            .unwrap();
        drop(connection);

        summarize_missing_days_sync(&path).unwrap();

        let connection = Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .query_row("SELECT COUNT(*) FROM signal_records", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert!(connection
            .query_row(
                "SELECT training_payload IS NOT NULL FROM signal_daily_summary WHERE day_start_ms = ?1 AND kind = 5",
                [day_start],
                |row| row.get::<_, bool>(0),
            )
            .unwrap());
        drop(connection);
        std::fs::remove_file(path).unwrap();
    }
}

fn read_summaries(path: &PathBuf, days: i64) -> anyhow::Result<Vec<DailySummary>> {
    let connection = open_database(path)?;
    let mut statement = connection.prepare(
        r#"SELECT day_start_ms, kind, record_count, compressed_payload_bytes, first_timestamp, last_timestamp
         , COALESCE(length(training_payload), 0)
         FROM signal_daily_summary
         WHERE day_start_ms >= COALESCE((SELECT MAX(day_start_ms) FROM signal_daily_summary), 0) - (?1 - 1) * 86400000
         ORDER BY day_start_ms DESC, kind"#,
    )?;
    let rows = statement.query_map([days], |row| {
        let day_start_ms = row.get::<_, i64>(0)?;
        let kind = row.get::<_, i64>(1)?;
        Ok(DailySummary {
            day_utc: utc_date(day_start_ms.div_euclid(DAY_MS)),
            record_type: record_type(kind),
            record_count: row.get(2)?,
            compressed_payload_bytes: row.get(3)?,
            first_record_timestamp: row.get(4)?,
            last_record_timestamp: row.get(5)?,
            training_archive_bytes: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn open_database(path: &PathBuf) -> anyhow::Result<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    connection.execute_batch("PRAGMA synchronous=NORMAL;")?;
    Ok(connection)
}

pub async fn initialize() -> anyhow::Result<()> {
    let path = database_path();
    tokio::task::spawn_blocking(move || {
        let connection = Connection::open(&path)?;
        connection.busy_timeout(Duration::from_secs(5))?;
        initialize_schema(&connection)
    })
    .await
    .map_err(|error| anyhow::anyhow!("daily summary initialization failed: {error}"))?
}

fn initialize_schema(connection: &Connection) -> anyhow::Result<()> {
    connection.execute_batch(
        r#"CREATE TABLE IF NOT EXISTS signal_daily_summary (
           day_start_ms INTEGER NOT NULL,
           kind INTEGER NOT NULL,
           record_count INTEGER NOT NULL,
           compressed_payload_bytes INTEGER NOT NULL,
           first_timestamp INTEGER NOT NULL,
           last_timestamp INTEGER NOT NULL,
           training_payload BLOB,
           PRIMARY KEY(day_start_ms, kind)
         );
         CREATE TABLE IF NOT EXISTS signal_daily_summary_runs (
           day_start_ms INTEGER PRIMARY KEY,
           summarized_at_ms INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS auto_trader_audit_daily_summary (
           day_start_ms INTEGER NOT NULL,
           stage TEXT NOT NULL,
           symbol TEXT NOT NULL,
           record_count INTEGER NOT NULL,
           PRIMARY KEY(day_start_ms, stage, symbol)
         );"#,
    )?;
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(signal_daily_summary)")?;
        statement
            .query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !columns.iter().any(|column| column == "training_payload") {
        connection.execute(
            "ALTER TABLE signal_daily_summary ADD COLUMN training_payload BLOB",
            [],
        )?;
    }
    Ok(())
}

fn database_path() -> PathBuf {
    std::env::var_os("SIGNALS_DB_FILE")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("SIGNALS_FILE")
                .map(|path| PathBuf::from(path).with_extension("sqlite"))
        })
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_default()
                .join("signals.sqlite")
        })
}
include!("format.rs");
