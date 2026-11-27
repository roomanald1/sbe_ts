use rusqlite::OptionalExtension;

fn settings_path() -> PathBuf {
    if let Some(path) = std::env::var_os("AUTO_TRADER_STATE_FILE") {
        return PathBuf::from(path);
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("auto_trader_state.json")
}
fn persist_settings(path: &PathBuf, settings: &Settings) -> Result<(), (StatusCode, String)> {
    persist_settings_snapshot(path, settings)
}

fn sqlite_write_guard() -> std::sync::MutexGuard<'static, ()> {
    static SQLITE_WRITE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
        std::sync::OnceLock::new();
    SQLITE_WRITE_LOCK
        .get_or_init(|| std::sync::Mutex::new(()))
        .lock()
        .unwrap_or_else(|error| error.into_inner())
}

/// Snapshot persistence is serialized, so this cache can safely retain the
/// last committed value per database. Most AutoTrader updates change only a
/// small part of Settings; rewriting every normalized row turns a durable
/// reservation into a full-state write on the signal critical path.
fn committed_settings_snapshots() -> &'static std::sync::Mutex<
    std::collections::HashMap<PathBuf, serde_json::Map<String, serde_json::Value>>,
> {
    static SNAPSHOTS: std::sync::OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<PathBuf, serde_json::Map<String, serde_json::Value>>,
        >,
    > = std::sync::OnceLock::new();
    SNAPSHOTS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn settings_database_path(json_path: &PathBuf) -> PathBuf {
    json_path.with_extension("sqlite")
}

fn ensure_dated_carry_schema(connection: &rusqlite::Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "CREATE TABLE IF NOT EXISTS dated_carry_settings (
            id INTEGER PRIMARY KEY CHECK (id = 1), enabled INTEGER NOT NULL,
            max_spot_notional_usdt REAL NOT NULL, checked_at_ms INTEGER NOT NULL,
            last_error TEXT
         );
         CREATE TABLE IF NOT EXISTS dated_carry_candidates (
            future_symbol TEXT PRIMARY KEY, spot_symbol TEXT NOT NULL, expiry_ms INTEGER NOT NULL,
            days_to_expiry REAL NOT NULL, quantity REAL NOT NULL, spot_ask REAL NOT NULL,
            future_bid REAL NOT NULL, spot_bid REAL NOT NULL, future_ask REAL NOT NULL,
            basis_percent REAL NOT NULL, net_after_budget_percent REAL NOT NULL,
            eligible INTEGER NOT NULL, reason TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dated_carry_positions (
            id INTEGER PRIMARY KEY AUTOINCREMENT, future_symbol TEXT NOT NULL, spot_symbol TEXT NOT NULL, expiry_ms INTEGER NOT NULL,
            entry_ms INTEGER NOT NULL, quantity REAL NOT NULL, spot_entry_price REAL NOT NULL,
            future_entry_price REAL NOT NULL, entry_fees REAL NOT NULL, status TEXT NOT NULL,
            scout_parameter_key TEXT, scout_parameter_value REAL, spot_exit_twap REAL,
            settlement_price REAL, net_pnl_usdt REAL, settlement_source TEXT, spot_mark_bid REAL,
            future_mark_ask REAL, spot_leg_pnl_usdt REAL, future_leg_pnl_usdt REAL,
            estimated_exit_fees_usdt REAL, estimated_exit_slippage_usdt REAL,
            marked_net_pnl_usdt REAL, mark_checked_at_ms INTEGER, closed_at_ms INTEGER
         );
         ",
    )?;
    let has_closed_at: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_table_info('dated_carry_positions') WHERE name='closed_at_ms')",
        [],
        |row| row.get(0),
    )?;
    if !has_closed_at {
        connection.execute(
            "ALTER TABLE dated_carry_positions ADD COLUMN closed_at_ms INTEGER",
            [],
        )?;
    }
    Ok(())
}

fn replace_dated_carry_rows(
    transaction: &rusqlite::Transaction<'_>,
    book: &DatedCarryBook,
) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO dated_carry_settings(id, enabled, max_spot_notional_usdt, checked_at_ms, last_error)
         VALUES (1, ?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,
         max_spot_notional_usdt=excluded.max_spot_notional_usdt, checked_at_ms=excluded.checked_at_ms,
         last_error=excluded.last_error",
        rusqlite::params![book.enabled as i64, book.max_spot_notional_usdt, book.checked_at_ms as i64, book.last_error],
    )?;
    for table in ["dated_carry_candidates", "dated_carry_positions"] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    for item in &book.candidates {
        transaction.execute("INSERT INTO dated_carry_candidates VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            rusqlite::params![item.future_symbol,item.spot_symbol,item.expiry_ms as i64,item.days_to_expiry,item.quantity,item.spot_ask,item.future_bid,item.spot_bid,item.future_ask,item.basis_percent,item.net_after_budget_percent,item.eligible as i64,item.reason])?;
    }
    for item in &book.positions {
        transaction.execute("INSERT INTO dated_carry_positions(future_symbol,spot_symbol,expiry_ms,entry_ms,quantity,spot_entry_price,future_entry_price,entry_fees,status,spot_exit_twap,settlement_price,net_pnl_usdt,settlement_source,spot_mark_bid,future_mark_ask,spot_leg_pnl_usdt,future_leg_pnl_usdt,estimated_exit_fees_usdt,estimated_exit_slippage_usdt,marked_net_pnl_usdt,mark_checked_at_ms,closed_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22)",
            rusqlite::params![item.future_symbol,item.spot_symbol,item.expiry_ms as i64,item.entry_ms as i64,item.quantity,item.spot_entry_price,item.future_entry_price,item.entry_fees,item.status,item.spot_exit_twap,item.settlement_price,item.net_pnl_usdt,item.settlement_source,item.spot_mark_bid,item.future_mark_ask,item.spot_leg_pnl_usdt,item.future_leg_pnl_usdt,item.estimated_exit_fees_usdt,item.estimated_exit_slippage_usdt,item.marked_net_pnl_usdt,item.mark_checked_at_ms.map(|v|v as i64),item.closed_at_ms.map(|v|v as i64)])?;
    }
    Ok(())
}

fn read_dated_carry_database(path: &PathBuf) -> rusqlite::Result<Option<DatedCarryBook>> {
    let connection = rusqlite::Connection::open(path)?;
    // The database may already exist from an older auto-trader version that
    // did not create the dated-carry tables. Make reads safe across upgrades,
    // including the first startup after deploying a new binary.
    ensure_dated_carry_schema(&connection)?;
    let transaction = connection.unchecked_transaction()?;
    let settings: Option<(i64, f64, i64, Option<String>)> = transaction.query_row("SELECT enabled,max_spot_notional_usdt,checked_at_ms,last_error FROM dated_carry_settings WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((enabled, max_spot_notional_usdt, checked_at_ms, last_error)) = settings else {
        return Ok(None);
    };
    let mut book = DatedCarryBook {
        enabled: enabled != 0,
        max_spot_notional_usdt,
        checked_at_ms: checked_at_ms as u64,
        last_error,
        ..DatedCarryBook::default()
    };
    book.candidates = {
        let mut stmt = transaction.prepare("SELECT spot_symbol,future_symbol,expiry_ms,days_to_expiry,quantity,spot_ask,future_bid,spot_bid,future_ask,basis_percent,net_after_budget_percent,eligible,reason FROM dated_carry_candidates ORDER BY future_symbol")?;
        stmt.query_map([], |r| {
            Ok(DatedCarryCandidate {
                spot_symbol: r.get(0)?,
                future_symbol: r.get(1)?,
                expiry_ms: r.get::<_, i64>(2)? as u64,
                days_to_expiry: r.get(3)?,
                quantity: r.get(4)?,
                spot_ask: r.get(5)?,
                future_bid: r.get(6)?,
                spot_bid: r.get(7)?,
                future_ask: r.get(8)?,
                basis_percent: r.get(9)?,
                net_after_budget_percent: r.get(10)?,
                eligible: r.get::<_, i64>(11)? != 0,
                reason: r.get(12)?,
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    book.positions = {
        let mut stmt = transaction.prepare("SELECT future_symbol,spot_symbol,expiry_ms,entry_ms,quantity,spot_entry_price,future_entry_price,entry_fees,status,spot_exit_twap,settlement_price,net_pnl_usdt,settlement_source,spot_mark_bid,future_mark_ask,spot_leg_pnl_usdt,future_leg_pnl_usdt,estimated_exit_fees_usdt,estimated_exit_slippage_usdt,marked_net_pnl_usdt,mark_checked_at_ms,closed_at_ms FROM dated_carry_positions ORDER BY id")?;
        stmt.query_map([], |r| {
            Ok(DatedCarryPosition {
                future_symbol: r.get(0)?,
                spot_symbol: r.get(1)?,
                expiry_ms: r.get::<_, i64>(2)? as u64,
                entry_ms: r.get::<_, i64>(3)? as u64,
                quantity: r.get(4)?,
                spot_entry_price: r.get(5)?,
                future_entry_price: r.get(6)?,
                entry_fees: r.get(7)?,
                status: r.get(8)?,
                spot_exit_twap: r.get(9)?,
                settlement_price: r.get(10)?,
                net_pnl_usdt: r.get(11)?,
                settlement_source: r.get(12)?,
                spot_mark_bid: r.get(13)?,
                future_mark_ask: r.get(14)?,
                spot_leg_pnl_usdt: r.get(15)?,
                future_leg_pnl_usdt: r.get(16)?,
                estimated_exit_fees_usdt: r.get(17)?,
                estimated_exit_slippage_usdt: r.get(18)?,
                marked_net_pnl_usdt: r.get(19)?,
                mark_checked_at_ms: r.get::<_, Option<i64>>(20)?.map(|v| v as u64),
                closed_at_ms: r.get::<_, Option<i64>>(21)?.map(|v| v as u64),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    transaction.commit()?;
    Ok(Some(book))
}

fn persist_dated_carry_book(path: &PathBuf, book: &DatedCarryBook) -> Result<(), String> {
    let _sqlite_write_guard = sqlite_write_guard();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = rusqlite::Connection::open(path).map_err(|error| error.to_string())?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(|error| error.to_string())?;
    connection
        .execute_batch("PRAGMA journal_mode = WAL;")
        .map_err(|e| e.to_string())?;
    ensure_dated_carry_schema(&connection).map_err(|e| e.to_string())?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|e| e.to_string())?;
    replace_dated_carry_rows(&transaction, book).map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}

#[derive(Clone, Serialize, Deserialize)]
struct DurableEntryReservation {
    schema_version: u16,
    decision_id: String,
    order: TraderOrder,
    lifecycle_identity: CanonicalLifecycleIdentity,
    canonical_lifecycle_epoch: String,
    canonical_lifecycle_sequence: u64,
    exploratory_event_key: Option<String>,
    reserved_at_ms: u64,
}

fn merge_durable_entry_reservations(
    connection: &rusqlite::Connection,
    settings: &mut Settings,
) -> rusqlite::Result<()> {
    let has_intents: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='durable_entry_reservations')",
        [],
        |row| row.get(0),
    )?;
    if !has_intents {
        return Ok(());
    }
    let mut statement =
        connection.prepare("SELECT payload FROM durable_entry_reservations ORDER BY rowid")?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for payload in rows {
        let reservation =
            serde_json::from_str::<DurableEntryReservation>(&payload).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        let has_order = settings
            .orders
            .iter()
            .any(|order| order.decision_id.as_deref() == Some(&reservation.decision_id));
        if !has_order {
            settings.orders.push(reservation.order);
        }
        settings
            .canonical_lifecycle_links
            .entry(reservation.decision_id)
            .or_insert(reservation.lifecycle_identity);
        if settings.canonical_lifecycle_epoch.is_empty()
            || (settings.canonical_lifecycle_sequence == 0
                && settings.canonical_lifecycle_links.is_empty())
        {
            settings.canonical_lifecycle_epoch = reservation.canonical_lifecycle_epoch;
        }
        settings.canonical_lifecycle_sequence = settings
            .canonical_lifecycle_sequence
            .max(reservation.canonical_lifecycle_sequence);
        if let Some(event_key) = reservation.exploratory_event_key {
            settings
                .exploratory_event_reservations
                .entry(event_key)
                .or_insert(reservation.reserved_at_ms);
        }
    }
    settings.used_notional_today = allocated_notional(settings);
    Ok(())
}

fn mark_restored_price_paths_interrupted(settings: &mut Settings) {
    let restore_epoch_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let next_epoch = |position: &TraderPosition| {
        format!(
            "auto-trader-position-path:v1:restart:{restore_epoch_ms}:{}",
            position.trade_id.as_deref().unwrap_or("unlinked")
        )
    };
    for position in settings.positions.values_mut() {
        if position.quantity.abs() > 1e-12 {
            position.price_path_restart_interrupted = true;
            position.price_path_event_metrics_complete = false;
            let epoch = next_epoch(position);
            position.price_path_epoch = Some(epoch);
        }
    }
    for lots in settings.strategy_positions.values_mut() {
        for position in lots.values_mut() {
            if position.quantity.abs() > 1e-12 {
                position.price_path_restart_interrupted = true;
                position.price_path_event_metrics_complete = false;
                let epoch = next_epoch(position);
                position.price_path_epoch = Some(epoch);
            }
        }
    }
}

fn read_settings_database(path: &PathBuf) -> rusqlite::Result<Option<Settings>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open(path)?;
    let has_durable_snapshot: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='auto_trader_durable_snapshot')",
        [],
        |row| row.get(0),
    )?;
    if has_durable_snapshot {
        let payload = connection
            .query_row(
                "SELECT payload FROM auto_trader_durable_snapshot WHERE id=1",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if let Some(payload) = payload {
            let mut settings = serde_json::from_str(&payload).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            merge_durable_entry_reservations(&connection, &mut settings)?;
            hydrate_persisted_trade_paths(&connection, &mut settings)?;
            mark_restored_price_paths_interrupted(&mut settings);
            return Ok(Some(settings));
        }
    }
    let has_old_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='auto_trader_state')",
        [], |row| row.get(0),
    )?;
    let old_payload: Option<Vec<u8>> = if has_old_table {
        connection
            .query_row(
                "SELECT payload FROM auto_trader_state WHERE id=1",
                [],
                |row| row.get(0),
            )
            .optional()?
    } else {
        None
    };
    let transaction = connection.unchecked_transaction()?;
    let mut fields = serde_json::Map::new();
    let mut statement = transaction.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'setting_%' ORDER BY name",
    )?;
    let tables = statement
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for table in tables {
        let field = table.strip_prefix("setting_").unwrap_or(&table);
        let sql = format!("SELECT value FROM {table} WHERE id=1");
        let value: Option<String> = transaction
            .query_row(&sql, [], |row| row.get(0))
            .optional()?;
        if let Some(value) = value {
            fields.insert(
                field.to_owned(),
                serde_json::from_str(&value).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?,
            );
        }
    }
    let has_normalized = !fields.is_empty();
    transaction.commit()?;
    if has_normalized {
        let mut settings =
            serde_json::from_value(serde_json::Value::Object(fields)).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
        merge_durable_entry_reservations(&connection, &mut settings)?;
        hydrate_persisted_trade_paths(&connection, &mut settings)?;
        mark_restored_price_paths_interrupted(&mut settings);
        return Ok(Some(settings));
    }
    if let Some(payload) = old_payload {
        let settings: Settings = rmp_serde::from_slice(&payload).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                0,
                rusqlite::types::Type::Blob,
                Box::new(error),
            )
        })?;
        persist_settings_snapshot(path, &settings).map_err(|(_, error)| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
        })?;
        let mut settings = settings;
        merge_durable_entry_reservations(&connection, &mut settings)?;
        hydrate_persisted_trade_paths(&connection, &mut settings)?;
        mark_restored_price_paths_interrupted(&mut settings);
        return Ok(Some(settings));
    }
    let has_pending_intents: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='durable_entry_reservations')",
        [],
        |row| row.get(0),
    )?;
    if has_pending_intents {
        let mut settings = Settings::default();
        merge_durable_entry_reservations(&connection, &mut settings)?;
        return Ok(Some(settings));
    }
    Ok(None)
}

