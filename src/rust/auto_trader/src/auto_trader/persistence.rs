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
            marked_net_pnl_usdt REAL, mark_checked_at_ms INTEGER
         );
         CREATE TABLE IF NOT EXISTS dated_carry_scout_probes (
            probe_id INTEGER PRIMARY KEY, opened_at_ms INTEGER NOT NULL, parameter_key TEXT NOT NULL,
            parameter_value REAL NOT NULL, would_enter INTEGER NOT NULL,
            spot_symbol TEXT NOT NULL, future_symbol TEXT NOT NULL, expiry_ms INTEGER NOT NULL,
            days_to_expiry REAL NOT NULL, quantity REAL NOT NULL, spot_ask REAL NOT NULL,
            future_bid REAL NOT NULL, spot_bid REAL NOT NULL, future_ask REAL NOT NULL,
            basis_percent REAL NOT NULL, net_after_budget_percent REAL NOT NULL,
            eligible INTEGER NOT NULL, reason TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS dated_carry_scout_probe_parameters (
            probe_id INTEGER NOT NULL REFERENCES dated_carry_scout_probes(probe_id) ON DELETE CASCADE,
            map_name TEXT NOT NULL CHECK (map_name IN ('parameters','tested_parameters')),
            parameter_key TEXT NOT NULL, parameter_value REAL NOT NULL,
            PRIMARY KEY(probe_id,map_name,parameter_key)
         );
         CREATE TABLE IF NOT EXISTS dated_carry_scout_cooldowns (
            cooldown_key TEXT PRIMARY KEY, started_day INTEGER NOT NULL
         );",
    )
}

