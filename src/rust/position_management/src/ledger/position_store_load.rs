// Trades are append-only after load. Capture metadata and its trade cutoff
// together, then copy that immutable prefix in short, fair lock acquisitions.
async fn snapshot_position_ledger(ledger: &Mutex<Ledger>) -> (Ledger, f64, f64) {
    let wait_started = Instant::now();
    let guard = ledger.lock().await;
    let mut wait_ms = wait_started.elapsed().as_secs_f64() * 1_000.0;
    let hold_started = Instant::now();
    let trade_count = guard.trades.len();
    let mut snapshot = Ledger {
        trades: Vec::new(),
        initial_cash: guard.initial_cash,
        topups: guard.topups.clone(),
        max_leverage: guard.max_leverage,
        orders: guard.orders.clone(),
        barrier_orders: guard.barrier_orders.clone(),
        scheduled_runs: guard.scheduled_runs.clone(),
    };
    drop(guard);
    let mut max_hold_ms = hold_started.elapsed().as_secs_f64() * 1_000.0;
    // Allocation and all report calculations happen without the ledger lock.
    snapshot.trades.reserve_exact(trade_count);
    while snapshot.trades.len() < trade_count {
        tokio::task::yield_now().await;
        let start = snapshot.trades.len();
        // Use larger chunks to avoid hundreds of lock reacquisitions and scheduler
        // yields for a large ledger, while still keeping each critical section bounded.
        let end = (start + 1_024).min(trade_count);
        let wait_started = Instant::now();
        let guard = ledger.lock().await;
        wait_ms += wait_started.elapsed().as_secs_f64() * 1_000.0;
        let hold_started = Instant::now();
        snapshot.trades.extend_from_slice(&guard.trades[start..end]);
        drop(guard);
        max_hold_ms = max_hold_ms.max(hold_started.elapsed().as_secs_f64() * 1_000.0);
    }
    (snapshot, wait_ms, max_hold_ms)
}

fn record_position_persistence_sample(
    samples: &StdMutex<BTreeMap<String, Vec<f64>>>,
    phase: &str,
    elapsed_ms: f64,
) {
    if let Ok(mut samples) = samples.lock() {
        let values = samples.entry(phase.to_owned()).or_default();
        values.push(elapsed_ms);
        if values.len() > 500 {
            values.remove(0);
        }
    }
}

impl PositionStore {
    fn book_slot(&self, symbol: &str) -> Arc<ArcSwapOption<BookSnapshot>> {
        let mut books = self.books.write().expect("book map poisoned");
        Arc::clone(
            books
                .entry(symbol.to_owned())
                .or_insert_with(|| Arc::new(ArcSwapOption::empty())),
        )
    }

    fn book(&self, symbol: &str) -> Option<Arc<BookSnapshot>> {
        let slot = self
            .books
            .read()
            .expect("book map poisoned")
            .get(symbol)
            .cloned()?;
        slot.load_full()
    }

    fn books_snapshot(&self) -> HashMap<String, Arc<BookSnapshot>> {
        self.books
            .read()
            .expect("book map poisoned")
            .iter()
            .filter_map(|(symbol, slot)| slot.load_full().map(|book| (symbol.clone(), book)))
            .collect()
    }