fn hydrate_persisted_trade_paths(
    connection: &rusqlite::Connection,
    settings: &mut Settings,
) -> rusqlite::Result<()> {
    const MEMORY_WINDOW: usize = 4_096;
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='trade_price_path_samples')",
        [], |row| row.get(0),
    )?;
    if !table_exists {
        return Ok(());
    }
    fn hydrate_one(
        connection: &rusqlite::Connection,
        position: &mut TraderPosition,
        memory_window: usize,
    ) -> rusqlite::Result<()> {
        let Some(trade_id) = position
            .trade_id
            .as_deref()
            .filter(|trade_id| !trade_id.trim().is_empty())
        else {
            return Ok(());
        };
        let total_rows: i64 = connection.query_row(
            "SELECT COUNT(*) FROM trade_price_path_samples WHERE trade_id=?1",
            [trade_id],
            |row| row.get(0),
        )?;
        if total_rows == 0 {
            return Ok(());
        }
        let mut statement = connection.prepare(
            "SELECT sample_json FROM trade_price_path_samples WHERE trade_id=?1 ORDER BY sample_cursor DESC LIMIT ?2",
        )?;
        let newest = statement
            .query_map(
                rusqlite::params![trade_id, i64::try_from(memory_window).unwrap_or(i64::MAX)],
                |row| {
                    let payload: String = row.get(0)?;
                    serde_json::from_str::<TradePriceSample>(&payload).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        drop(statement);
        let mut samples = position
            .price_path
            .drain(..)
            .map(|sample| ((sample.epoch.clone(), sample.sequence), sample))
            .collect::<std::collections::BTreeMap<_, _>>();
        for sample in newest.into_iter().rev() {
            samples
                .entry((sample.epoch.clone(), sample.sequence))
                .or_insert(sample);
        }
        let mut samples = samples.into_values().collect::<Vec<_>>();
        samples.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then(left.sequence.cmp(&right.sequence))
        });
        if samples.len() > memory_window {
            samples.drain(..samples.len() - memory_window);
        }
        position.price_path_total_samples = position.price_path_total_samples.max(
            samples
                .iter()
                .map(|sample| sample.sequence)
                .max()
                .unwrap_or_default(),
        );
        position.price_path_memory_window_truncated =
            i64::try_from(total_rows).unwrap_or(i64::MAX) > samples.len() as i64;
        position.price_path = samples;
        Ok(())
    }
    for position in settings.positions.values_mut() {
        hydrate_one(connection, position, MEMORY_WINDOW)?;
    }
    for positions in settings.strategy_positions.values_mut() {
        for position in positions.values_mut() {
            hydrate_one(connection, position, MEMORY_WINDOW)?;
        }
    }
    for observation in &mut settings.post_exit_observations {
        // Pending post-exit observations are the durable owner after an exit.
        // A process restart breaks the 1s continuity contract even when the
        // next quote arrives quickly, so never bridge it as a complete path.
        observation["path_restart_interrupted"] = serde_json::Value::Bool(true);
    }
    Ok(())
}