fn replace_dated_carry_rows(transaction: &rusqlite::Transaction<'_>, book: &DatedCarryBook) -> rusqlite::Result<()> {
    transaction.execute(
        "INSERT INTO dated_carry_settings(id, enabled, max_spot_notional_usdt, checked_at_ms, last_error)
         VALUES (1, ?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET enabled=excluded.enabled,
         max_spot_notional_usdt=excluded.max_spot_notional_usdt, checked_at_ms=excluded.checked_at_ms,
         last_error=excluded.last_error",
        rusqlite::params![book.enabled as i64, book.max_spot_notional_usdt, book.checked_at_ms as i64, book.last_error],
    )?;
    for table in ["dated_carry_scout_probe_parameters", "dated_carry_candidates", "dated_carry_positions", "dated_carry_scout_probes", "dated_carry_scout_cooldowns"] {
        transaction.execute(&format!("DELETE FROM {table}"), [])?;
    }
    for item in &book.candidates {
        transaction.execute("INSERT INTO dated_carry_candidates VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            rusqlite::params![item.future_symbol,item.spot_symbol,item.expiry_ms as i64,item.days_to_expiry,item.quantity,item.spot_ask,item.future_bid,item.spot_bid,item.future_ask,item.basis_percent,item.net_after_budget_percent,item.eligible as i64,item.reason])?;
    }
    for item in &book.positions {
        transaction.execute("INSERT INTO dated_carry_positions(future_symbol,spot_symbol,expiry_ms,entry_ms,quantity,spot_entry_price,future_entry_price,entry_fees,status,scout_parameter_key,scout_parameter_value,spot_exit_twap,settlement_price,net_pnl_usdt,settlement_source,spot_mark_bid,future_mark_ask,spot_leg_pnl_usdt,future_leg_pnl_usdt,estimated_exit_fees_usdt,estimated_exit_slippage_usdt,marked_net_pnl_usdt,mark_checked_at_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23)",
            rusqlite::params![item.future_symbol,item.spot_symbol,item.expiry_ms as i64,item.entry_ms as i64,item.quantity,item.spot_entry_price,item.future_entry_price,item.entry_fees,item.status,item.scout_parameter_key,item.scout_parameter_value,item.spot_exit_twap,item.settlement_price,item.net_pnl_usdt,item.settlement_source,item.spot_mark_bid,item.future_mark_ask,item.spot_leg_pnl_usdt,item.future_leg_pnl_usdt,item.estimated_exit_fees_usdt,item.estimated_exit_slippage_usdt,item.marked_net_pnl_usdt,item.mark_checked_at_ms.map(|v|v as i64)])?;
    }
    for (i, item) in book.scout_probes.iter().enumerate() {
        let c = &item.candidate;
        transaction.execute("INSERT INTO dated_carry_scout_probes VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18)",
            rusqlite::params![i as i64,item.opened_at_ms as i64,item.parameter_key,item.parameter_value,item.would_enter as i64,c.spot_symbol,c.future_symbol,c.expiry_ms as i64,c.days_to_expiry,c.quantity,c.spot_ask,c.future_bid,c.spot_bid,c.future_ask,c.basis_percent,c.net_after_budget_percent,c.eligible as i64,c.reason])?;
        for (map_name, map) in [("parameters", &item.parameters), ("tested_parameters", &item.tested_parameters)] {
            for (key,value) in map { transaction.execute("INSERT INTO dated_carry_scout_probe_parameters VALUES (?1,?2,?3,?4)",rusqlite::params![i as i64,map_name,key,value])?; }
        }
    }
    for (key, day) in &book.scout_last_started {
        transaction.execute("INSERT INTO dated_carry_scout_cooldowns(cooldown_key,started_day) VALUES (?1,?2)", rusqlite::params![key, *day as i64])?;
    }
    Ok(())
}

fn read_dated_carry_database(path: &PathBuf) -> rusqlite::Result<Option<DatedCarryBook>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open(path)?;
    let has_old_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='dated_carry_state')",
        [], |row| row.get(0),
    )?;
    let old_payload: Option<Vec<u8>> = if has_old_table {
        connection.query_row("SELECT payload FROM dated_carry_state WHERE id=1", [], |row| row.get(0)).optional()?
    } else { None };
    ensure_dated_carry_schema(&connection)?;
    if let Some(payload) = old_payload {
        let book: DatedCarryBook = rmp_serde::from_slice(&payload).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(e)))?;
        let transaction = connection.unchecked_transaction()?;
        replace_dated_carry_rows(&transaction, &book)?;
        transaction.execute_batch("DROP TABLE dated_carry_state")?;
        transaction.commit()?;
    }
    let transaction = connection.unchecked_transaction()?;
    let settings: Option<(i64, f64, i64, Option<String>)> = transaction.query_row("SELECT enabled,max_spot_notional_usdt,checked_at_ms,last_error FROM dated_carry_settings WHERE id=1", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((enabled, max_spot_notional_usdt, checked_at_ms, last_error)) = settings else { return Ok(None); };
    let mut book = DatedCarryBook { enabled: enabled != 0, max_spot_notional_usdt, checked_at_ms: checked_at_ms as u64, last_error, ..DatedCarryBook::default() };
    book.candidates = {
        let mut stmt = transaction.prepare("SELECT spot_symbol,future_symbol,expiry_ms,days_to_expiry,quantity,spot_ask,future_bid,spot_bid,future_ask,basis_percent,net_after_budget_percent,eligible,reason FROM dated_carry_candidates ORDER BY future_symbol")?;
        stmt.query_map([], |r| Ok(DatedCarryCandidate {spot_symbol:r.get(0)?,future_symbol:r.get(1)?,expiry_ms:r.get::<_,i64>(2)? as u64,days_to_expiry:r.get(3)?,quantity:r.get(4)?,spot_ask:r.get(5)?,future_bid:r.get(6)?,spot_bid:r.get(7)?,future_ask:r.get(8)?,basis_percent:r.get(9)?,net_after_budget_percent:r.get(10)?,eligible:r.get::<_,i64>(11)?!=0,reason:r.get(12)?}))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    book.positions = {
        let mut stmt = transaction.prepare("SELECT future_symbol,spot_symbol,expiry_ms,entry_ms,quantity,spot_entry_price,future_entry_price,entry_fees,status,scout_parameter_key,scout_parameter_value,spot_exit_twap,settlement_price,net_pnl_usdt,settlement_source,spot_mark_bid,future_mark_ask,spot_leg_pnl_usdt,future_leg_pnl_usdt,estimated_exit_fees_usdt,estimated_exit_slippage_usdt,marked_net_pnl_usdt,mark_checked_at_ms FROM dated_carry_positions ORDER BY id")?;
        stmt.query_map([], |r| Ok(DatedCarryPosition {future_symbol:r.get(0)?,spot_symbol:r.get(1)?,expiry_ms:r.get::<_,i64>(2)? as u64,entry_ms:r.get::<_,i64>(3)? as u64,quantity:r.get(4)?,spot_entry_price:r.get(5)?,future_entry_price:r.get(6)?,entry_fees:r.get(7)?,status:r.get(8)?,scout_parameter_key:r.get(9)?,scout_parameter_value:r.get(10)?,spot_exit_twap:r.get(11)?,settlement_price:r.get(12)?,net_pnl_usdt:r.get(13)?,settlement_source:r.get(14)?,spot_mark_bid:r.get(15)?,future_mark_ask:r.get(16)?,spot_leg_pnl_usdt:r.get(17)?,future_leg_pnl_usdt:r.get(18)?,estimated_exit_fees_usdt:r.get(19)?,estimated_exit_slippage_usdt:r.get(20)?,marked_net_pnl_usdt:r.get(21)?,mark_checked_at_ms:r.get::<_,Option<i64>>(22)?.map(|v|v as u64)}))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    let probe_rows: Vec<(i64,u64,String,f64,bool,DatedCarryCandidate)> = {
        let mut stmt=transaction.prepare("SELECT probe_id,opened_at_ms,parameter_key,parameter_value,would_enter,spot_symbol,future_symbol,expiry_ms,days_to_expiry,quantity,spot_ask,future_bid,spot_bid,future_ask,basis_percent,net_after_budget_percent,eligible,reason FROM dated_carry_scout_probes ORDER BY probe_id")?;
        stmt.query_map([],|r| Ok((r.get(0)?,r.get::<_,i64>(1)? as u64,r.get(2)?,r.get(3)?,r.get::<_,i64>(4)?!=0,DatedCarryCandidate{spot_symbol:r.get(5)?,future_symbol:r.get(6)?,expiry_ms:r.get::<_,i64>(7)? as u64,days_to_expiry:r.get(8)?,quantity:r.get(9)?,spot_ask:r.get(10)?,future_bid:r.get(11)?,spot_bid:r.get(12)?,future_ask:r.get(13)?,basis_percent:r.get(14)?,net_after_budget_percent:r.get(15)?,eligible:r.get::<_,i64>(16)?!=0,reason:r.get(17)?})))?.collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (id,opened_at_ms,parameter_key,parameter_value,would_enter,candidate) in probe_rows {
        let mut parameters=BTreeMap::new(); let mut tested_parameters=BTreeMap::new();
        let mut stmt=transaction.prepare("SELECT map_name,parameter_key,parameter_value FROM dated_carry_scout_probe_parameters WHERE probe_id=?1")?;
        for row in stmt.query_map([id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,f64>(2)?)))? { let (map,key,value)=row?; if map=="parameters" {parameters.insert(key,value);} else {tested_parameters.insert(key,value);} }
        book.scout_probes.push(DatedCarryScoutScout{candidate,opened_at_ms,parameter_key,parameter_value,parameters,tested_parameters,would_enter});
    }
    book.scout_last_started = {
        let mut stmt = transaction.prepare("SELECT cooldown_key,started_day FROM dated_carry_scout_cooldowns")?;
        stmt.query_map([], |r| Ok((r.get(0)?, r.get::<_,i64>(1)? as u64)))?.collect::<rusqlite::Result<BTreeMap<_,_>>>()?
    };
    transaction.commit()?;
    Ok(Some(book))
}

fn persist_dated_carry_book(path: &PathBuf, book: &DatedCarryBook) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let connection = rusqlite::Connection::open(path).map_err(|error| error.to_string())?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(|error| error.to_string())?;
    connection.execute_batch("PRAGMA journal_mode = WAL;").map_err(|e| e.to_string())?;
    ensure_dated_carry_schema(&connection).map_err(|e| e.to_string())?;
    let transaction = connection.unchecked_transaction().map_err(|e| e.to_string())?;
    replace_dated_carry_rows(&transaction, book).map_err(|e| e.to_string())?;
    transaction.commit().map_err(|e| e.to_string())
}

fn read_settings_database(path: &PathBuf) -> rusqlite::Result<Option<Settings>> {
    if !path.exists() {
        return Ok(None);
    }
    let connection = rusqlite::Connection::open(path)?;
    let has_old_table: bool = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='auto_trader_state')",
        [], |row| row.get(0),
    )?;
    let old_payload: Option<Vec<u8>> = if has_old_table {
        connection.query_row("SELECT payload FROM auto_trader_state WHERE id=1", [], |row| row.get(0)).optional()?
    } else { None };
    let transaction = connection.unchecked_transaction()?;
    let mut fields = serde_json::Map::new();
    let mut statement = transaction.prepare("SELECT name FROM sqlite_master WHERE type='table' AND name LIKE 'setting_%' ORDER BY name")?;
    let tables = statement.query_map([], |row| row.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
    drop(statement);
    for table in tables {
        let field = table.strip_prefix("setting_").unwrap_or(&table);
        let sql = format!("SELECT value FROM {table} WHERE id=1");
        let value: Option<String> = transaction.query_row(&sql, [], |row| row.get(0)).optional()?;
        if let Some(value) = value {
            fields.insert(field.to_owned(), serde_json::from_str(&value).map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error)))?);
        }
    }
    let has_normalized = !fields.is_empty();
    transaction.commit()?;
    if has_normalized {
        return serde_json::from_value(serde_json::Value::Object(fields)).map(Some).map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(error)));
    }
    if let Some(payload) = old_payload {
        let settings: Settings = rmp_serde::from_slice(&payload).map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Blob, Box::new(error)))?;
        persist_settings_snapshot(path, &settings).map_err(|(_, error)| {
            rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error)))
        })?;
        return Ok(Some(settings));
    }
    Ok(None)
}