    async fn load(spot_symbols: Arc<Vec<String>>) -> anyhow::Result<Self> {
        let json_path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("positions.json"));
        let path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .map(|path| path.with_extension("sqlite"))
            .unwrap_or_else(|| json_path.with_extension("sqlite"));
        Self::load_from_path(path, spot_symbols).await
    }

    async fn load_from_path(path: PathBuf, spot_symbols: Arc<Vec<String>>) -> anyhow::Result<Self> {
        let mut ledger = tokio::task::spawn_blocking({
            let path = path.clone();
            move || load_sqlite_ledger(&path)
        })
        .await??;
        for order in &mut ledger.orders {
            if order.maker_fee_rate == 0.0 {
                order.maker_fee_rate = configured_fee_rates().maker;
            }
            order.estimated_fee =
                order.remaining_quantity * order.limit_price * -order.maker_fee_rate;
        }
        let mut ledger_migrated = false;
        for order in &mut ledger.barrier_orders {
            if order.status == "triggering" {
                order.status = "interrupted".to_owned();
                order.error = Some("Backend restarted during barrier execution; review fills before replacing this order".to_owned());
                ledger_migrated = true;
            } else if order.status == "unfilled"
                && order.triggered_at.is_some()
                && order.source == "auto"
                && order
                    .parent_order_id
                    .as_deref()
                    .is_some_and(|id| id.starts_with("auto-stop-"))
            {
                // Older versions left triggered AutoTrader stops as "unfilled" when
                // the reduce-only fill returned no quantity. They are terminal, not armed.
                order.status = "cancelled".to_owned();
                ledger_migrated = true;
            }
        }
        let mut vwap_runs = HashMap::new();
        for run in &mut ledger.scheduled_runs {
            if run.status == "running" {
                run.status = "interrupted".to_owned();
                run.error = Some("Position manager restarted; scheduled execution stopped. Persisted fills are retained.".to_owned());
                ledger_migrated = true;
            }
            vwap_runs.insert(
                run.id.clone(),
                VwapRunEntry {
                    status: Arc::new(Mutex::new(run.clone())),
                    cancelled: Arc::new(AtomicBool::new(true)),
                },
            );
        }
        crate::service_log!(
            "INFO",
            "Loaded {} paper fills and {} resting orders from {}",
            ledger.trades.len(),
            ledger.orders.len(),
            path.display()
        );
        let (state_updates, _) = broadcast::channel(32);
        let store = Self {
            path,
            ledger: Arc::new(Mutex::new(ledger)),
            persistence_notify: Arc::new(Notify::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            books: StdRwLock::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols,
            vwap_runs: Mutex::new(vwap_runs),
        };
        {
            let ledger = store.ledger.lock().await;
            with_ledger_trade_index(&store, &ledger, |_| ());
        }
        let writer_ledger = Arc::clone(&store.ledger);
        let writer_notify = Arc::clone(&store.persistence_notify);
        let writer_timings = Arc::clone(&store.persistence_timings_ms);
        let writer_path = store.path.clone();
        tokio::spawn(async move {
            loop {
                writer_notify.notified().await;
                // Let a burst of fills and book updates collapse into one current snapshot.
                tokio::time::sleep(Duration::from_millis(10)).await;
                let copy_started = Instant::now();
                let (ledger_snapshot, wait_ms, max_hold_ms) =
                    snapshot_position_ledger(&writer_ledger).await;
                record_position_persistence_sample(&writer_timings, "snapshot_lock_wait", wait_ms);
                record_position_persistence_sample(
                    &writer_timings,
                    "snapshot_max_lock_hold",
                    max_hold_ms,
                );
                record_position_persistence_sample(
                    &writer_timings,
                    "snapshot_copy",
                    copy_started.elapsed().as_secs_f64() * 1_000.0,
                );
                let serialization_started = Instant::now();
                let snapshot = match tokio::task::spawn_blocking(move || ledger_snapshot).await {
                    Ok(snapshot) => snapshot,
                    Err(error) => {
                        crate::service_log!("ERROR", "Position ledger serializer failed: {error}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        writer_notify.notify_one();
                        continue;
                    }
                };
                record_position_persistence_sample(
                    &writer_timings,
                    "snapshot_serialize",
                    serialization_started.elapsed().as_secs_f64() * 1_000.0,
                );
                let path = writer_path.clone();
                let write_started = Instant::now();
                match tokio::task::spawn_blocking(move || save_sqlite_ledger(&path, &snapshot))
                    .await
                {
                    Ok(Ok(())) => {
                        record_position_persistence_sample(
                            &writer_timings,
                            "snapshot_write",
                            write_started.elapsed().as_secs_f64() * 1_000.0,
                        );
                    }
                    Ok(Err(error)) => {
                        crate::service_log!(
                            "ERROR",
                            "Position ledger snapshot write failed: {error:#}"
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        writer_notify.notify_one();
                    }
                    Err(error) => {
                        crate::service_log!(
                            "ERROR",
                            "Position ledger snapshot writer failed: {error}"
                        );
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        writer_notify.notify_one();
                    }
                }
            }
        });
        if ledger_migrated {
            let ledger = store.ledger.lock().await;
            store.persist(&ledger).await?;
        }
        Ok(store)
    }
    async fn persist(&self, _ledger: &Ledger) -> anyhow::Result<()> {
        // Snapshotting and disk I/O are handled by one coalescing background writer.
        // It reads the latest ledger after the caller releases its ledger lock.
        self.persistence_notify.notify_one();
        let _ = self.state_updates.send(());
        Ok(())
    }
    async fn record_ticker_sample(&self, instrument: String, price: f64, timestamp: u64) {
        let mut histories = self.hedge_samples.lock().await;
        let samples = histories.entry(instrument).or_default();
        if samples
            .last()
            .map_or(true, |(last, _)| timestamp.saturating_sub(*last) >= 10_000)
        {
            samples.push((timestamp, price));
            samples.retain(|(time, _)| timestamp.saturating_sub(*time) <= 30 * 60_000);
        }
    }
}

fn load_sqlite_ledger(path: &std::path::Path) -> anyhow::Result<Ledger> {
    if !path.exists() {
        return Ok(Ledger {
            initial_cash: default_account_cash(),
            ..Ledger::default()
        });
    }
    let db = rusqlite::Connection::open(path)?;
    let mut ledger = Ledger {
        initial_cash: default_account_cash(),
        ..Ledger::default()
    };
    if let Some(payload) = rusqlite::OptionalExtension::optional(db.query_row(
        "SELECT payload FROM ledger_meta WHERE id=1",
        [],
        |r| r.get::<_, String>(0),
    ))? {
        let meta: serde_json::Value = serde_json::from_str(&payload)?;
        ledger.initial_cash = meta["initial_cash"]
            .as_f64()
            .unwrap_or_else(default_account_cash);
        ledger.max_leverage = meta["max_leverage"].as_f64();
    }
    macro_rules! read_rows {
        ($table:literal, $field:ident, $ty:ty) => {{
            let mut stmt =
                db.prepare(concat!("SELECT payload FROM ", $table, " ORDER BY rowid"))?;
            ledger.$field = stmt
                .query_map([], |r| r.get::<_, String>(0))?
                .map(|r| serde_json::from_str::<$ty>(&r?).map_err(Into::into))
                .collect::<anyhow::Result<Vec<_>>>()?;
        }};
    }
    read_rows!("trades", trades, PositionTrade);
    read_rows!("topups", topups, AccountTopup);
    read_rows!("resting_orders", orders, RestingOrder);
    read_rows!("barrier_orders", barrier_orders, BarrierOrder);
    read_rows!("scheduled_runs", scheduled_runs, VwapRunStatus);
    Ok(ledger)
}

fn save_sqlite_ledger(path: &std::path::Path, ledger: &Ledger) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut db = rusqlite::Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS ledger_meta(id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS trades(id TEXT PRIMARY KEY, symbol TEXT NOT NULL, timestamp INTEGER NOT NULL, side TEXT NOT NULL, quantity REAL NOT NULL, execution_price REAL NOT NULL, payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS trades_symbol_time ON trades(symbol,timestamp); CREATE TABLE IF NOT EXISTS topups(id INTEGER PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS resting_orders(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS barrier_orders(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS scheduled_runs(id TEXT PRIMARY KEY, payload TEXT NOT NULL);")?;
    let tx = db.transaction()?;
    tx.execute("INSERT OR REPLACE INTO ledger_meta(id,payload) VALUES(1,?1)",[serde_json::to_string(&serde_json::json!({"initial_cash":ledger.initial_cash,"max_leverage":ledger.max_leverage}))?])?;
    tx.execute("DELETE FROM trades", [])?;
    tx.execute("DELETE FROM topups", [])?;
    tx.execute("DELETE FROM resting_orders", [])?;
    tx.execute("DELETE FROM barrier_orders", [])?;
    tx.execute("DELETE FROM scheduled_runs", [])?;
    for t in &ledger.trades {
        let side = match t.side {
            TradeSide::Buy => "buy",
            TradeSide::Sell => "sell",
        };
        tx.execute(
            "INSERT INTO trades VALUES(?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                t.id,
                t.symbol,
                i64::try_from(t.timestamp)?,
                side,
                t.quantity,
                t.execution_price,
                serde_json::to_string(t)?
            ],
        )?;
    }
    macro_rules! write_rows {
        ($table:literal, $items:expr) => {
            for item in $items {
                tx.execute(
                    concat!("INSERT INTO ", $table, "(id,payload) VALUES(?1,?2)"),
                    rusqlite::params![item.id, serde_json::to_string(item)?],
                )?;
            }
        };
    }
    for (i, item) in ledger.topups.iter().enumerate() {
        tx.execute(
            "INSERT INTO topups(id,payload) VALUES(?1,?2)",
            rusqlite::params![i64::try_from(i)?, serde_json::to_string(item)?],
        )?;
    }
    write_rows!("resting_orders", &ledger.orders);
    write_rows!("barrier_orders", &ledger.barrier_orders);
    write_rows!("scheduled_runs", &ledger.scheduled_runs);
    tx.commit()?;
    Ok(())
}

#[cfg(test)]
mod ledger_snapshot_tests {
    use super::*;

    #[tokio::test]
    async fn batched_snapshot_keeps_cutoff_consistent_during_append() {
        let trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "original", "symbol": "BTC-USDT", "side": "buy",
            "quantity": 1.0, "execution_price": 100.0, "timestamp": 1
        }))
        .unwrap();
        let ledger = Mutex::new(Ledger {
            trades: vec![trade.clone(); 300],
            initial_cash: 1000.0,
            ..Ledger::default()
        });
        let (snapshot, ()) = tokio::join!(snapshot_position_ledger(&ledger), async {
            let mut ledger = ledger.lock().await;
            ledger.trades.push(trade);
            ledger.initial_cash = 2000.0;
        });
        assert_eq!(snapshot.0.trades.len(), 300);
        assert_eq!(snapshot.0.initial_cash, 1000.0);
        assert!(snapshot.0.trades.iter().all(|trade| trade.id == "original"));
        assert_eq!(ledger.lock().await.trades.len(), 301);
    }

    #[test]
    fn sqlite_ledger_round_trip_preserves_metadata_trades_and_topups() {
        let path = std::env::temp_dir().join(format!(
            "position-management-coverage-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "round-trip", "symbol": "BTC-USDT", "side": "buy",
            "quantity": 0.25, "execution_price": 40000.0, "timestamp": 123
        })).unwrap();
        let ledger = Ledger {
            trades: vec![trade], initial_cash: 12_345.0,
            topups: vec![AccountTopup { amount: 50.0, timestamp: 456 }],
            max_leverage: Some(3.0), ..Ledger::default()
        };
        save_sqlite_ledger(&path, &ledger).unwrap();
        let restored = load_sqlite_ledger(&path).unwrap();
        assert_eq!(restored.initial_cash, 12_345.0);
        assert_eq!(restored.max_leverage, Some(3.0));
        assert_eq!(restored.trades.len(), 1);
        assert_eq!(restored.trades[0].id, "round-trip");
        assert_eq!(restored.topups.len(), 1);
        assert_eq!(restored.topups[0].amount, 50.0);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn loading_store_recovers_interrupted_barriers_and_scheduled_runs() {
        let path = std::env::temp_dir().join(format!(
            "position-management-load-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos(),
        ));
        let barrier: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id": "triggering", "symbol": "BTC-USDT", "side": "sell",
            "quantity": 1.0, "remaining_quantity": 1.0, "filled_quantity": 0.0,
            "trigger_price": 100.0, "trigger_direction": "below", "created_at": 1,
            "status": "triggering"
        })).unwrap();
        let auto_stop: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id": "legacy-stop", "symbol": "BTC-USDT", "side": "sell",
            "quantity": 1.0, "remaining_quantity": 1.0, "filled_quantity": 0.0,
            "trigger_price": 100.0, "trigger_direction": "below", "created_at": 1,
            "triggered_at": 2, "status": "unfilled", "source": "auto",
            "parent_order_id": "auto-stop-parent"
        })).unwrap();
        let run: VwapRunStatus = serde_json::from_value(serde_json::json!({
            "id": "running", "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
            "executed_quantity": 0.0, "strategy": "vwap", "duration_minutes": 5,
            "started_at": 1, "status": "running", "market_vwap": 0.0,
            "execution_vwap": 0.0, "slippage": 0.0
        })).unwrap();
        save_sqlite_ledger(&path, &Ledger {
            barrier_orders: vec![barrier, auto_stop], scheduled_runs: vec![run],
            ..Ledger::default()
        }).unwrap();
        let store = PositionStore::load_from_path(path.clone(), Arc::new(vec!["BTC-USDT".into()]))
            .await.unwrap();
        let ledger = store.ledger.lock().await;
        assert_eq!(ledger.barrier_orders[0].status, "interrupted");
        assert_eq!(ledger.barrier_orders[1].status, "cancelled");
        assert_eq!(ledger.scheduled_runs[0].status, "interrupted");
        assert!(store.vwap_runs.lock().await["running"].cancelled.load(Ordering::Relaxed));
        drop(ledger);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