fn read_trade_price_path_page(
    path: &PathBuf,
    trade_id: &str,
    after_timestamp_ms: Option<u64>,
    after_cursor: Option<i64>,
    limit: usize,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let connection = rusqlite::Connection::open(path).map_err(settings_persistence_error)?;
    let table_exists: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='trade_price_path_samples')",
        [], |row| row.get(0),
    ).map_err(settings_persistence_error)?;
    if !table_exists {
        return Ok(serde_json::json!({
            "schema_version": 1, "trade_id": trade_id, "coverage_status": "no_durable_path",
            "history_complete": false, "total_sample_count": 0, "segments": [], "samples": [],
            "next_timestamp_ms": null, "next_cursor": null, "has_more": false
        }));
    }
    let total_count: u64 = connection
        .query_row(
            "SELECT COUNT(*) FROM trade_price_path_samples WHERE trade_id=?1",
            [trade_id],
            |row| row.get::<_, i64>(0).map(|value| value.max(0) as u64),
        )
        .map_err(settings_persistence_error)?;
    let mut segments = Vec::new();
    let mut segment_count_total = 0_u64;
    {
        let mut statement = connection.prepare(
            "SELECT path_epoch,first_sequence,last_sequence,sample_count,first_timestamp_ms,last_timestamp_ms,
                    maximum_gap_ms,complete,missing_reasons_json
             FROM trade_price_path_segments WHERE trade_id=?1 ORDER BY first_timestamp_ms,path_epoch",
        ).map_err(settings_persistence_error)?;
        let rows = statement
            .query_map([trade_id], |row| {
                let reasons_json: String = row.get(8)?;
                let reasons =
                    serde_json::from_str::<Vec<String>>(&reasons_json).unwrap_or_default();
                Ok(serde_json::json!({
                    "path_epoch":row.get::<_,String>(0)?,
                    "first_sequence":row.get::<_,i64>(1)?,
                    "last_sequence":row.get::<_,i64>(2)?,
                    "sample_count":row.get::<_,i64>(3)?,
                    "first_timestamp_ms":row.get::<_,i64>(4)?,
                    "last_timestamp_ms":row.get::<_,i64>(5)?,
                    "maximum_gap_ms":row.get::<_,i64>(6)?,
                    "complete":row.get::<_,i64>(7)? != 0,
                    "missing_reasons":reasons
                }))
            })
            .map_err(settings_persistence_error)?;
        for row in rows {
            let segment = row.map_err(settings_persistence_error)?;
            segment_count_total = segment_count_total
                .saturating_add(segment["sample_count"].as_u64().unwrap_or_default());
            segments.push(segment);
        }
    }
    let query_limit = limit.saturating_add(1);
    let mut statement = if after_timestamp_ms.is_some() && after_cursor.is_some() {
        connection.prepare(
            "SELECT sample_cursor,path_epoch,sequence,timestamp_ms,sample_json FROM trade_price_path_samples
             WHERE trade_id=?1 AND (timestamp_ms>?2 OR (timestamp_ms=?2 AND sample_cursor>?3))
             ORDER BY timestamp_ms,sample_cursor LIMIT ?4",
        )
    } else if after_timestamp_ms.is_none() && after_cursor.is_none() {
        connection.prepare(
            "SELECT sample_cursor,path_epoch,sequence,timestamp_ms,sample_json FROM trade_price_path_samples
             WHERE trade_id=?1 ORDER BY timestamp_ms,sample_cursor LIMIT ?2",
        )
    } else {
        return Err((StatusCode::BAD_REQUEST, "after_timestamp_ms and after_cursor must be supplied together".into()));
    }.map_err(settings_persistence_error)?;
    let rows = if let (Some(timestamp), Some(cursor)) = (after_timestamp_ms, after_cursor) {
        statement
            .query_map(
                rusqlite::params![
                    trade_id,
                    i64::try_from(timestamp).map_err(settings_persistence_error)?,
                    cursor,
                    i64::try_from(query_limit).map_err(settings_persistence_error)?
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(settings_persistence_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(settings_persistence_error)?
    } else {
        statement
            .query_map(
                rusqlite::params![
                    trade_id,
                    i64::try_from(query_limit).map_err(settings_persistence_error)?
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(settings_persistence_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(settings_persistence_error)?
    };
    let has_more = rows.len() > limit;
    let rows = rows.into_iter().take(limit).collect::<Vec<_>>();
    let mut samples = Vec::with_capacity(rows.len());
    for (cursor, epoch, sequence, timestamp, payload) in rows {
        let sample: serde_json::Value =
            serde_json::from_str(&payload).map_err(settings_persistence_error)?;
        samples.push(serde_json::json!({
            "cursor":cursor,"path_epoch":epoch,"sequence":sequence,"timestamp_ms":timestamp,"sample":sample
        }));
    }
    let tail = samples.last();
    let next_timestamp_ms = tail.and_then(|row| row["timestamp_ms"].as_u64());
    let next_cursor = tail.and_then(|row| row["cursor"].as_i64());
    let history_complete = segments.len() == 1
        && segment_count_total == total_count
        && segments
            .first()
            .is_some_and(|segment| segment["complete"].as_bool() == Some(true));
    let mut missing_reasons = segments
        .iter()
        .flat_map(|segment| {
            segment["missing_reasons"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .collect::<std::collections::BTreeSet<_>>();
    if segments.len() > 1 {
        missing_reasons.insert("path_spans_process_or_position_epochs".into());
    }
    if segment_count_total != total_count {
        missing_reasons.insert("segment_sample_count_mismatch".into());
    }
    Ok(serde_json::json!({
        "schema_version":1,"trade_id":trade_id,
        "coverage_status":if history_complete {"complete"} else if total_count==0 {"no_durable_path"} else {"incomplete"},
        "history_complete":history_complete,"total_sample_count":total_count,
        "segments":segments,"missing_reasons":missing_reasons,
        "samples":samples,"next_timestamp_ms":if has_more {next_timestamp_ms} else {None},
        "next_cursor":if has_more {next_cursor} else {None},"has_more":has_more
    }))
}

#[derive(serde::Deserialize)]
struct TradePricePathQuery {
    trade_id: String,
    after_timestamp_ms: Option<u64>,
    after_cursor: Option<i64>,
    limit: Option<usize>,
}

async fn get_trade_price_path(
    State(state): State<AutoTraderState>,
    axum::extract::Query(query): axum::extract::Query<TradePricePathQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    if query.trade_id.trim().is_empty()
        || query.trade_id.len() > 256
        || query.trade_id.chars().any(char::is_control)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "A stable trade_id is required".into(),
        ));
    }
    let limit = query.limit.unwrap_or(250).clamp(1, 1_000);
    let path = settings_database_path(&state.settings_path);
    let trade_id = query.trade_id;
    let after_timestamp = query.after_timestamp_ms;
    let after_cursor = query.after_cursor;
    let page = tokio::task::spawn_blocking(move || {
        read_trade_price_path_page(&path, &trade_id, after_timestamp, after_cursor, limit)
    })
    .await
    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))??;
    Ok(Json(page))
}

fn persist_settings_snapshot(
    path: &PathBuf,
    settings: &Settings,
) -> Result<(), (StatusCode, String)> {
    let _sqlite_write_guard = sqlite_write_guard();
    let database_existed = path.exists();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(settings_persistence_error)?;
    }
    let connection = rusqlite::Connection::open(path).map_err(settings_persistence_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(settings_persistence_error)?;
    connection
        .execute_batch("PRAGMA journal_mode=WAL;")
        .map_err(settings_persistence_error)?;
    let value = serde_json::to_value(settings).map_err(settings_persistence_error)?;
    let object = value
        .as_object()
        .ok_or_else(|| settings_persistence_error("Settings did not serialize as an object"))?;
    if !database_existed {
        committed_settings_snapshots()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .remove(path);
    }
    let committed = committed_settings_snapshots()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .get(path)
        .cloned();
    let transaction = connection
        .unchecked_transaction()
        .map_err(settings_persistence_error)?;
    persist_trade_price_path_samples(&transaction, settings)?;
    for (field, value) in object {
        if committed.as_ref().and_then(|snapshot| snapshot.get(field)) == Some(value) {
            continue;
        }
        let table = format!("setting_{field}");
        transaction.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);")).map_err(settings_persistence_error)?;
        transaction.execute(&format!("INSERT INTO {table}(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value"), [serde_json::to_string(value).map_err(settings_persistence_error)?]).map_err(settings_persistence_error)?;
    }
    if committed.is_none() {
        transaction
            .execute_batch("DROP TABLE IF EXISTS auto_trader_state;")
            .map_err(settings_persistence_error)?;
    }
    prune_durable_entry_reservations(&transaction, settings).map_err(settings_persistence_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS auto_trader_durable_snapshot (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);",
        )
        .map_err(settings_persistence_error)?;
    transaction
        .execute(
            "INSERT INTO auto_trader_durable_snapshot(id,payload) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
            [serde_json::to_string(settings).map_err(settings_persistence_error)?],
        )
        .map_err(settings_persistence_error)?;
    transaction.commit().map_err(settings_persistence_error)?;
    committed_settings_snapshots()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .insert(path.clone(), object.clone());
    Ok(())
}

#[derive(Default)]
struct PersistedPathSegmentProgress {
    first_sequence: u64,
    last_sequence: u64,
    sample_count: u64,
    first_timestamp_ms: u64,
    last_timestamp_ms: u64,
    maximum_gap_ms: u64,
    complete: bool,
    missing_reasons: std::collections::BTreeSet<String>,
}

fn persist_trade_price_path_samples(
    transaction: &rusqlite::Transaction<'_>,
    settings: &Settings,
) -> Result<(), (StatusCode, String)> {
    const MAX_PATH_GAP_MS: u64 = 1_000;
    transaction
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS trade_price_path_samples (
             sample_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
             trade_id TEXT NOT NULL,
             path_epoch TEXT NOT NULL,
             sequence INTEGER NOT NULL,
             timestamp_ms INTEGER NOT NULL,
             sample_json TEXT NOT NULL,
             UNIQUE(trade_id,path_epoch,sequence)
         );
         CREATE INDEX IF NOT EXISTS trade_price_path_samples_page
             ON trade_price_path_samples(trade_id,sample_cursor);
         CREATE INDEX IF NOT EXISTS trade_price_path_samples_chronological_page
             ON trade_price_path_samples(trade_id,timestamp_ms,sample_cursor);
         CREATE TABLE IF NOT EXISTS trade_price_path_segments (
             trade_id TEXT NOT NULL,
             path_epoch TEXT NOT NULL,
             first_sequence INTEGER NOT NULL,
             last_sequence INTEGER NOT NULL,
             sample_count INTEGER NOT NULL,
             first_timestamp_ms INTEGER NOT NULL,
             last_timestamp_ms INTEGER NOT NULL,
             maximum_gap_ms INTEGER NOT NULL,
             complete INTEGER NOT NULL,
             missing_reasons_json TEXT NOT NULL,
             PRIMARY KEY(trade_id,path_epoch)
         );",
        )
        .map_err(settings_persistence_error)?;

    for position in settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.values())
    {
        let (Some(trade_id), Some(active_epoch)) = (
            position
                .trade_id
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
            position
                .price_path_epoch
                .as_deref()
                .filter(|value| !value.trim().is_empty()),
        ) else {
            continue;
        };
        let epochs = position
            .price_path
            .iter()
            .filter_map(|sample| sample.epoch.as_deref())
            .collect::<std::collections::BTreeSet<_>>();
        for epoch in epochs {
            let mut progress = transaction.query_row(
                "SELECT first_sequence,last_sequence,sample_count,first_timestamp_ms,last_timestamp_ms,
                        maximum_gap_ms,complete,missing_reasons_json
                 FROM trade_price_path_segments WHERE trade_id=?1 AND path_epoch=?2",
                rusqlite::params![trade_id, epoch],
                |row| {
                    let reasons_json: String = row.get(7)?;
                    let reasons = serde_json::from_str::<Vec<String>>(&reasons_json)
                        .unwrap_or_default().into_iter().collect::<std::collections::BTreeSet<_>>();
                    Ok(PersistedPathSegmentProgress {
                        first_sequence: row.get::<_, i64>(0)? as u64,
                        last_sequence: row.get::<_, i64>(1)? as u64,
                        sample_count: row.get::<_, i64>(2)? as u64,
                        first_timestamp_ms: row.get::<_, i64>(3)? as u64,
                        last_timestamp_ms: row.get::<_, i64>(4)? as u64,
                        maximum_gap_ms: row.get::<_, i64>(5)? as u64,
                        complete: row.get::<_, i64>(6)? != 0,
                        missing_reasons: reasons,
                    })
                },
            ).optional().map_err(settings_persistence_error)?.unwrap_or_default();
            if progress.sample_count == 0 {
                progress.complete = true;
            }
            let mut samples = position
                .price_path
                .iter()
                .filter(|sample| sample.epoch.as_deref() == Some(epoch))
                .collect::<Vec<_>>();
            samples.sort_by_key(|sample| sample.sequence);
            let persisted_last_sequence = progress.last_sequence;
            for sample in samples
                .into_iter()
                .filter(|sample| sample.sequence > persisted_last_sequence)
            {
                let quote_identity_complete = sample
                    .quote_source_system
                    .as_deref()
                    .is_some_and(|value| !value.trim().is_empty())
                    && sample
                        .quote_source_epoch
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty())
                    && sample.quote_source_cursor.is_some_and(|cursor| cursor > 0)
                    && sample
                        .quote_source_record_id
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty());
                let source_fresh = sample.source_fresh == Some(true)
                    && sample.freshness_contract_version == Some(1)
                    && sample.freshness_max_age_ms.is_some()
                    && sample.source_updated_at_ms.is_some();
                if progress.sample_count == 0 {
                    progress.first_sequence = sample.sequence;
                    progress.first_timestamp_ms = sample.timestamp;
                    if sample.sequence != 1
                        && epoch == active_epoch
                        && !position.price_path_restart_interrupted
                    {
                        progress
                            .missing_reasons
                            .insert("initial_sequence_missing".to_owned());
                    }
                    if !position.price_path_restart_interrupted
                        && position.entry_timestamp_ms > 0
                        && (sample.timestamp < position.entry_timestamp_ms
                            || sample.timestamp.saturating_sub(position.entry_timestamp_ms)
                                > MAX_PATH_GAP_MS)
                    {
                        progress
                            .missing_reasons
                            .insert("entry_to_first_sample_gap_exceeds_bound".to_owned());
                    }
                } else {
                    let gap = sample.timestamp.saturating_sub(progress.last_timestamp_ms);
                    progress.maximum_gap_ms = progress.maximum_gap_ms.max(gap);
                    if sample.sequence != progress.last_sequence.saturating_add(1) {
                        progress
                            .missing_reasons
                            .insert("path_sequence_gap".to_owned());
                    }
                    if sample.timestamp <= progress.last_timestamp_ms || gap > MAX_PATH_GAP_MS {
                        progress
                            .missing_reasons
                            .insert("path_time_gap_exceeds_bound".to_owned());
                    }
                }
                if !source_fresh {
                    progress
                        .missing_reasons
                        .insert("source_freshness_incomplete".to_owned());
                }
                if !quote_identity_complete {
                    progress
                        .missing_reasons
                        .insert("quote_source_identity_missing".to_owned());
                }
                if position.price_path_restart_interrupted {
                    progress
                        .missing_reasons
                        .insert("process_restart_interrupted".to_owned());
                }
                if position.price_path_truncated {
                    progress
                        .missing_reasons
                        .insert("legacy_raw_path_truncated".to_owned());
                }
                transaction.execute(
                    "INSERT OR IGNORE INTO trade_price_path_samples(trade_id,path_epoch,sequence,timestamp_ms,sample_json)
                     VALUES(?1,?2,?3,?4,?5)",
                    rusqlite::params![trade_id, epoch, i64::try_from(sample.sequence).map_err(settings_persistence_error)?,
                        i64::try_from(sample.timestamp).map_err(settings_persistence_error)?,
                        serde_json::to_string(sample).map_err(settings_persistence_error)?],
                ).map_err(settings_persistence_error)?;
                progress.last_sequence = sample.sequence;
                progress.last_timestamp_ms = sample.timestamp;
                progress.sample_count = progress.sample_count.saturating_add(1);
            }
            if position.price_path_restart_interrupted {
                progress
                    .missing_reasons
                    .insert("process_restart_interrupted".to_owned());
            }
            if position.price_path_truncated {
                progress
                    .missing_reasons
                    .insert("legacy_raw_path_truncated".to_owned());
            }
            if position.price_path_finalized {
                progress
                    .missing_reasons
                    .remove("post_exit_observation_pending");
            } else {
                progress
                    .missing_reasons
                    .insert("post_exit_observation_pending".to_owned());
            }
            progress.complete =
                progress.missing_reasons.is_empty() && position.price_path_finalized;
            transaction.execute(
                "INSERT INTO trade_price_path_segments(trade_id,path_epoch,first_sequence,last_sequence,
                     sample_count,first_timestamp_ms,last_timestamp_ms,maximum_gap_ms,complete,missing_reasons_json)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(trade_id,path_epoch) DO UPDATE SET
                     last_sequence=excluded.last_sequence,sample_count=excluded.sample_count,
                     last_timestamp_ms=excluded.last_timestamp_ms,maximum_gap_ms=excluded.maximum_gap_ms,
                     complete=excluded.complete,missing_reasons_json=excluded.missing_reasons_json",
                rusqlite::params![trade_id, epoch, i64::try_from(progress.first_sequence).map_err(settings_persistence_error)?,
                    i64::try_from(progress.last_sequence).map_err(settings_persistence_error)?,
                    i64::try_from(progress.sample_count).map_err(settings_persistence_error)?,
                    i64::try_from(progress.first_timestamp_ms).map_err(settings_persistence_error)?,
                    i64::try_from(progress.last_timestamp_ms).map_err(settings_persistence_error)?,
                    i64::try_from(progress.maximum_gap_ms).map_err(settings_persistence_error)?,
                    if progress.complete { 1_i64 } else { 0_i64 },
                    serde_json::to_string(&progress.missing_reasons.into_iter().collect::<Vec<_>>()).map_err(settings_persistence_error)?],
            ).map_err(settings_persistence_error)?;
        }
    }
    // A closed position is removed from strategy_positions during fill
    // accounting. Its existing post-exit observation retains the captured
    // pre-exit window and post-exit samples until the worker finishes; append
    // those rows through the same transaction and segment validator.
    for observation in &settings.post_exit_observations {
        let Some(trade_id) = observation["trade_id"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
        else {
            continue;
        };
        let mut samples = Vec::new();
        for value in observation["pre_exit_price_path"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(observation["price_path"].as_array().into_iter().flatten())
        {
            if let Ok(sample) = serde_json::from_value::<TradePriceSample>(value.clone()) {
                samples.push(sample);
            }
        }
        if samples.is_empty() {
            continue;
        }
        let path_epoch = observation["pre_exit_price_path_evidence"]["path_epochs"]
            .as_array()
            .and_then(|epochs| epochs.first())
            .and_then(serde_json::Value::as_str)
            .or_else(|| samples.iter().find_map(|sample| sample.epoch.as_deref()));
        let Some(path_epoch) = path_epoch else {
            continue;
        };
        let mut pending = Settings::default();
        let mut position = TraderPosition::default();
        position.trade_id = Some(trade_id.to_owned());
        position.price_path_epoch = Some(path_epoch.to_owned());
        position.entry_timestamp_ms = observation["entry_context"]["timestamp_ms"]
            .as_u64()
            .unwrap_or_default();
        position.price_path_total_samples =
            observation["pre_exit_price_path_evidence"]["total_sample_count"]
                .as_u64()
                .unwrap_or(samples.len() as u64);
        position.price_path_truncated =
            observation["pre_exit_price_path_evidence"]["missing_coverage"]
                .as_array()
                .is_some_and(|reasons| {
                    reasons
                        .iter()
                        .any(|reason| reason == "price_path_retention_truncated")
                });
        position.price_path_restart_interrupted =
            observation["path_restart_interrupted"].as_bool() == Some(true);
        position.price_path_finalized =
            observation["completion_pending_durable"].as_bool() == Some(true);
        position.price_path = samples;
        pending
            .strategy_positions
            .entry(
                observation["symbol"]
                    .as_str()
                    .unwrap_or("unknown")
                    .to_owned(),
            )
            .or_default()
            .insert(
                observation["strategy"]
                    .as_str()
                    .unwrap_or("post-exit")
                    .to_owned(),
                position,
            );
        persist_trade_price_path_samples(transaction, &pending)?;
    }
    Ok(())
}

fn prune_durable_entry_reservations(
    transaction: &rusqlite::Transaction<'_>,
    settings: &Settings,
) -> rusqlite::Result<()> {
    transaction.execute_batch(
        "CREATE TABLE IF NOT EXISTS durable_entry_reservations (decision_id TEXT PRIMARY KEY, payload TEXT NOT NULL);",
    )?;
    let retained_intent_ids = settings
        .orders
        .iter()
        .filter(|order| matches!(order.status.as_str(), "submitting" | "unknown_submission"))
        .filter(|order| {
            order
                .revalidation_snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.submission_request.as_ref())
                .is_some_and(persisted_entry_request_is_replayable)
        })
        .filter_map(|order| order.decision_id.clone())
        .collect::<BTreeSet<_>>();
    let existing_intent_ids = {
        let mut statement =
            transaction.prepare("SELECT decision_id FROM durable_entry_reservations")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for decision_id in existing_intent_ids {
        if !retained_intent_ids.contains(&decision_id) {
            transaction.execute(
                "DELETE FROM durable_entry_reservations WHERE decision_id=?1",
                [decision_id],
            )?;
        }
    }
    Ok(())
}

fn persist_durable_settings_snapshot(
    path: &PathBuf,
    settings: &Settings,
) -> Result<(), (StatusCode, String)> {
    let _sqlite_write_guard = sqlite_write_guard();
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(settings_persistence_error)?;
    }
    let connection = rusqlite::Connection::open(path).map_err(settings_persistence_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(settings_persistence_error)?;
    connection
        .execute_batch(
            "PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS auto_trader_durable_snapshot (id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL);",
        )
        .map_err(settings_persistence_error)?;
    let payload = serde_json::to_string(settings).map_err(settings_persistence_error)?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(settings_persistence_error)?;
    persist_trade_price_path_samples(&transaction, settings)?;
    transaction
        .execute(
            "INSERT INTO auto_trader_durable_snapshot(id,payload) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET payload=excluded.payload",
            [payload],
        )
        .map_err(settings_persistence_error)?;
    prune_durable_entry_reservations(&transaction, settings).map_err(settings_persistence_error)?;
    transaction.commit().map_err(settings_persistence_error)
}

struct DurableEntryWriteResult {
    result: Result<(), (StatusCode, String)>,
    stage_timings_ms: Vec<(&'static str, f64)>,
}

fn persist_durable_entry_reservation_record(
    path: &PathBuf,
    reservations: &[DurableEntryReservation],
) -> DurableEntryWriteResult {
    // One bounded group commit amortizes SQLite transaction/fsync overhead.
    // Callers are acknowledged only after this transaction commits.
    let mut stage_timings_ms = Vec::with_capacity(8);
    let total_started = Instant::now();
    let result = (|| {
        let preparation_started = Instant::now();
        let serialized = reservations
            .iter()
            .map(|reservation| {
                serde_json::to_string(reservation)
                    .map(|payload| (reservation.decision_id.clone(), payload))
                    .map_err(settings_persistence_error)
            })
            .collect::<Result<Vec<_>, _>>()?;
        stage_timings_ms.push((
            "entry_intent_serialization",
            preparation_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        let directory_started = Instant::now();
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).map_err(settings_persistence_error)?;
        }
        stage_timings_ms.push((
            "entry_intent_directory_prepare",
            directory_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        let database_existed = path.exists();
        let open_started = Instant::now();
        let mut connection =
            rusqlite::Connection::open(path).map_err(settings_persistence_error)?;
        connection
            .busy_timeout(Duration::from_secs(5))
            .map_err(settings_persistence_error)?;
        stage_timings_ms.push((
            "entry_intent_connection_open",
            open_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        let schema_started = Instant::now();
        if !database_existed {
            connection
                .execute_batch("PRAGMA journal_mode=WAL;")
                .map_err(settings_persistence_error)?;
        }
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS durable_entry_reservations (decision_id TEXT PRIMARY KEY, payload TEXT NOT NULL);",
            )
            .map_err(settings_persistence_error)?;
        stage_timings_ms.push((
            "entry_intent_schema_setup",
            schema_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        // BEGIN IMMEDIATE makes SQLite writer-lock acquisition observable as
        // its own stage. The subsequent insert and commit remain in the same
        // transaction and retain the existing durable acknowledgement point.
        let sqlite_lock_started = Instant::now();
        let transaction = connection
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(settings_persistence_error)?;
        stage_timings_ms.push((
            "entry_intent_sqlite_lock_wait",
            sqlite_lock_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        let statement_started = Instant::now();
        let mut statement = transaction
            .prepare_cached(
                "INSERT INTO durable_entry_reservations(decision_id,payload) VALUES(?1,?2) ON CONFLICT(decision_id) DO UPDATE SET payload=excluded.payload",
            )
            .map_err(settings_persistence_error)?;
        stage_timings_ms.push((
            "entry_intent_sql_prepare",
            statement_started.elapsed().as_secs_f64() * 1_000.0,
        ));
        let sql_started = Instant::now();
        for (decision_id, payload) in &serialized {
            statement
                .execute(rusqlite::params![decision_id, payload])
                .map_err(settings_persistence_error)?;
        }
        drop(statement);
        stage_timings_ms.push((
            "entry_intent_sql_execute",
            sql_started.elapsed().as_secs_f64() * 1_000.0,
        ));

        let commit_started = Instant::now();
        transaction.commit().map_err(settings_persistence_error)?;
        stage_timings_ms.push((
            "entry_intent_durable_commit",
            commit_started.elapsed().as_secs_f64() * 1_000.0,
        ));
        Ok(())
    })();
    stage_timings_ms.push((
        "entry_intent_sqlite_write_total",
        total_started.elapsed().as_secs_f64() * 1_000.0,
    ));
    DurableEntryWriteResult {
        result,
        stage_timings_ms,
    }
}

struct PendingDurableEntryReservation {
    reservation: DurableEntryReservation,
    completion: tokio::sync::oneshot::Sender<Result<(), (StatusCode, String)>>,
}

fn schedule_durable_entry_reservation_batch(state: &AutoTraderState) {
    if state
        .durable_entry_reservation_batch_scheduled
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    let state = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(SETTINGS_PERSIST_DURABLE_BATCH_WINDOW).await;
        let pending = {
            let mut queue = state.durable_entry_reservation_batch.lock().await;
            let pending = std::mem::take(&mut *queue);
            // Reset while holding the queue lock so a concurrent enqueue is
            // either included above or schedules the next transaction.
            state
                .durable_entry_reservation_batch_scheduled
                .store(false, Ordering::Release);
            pending
        };
        if pending.is_empty() {
            return;
        }
        let reservations = pending
            .iter()
            .map(|item| item.reservation.clone())
            .collect::<Vec<_>>();
        record_settings_persist_sample(
            &state.settings_persist_timings_ms,
            "entry_intent_batch_size",
            reservations.len() as f64,
        );
        let write_lock_started = Instant::now();
        let _write_guard = state.settings_persist_write_lock.clone().lock_owned().await;
        record_settings_persist_sample(
            &state.settings_persist_timings_ms,
            "entry_intent_writer_lock_wait",
            write_lock_started.elapsed().as_secs_f64() * 1_000.0,
        );
        let path = state.settings_path.clone();
        let write_started = Instant::now();
        let persistence = tokio::task::spawn_blocking(move || {
            persist_durable_entry_reservation_record(&path, &reservations)
        })
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()));
        let result = match persistence {
            Ok(persistence) => {
                for (stage, elapsed_ms) in persistence.stage_timings_ms {
                    record_settings_persist_sample(
                        &state.settings_persist_timings_ms,
                        stage,
                        elapsed_ms,
                    );
                }
                persistence.result
            }
            Err(error) => Err(error),
        };
        record_settings_persist_sample(
            &state.settings_persist_timings_ms,
            "entry_intent_write",
            write_started.elapsed().as_secs_f64() * 1_000.0,
        );
        for item in pending {
            let _ = item.completion.send(result.clone());
        }
    });
}

async fn persist_entry_reservation_durable(
    state: &AutoTraderState,
    decision_id: &str,
    exploratory_event_key: Option<&str>,
) -> Result<(), (StatusCode, String)> {
    let settings_guard = Arc::clone(&state.settings).lock_owned().await;
    let Some(order) = settings_guard
        .orders
        .iter()
        .find(|order| order.decision_id.as_deref() == Some(decision_id))
        .cloned()
    else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Durable entry reservation is missing from shared settings".to_owned(),
        ));
    };
    let Some(identity) = settings_guard
        .canonical_lifecycle_links
        .get(decision_id)
        .cloned()
    else {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Durable entry reservation has no canonical lifecycle identity".to_owned(),
        ));
    };
    let replayable = order
        .revalidation_snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.submission_request.as_ref())
        .is_some_and(persisted_entry_request_is_replayable);
    if !replayable {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Durable entry reservation has no replayable Position Management request".to_owned(),
        ));
    }
    let reserved_at_ms = exploratory_event_key
        .and_then(|key| {
            settings_guard
                .exploratory_event_reservations
                .get(key)
                .copied()
        })
        .unwrap_or_else(current_time_ms);
    let reservation = DurableEntryReservation {
        schema_version: 1,
        decision_id: decision_id.to_owned(),
        order,
        lifecycle_identity: identity,
        canonical_lifecycle_epoch: settings_guard.canonical_lifecycle_epoch.clone(),
        canonical_lifecycle_sequence: settings_guard.canonical_lifecycle_sequence,
        exploratory_event_key: exploratory_event_key.map(str::to_owned),
        reserved_at_ms,
    };
    drop(settings_guard);
    let (completion, result) = tokio::sync::oneshot::channel();
    state
        .durable_entry_reservation_batch
        .lock()
        .await
        .push(PendingDurableEntryReservation {
            reservation,
            completion,
        });
    schedule_durable_entry_reservation_batch(state);
    result
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?
}