fn persist_settings_snapshot(
    path: &PathBuf,
    settings: &Settings,
) -> Result<(), (StatusCode, String)> {
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(settings_persistence_error)?;
    }
    let connection = rusqlite::Connection::open(path).map_err(settings_persistence_error)?;
    connection.busy_timeout(Duration::from_secs(5)).map_err(settings_persistence_error)?;
    connection.execute_batch("PRAGMA journal_mode=WAL;").map_err(settings_persistence_error)?;
    let value = serde_json::to_value(settings).map_err(settings_persistence_error)?;
    let object = value.as_object().ok_or_else(|| settings_persistence_error("Settings did not serialize as an object"))?;
    let transaction = connection.unchecked_transaction().map_err(settings_persistence_error)?;
    for (field, value) in object {
        let table = format!("setting_{field}");
        transaction.execute_batch(&format!("CREATE TABLE IF NOT EXISTS {table} (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);")).map_err(settings_persistence_error)?;
        transaction.execute(&format!("INSERT INTO {table}(id,value) VALUES(1,?1) ON CONFLICT(id) DO UPDATE SET value=excluded.value"), [serde_json::to_string(value).map_err(settings_persistence_error)?]).map_err(settings_persistence_error)?;
    }
    transaction.execute_batch("DROP TABLE IF EXISTS auto_trader_state;").map_err(settings_persistence_error)?;
    transaction.commit().map_err(settings_persistence_error)
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
        assert_eq!(serde_json::to_value(read_settings_database(&path).unwrap().unwrap()).unwrap(), serde_json::to_value(settings).unwrap());
        let connection = rusqlite::Connection::open(&path).unwrap();
        let old_table: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='auto_trader_state'", [], |row| row.get(0)).unwrap();
        assert_eq!(old_table, 0);
        let tables: i64 = connection.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name LIKE 'setting_%'", [], |row| row.get(0)).unwrap();
        assert!(tables > 50);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn migrates_legacy_settings_blob_on_read() {
        let path = temporary_database_path();
        let settings = Settings::default();
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE auto_trader_state (id INTEGER PRIMARY KEY, payload BLOB NOT NULL);").unwrap();
        connection.execute("INSERT INTO auto_trader_state VALUES(1,?1)", [rmp_serde::to_vec_named(&settings).unwrap()]).unwrap();
        drop(connection);

        let loaded = read_settings_database(&path).unwrap().unwrap();
        assert_eq!(serde_json::to_value(loaded).unwrap(), serde_json::to_value(settings).unwrap());
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
                scout_parameter_key: Some("carry_cost_gate_percent".into()),
                scout_parameter_value: Some(0.5),
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
            }],
            checked_at_ms: 1_700_000_100_000,
            last_error: Some("recoverable scan warning".into()),
            scout_probes: vec![DatedCarryScoutScout {
                candidate: candidate.clone(),
                opened_at_ms: 1_700_000_000_000,
                parameter_key: "carry_cost_gate_percent".into(),
                parameter_value: 0.5,
                parameters: BTreeMap::from([("minimum_days".into(), 5.0)]),
                tested_parameters: BTreeMap::from([("carry_cost_gate_percent".into(), 0.5)]),
                would_enter: true,
            }],
            scout_last_started: BTreeMap::from([("BTCUSDT:gate".into(), 20_000)]),
        };

        persist_dated_carry_book(&path, &book).unwrap();
        let loaded = read_dated_carry_database(&path).unwrap().unwrap();
        assert_eq!(serde_json::to_value(&loaded).unwrap(), serde_json::to_value(&book).unwrap());

        let connection = rusqlite::Connection::open(&path).unwrap();
        let candidate_columns: i64 = connection
            .query_row("SELECT COUNT(*) FROM pragma_table_info('dated_carry_candidates')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(candidate_columns, 13);
        assert_eq!(
            connection.query_row("SELECT COUNT(*) FROM dated_carry_scout_probe_parameters", [], |row| row.get::<_, i64>(0)).unwrap(),
            2,
        );

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
    state.settings_persist_notify.notify_one();
    let _ = state.state_updates.send(());
    Ok(())
}

