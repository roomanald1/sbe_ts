use axum::{Json, Router, extract::Query, http::StatusCode, routing::get};
use flate2::{read::MultiGzDecoder, write::GzEncoder, Compression};
use insights::signals::StoredSignalRecord;
use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const DAY_MS: i64 = 86_400_000;

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

pub fn spawn_nightly() {
    tokio::spawn(async {
        if let Err(error) = summarize_missing_days().await {
            eprintln!("Failed to create signal daily summaries: {error:#}");
        }
        loop {
            let now = now_ms();
            let next_midnight = (now.div_euclid(DAY_MS) + 1) * DAY_MS;
            tokio::time::sleep(Duration::from_millis((next_midnight - now).max(1) as u64)).await;
            if let Err(error) = summarize_missing_days().await {
                eprintln!("Failed to create signal daily summaries: {error:#}");
            }
        }
    });
}

async fn summarize_missing_days() -> anyhow::Result<()> {
    let path = database_path();
    tokio::task::spawn_blocking(move || summarize_missing_days_sync(&path))
        .await
        .map_err(|error| anyhow::anyhow!("daily summary task failed: {error}"))?
}

fn summarize_missing_days_sync(path: &PathBuf) -> anyhow::Result<()> {
    let connection = open_database(path)?;
    let today_start = now_ms().div_euclid(DAY_MS) * DAY_MS;
    let retention_start = today_start - 5 * DAY_MS;
    let day_starts = {
        let mut statement = connection.prepare(
            "SELECT DISTINCT (timestamp / ?1) * ?1 FROM signal_records WHERE timestamp < ?2 ORDER BY 1",
        )?;
        statement.query_map(params![DAY_MS, today_start], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    let mut deleted_records = 0usize;
    for day_start in day_starts {
        let transaction = connection.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM signal_daily_summary WHERE day_start_ms = ?1",
            [day_start],
        )?;
        let aggregates = {
            let mut statement = transaction.prepare(
                r#"SELECT kind, COUNT(*), COALESCE(SUM(length(payload)), 0), MIN(timestamp), MAX(timestamp)
                   FROM signal_records
                   WHERE timestamp >= ?1 AND timestamp < ?2
                   GROUP BY kind"#,
            )?;
            statement
                .query_map(params![day_start, day_start + DAY_MS], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, i64>(3)?, row.get::<_, i64>(4)?))
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?
        };
        for (kind, count, bytes, first, last) in aggregates {
            transaction.execute(
                r#"INSERT INTO signal_daily_summary
                   (day_start_ms, kind, record_count, compressed_payload_bytes, first_timestamp, last_timestamp, training_payload)
                   VALUES (?1, ?2, ?3, ?4, ?5, ?6, NULL)"#,
                params![day_start, kind, count, bytes, first, last],
            )?;
        }
        let training_payloads = if day_start < retention_start {
            let mut statement = transaction.prepare(
                "SELECT kind, payload FROM signal_records WHERE timestamp >= ?1 AND timestamp < ?2 AND kind IN (0, 1, 4, 5) ORDER BY kind, timestamp, id",
            )?;
            let rows = statement.query_map(params![day_start, day_start + DAY_MS], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, Vec<u8>>(1)?))
            })?;
            let mut grouped = std::collections::BTreeMap::<i64, Vec<StoredSignalRecord>>::new();
            for row in rows {
                let (kind, compressed) = row?;
                let mut decoder = MultiGzDecoder::new(compressed.as_slice());
                let mut payload = Vec::new();
                decoder.read_to_end(&mut payload)?;
                grouped.entry(kind).or_default().push(bincode::deserialize(&payload)?);
            }
            grouped.into_iter().map(|(kind, records)| {
                let serialized = bincode::serialize(&records)?;
                let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
                encoder.write_all(&serialized)?;
                Ok::<_, anyhow::Error>((kind, encoder.finish()?))
            }).collect::<anyhow::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        for (kind, payload) in training_payloads {
            transaction.execute(
                "UPDATE signal_daily_summary SET training_payload = ?3 WHERE day_start_ms = ?1 AND kind = ?2",
                params![day_start, kind, payload],
            )?;
        }
        if day_start < retention_start {
            deleted_records += transaction.execute(
                "DELETE FROM signal_records WHERE timestamp >= ?1 AND timestamp < ?2",
                params![day_start, day_start + DAY_MS],
            )?;
        }
        transaction.execute(
            "INSERT OR REPLACE INTO signal_daily_summary_runs(day_start_ms, summarized_at_ms) VALUES (?1, ?2)",
            params![day_start, now_ms()],
        )?;
        transaction.commit()?;
    }
    if deleted_records > 0 {
        connection.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM;")?;
    }
    Ok(())
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
    connection.execute_batch(
        r#"PRAGMA journal_mode=WAL;
         CREATE TABLE IF NOT EXISTS signal_daily_summary (
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
         );"#,
    )?;
    let columns = {
        let mut statement = connection.prepare("PRAGMA table_info(signal_daily_summary)")?;
        statement.query_map([], |row| row.get::<_, String>(1))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if !columns.iter().any(|column| column == "training_payload") {
        connection.execute("ALTER TABLE signal_daily_summary ADD COLUMN training_payload BLOB", [])?;
    }
    Ok(connection)
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

fn record_type(kind: i64) -> String {
    match kind {
        0 => "snapshot".to_owned(),
        1 => "outcome".to_owned(),
        2 => "evaluation".to_owned(),
        3 => "book_snapshot".to_owned(),
        4 => "research_features".to_owned(),
        5 => "external_market".to_owned(),
        _ => format!("unknown_{kind}"),
    }
}

fn utc_date(days_since_epoch: i64) -> String {
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_era - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