fn settings_persistence_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Could not save Auto Trader state to SQLite: {error}"),
    )
}

#[cfg(test)]
mod sqlite_state_tests {
    use super::*;

    fn temporary_database_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("auto-trader-state-{nonce}.sqlite"))
    }

    #[test]
    fn settings_snapshot_round_trips_as_individual_sqlite_tables() {
        let path = temporary_database_path();
        let settings = Settings::default();
        persist_settings_snapshot(&path, &settings).unwrap();
        assert_eq!(
            serde_json::to_value(read_settings_database(&path).unwrap().unwrap()).unwrap(),
            serde_json::to_value(settings).unwrap()
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        let old_table: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='auto_trader_state'", [], |row| row.get(0)).unwrap();
        assert_eq!(old_table, 0);
        let tables: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'setting_%'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(tables > 50);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn repeated_snapshot_updates_changed_rows_without_rewriting_unchanged_rows() {
        let path = temporary_database_path();
        let mut settings = Settings::default();
        persist_settings_snapshot(&path, &settings).unwrap();

        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE persistence_update_counts (table_name TEXT PRIMARY KEY, updates INTEGER NOT NULL DEFAULT 0);
                 INSERT INTO persistence_update_counts(table_name) VALUES ('setting_enabled'), ('setting_strategies');
                 CREATE TRIGGER count_enabled_update AFTER UPDATE ON setting_enabled BEGIN
                    UPDATE persistence_update_counts SET updates = updates + 1 WHERE table_name = 'setting_enabled';
                 END;
                 CREATE TRIGGER count_strategies_update AFTER UPDATE ON setting_strategies BEGIN
                    UPDATE persistence_update_counts SET updates = updates + 1 WHERE table_name = 'setting_strategies';
                 END;",
            )
            .unwrap();
        drop(connection);

        settings.enabled = true;
        persist_settings_snapshot(&path, &settings).unwrap();

        let connection = rusqlite::Connection::open(&path).unwrap();
        let enabled_updates: i64 = connection
            .query_row(
                "SELECT updates FROM persistence_update_counts WHERE table_name='setting_enabled'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let unchanged_updates: i64 = connection
            .query_row(
                "SELECT updates FROM persistence_update_counts WHERE table_name='setting_strategies'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(enabled_updates, 1);
        assert_eq!(unchanged_updates, 0);
        assert!(read_settings_database(&path).unwrap().unwrap().enabled);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn restored_open_price_path_keeps_samples_and_is_marked_interrupted_across_checkpoints() {
        let path = temporary_database_path();
        let mut settings = Settings::default();
        let position = TraderPosition {
            quantity: 2.0,
            average_entry_price: 100.0,
            trade_id: Some("trade-path-restart".into()),
            entry_timestamp_ms: 1_000,
            price_path_epoch: Some("epoch-before-restart".into()),
            price_path: vec![TradePriceSample {
                timestamp: 1_000,
                bid: 99.5,
                ask: 100.5,
                sequence: 17,
                epoch: Some("epoch-before-restart".into()),
                source_updated_at_ms: Some(990),
                freshness_contract_version: Some(1),
                freshness_max_age_ms: Some(5_000),
                source_fresh: Some(true),
                ..TradePriceSample::default()
            }],
            price_path_total_samples: 1,
            price_path_truncated: false,
            initial_risk_reference: Some(InitialRiskReference {
                schema_version: 1,
                quote_currency: Some("USDT".into()),
                stop_loss_percent: Some(0.5),
                quote_amount: Some(1.0),
                fill_ids: vec!["pm-fill:v1:paper:17".into()],
                status: "complete".into(),
            }),
            ..TraderPosition::default()
        };
        settings
            .strategy_positions
            .entry("BTC-USDT".into())
            .or_default()
            .insert("momentum".into(), position.clone());
        persist_durable_settings_snapshot(&path, &settings).unwrap();

        let mut restored = read_settings_database(&path).unwrap().unwrap();
        let restored_position = &restored.strategy_positions["BTC-USDT"]["momentum"];
        assert_eq!(
            serde_json::to_value(&restored_position.price_path).unwrap(),
            serde_json::to_value(&position.price_path).unwrap()
        );
        assert_eq!(restored_position.price_path_total_samples, 1);
        assert!(!restored_position.price_path_truncated);
        assert!(restored_position.price_path_restart_interrupted);
        assert_ne!(
            restored_position.price_path_epoch,
            position.price_path_epoch
        );
        assert_eq!(
            restored_position
                .initial_risk_reference
                .as_ref()
                .unwrap()
                .quote_amount,
            Some(1.0)
        );
        let evidence = price_path_excursion_evidence(restored_position, 2_000);
        assert_eq!(evidence["complete"], false);
        assert!(
            evidence["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "price_path_restart_interrupted")
        );

        persist_durable_settings_snapshot(&path, &restored).unwrap();
        restored = read_settings_database(&path).unwrap().unwrap();
        let restored_position = &restored.strategy_positions["BTC-USDT"]["momentum"];
        assert_eq!(
            serde_json::to_value(&restored_position.price_path).unwrap(),
            serde_json::to_value(&position.price_path).unwrap()
        );
        assert!(restored_position.price_path_restart_interrupted);
        assert_eq!(restored_position.price_path[0].sequence, 17);
        assert_eq!(
            restored_position.price_path[0].epoch.as_deref(),
            Some("epoch-before-restart")
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn durable_reservation_and_learning_event_reuse_ids_after_sqlite_restart() {
        let path = temporary_database_path();
        let mut state = benchmark_snapshot_state(Settings::default());
        state.settings_path = path.clone();
        let first = {
            let mut settings = state.settings.lock().await;
            let first = canonical_lifecycle_identity_for_reservation(
                &mut settings,
                "durable-decision",
                Some("profile-v1".into()),
                None,
                None,
                false,
                None,
                None,
            )
            .unwrap();
            record_strategy_trade_outcome(
                &mut settings,
                "BTC-USDT",
                Some("live-momentum"),
                Some("durable-context"),
                false,
                1.25,
                Some(0.8),
                Some(&serde_json::json!({"strategy_parameters": {"minimum_momentum": 0.003}})),
                Some("durable-decision"),
                None,
            );
            first
        };
        request_settings_persist(&state).unwrap();
        persist_settings_snapshot_durable(&state).await.unwrap();

        let mut restarted = read_settings_database(&path).unwrap().unwrap();
        let retried = canonical_lifecycle_identity_for_reservation(
            &mut restarted,
            "durable-decision",
            Some("profile-v1".into()),
            None,
            None,
            false,
            None,
            None,
        )
        .unwrap();
        assert_eq!(retried, first);
        assert_eq!(restarted.canonical_lifecycle_sequence, 1);
        let Some(FeedbackEvent::TradeOutcome {
            schema_version,
            trade_id,
            opportunity_id,
            episode_id,
            execution_mode,
            entry_policy,
            source,
            ..
        }) = restarted.feedback_outbox.first()
        else {
            panic!("persisted canonical Learning event is missing")
        };
        assert_eq!(*schema_version, Some(1));
        assert_eq!(trade_id.as_deref(), Some(first.trade_id.as_str()));
        assert_eq!(
            opportunity_id.as_deref(),
            Some(first.opportunity_id.as_str())
        );
        assert_eq!(episode_id.as_deref(), Some(first.episode_id.as_str()));
        assert_eq!(execution_mode.as_deref(), Some("paper"));
        assert_eq!(entry_policy.as_deref(), Some("strict"));
        assert_eq!(source.as_deref(), Some("auto_trader_entry_reservation"));

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn concurrent_durable_waiters_share_the_newest_snapshot_commit() {
        let path = temporary_database_path();
        let mut state = benchmark_snapshot_state(Settings::default());
        state.settings_path = path.clone();
        {
            let mut settings = state.settings.lock().await;
            settings.last_decision = "reservation batch is durable".to_owned();
            request_settings_persist(&state).unwrap();
            request_settings_persist(&state).unwrap();
        }

        let (first, second) = tokio::join!(
            persist_settings_snapshot_durable(&state),
            persist_settings_snapshot_durable(&state),
        );
        first.unwrap();
        second.unwrap();
        assert_eq!(
            state
                .settings_persist_durable_revision
                .load(Ordering::Acquire),
            2
        );
        let timings = state.settings_persist_timings_ms.lock().unwrap();
        assert_eq!(
            timings
                .get("durable_snapshot_write")
                .map(Vec::len)
                .unwrap_or_default(),
            1,
            "one committed snapshot should satisfy concurrent callers whose mutations it contains"
        );
        drop(timings);
        let restored = read_settings_database(&path).unwrap().unwrap();
        assert_eq!(restored.last_decision, "reservation batch is durable");

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn migrates_legacy_settings_blob_on_read() {
        let path = temporary_database_path();
        let mut settings = Settings::default();
        settings.positions.insert(
            "BTC-USDT".into(),
            TraderPosition {
                quantity: 1.0,
                price_path: vec![TradePriceSample {
                    timestamp: 1_000,
                    bid: 99.0,
                    ask: 101.0,
                    ..TradePriceSample::default()
                }],
                price_path_total_samples: 1,
                ..TraderPosition::default()
            },
        );
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE auto_trader_state (id INTEGER PRIMARY KEY, payload BLOB NOT NULL);",
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO auto_trader_state VALUES(1,?1)",
                [rmp_serde::to_vec_named(&settings).unwrap()],
            )
            .unwrap();
        drop(connection);

        let loaded = read_settings_database(&path).unwrap().unwrap();
        let restored = loaded.positions.get("BTC-USDT").unwrap();
        let original = settings.positions.get("BTC-USDT").unwrap();
        assert!(restored.price_path_restart_interrupted);
        assert_eq!(
            serde_json::to_value(&restored.price_path).unwrap(),
            serde_json::to_value(&original.price_path).unwrap()
        );
        assert_eq!(
            restored.price_path_total_samples,
            original.price_path_total_samples
        );
        assert!(
            restored
                .price_path_epoch
                .as_deref()
                .is_some_and(|epoch| epoch.starts_with("auto-trader-position-path:v1:restart:"))
        );
        assert_ne!(restored.price_path_epoch, original.price_path_epoch);
        assert_eq!(loaded.stop_loss_percent, settings.stop_loss_percent);
        assert_eq!(loaded.take_profit_percent, settings.take_profit_percent);
        let connection = rusqlite::Connection::open(&path).unwrap();
        let old_table: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='auto_trader_state'", [], |row| row.get(0)).unwrap();
        assert_eq!(old_table, 0);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn normalized_dated_carry_rows_round_trip() {
        let path = temporary_database_path();
        let candidate = DatedCarryCandidate {
            spot_symbol: "BTCUSDT".into(),
            future_symbol: "BTCUSDT_261225".into(),
            expiry_ms: 1_800_000_000_000,
            days_to_expiry: 30.0,
            quantity: 0.01,
            spot_ask: 100_000.0,
            future_bid: 101_000.0,
            spot_bid: 99_999.0,
            future_ask: 101_001.0,
            basis_percent: 1.0,
            net_after_budget_percent: 0.5,
            eligible: true,
            reason: "positive basis".into(),
        };
        let book = DatedCarryBook {
            enabled: true,
            max_spot_notional_usdt: 2_500.0,
            candidates: vec![candidate.clone()],
            positions: vec![DatedCarryPosition {
                spot_symbol: candidate.spot_symbol.clone(),
                future_symbol: candidate.future_symbol.clone(),
                expiry_ms: candidate.expiry_ms,
                entry_ms: 1_700_000_000_000,
                quantity: 0.01,
                spot_entry_price: 100_050.0,
                future_entry_price: 100_950.0,
                entry_fees: 1.5,
                status: "open".into(),
                spot_exit_twap: None,
                settlement_price: None,
                net_pnl_usdt: None,
                settlement_source: None,
                spot_mark_bid: Some(99_999.0),
                future_mark_ask: Some(101_001.0),
                spot_leg_pnl_usdt: Some(-0.51),
                future_leg_pnl_usdt: Some(-0.51),
                estimated_exit_fees_usdt: Some(1.52),
                estimated_exit_slippage_usdt: Some(1.01),
                marked_net_pnl_usdt: Some(-4.55),
                mark_checked_at_ms: Some(1_700_000_100_000),
                closed_at_ms: None,
            }],
            checked_at_ms: 1_700_000_100_000,
            last_error: Some("recoverable scan warning".into()),
        };

        persist_dated_carry_book(&path, &book).unwrap();
        let loaded = read_dated_carry_database(&path).unwrap().unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&book).unwrap()
        );

        let connection = rusqlite::Connection::open(&path).unwrap();
        let candidate_columns: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('dated_carry_candidates')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(candidate_columns, 13);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}

fn record_settings_persist_sample(
    timings: &std::sync::Mutex<BTreeMap<String, Vec<f64>>>,
    phase: &str,
    elapsed_ms: f64,
) {
    if let Ok(mut timings) = timings.lock() {
        let values = timings.entry(phase.to_owned()).or_default();
        values.push(elapsed_ms);
        if values.len() > 500 {
            values.remove(0);
        }
    }
}

fn request_settings_persist(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    state
        .settings_persist_revision
        .fetch_add(1, Ordering::AcqRel);
    state.settings_persist_notify.notify_one();
    let _ = state.state_updates.send(());
    Ok(())
}

const SETTINGS_PERSIST_DURABLE_BATCH_WINDOW: Duration = Duration::from_micros(250);

struct DurableSnapshotWaiter(Arc<AtomicU64>);

impl Drop for DurableSnapshotWaiter {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Persist a settings snapshot under the same serialized writer lock used by
/// the coalescing worker, and return only after its SQLite transaction commits.
/// Entry reservations use this before external order submission so a restart
/// can retry with the same canonical client order ID.
async fn persist_settings_snapshot_durable(
    state: &AutoTraderState,
) -> Result<(), (StatusCode, String)> {
    // Let a small cohort of concurrent reservation/fill callers join one
    // snapshot transaction. The committed Settings snapshot carries all
    // revisions visible under the Settings mutex, so each waiter whose
    // mutation is included can return after the same durable commit.
    let notified = state.settings_persist_batch_notify.notified();
    let waiter_counter = Arc::clone(&state.settings_persist_durable_waiters);
    let previous_waiters = waiter_counter.fetch_add(1, Ordering::AcqRel);
    let _waiter = DurableSnapshotWaiter(waiter_counter);
    if previous_waiters == 0 {
        let _ = tokio::time::timeout(SETTINGS_PERSIST_DURABLE_BATCH_WINDOW, notified).await;
    } else {
        state.settings_persist_batch_notify.notify_one();
    }
    let write_lock_wait_started = Instant::now();
    let _write_guard = Arc::clone(&state.settings_persist_write_lock)
        .lock_owned()
        .await;
    record_settings_persist_sample(
        &state.settings_persist_timings_ms,
        "durable_writer_lock_wait",
        write_lock_wait_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let snapshot_started = Instant::now();
    let settings_guard = Arc::clone(&state.settings).lock_owned().await;
    let target_revision = state.settings_persist_revision.load(Ordering::Acquire);
    if target_revision
        <= state
            .settings_persist_durable_revision
            .load(Ordering::Acquire)
    {
        return Ok(());
    }
    let snapshot = tokio::task::spawn_blocking(move || settings_guard.clone())
        .await
        .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    record_settings_persist_sample(
        &state.settings_persist_timings_ms,
        "durable_snapshot_copy",
        snapshot_started.elapsed().as_secs_f64() * 1_000.0,
    );
    let path = state.settings_path.clone();
    let write_started = Instant::now();
    let result =
        tokio::task::spawn_blocking(move || persist_durable_settings_snapshot(&path, &snapshot))
            .await
            .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    record_settings_persist_sample(
        &state.settings_persist_timings_ms,
        "durable_snapshot_write",
        write_started.elapsed().as_secs_f64() * 1_000.0,
    );
    if result.is_ok() {
        state
            .settings_persist_durable_revision
            .store(target_revision, Ordering::Release);
    }
    result
}

// Coalesce 1-second path samples into the existing 5-second Settings snapshot
// writer. Restart continuity remains fail-closed because a crash can leave a
// gap between the last in-memory sample and this checkpoint.
const PRICE_PATH_PERSIST_INTERVAL: Duration = Duration::from_secs(5);
const SETTINGS_PERSIST_COALESCE_WINDOW: Duration = Duration::from_millis(250);
const SETTINGS_NORMALIZED_PROJECTION_INTERVAL: Duration = Duration::from_secs(60);

fn request_price_path_persist_if_due(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let now = Instant::now();
    let mut last_persisted = state
        .price_path_last_checkpoint_requested_at
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if last_persisted.is_none_or(|last| now.duration_since(last) >= PRICE_PATH_PERSIST_INTERVAL) {
        *last_persisted = Some(now);
        request_settings_persist(state)?;
    }
    Ok(())
}

fn start_settings_persistence_worker(state: &AutoTraderState) {
    let settings = Arc::clone(&state.settings);
    let path = state.settings_path.clone();
    let notify = Arc::clone(&state.settings_persist_notify);
    let write_lock = Arc::clone(&state.settings_persist_write_lock);
    let persist_revision = Arc::clone(&state.settings_persist_revision);
    let durable_revision = Arc::clone(&state.settings_persist_durable_revision);
    let durable_waiters = Arc::clone(&state.settings_persist_durable_waiters);
    let timings = Arc::clone(&state.settings_persist_timings_ms);
    tokio::spawn(async move {
        let mut normalized_revision = 0_u64;
        let mut last_normalized_projection = Instant::now();
        loop {
            notify.notified().await;
            tokio::time::sleep(SETTINGS_PERSIST_COALESCE_WINDOW).await;
            let write_guard = Arc::clone(&write_lock).lock_owned().await;
            let lock_wait_started = Instant::now();
            let settings_guard = Arc::clone(&settings).lock_owned().await;
            let target_revision = persist_revision.load(Ordering::Acquire);
            if target_revision <= durable_revision.load(Ordering::Acquire) {
                drop(settings_guard);
                drop(write_guard);
                continue;
            }
            record_settings_persist_sample(
                &timings,
                "snapshot_lock_wait",
                lock_wait_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let copy_started = Instant::now();
            let settings_snapshot =
                match tokio::task::spawn_blocking(move || settings_guard.clone()).await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        drop(write_guard);
                        crate::service_log!(
                            "ERROR",
                            "Auto Trader state snapshot copier failed: {error}"
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        notify.notify_one();
                        continue;
                    }
                };
            record_settings_persist_sample(
                &timings,
                "snapshot_copy",
                copy_started.elapsed().as_secs_f64() * 1_000.0,
            );
            let snapshot_path = path.clone();
            let write_started = Instant::now();
            match tokio::task::spawn_blocking(move || {
                persist_durable_settings_snapshot(&snapshot_path, &settings_snapshot)
            })
            .await
            {
                Ok(Ok(())) => {
                    durable_revision.store(target_revision, Ordering::Release);
                    record_settings_persist_sample(
                        &timings,
                        "snapshot_write",
                        write_started.elapsed().as_secs_f64() * 1_000.0,
                    );
                    drop(write_guard);
                    // Normalized setting_* rows remain useful to diagnostic
                    // queries, while recovery reads the atomic durable JSON
                    // snapshot above. Refresh the slower projection during
                    // idle periods so it cannot hold up new durable intents.
                    if target_revision > normalized_revision
                        && last_normalized_projection.elapsed()
                            >= SETTINGS_NORMALIZED_PROJECTION_INTERVAL
                    {
                        if let Ok(projection_guard) = Arc::clone(&write_lock).try_lock_owned() {
                            tokio::task::yield_now().await;
                            if durable_waiters.load(Ordering::Acquire) == 0
                                && persist_revision.load(Ordering::Acquire)
                                    == durable_revision.load(Ordering::Acquire)
                            {
                                let settings_guard = Arc::clone(&settings).lock_owned().await;
                                let projection = match tokio::task::spawn_blocking(move || {
                                    settings_guard.clone()
                                })
                                .await
                                {
                                    Ok(projection) => projection,
                                    Err(error) => {
                                        drop(projection_guard);
                                        crate::service_log!(
                                            "ERROR",
                                            "Auto Trader normalized settings projection snapshot failed: {error}"
                                        );
                                        continue;
                                    }
                                };
                                let path = path.clone();
                                let projection_started = Instant::now();
                                match tokio::task::spawn_blocking(move || {
                                    persist_settings_snapshot(&path, &projection)
                                })
                                .await
                                {
                                    Ok(Ok(())) => {
                                        normalized_revision = target_revision;
                                        last_normalized_projection = Instant::now();
                                        record_settings_persist_sample(
                                            &timings,
                                            "normalized_projection_write",
                                            projection_started.elapsed().as_secs_f64() * 1_000.0,
                                        );
                                    }
                                    Ok(Err((_, error))) => crate::service_log!(
                                        "ERROR",
                                        "Auto Trader normalized settings projection failed: {error}"
                                    ),
                                    Err(error) => crate::service_log!(
                                        "ERROR",
                                        "Auto Trader normalized settings projection task failed: {error}"
                                    ),
                                }
                            }
                            drop(projection_guard);
                        }
                    }
                }
                Ok(Err((_, error))) => {
                    drop(write_guard);
                    crate::service_log!(
                        "ERROR",
                        "Auto Trader state snapshot write failed: {error}"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                }
                Err(error) => {
                    drop(write_guard);
                    crate::service_log!(
                        "ERROR",
                        "Auto Trader state snapshot writer failed: {error}"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                }
            }
        }
    });
}

fn utc_day() -> String {
    let days = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 86_400;
    days.to_string()
}
fn scheduled_stop_headroom_multiplier(order_type: &str, strategy: Option<&str>) -> f64 {
    let scheduled = order_type.starts_with("Vwap") || order_type.starts_with("Twap");
    if scheduled
        && Strategy::from_name(strategy.unwrap_or_default())
            .is_some_and(Strategy::uses_protective_stops)
    {
        2.0
    } else {
        1.0
    }
}

fn allocated_notional(settings: &Settings) -> f64 {
    allocated_notional_for_category(settings, None) + settings.dated_carry_reserved_usd.max(0.0)
}

fn allocated_notional_for_symbol(settings: &Settings, symbol: &str) -> f64 {
    let quote_rate = quote_usd_rate(settings, symbol);
    let open_exposure = if let Some(lots) = settings.strategy_positions.get(symbol) {
        lots.values()
            .filter(|position| !position.scout_entry)
            .map(|position| {
                position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate
            })
            .sum()
    } else {
        settings
            .positions
            .get(symbol)
            .filter(|position| !position.scout_entry)
            .map_or(0.0, |position| {
                position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate
            })
    };
    let mut pending_exposure = 0.0;
    let mut pending_quantity = 0.0;
    let current_quantity = settings
        .positions
        .get(symbol)
        .filter(|position| !position.scout_entry)
        .map_or(0.0, |position| position.quantity);
    for order in settings.orders.iter().filter(|order| {
        order.symbol == symbol
            && order.purpose == "signal"
            && matches!(
                order.status.as_str(),
                "submitting" | "accepted" | "resting" | "partially_filled"
            )
            && order.quantity > order.filled_quantity + 1e-12
    }) {
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let net_quantity = current_quantity + pending_quantity;
        let exposure_increase = if net_quantity == 0.0 || net_quantity.signum() == direction {
            remaining
        } else {
            (remaining - net_quantity.abs()).max(0.0)
        };
        let reference_notional = if order.reserved_notional > 0.0 {
            order.reserved_notional
        } else {
            order.average_price.unwrap_or(0.0) * order.quantity * quote_rate
        };
        pending_exposure += reference_notional * exposure_increase / order.quantity;
        pending_quantity += direction * remaining;
    }
    open_exposure + pending_exposure
}

fn allocated_notional_for_category(settings: &Settings, category: Option<StrategyCategory>) -> f64 {
    let open_positions = if let Some(category) = category {
        let mut lots_total = 0.0;
        for (symbol, lots) in &settings.strategy_positions {
            for (strategy, position) in lots {
                if strategy_category(Some(strategy)) != category || position.scout_entry {
                    continue;
                }
                let value = position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol);
                lots_total += value
                    * scheduled_stop_headroom_multiplier(
                        &position.entry_order_type,
                        Some(strategy.as_str()),
                    );
            }
        }
        if settings.strategy_positions.is_empty() {
            settings
                .positions
                .iter()
                .filter(|(_, position)| {
                    !position.scout_entry
                        && strategy_category(position.strategy.as_deref()) == category
                })
                .map(|(symbol, position)| {
                    position.quantity.abs()
                        * position.average_entry_price.max(0.0)
                        * quote_usd_rate(settings, symbol)
                        * scheduled_stop_headroom_multiplier(
                            &position.entry_order_type,
                            position.strategy.as_deref(),
                        )
                })
                .sum()
        } else {
            lots_total
        }
    } else if !settings.strategy_positions.is_empty() {
        settings
            .strategy_positions
            .iter()
            .flat_map(|(symbol, lots)| {
                lots.iter()
                    .filter(|(_, position)| !position.scout_entry)
                    .map(move |(strategy, position)| {
                        let value = position.quantity.abs()
                            * position.average_entry_price.max(0.0)
                            * quote_usd_rate(settings, symbol);
                        value
                            * scheduled_stop_headroom_multiplier(
                                &position.entry_order_type,
                                Some(strategy.as_str()),
                            )
                    })
            })
            .sum()
    } else {
        settings
            .positions
            .iter()
            .filter(|(_, position)| !position.scout_entry)
            .map(|(symbol, position)| {
                let value = position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol);
                value
                    * scheduled_stop_headroom_multiplier(
                        &position.entry_order_type,
                        position.strategy.as_deref(),
                    )
            })
            .sum()
    };
    let mut pending_exposure = 0.0;
    let mut pending_quantity = BTreeMap::<String, f64>::new();
    let mut pending_category_quantity = BTreeMap::<(String, String), f64>::new();
    for order in &settings.orders {
        if matches!(order.purpose.as_str(), "stop-loss" | "scout")
            || !matches!(
                order.status.as_str(),
                "submitting" | "accepted" | "resting" | "partially_filled"
            )
            || order.quantity <= 0.0
        {
            continue;
        }
        let remaining = (order.quantity - order.filled_quantity).max(0.0);
        if remaining <= 1e-12 {
            continue;
        }
        let current_quantity = if let Some(category) = category {
            let category_key = strategy_category_key(category);
            settings
                .strategy_positions
                .get(&order.symbol)
                .map(|lots| {
                    lots.iter()
                        .filter(|(strategy, position)| {
                            !position.scout_entry
                                && strategy_category(Some(strategy.as_str())) == category
                        })
                        .map(|(_, position)| position.quantity)
                        .sum::<f64>()
                })
                .unwrap_or_else(|| {
                    settings
                        .positions
                        .get(&order.symbol)
                        .filter(|position| {
                            !position.scout_entry
                                && strategy_category(position.strategy.as_deref()) == category
                        })
                        .filter(|position| !position.scout_entry)
                        .map(|position| position.quantity)
                        .unwrap_or(0.0)
                })
                + pending_category_quantity
                    .get(&(order.symbol.clone(), category_key.to_owned()))
                    .copied()
                    .unwrap_or(0.0)
        } else {
            settings
                .positions
                .get(&order.symbol)
                .filter(|position| !position.scout_entry)
                .map(|position| position.quantity)
                .unwrap_or(0.0)
                + pending_quantity.get(&order.symbol).copied().unwrap_or(0.0)
        };
        let direction = if order.side == "buy" { 1.0 } else { -1.0 };
        let increase_quantity = if current_quantity == 0.0 || current_quantity.signum() == direction
        {
            remaining
        } else {
            (remaining - current_quantity.abs()).max(0.0)
        };
        if category.is_none_or(|category| strategy_category(order.strategy.as_deref()) == category)
        {
            let reference_notional = if order.reserved_notional > 0.0 {
                order.reserved_notional
            } else {
                order.average_price.unwrap_or(0.0)
                    * order.quantity
                    * quote_usd_rate(settings, &order.symbol)
            };
            pending_exposure += reference_notional * increase_quantity / order.quantity
                * scheduled_stop_headroom_multiplier(&order.order_type, order.strategy.as_deref());
            if let Some(category) = category {
                *pending_category_quantity
                    .entry((
                        order.symbol.clone(),
                        strategy_category_key(category).to_owned(),
                    ))
                    .or_default() += direction * remaining;
            }
        }
        *pending_quantity.entry(order.symbol.clone()).or_default() += direction * remaining;
    }
    open_positions + pending_exposure
}

fn total_open_position_count(settings: &Settings) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
            .count();
    }
    settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.values())
        .filter(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
        .count()
}

fn category_open_position_count(settings: &Settings, category: StrategyCategory) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| {
                position.quantity.abs() > 1e-12
                    && !position.scout_entry
                    && strategy_category(position.strategy.as_deref()) == category
            })
            .count();
    }
    settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.iter())
        .filter(|(strategy, position)| {
            strategy_category(Some(strategy.as_str())) == category
                && position.quantity.abs() > 1e-12
                && !position.scout_entry
        })
        .count()
}

fn category_position_limit(settings: &Settings, category: StrategyCategory) -> usize {
    match category {
        StrategyCategory::Intraday => settings.max_intraday_open_positions,
        StrategyCategory::Daily => settings.max_daily_open_positions,
    }
}

fn is_independent_daily_strategy(strategy: &str) -> bool {
    Strategy::from_name(strategy)
        .is_some_and(|kind| !kind.is_live_insight() && !kind.is_dated_carry())
}

fn enabled_daily_strategy_count(settings: &Settings) -> usize {
    settings
        .strategies
        .iter()
        .filter(|strategy| !strategy.is_live_insight() && !strategy.is_dated_carry())
        .count()
        .max(1)
}

fn allocated_notional_for_candidate(settings: &Settings, strategy: &str, symbol: &str) -> f64 {
    candidate_used_notional(settings, strategy, symbol)
        + candidate_reserved_notional(settings, strategy, symbol)
}

fn candidate_used_notional(settings: &Settings, strategy: &str, symbol: &str) -> f64 {
    let position = settings
        .strategy_positions
        .get(symbol)
        .and_then(|lots| lots.get(strategy))
        .or_else(|| {
            settings.positions.get(symbol).filter(|position| {
                settings.strategy_positions.is_empty()
                    && position.strategy.as_deref() == Some(strategy)
            })
        });
    let open = position
        .filter(|position| !position.scout_entry)
        .map(|position| {
            position.quantity.abs()
                * position.average_entry_price.max(0.0)
                * quote_usd_rate(settings, symbol)
                * scheduled_stop_headroom_multiplier(&position.entry_order_type, Some(strategy))
        })
        .unwrap_or(0.0);
    open
}

fn candidate_reserved_notional(settings: &Settings, strategy: &str, symbol: &str) -> f64 {
    settings
        .orders
        .iter()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && order.symbol == symbol
                && order.purpose != "stop-loss"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > order.filled_quantity + 1e-12
        })
        .map(|order| {
            let remaining_fraction = ((order.quantity - order.filled_quantity).max(0.0)
                / order.quantity)
                .clamp(0.0, 1.0);
            let reference = if order.reserved_notional > 0.0 {
                order.reserved_notional
            } else {
                order.average_price.unwrap_or(0.0)
                    * order.quantity
                    * quote_usd_rate(settings, symbol)
            };
            reference
                * remaining_fraction
                * scheduled_stop_headroom_multiplier(&order.order_type, Some(strategy))
        })
        .sum::<f64>()
}

fn daily_strategy_open_position_count(settings: &Settings, strategy: &str) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| {
                position.quantity.abs() > 1e-12
                    && !position.scout_entry
                    && position.strategy.as_deref() == Some(strategy)
            })
            .count();
    }
    settings
        .strategy_positions
        .values()
        .filter(|lots| {
            lots.get(strategy)
                .is_some_and(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
        })
        .count()
}