fn start_settings_persistence_worker(state: &AutoTraderState) {
    let settings = Arc::clone(&state.settings);
    let path = state.settings_path.clone();
    let notify = Arc::clone(&state.settings_persist_notify);
    let timings = Arc::clone(&state.settings_persist_timings_ms);
    tokio::spawn(async move {
        loop {
            notify.notified().await;
            tokio::time::sleep(Duration::from_millis(10)).await;
            let lock_wait_started = Instant::now();
            let settings_guard = Arc::clone(&settings).lock_owned().await;
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
            let path = path.clone();
            let write_started = Instant::now();
            match tokio::task::spawn_blocking(move || persist_settings_snapshot(&path, &settings_snapshot)).await {
                Ok(Ok(())) => record_settings_persist_sample(
                    &timings,
                    "snapshot_write",
                    write_started.elapsed().as_secs_f64() * 1_000.0,
                ),
                Ok(Err((_, error))) => {
                    crate::service_log!(
                        "ERROR",
                        "Auto Trader state snapshot write failed: {error}"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    notify.notify_one();
                }
                Err(error) => {
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
    if scheduled && Strategy::from_name(strategy.unwrap_or_default())
        .is_some_and(Strategy::uses_protective_stops)
    {
        2.0
    } else {
        1.0
    }
}

fn allocated_notional(settings: &Settings) -> f64 {
    allocated_notional_for_category(settings, None)
}

fn allocated_notional_for_symbol(settings: &Settings, symbol: &str) -> f64 {
    let quote_rate = quote_usd_rate(settings, symbol);
    let open_exposure = if let Some(lots) = settings.strategy_positions.get(symbol) {
        lots.values()
            .filter(|position| !position.scout_entry)
            .map(|position| position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate)
            .sum()
    } else {
        settings.positions.get(symbol).filter(|position| !position.scout_entry).map_or(0.0, |position| {
            position.quantity.abs() * position.average_entry_price.max(0.0) * quote_rate
        })
    };
    let mut pending_exposure = 0.0;
    let mut pending_quantity = 0.0;
    let current_quantity = settings.positions.get(symbol).filter(|position| !position.scout_entry).map_or(0.0, |position| position.quantity);
    for order in settings.orders.iter().filter(|order| {
        order.symbol == symbol
            && order.purpose == "signal"
            && matches!(order.status.as_str(), "submitting" | "accepted" | "resting" | "partially_filled")
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
                lots_total += value * scheduled_stop_headroom_multiplier(
                    &position.entry_order_type,
                    Some(strategy.as_str()),
                );
            }
        }
        if settings.strategy_positions.is_empty() {
            settings
                .positions
                .iter()
                .filter(|(_, position)| !position.scout_entry && strategy_category(position.strategy.as_deref()) == category)
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
        settings.strategy_positions.iter().flat_map(|(symbol, lots)| {
            lots.iter().filter(|(_, position)| !position.scout_entry).map(move |(strategy, position)| {
                let value = position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol);
                value * scheduled_stop_headroom_multiplier(
                    &position.entry_order_type,
                    Some(strategy.as_str()),
                )
            })
        }).sum()
    } else {
        settings.positions.iter().filter(|(_, position)| !position.scout_entry).map(|(symbol, position)| {
            let value = position.quantity.abs()
                * position.average_entry_price.max(0.0)
                * quote_usd_rate(settings, symbol);
            value * scheduled_stop_headroom_multiplier(
                &position.entry_order_type,
                position.strategy.as_deref(),
            )
        }).sum()
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
                            !position.scout_entry && strategy_category(Some(strategy.as_str())) == category
                        })
                        .map(|(_, position)| position.quantity)
                        .sum::<f64>()
                })
                .unwrap_or_else(|| {
                    settings
                        .positions
                        .get(&order.symbol)
                        .filter(|position| {
                            !position.scout_entry && strategy_category(position.strategy.as_deref()) == category
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

fn category_notional_limit(settings: &Settings, category: StrategyCategory) -> f64 {
    match category {
        StrategyCategory::Intraday => settings.intraday_notional_limit,
        StrategyCategory::Daily => settings.daily_strategy_notional_limit,
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

fn daily_strategy_notional_share(settings: &Settings) -> f64 {
    settings.daily_strategy_notional_limit / enabled_daily_strategy_count(settings) as f64
}

fn daily_strategy_budget(settings: &Settings, strategy: &str) -> f64 {
    if settings.daily_strategy_budgets.is_empty() {
        return daily_strategy_notional_share(settings);
    }
    if let Some(budget) = settings.daily_strategy_budgets.get(strategy) {
        return *budget;
    }
    let enabled = settings
        .strategies
        .iter()
        .filter(|candidate| is_independent_daily_strategy(candidate.as_str()))
        .collect::<Vec<_>>();
    let missing = enabled
        .iter()
        .filter(|candidate| !settings.daily_strategy_budgets.contains_key(candidate.as_str()))
        .count();
    if missing == 0 || !enabled.iter().any(|candidate| candidate.as_str() == strategy) {
        return 0.0;
    }
    let configured = enabled
        .iter()
        .filter_map(|candidate| settings.daily_strategy_budgets.get(candidate.as_str()))
        .sum::<f64>();
    (settings.daily_strategy_notional_limit - configured).max(0.0) / missing as f64
}

fn enabled_intraday_strategy_count(settings: &Settings) -> usize {
    settings
        .strategies
        .iter()
        .filter(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
        .count()
        .max(1)
}

fn intraday_strategy_notional_share(settings: &Settings) -> f64 {
    settings.intraday_notional_limit / enabled_intraday_strategy_count(settings) as f64
}

fn intraday_strategy_budget(settings: &Settings, strategy: &str) -> f64 {
    if settings.intraday_strategy_budgets.is_empty() {
        return intraday_strategy_notional_share(settings);
    }
    if let Some(budget) = settings.intraday_strategy_budgets.get(strategy) {
        return *budget;
    }
    let enabled = settings
        .strategies
        .iter()
        .filter(|candidate| candidate.is_live_insight() && !candidate.is_legacy_composite())
        .collect::<Vec<_>>();
    let missing = enabled
        .iter()
        .filter(|candidate| !settings.intraday_strategy_budgets.contains_key(candidate.as_str()))
        .count();
    if missing == 0 || !enabled.iter().any(|candidate| candidate.as_str() == strategy) {
        return 0.0;
    }
    let configured = enabled
        .iter()
        .filter_map(|candidate| settings.intraday_strategy_budgets.get(candidate.as_str()))
        .sum::<f64>();
    (settings.intraday_notional_limit - configured).max(0.0) / missing as f64
}

fn allocated_notional_for_strategy(settings: &Settings, strategy: &str) -> f64 {
    let open_positions = if settings.strategy_positions.is_empty() {
        settings
            .positions
            .iter()
            .filter(|(_, position)| position.strategy.as_deref() == Some(strategy))
            .map(|(symbol, position)| {
                position.quantity.abs()
                    * position.average_entry_price.max(0.0)
                    * quote_usd_rate(settings, symbol)
                    * scheduled_stop_headroom_multiplier(
                        &position.entry_order_type,
                        position.strategy.as_deref(),
                    )
            })
            .sum::<f64>()
    } else {
        settings
            .strategy_positions
            .iter()
            .filter_map(|(symbol, lots)| {
                lots.get(strategy).map(|position| {
                    position.quantity.abs()
                        * position.average_entry_price.max(0.0)
                        * quote_usd_rate(settings, symbol)
                        * scheduled_stop_headroom_multiplier(
                            &position.entry_order_type,
                            Some(strategy),
                        )
                })
            })
            .sum::<f64>()
    };
    let pending_orders = settings
        .orders
        .iter()
        .filter(|order| {
            order.strategy.as_deref() == Some(strategy)
                && order.purpose != "stop-loss"
                && matches!(
                    order.status.as_str(),
                    "submitting" | "accepted" | "resting" | "partially_filled"
                )
                && order.quantity > 0.0
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
                    * quote_usd_rate(settings, &order.symbol)
            };
            reference
                * remaining_fraction
                * scheduled_stop_headroom_multiplier(&order.order_type, order.strategy.as_deref())
        })
        .sum::<f64>();
    open_positions + pending_orders
}

fn daily_strategy_open_position_count(settings: &Settings, strategy: &str) -> usize {
    if settings.strategy_positions.is_empty() {
        return settings
            .positions
            .values()
            .filter(|position| {
                position.quantity.abs() > 1e-12 && !position.scout_entry && position.strategy.as_deref() == Some(strategy)
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
    settings
        .orders
        .iter()
        .filter(|order| {
            order.purpose == "signal"
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
    let base =
        std::env::var("POSITION_MANAGEMENT_URL").unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let response = state
        .client
        .get(format!("{base}/positions/usd-rates"))
        .send()
        .await
        .map_err(upstream_error)?
        .error_for_status()
        .map_err(upstream_error)?
        .json::<serde_json::Value>()
        .await
        .map_err(upstream_error)?;
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
    let mut settings = state.settings.lock().await;
    settings.usd_rates = parsed;
    settings.used_notional_today = allocated_notional(&settings);
    persist_settings(&state.settings_path, &settings)?;
    Ok(())
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
    fn daily_budget_and_position_slots_are_split_across_selected_daily_strategies() {
        let mut settings = Settings::default();
        settings.daily_strategy_notional_limit = 600.0;
        settings.max_daily_open_positions = 10;
        settings.strategies = vec![
            Strategy::Donchian55Btc200,
            Strategy::Trend50200,
            Strategy::LiveMomentum,
            Strategy::DatedCarry,
        ];
        assert_eq!(enabled_daily_strategy_count(&settings), 2);
        assert_eq!(daily_strategy_notional_share(&settings), 300.0);
        assert_eq!(daily_strategy_position_limit(&settings), 5);

        settings.strategies.push(Strategy::Rsi2OversoldRebound);
        assert_eq!(enabled_daily_strategy_count(&settings), 3);
        assert_eq!(daily_strategy_notional_share(&settings), 200.0);
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
        settings.positions.insert("ETH-USDT".into(), position("live-momentum", 2.0, 10.0));
        settings.orders.push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 100.0);

        settings.orders[0].side = "sell".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 60.0);
        settings.orders[0].purpose = "stop-loss".into();
        assert_eq!(allocated_notional_for_symbol(&settings, "ETH-USDT"), 20.0);
    }

    #[test]
    fn category_allocation_and_open_counts_support_strategy_lots_and_legacy_positions() {
        let mut settings = settings_with_rates();
        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "donchian-20-btc200".into(), position("donchian-20-btc200", 2.0, 10.0));
        settings.strategy_positions.entry("BTC-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", -1.0, 50.0));
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)), 20.0);
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Intraday)), 50.0);
        assert_eq!(allocated_notional(&settings), 70.0);
        assert_eq!(total_open_position_count(&settings), 2);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Daily), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);

        settings.strategy_positions.clear();
        settings.positions.insert("ETH-USDT".into(), position("donchian-20-btc200", 2.0, 10.0));
        settings.positions.insert("BTC-USDT".into(), position("live-momentum", -1.0, 50.0));
        assert_eq!(allocated_notional_for_category(&settings, Some(StrategyCategory::Daily)), 20.0);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);
    }

    #[test]
    fn scout_positions_do_not_consume_real_open_position_caps() {
        let mut settings = settings_with_rates();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings.strategy_positions.entry("ETH-USDT".into()).or_default()
            .insert("live-momentum".into(), scout.clone());
        settings.strategy_positions.entry("BTC-USDT".into()).or_default()
            .insert("live-trade-flow".into(), position("live-trade-flow", 1.0, 50.0));
        settings.strategy_positions.entry("SOL-USDT".into()).or_default()
            .insert("trend-50-200".into(), scout);

        assert_eq!(total_open_position_count(&settings), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 1);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Daily), 0);

        settings.strategy_positions.clear();
        let mut scout = position("live-momentum", 1.0, 10.0);
        scout.scout_entry = true;
        settings.positions.insert("ETH-USDT".into(), scout);
        assert_eq!(total_open_position_count(&settings), 0);
        assert_eq!(category_open_position_count(&settings, StrategyCategory::Intraday), 0);
    }

    #[test]
    fn strategy_budgets_allocate_configured_remainder_and_exposure() {
        let mut settings = settings_with_rates();
        settings.strategies = vec![
            Strategy::Donchian20Btc200,
            Strategy::Donchian55Btc200,
            Strategy::LiveMomentum,
            Strategy::LiveTradeFlow,
        ];
        settings.daily_strategy_notional_limit = 300.0;
        settings.daily_strategy_budgets.insert("donchian-20-btc200".into(), 80.0);
        assert_eq!(enabled_daily_strategy_count(&settings), 2);
        assert_eq!(daily_strategy_notional_share(&settings), 150.0);
        assert_eq!(daily_strategy_budget(&settings, "donchian-20-btc200"), 80.0);
        assert_eq!(daily_strategy_budget(&settings, "donchian-55-btc200"), 220.0);
        assert_eq!(daily_strategy_budget(&settings, "not-enabled"), 0.0);

        settings.intraday_notional_limit = 120.0;
        settings.intraday_strategy_budgets.insert("live-momentum".into(), 45.0);
        assert_eq!(enabled_intraday_strategy_count(&settings), 2);
        assert_eq!(intraday_strategy_notional_share(&settings), 60.0);
        assert_eq!(intraday_strategy_budget(&settings, "live-momentum"), 45.0);
        assert_eq!(intraday_strategy_budget(&settings, "live-trade-flow"), 75.0);

        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", 2.0, 10.0));
        settings.orders.push(pending_order("ETH-USDT", "live-momentum", "buy", 5.0));
        assert_eq!(allocated_notional_for_strategy(&settings, "live-momentum"), 100.0);
        assert_eq!(daily_strategy_open_position_count(&settings, "live-momentum"), 1);
        assert_eq!(daily_strategy_open_position_count(&settings, "donchian-20-btc200"), 0);
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
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday), 1);
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Daily), 1);
        assert_eq!(pending_entry_position_count_for_strategy(&settings, "live-momentum"), 1);
        settings.orders.push(pending_order("SOL-USDT", "live-order-book", "buy", 2.0));
        settings.orders.last_mut().unwrap().purpose = "scout".into();
        assert_eq!(pending_entry_position_count_for_category(&settings, StrategyCategory::Intraday), 1);

        settings.strategy_positions.entry("ETH-USDT".into()).or_default().insert(
            "live-momentum".into(), position("live-momentum", 1.0, 10.0));
        assert_eq!(pending_entry_position_count_for_strategy(&settings, "live-momentum"), 0);
    }

    #[test]
    fn category_limits_and_quote_conversion_use_safe_fallbacks() {
        let mut settings = settings_with_rates();
        settings.max_daily_open_positions = 7;
        settings.strategies = vec![Strategy::Donchian20Btc200, Strategy::Donchian55Btc200];
        settings.daily_strategy_notional_limit = 320.0;
        settings.intraday_notional_limit = 90.0;
        assert_eq!(category_position_limit(&settings, StrategyCategory::Daily), 7);
        assert_eq!(category_notional_limit(&settings, StrategyCategory::Daily), 320.0);
        assert_eq!(category_notional_limit(&settings, StrategyCategory::Intraday), 90.0);
        assert_eq!(daily_strategy_position_limit(&settings), 4);
        assert_eq!(quote_usd_rate(&settings, "ETH-USDT"), 1.0);
        assert_eq!(quote_usd_rate_for_map(&BTreeMap::new(), "ETH-XYZ"), 10.0);
        assert_eq!(quote_usd_rate_for_map(&BTreeMap::from([("XYZ".into(), f64::NAN)]), "ETH-XYZ"), 10.0);
        assert_eq!(scheduled_stop_headroom_multiplier("Vwap", Some("live-momentum")), 2.0);
        assert_eq!(scheduled_stop_headroom_multiplier("Limit", Some("live-momentum")), 1.0);
        assert!(!is_independent_daily_strategy("live-momentum"));
        assert!(is_independent_daily_strategy("donchian-20-btc200"));
    }
}