fn pending_entry_strategy_keys(settings: &Settings) -> BTreeSet<(String, String)> {
    let now_ms = current_time_ms();
    settings
        .orders
        .iter()
        .filter(|order| {
            order.purpose == "signal"
                && order
                    .expires_at
                    .is_none_or(|expires_at| expires_at > now_ms)
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > order.filled_quantity + 1e-12
        })
        .filter_map(|order| {
            let strategy = order.strategy.as_deref()?;
            let existing = settings
                .strategy_positions
                .get(&order.symbol)
                .and_then(|lots| lots.get(strategy))
                .is_some_and(|position| position.quantity.abs() > 1e-12 && !position.scout_entry)
                || (settings.strategy_positions.is_empty()
                    && settings
                        .positions
                        .get(&order.symbol)
                        .is_some_and(|position| {
                            position.quantity.abs() > 1e-12
                                && !position.scout_entry
                                && position.strategy.as_deref() == Some(strategy)
                        }));
            (!existing).then(|| (order.symbol.clone(), strategy.to_owned()))
        })
        .collect()
}

fn pending_entry_position_count_for_category(
    settings: &Settings,
    category: StrategyCategory,
) -> usize {
    pending_entry_strategy_keys(settings)
        .iter()
        .filter(|(_, strategy)| strategy_category(Some(strategy)) == category)
        .count()
}

fn pending_entry_position_count_for_strategy(settings: &Settings, strategy: &str) -> usize {
    pending_entry_strategy_keys(settings)
        .iter()
        .filter(|(_, candidate)| candidate == &strategy)
        .count()
}

fn daily_strategy_position_limit(settings: &Settings) -> usize {
    settings
        .max_daily_open_positions
        .div_ceil(enabled_daily_strategy_count(settings))
        .max(1)
}

fn quote_usd_rate(settings: &Settings, symbol: &str) -> f64 {
    quote_usd_rate_for_map(&settings.usd_rates, symbol)
}

fn quote_usd_rate_for_map(rates: &BTreeMap<String, f64>, symbol: &str) -> f64 {
    symbol
        .split_once('-')
        .and_then(|(_, quote)| rates.get(quote))
        .copied()
        .filter(|rate| rate.is_finite() && *rate > 0.0)
        // Unknown quote currencies reserve conservatively until the USD-rate feed is available.
        .unwrap_or(10.0)
}

async fn refresh_usd_rates(state: &AutoTraderState) -> Result<(), (StatusCode, String)> {
    let response = crate::service_rpc::position_get("/positions/usd-rates")
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let Some(rates) = response.get("rates").and_then(serde_json::Value::as_object) else {
        return Err((
            StatusCode::BAD_GATEWAY,
            "USD rate response has no rates map".into(),
        ));
    };
    let mut parsed = default_usd_rates();
    for (currency, value) in rates {
        if let Some(rate) = value
            .as_f64()
            .filter(|rate| rate.is_finite() && *rate > 0.0)
        {
            parsed.insert(currency.clone(), rate);
        }
    }
    parsed.insert("USD".into(), 1.0);
    {
        let mut settings = state.settings.lock().await;
        settings.usd_rates = parsed;
        settings.used_notional_today = allocated_notional(&settings);
    }
    request_settings_persist(state)
}

fn reset_daily_budget(settings: &mut Settings) {
    let day = utc_day();
    if settings.utc_day != day {
        settings.utc_day = day.clone();
    }
    settings.used_notional_today = allocated_notional(settings);
    if settings.pnl_utc_day != day {
        settings.pnl_utc_day = day;
        settings.daily_realized_pnl = 0.0;
        settings.daily_loss_limit_reduced = false;
    }
}

#[cfg(test)]
mod daily_strategy_allocator_tests {
    use super::*;

    #[test]
    fn daily_position_slots_remain_split_across_selected_daily_strategies() {
        let mut settings = Settings::default();
        settings.max_daily_open_positions = 10;
        settings.strategies = vec![
            Strategy::Donchian55Btc200,
            Strategy::Trend50200,
            Strategy::LiveMomentum,
            Strategy::DatedCarry,
        ];
        assert_eq!(enabled_daily_strategy_count(&settings), 2);
        assert_eq!(daily_strategy_position_limit(&settings), 5);

        settings.strategies.push(Strategy::Rsi2OversoldRebound);
        assert_eq!(enabled_daily_strategy_count(&settings), 3);
        assert_eq!(daily_strategy_position_limit(&settings), 4);
    }
}

#[cfg(test)]
mod persistence_allocation_tests {
    use super::*;

    fn position(strategy: &str, quantity: f64, price: f64) -> TraderPosition {
        serde_json::from_value(serde_json::json!({
            "strategy": strategy,
            "quantity": quantity,
            "average_entry_price": price,
            "entry_fees": 0.0,
            "realized_pnl": 0.0,
            "entry_order_type": "Limit"
        }))
        .unwrap()
    }

    fn pending_order(symbol: &str, strategy: &str, side: &str, quantity: f64) -> TraderOrder {
        serde_json::from_value(serde_json::json!({
            "timestamp": 1,
            "symbol": symbol,
            "side": side,
            "order_type": "Limit",
            "purpose": "signal",
            "strategy": strategy,
            "quantity": quantity,
            "filled_quantity": 1.0,
            "reserved_notional": 100.0,
            "average_price": 10.0,
            "status": "accepted",
            "realized_pnl": 0.0
        }))
        .unwrap()
    }

    fn settings_with_rates() -> Settings {
        let mut settings = Settings::default();
        settings.usd_rates.insert("USDT".into(), 1.0);
        settings
    }

    #[test]
    fn allocated_symbol_exposure_counts_open_lots_and_only_incremental_pending_risk() {
        let mut settings = settings_with_rates();
        settings
            .positions
            .insert("ETH-USDT".into(), position("live-momentum", 2.0, 10.0));
        settings
            .orders
            .push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 100.0);

        settings.orders[0].side = "sell".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 60.0);
        settings.orders[0].purpose = "stop-loss".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 20.0);
    }

    #[test]
    fn category_allocation_and_open_counts_support_strategy_lots_and_legacy_positions() {
        let mut settings = settings_with_rates();
        settings
            .strategy_positions
            .entry("ETH-USDT".into())
            .or_default()
            .insert(
                "donchian-20-btc200".into(),
                position("donchian-20-btc200", 2.0, 10.0),
            );
        settings
            .strategy_positions
            .entry("BTC-USDT".into())
            .or_default()
            .insert(
                "live-momentum".into(),
                position("live-momentum", -1.0, 50.0),
            );
        assert_eq!(
            allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)),
            20.0
        );
        assert_eq!(
            allocated_notional_for_category(&settings, Some(StrategyCategory::Intraday)),
            50.0
        );
        assert_eq!(allocated_notional(&settings), 70.0);
        assert_eq!(total_open_position_count(&settings), 2);
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Daily),
            1
        );
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Intraday),
            1
        );

        settings.strategy_positions.clear();
        settings
            .positions
            .insert("ETH-USDT".into(), position("donchian-20-btc200", 2.0, 10.0));
        settings
            .positions
            .insert("BTC-USDT".into(), position("live-momentum", -1.0, 50.0));
        assert_eq!(
            allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)),
            20.0
        );
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Intraday),
            1
        );
    }

    #[test]
    fn scout_positions_do_not_consume_real_open_position_caps() {
        let mut settings = settings_with_rates();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings
            .strategy_positions
            .entry("ETH-USDT".into())
            .or_default()
            .insert("live-momentum".into(), scout.clone());
        settings
            .strategy_positions
            .entry("BTC-USDT".into())
            .or_default()
            .insert(
                "live-trade-flow".into(),
                position("live-trade-flow", 1.0, 50.0),
            );
        settings
            .strategy_positions
            .entry("SOL-USDT".into())
            .or_default()
            .insert("trend-50-200".into(), scout);

        assert_eq!(total_open_position_count(&settings), 1);
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Intraday),
            1
        );
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Daily),
            0
        );

        settings.strategy_positions.clear();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings.positions.insert("ETH-USDT".into(), scout);
        assert_eq!(total_open_position_count(&settings), 0);
        assert_eq!(
            category_open_position_count(&settings, StrategyCategory::Intraday),
            0
        );
    }

    #[test]
    fn candidate_exposure_and_position_slots_are_independent_of_legacy_budgets() {
        let mut settings = settings_with_rates();
        settings.strategies = vec![
            Strategy::Donchian20Btc200,
            Strategy::Donchian55Btc200,
            Strategy::LiveMomentum,
            Strategy::LiveTradeFlow,
        ];
        settings.daily_strategy_notional_limit = 0.0;
        settings
            .daily_strategy_budgets
            .insert("donchian-20-btc200".into(), 0.0);
        settings
            .daily_strategy_budgets
            .insert("donchian-55-btc200".into(), 9_000.0);
        settings.intraday_notional_limit = 0.0;
        settings
            .intraday_strategy_budgets
            .insert("live-momentum".into(), 0.0);

        settings
            .strategy_positions
            .entry("ETH-USDT".into())
            .or_default()
            .insert("live-momentum".into(), position("live-momentum", 2.0, 10.0));
        settings
            .orders
            .push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(
            allocated_notional_for_candidate(&settings, "live-momentum", "ETH-USDT"),
            100.0
        );
        assert_eq!(
            daily_strategy_open_position_count(&settings, "live-momentum"),
            1
        );
        assert_eq!(
            daily_strategy_open_position_count(&settings, "donchian-20-btc200"),
            0
        );
    }

    #[test]
    fn pending_entries_are_unique_and_existing_strategy_positions_are_excluded() {
        let mut settings = settings_with_rates();
        settings.orders = vec![
            pending_order("ETH-USDT", "live-momentum", "buy", 5.0),
            pending_order("ETH-USDT", "live-momentum", "buy", 3.0),
            pending_order("BTC-USDT", "donchian-20-btc200", "buy", 2.0),
        ];
        let keys = pending_entry_strategy_keys(&settings);
        assert_eq!(keys.len(), 2);
        assert_eq!(
            pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday),
            1
        );
        assert_eq!(
            pending_entry_position_count_for_category(&settings, StrategyCategory::Daily),
            1
        );
        assert_eq!(
            pending_entry_position_count_for_strategy(&settings, "live-momentum"),
            1
        );
        settings
            .orders
            .push(pending_order("SOL-USDT", "live-order-book", "buy", 2.0));
        settings.orders.last_mut().unwrap().purpose = "scout".into();
        assert_eq!(
            pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday),
            1
        );

        settings
            .strategy_positions
            .entry("ETH-USDT".into())
            .or_default()
            .insert("live-momentum".into(), position("live-momentum", 1.0, 10.0));
        assert_eq!(
            pending_entry_position_count_for_strategy(&settings, "live-momentum"),
            0
        );
    }

    #[test]
    fn position_slot_limits_and_quote_conversion_use_safe_fallbacks() {
        let mut settings = settings_with_rates();
        settings.max_daily_open_positions = 7;
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        assert_eq!(
            category_position_limit(&settings, StrategyCategory::Daily),
            7
        );
        assert_eq!(daily_strategy_position_limit(&settings), 4);
        assert_eq!(quote_usd_rate(&settings, "ETH-USDT"), 1.0);
        assert_eq!(quote_usd_rate_for_map(&BTreeMap::new(), "ETH-XYZ"), 10.0);
        assert_eq!(
            quote_usd_rate_for_map(&BTreeMap::from([("XYZ".into(), f64::NAN)]), "ETH-XYZ"),
            10.0
        );
        assert_eq!(
            scheduled_stop_headroom_multiplier("Vwap", Some("live-momentum")),
            2.0
        );
        assert_eq!(
            scheduled_stop_headroom_multiplier("Limit", Some("live-momentum")),
            1.0
        );
        assert!(!is_independent_daily_strategy("live-momentum"));
        assert!(is_independent_daily_strategy("donchian-20-btc200"));
    }
}

#[cfg(test)]
mod trade_price_path_persistence_tests {
    use super::*;

    fn temporary_database_path() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("auto-trader-price-path-{nonce}.sqlite"))
    }

    fn sample(sequence: u64, timestamp: u64, epoch: &str) -> TradePriceSample {
        TradePriceSample {
            timestamp,
            bid: 99.0 + sequence as f64 / 10_000.0,
            ask: 100.0 + sequence as f64 / 10_000.0,
            sequence,
            epoch: Some(epoch.to_owned()),
            source_updated_at_ms: Some(timestamp),
            freshness_contract_version: Some(1),
            freshness_max_age_ms: Some(1_000),
            source_fresh: Some(true),
            quote_source_system: Some("paper_exchange".into()),
            quote_source_epoch: Some("pex-epoch".into()),
            quote_source_cursor: Some(sequence),
            quote_source_record_id: Some(format!("quote-{sequence}")),
        }
    }

    fn position_with_samples(samples: Vec<TradePriceSample>, finalized: bool) -> Settings {
        let mut settings = Settings::default();
        let mut position = TraderPosition::default();
        position.trade_id = Some("trade-path-test".into());
        position.price_path_epoch = Some("path-epoch-1".into());
        position.entry_timestamp_ms = 1_000;
        position.price_path_total_samples = samples.last().map_or(0, |sample| sample.sequence);
        position.price_path_finalized = finalized;
        position.price_path = samples;
        settings
            .strategy_positions
            .entry("BTC-USDT".into())
            .or_default()
            .insert("live-momentum".into(), position);
        settings
    }

    fn append_snapshot(connection: &rusqlite::Connection, settings: &Settings) {
        let transaction = connection.unchecked_transaction().unwrap();
        persist_trade_price_path_samples(&transaction, settings).unwrap();
        transaction.commit().unwrap();
    }

    #[test]
    fn path_appends_are_idempotent_and_paged_chronologically() {
        let path = temporary_database_path();
        let connection = rusqlite::Connection::open(&path).unwrap();
        let settings = position_with_samples(
            (1..=3)
                .map(|sequence| sample(sequence, 1_000 + sequence * 1_000, "path-epoch-1"))
                .collect(),
            true,
        );
        append_snapshot(&connection, &settings);
        append_snapshot(&connection, &settings);
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM trade_price_path_samples WHERE trade_id='trade-path-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 3,
            "repeated Settings snapshots must not duplicate samples"
        );
        drop(connection);

        let first = read_trade_price_path_page(&path, "trade-path-test", None, None, 2).unwrap();
        assert_eq!(first["coverage_status"], "complete");
        assert_eq!(first["has_more"], true);
        assert_eq!(first["samples"][0]["timestamp_ms"], 2_000);
        assert_eq!(first["samples"][1]["timestamp_ms"], 3_000);
        let second = read_trade_price_path_page(
            &path,
            "trade-path-test",
            first["next_timestamp_ms"].as_u64(),
            first["next_cursor"].as_i64(),
            2,
        )
        .unwrap();
        assert_eq!(second["samples"].as_array().unwrap().len(), 1);
        assert_eq!(second["samples"][0]["timestamp_ms"], 4_000);
        assert_eq!(second["has_more"], false);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn close_before_checkpoint_is_recovered_from_pending_post_exit_path_and_restart_fails_closed() {
        let path = temporary_database_path();
        let connection = rusqlite::Connection::open(&path).unwrap();
        let path_samples = vec![
            sample(1, 2_000, "path-epoch-1"),
            sample(2, 3_000, "path-epoch-1"),
        ];
        let mut settings = Settings::default();
        settings.post_exit_observations.push(serde_json::json!({
            "event_id":"exit-timing:trade-path-test",
            "trade_id":"trade-path-test",
            "symbol":"BTC-USDT",
            "strategy":"live-momentum",
            "entry_context":{"timestamp_ms":1_000},
            "pre_exit_price_path":path_samples,
            "price_path":[],
            "pre_exit_price_path_evidence":{
                "path_epochs":["path-epoch-1"],"total_sample_count":2,"missing_coverage":[]
            },
            "completion_pending_durable":false
        }));
        append_snapshot(&connection, &settings);
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM trade_price_path_samples WHERE trade_id='trade-path-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 2,
            "closed path survives removal from active positions"
        );
        let incomplete =
            read_trade_price_path_page(&path, "trade-path-test", None, None, 10).unwrap();
        assert_eq!(
            incomplete["history_complete"], false,
            "pending post-exit observation is not finalized"
        );

        settings.post_exit_observations[0]["completion_pending_durable"] =
            serde_json::Value::Bool(true);
        append_snapshot(&connection, &settings);
        let complete =
            read_trade_price_path_page(&path, "trade-path-test", None, None, 10).unwrap();
        assert_eq!(
            complete["history_complete"], true,
            "completion follows the durable final batch"
        );
        settings.post_exit_observations[0]["path_restart_interrupted"] =
            serde_json::Value::Bool(true);
        append_snapshot(&connection, &settings);
        let restarted =
            read_trade_price_path_page(&path, "trade-path-test", None, None, 10).unwrap();
        assert_eq!(
            restarted["history_complete"], false,
            "restart gaps cannot remain complete"
        );
        assert!(
            restarted["missing_reasons"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "process_restart_interrupted")
        );
        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn durable_rows_exceed_memory_window_without_pruning() {
        let path = temporary_database_path();
        let connection = rusqlite::Connection::open(&path).unwrap();
        let settings = position_with_samples(
            (1..=4_100)
                .map(|sequence| sample(sequence, 1_000 + sequence * 1_000, "path-epoch-1"))
                .collect(),
            false,
        );
        append_snapshot(&connection, &settings);
        let mut restored = Settings::default();
        restored.strategy_positions = settings.strategy_positions.clone();
        hydrate_persisted_trade_paths(&connection, &mut restored).unwrap();
        let position = &restored.strategy_positions["BTC-USDT"]["live-momentum"];
        assert_eq!(position.price_path.len(), 4_096);
        assert!(position.price_path_memory_window_truncated);
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM trade_price_path_samples WHERE trade_id='trade-path-test'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            count, 4_100,
            "memory hydration must not cap durable history"
        );
        drop(connection);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
