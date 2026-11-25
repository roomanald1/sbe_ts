// Trades are append-only after load. Capture metadata and its trade cutoff
// together, then copy that immutable prefix in short, fair lock acquisitions.
async fn snapshot_position_ledger(ledger: &Mutex<Ledger>) -> (Ledger, f64, f64, f64) {
    let wait_started = Instant::now();
    let guard = ledger.lock().await;
    let mut wait_ms = wait_started.elapsed().as_secs_f64() * 1_000.0;
    let mut copy_ms = 0.0;
    let hold_started = Instant::now();
    let copy_started = Instant::now();
    let trade_count = guard.trades.len();
    let mut snapshot = Ledger {
        trades: Vec::new(),
        dated_carry_positions: guard.dated_carry_positions.clone(),
        initial_cash: guard.initial_cash,
        topups: guard.topups.clone(),
        max_leverage: guard.max_leverage,
        orders: guard.orders.clone(),
        barrier_orders: guard.barrier_orders.clone(),
        scheduled_runs: guard.scheduled_runs.clone(),
        no_fill_order_responses: guard.no_fill_order_responses.clone(),
        fill_id_epoch: guard.fill_id_epoch.clone(),
        fill_id_sequence: guard.fill_id_sequence,
    };
    copy_ms += copy_started.elapsed().as_secs_f64() * 1_000.0;
    drop(guard);
    let mut max_hold_ms = hold_started.elapsed().as_secs_f64() * 1_000.0;
    // Allocation and all report calculations happen without the ledger lock.
    snapshot.trades.reserve_exact(trade_count);
    while snapshot.trades.len() < trade_count {
        tokio::task::yield_now().await;
        let start = snapshot.trades.len();
        // Keep each clone section short so order and fill processing can acquire the
        // ledger promptly even when the persisted trade history is large.
        let end = (start + 256).min(trade_count);
        let wait_started = Instant::now();
        let guard = ledger.lock().await;
        // Report the worst individual acquisition. Summing waits across hundreds
        // of chunks makes this stage look like one long lock stall.
        wait_ms = wait_ms.max(wait_started.elapsed().as_secs_f64() * 1_000.0);
        let hold_started = Instant::now();
        snapshot.trades.extend_from_slice(&guard.trades[start..end]);
        copy_ms += hold_started.elapsed().as_secs_f64() * 1_000.0;
        drop(guard);
        max_hold_ms = max_hold_ms.max(hold_started.elapsed().as_secs_f64() * 1_000.0);
    }
    (snapshot, wait_ms, max_hold_ms, copy_ms)
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
    fn market_mark(&self, symbol: &str) -> Option<MarketMark> {
        self.market_marks
            .read()
            .expect("market mark map poisoned")
            .get(symbol)
            .cloned()
    }

    fn market_marks_snapshot(&self) -> HashMap<String, MarketMark> {
        self.market_marks
            .read()
            .expect("market mark map poisoned")
            .clone()
    }

    async fn load() -> anyhow::Result<Self> {
        let json_path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("positions.json"));
        let path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .map(|path| path.with_extension("sqlite"))
            .unwrap_or_else(|| json_path.with_extension("sqlite"));
        Self::load_from_path(path).await
    }

    async fn load_from_path(path: PathBuf) -> anyhow::Result<Self> {
        Self::load_from_path_with_writer(path, true).await
    }

    #[cfg(feature = "test-support")]
    async fn load_from_path_without_writer(path: PathBuf) -> anyhow::Result<Self> {
        Self::load_from_path_with_writer(path, false).await
    }

    async fn load_from_path_with_writer(
        path: PathBuf,
        start_background_writer: bool,
    ) -> anyhow::Result<Self> {
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
                run.error = Some("Recoverable scheduled execution requires review after Position Manager restart; automatic replay is disabled to prevent duplicate fills. Persisted cumulative progress and fills are retained.".to_owned());
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
        let (state_updates, _) = broadcast::channel(POSITION_UI_EVENT_QUEUE_CAPACITY);
        let store = Self {
            path,
            ledger: Arc::new(Mutex::new(ledger)),
            persistence_notify: Arc::new(Notify::new()),
            persistence_write_lock: Arc::new(Mutex::new(())),
            pending_client_order_commits: StdMutex::new(HashSet::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            market_marks: StdRwLock::new(HashMap::new()),
            market_mark_pollers: StdMutex::new(HashSet::new()),
            vwap_runs: Mutex::new(vwap_runs),
        };
        {
            let ledger = store.ledger.lock().await;
            with_ledger_trade_index(&store, &ledger, |_| ());
        }
        if start_background_writer {
            let writer_ledger = Arc::clone(&store.ledger);
            let writer_notify = Arc::clone(&store.persistence_notify);
            let writer_write_lock = Arc::clone(&store.persistence_write_lock);
            let writer_timings = Arc::clone(&store.persistence_timings_ms);
            let writer_path = store.path.clone();
            tokio::spawn(async move {
                let mut heartbeat = tokio::time::interval_at(
                    Instant::now() + Duration::from_secs(30),
                    Duration::from_secs(30),
                );
                heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
                loop {
                    tokio::select! {
                        _ = writer_notify.notified() => {},
                        _ = heartbeat.tick() => {},
                    }
                    // Let a burst of fills and book updates collapse into one current snapshot.
                    tokio::time::sleep(Duration::from_millis(10)).await;
                    // Serialize before snapshotting: a queued background write must
                    // never capture stale state and overwrite a durable commit.
                    let _write_guard = writer_write_lock.lock().await;
                    let (ledger_snapshot, wait_ms, max_hold_ms, copy_ms) =
                        snapshot_position_ledger(&writer_ledger).await;
                    record_position_persistence_sample(
                        &writer_timings,
                        "snapshot_lock_wait",
                        wait_ms,
                    );
                    record_position_persistence_sample(
                        &writer_timings,
                        "snapshot_max_lock_hold",
                        max_hold_ms,
                    );
                    record_position_persistence_sample(&writer_timings, "snapshot_copy", copy_ms);
                    let serialization_started = Instant::now();
                    let snapshot = match tokio::task::spawn_blocking(move || ledger_snapshot).await
                    {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            crate::service_log!(
                                "ERROR",
                                "Position ledger serializer failed: {error}"
                            );
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
        }
        if ledger_migrated {
            if start_background_writer {
                let ledger = store.ledger.lock().await;
                store.persist(&ledger).await?;
            } else {
                store.persist_durable_snapshot(None).await?;
            }
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

    fn mark_client_order_commit_pending(&self, client_order_id: &str) {
        self.pending_client_order_commits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(client_order_id.to_owned());
    }

    fn client_order_commit_pending(&self, client_order_id: &str) -> bool {
        self.pending_client_order_commits
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(client_order_id)
    }

    async fn persist_durable_snapshot(
        &self,
        committed_client_order_id: Option<&str>,
    ) -> anyhow::Result<()> {
        let _write_guard = self.persistence_write_lock.lock().await;
        let (snapshot, wait_ms, max_hold_ms, copy_ms) =
            snapshot_position_ledger(&self.ledger).await;
        record_position_persistence_sample(
            &self.persistence_timings_ms,
            "snapshot_lock_wait",
            wait_ms,
        );
        record_position_persistence_sample(
            &self.persistence_timings_ms,
            "snapshot_max_lock_hold",
            max_hold_ms,
        );
        record_position_persistence_sample(&self.persistence_timings_ms, "snapshot_copy", copy_ms);
        let path = self.path.clone();
        let write_started = Instant::now();
        let observed_at_ms = now_ms()?;
        tokio::task::spawn_blocking(move || {
            save_sqlite_ledger_at(&path, &snapshot, observed_at_ms, false)
        })
        .await??;
        record_position_persistence_sample(
            &self.persistence_timings_ms,
            "snapshot_write",
            write_started.elapsed().as_secs_f64() * 1_000.0,
        );
        if let Some(client_order_id) = committed_client_order_id {
            self.pending_client_order_commits
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .remove(client_order_id);
        }
        let _ = self.state_updates.send(());
        Ok(())
    }

    async fn persist_portfolio_evidence_baseline(&self) -> anyhow::Result<serde_json::Value> {
        let _write_guard = self.persistence_write_lock.lock().await;
        let (snapshot, _, _, _) = snapshot_position_ledger(&self.ledger).await;
        let path = self.path.clone();
        let observed_at_ms = now_ms()?;
        tokio::task::spawn_blocking(move || {
            save_sqlite_ledger_at(&path, &snapshot, observed_at_ms, true)
        })
        .await??;
        let path = self.path.clone();
        tokio::task::spawn_blocking(move || -> anyhow::Result<serde_json::Value> {
            let db = rusqlite::Connection::open(path)?;
            let payload = db.query_row(
                "SELECT payload FROM portfolio_evidence_intervals ORDER BY observed_at_ms DESC, rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )?;
            Ok(serde_json::from_str(&payload)?)
        })
        .await?
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
    db.execute(
        "CREATE TABLE IF NOT EXISTS dated_carry_positions(id TEXT PRIMARY KEY, payload TEXT NOT NULL)",
        [],
    )?;
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
        ledger.no_fill_order_responses = meta
            .get("no_fill_order_responses")
            .cloned()
            .map(serde_json::from_value::<Vec<NoFillOrderResponse>>)
            .transpose()?
            .unwrap_or_default();
        ledger.fill_id_epoch = meta
            .get("fill_id_epoch")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned);
        ledger.fill_id_sequence = meta
            .get("fill_id_sequence")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default();
        if ledger.no_fill_order_responses.len() > NO_FILL_IDEMPOTENCY_CAPACITY {
            let excess = ledger.no_fill_order_responses.len() - NO_FILL_IDEMPOTENCY_CAPACITY;
            ledger.no_fill_order_responses.drain(..excess);
        }
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
    read_rows!(
        "dated_carry_positions",
        dated_carry_positions,
        DatedCarryLedgerPosition
    );
    read_rows!("topups", topups, AccountTopup);
    read_rows!("resting_orders", orders, RestingOrder);
    read_rows!("barrier_orders", barrier_orders, BarrierOrder);
    read_rows!("scheduled_runs", scheduled_runs, VwapRunStatus);
    Ok(ledger)
}

fn save_sqlite_ledger(path: &std::path::Path, ledger: &Ledger) -> anyhow::Result<()> {
    let observed_at_ms = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64;
    save_sqlite_ledger_at(path, ledger, observed_at_ms, false)
}

/// Copy only cached conversion marks that are not future-dated and are fresh
/// under the existing USD-rate stream contract. Durable writes never fetch FX.
fn current_portfolio_fx_marks(
    observed_at_ms: u64,
) -> (BTreeMap<String, f64>, BTreeMap<String, u64>) {
    let Some(snapshot) = USD_RATE_ORDER_SNAPSHOT
        .get()
        .and_then(|rates| rates.load_full())
    else {
        return (BTreeMap::new(), BTreeMap::new());
    };
    let mut rates = BTreeMap::new();
    let mut updated_at = BTreeMap::new();
    for (currency, rate) in &snapshot.rates {
        let timestamp = snapshot
            .updated_at_by_currency
            .get(currency)
            .copied()
            .unwrap_or(snapshot.updated_at);
        if !rate.is_finite()
            || *rate <= 0.0
            || timestamp > observed_at_ms
            || (currency != "USD"
                && observed_at_ms.saturating_sub(timestamp)
                    > USD_RATE_STREAM_MAX_AGE.as_millis() as u64)
        {
            continue;
        }
        rates.insert(currency.clone(), *rate);
        updated_at.insert(currency.clone(), timestamp);
    }
    (rates, updated_at)
}

fn value_cash_balances_usd(
    balances: &BTreeMap<String, f64>,
    rates: &BTreeMap<String, f64>,
) -> Option<BTreeMap<String, f64>> {
    balances
        .iter()
        .map(|(currency, amount)| {
            let rate = rates.get(currency)?;
            let value = amount * rate;
            (amount.is_finite() && rate.is_finite() && value.is_finite())
                .then(|| (currency.clone(), value))
        })
        .collect()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PortfolioEvidenceIntervalV1 {
    schema_version: u32,
    evidence_id: String,
    portfolio_id: String,
    execution_mode: String,
    /// UTC half-open interval start: `[interval_start_utc_ms, interval_end_utc_ms)`.
    interval_start_utc_ms: Option<u64>,
    interval_end_utc_ms: u64,
    ledger_epoch: Option<String>,
    fill_cursor_start: Option<u64>,
    fill_cursor_end: u64,
    #[serde(default)]
    trade_row_cursor_end: usize,
    topup_cursor_start: Option<usize>,
    topup_cursor_end: usize,
    baseline_source: Option<String>,
    #[serde(default)]
    baseline_evidence_id: Option<String>,
    #[serde(default)]
    is_baseline: bool,
    #[serde(default)]
    opening_cash_amount: Option<f64>,
    #[serde(default)]
    opening_cash_currency: Option<String>,
    /// Per-currency opening balances and their as-of USD valuations. These are
    /// absent at the first baseline because no prior observed state exists.
    #[serde(default)]
    opening_cash_balances_by_currency: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    opening_cash_value_usd_by_currency: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    opening_cash_total_usd: Option<f64>,
    #[serde(default)]
    opening_fx_rates_usd_by_currency: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    opening_fx_rates_updated_at_by_currency: Option<BTreeMap<String, u64>>,
    known_symbols: Vec<String>,
    positions: Vec<serde_json::Value>,
    explicit_flat_symbols: Vec<String>,
    all_symbols_covered: bool,
    /// Coverage is scoped to symbols represented by the PM ledger, not the
    /// market's tradable-symbol universe.
    #[serde(default)]
    symbol_coverage_scope: String,
    cashflow_since_origin_by_currency: BTreeMap<String, f64>,
    #[serde(default)]
    cash_balances_by_currency: BTreeMap<String, f64>,
    cash_movements_by_currency: BTreeMap<String, f64>,
    /// External capital is separately identified from trade cashflows and fees.
    #[serde(default)]
    external_contributions_by_currency: BTreeMap<String, f64>,
    #[serde(default)]
    external_contribution_events: Vec<PortfolioCashContributionV1>,
    #[serde(default)]
    closing_cash_value_usd_by_currency: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    closing_fx_rates_usd_by_currency: BTreeMap<String, f64>,
    #[serde(default)]
    closing_fx_rates_updated_at_by_currency: BTreeMap<String, u64>,
    #[serde(default)]
    closing_cash_total_usd: Option<f64>,
    #[serde(default)]
    portfolio_return_complete: bool,
    #[serde(default)]
    portfolio_return_completeness_reasons: Vec<String>,
    #[serde(default)]
    flat_cash_period_net_pnl_usd: Option<f64>,
    #[serde(default)]
    flat_cash_period_return_pct: Option<f64>,
    #[serde(default)]
    portfolio_return_scope: String,
    #[serde(default)]
    unknown_cashflow_fills: usize,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    #[serde(default)]
    legacy_fill_rows_without_allocator_identity: usize,
    provenance: Vec<String>,
    heartbeat_at_utc_ms: u64,
    heartbeat_source: String,
    heartbeat_gap: bool,
    #[serde(default)]
    gap_before_baseline: bool,
    #[serde(default)]
    ledger_epoch_changed: bool,
    valuation_status: String,
    market_mark_at_utc_ms: Option<u64>,
    coverage_gap: bool,
    truncated: bool,
    complete: bool,
    #[serde(default)]
    completeness_scope: String,
    #[serde(default)]
    independent_execution_ledger_reconciled: bool,
    #[serde(default)]
    quantitative_eligibility: String,
    completeness_reasons: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PortfolioCashContributionV1 {
    contribution_id: String,
    source: String,
    source_cursor: usize,
    timestamp_utc_ms: u64,
    currency: String,
    amount: f64,
    #[serde(default)]
    value_usd: Option<f64>,
}

fn save_sqlite_ledger_at(
    path: &std::path::Path,
    ledger: &Ledger,
    observed_at_ms: u64,
    explicit_baseline_request: bool,
) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut db = rusqlite::Connection::open(path)?;
    db.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS ledger_meta(id INTEGER PRIMARY KEY CHECK(id=1), payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS trades(id TEXT PRIMARY KEY, symbol TEXT NOT NULL, timestamp INTEGER NOT NULL, side TEXT NOT NULL, quantity REAL NOT NULL, execution_price REAL NOT NULL, payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS trades_symbol_time ON trades(symbol,timestamp); CREATE TABLE IF NOT EXISTS dated_carry_positions(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS topups(id INTEGER PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS resting_orders(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS barrier_orders(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS scheduled_runs(id TEXT PRIMARY KEY, payload TEXT NOT NULL); CREATE TABLE IF NOT EXISTS portfolio_evidence_intervals(evidence_id TEXT PRIMARY KEY, observed_at_ms INTEGER NOT NULL, fill_cursor_end INTEGER NOT NULL, payload TEXT NOT NULL); CREATE INDEX IF NOT EXISTS portfolio_evidence_observed ON portfolio_evidence_intervals(observed_at_ms);")?;
    let tx = db.transaction()?;
    tx.execute(
        "INSERT OR REPLACE INTO ledger_meta(id,payload) VALUES(1,?1)",
        [serde_json::to_string(&serde_json::json!({
            "initial_cash": ledger.initial_cash,
            "max_leverage": ledger.max_leverage,
            "no_fill_order_responses": ledger.no_fill_order_responses,
            "fill_id_epoch": ledger.fill_id_epoch,
            "fill_id_sequence": ledger.fill_id_sequence,
        }))?],
    )?;
    tx.execute("DELETE FROM trades", [])?;
    tx.execute("DELETE FROM dated_carry_positions", [])?;
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
    for position in &ledger.dated_carry_positions {
        tx.execute(
            "INSERT INTO dated_carry_positions(id,payload) VALUES(?1,?2)",
            rusqlite::params![
                format!(
                    "{}:{}:{}",
                    position.future_symbol, position.expiry_ms, position.entry_ms
                ),
                serde_json::to_string(position)?
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
    let previous_payload = rusqlite::OptionalExtension::optional(tx.query_row(
            "SELECT payload FROM portfolio_evidence_intervals ORDER BY observed_at_ms DESC, rowid DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        ))?;
    let previous = previous_payload
        .map(|payload| serde_json::from_str::<PortfolioEvidenceIntervalV1>(&payload))
        .transpose()?;
    let fill_cursor_end = ledger.fill_id_sequence;
    let evidence_id = format!(
        "pm-portfolio-evidence:v1:{}:{}:{}:{}:{}:{}",
        ledger.fill_id_epoch.as_deref().unwrap_or("unknown"),
        observed_at_ms,
        fill_cursor_end,
        ledger.trades.len(),
        ledger.topups.len(),
        ledger.dated_carry_positions.len()
    );
    let summary = summarize_positions(&ledger.trades);
    let known_symbols =
        summary
            .positions
            .iter()
            .map(|position| position.symbol.clone())
            .chain(ledger.dated_carry_positions.iter().flat_map(|position| {
                [position.spot_symbol.clone(), position.future_symbol.clone()]
            }))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
    let explicit_flat_symbols = summary
        .positions
        .iter()
        .filter(|position| position.quantity.abs() <= EPSILON)
        .map(|position| position.symbol.clone())
        .chain(ledger.dated_carry_positions.iter().filter_map(|position| {
            (position.status.starts_with("closed") && position.closed_at_ms.is_some())
                .then(|| format!("carry:{}", position.future_symbol))
        }))
        .collect::<Vec<_>>();
    let mut cashflow_since_origin_by_currency = BTreeMap::new();
    let mut cash_balances_by_currency = BTreeMap::new();
    let mut cash_movements_by_currency = BTreeMap::new();
    let mut external_contributions_by_currency = BTreeMap::new();
    let mut external_contribution_events = Vec::new();
    *cash_balances_by_currency
        .entry("USD".to_owned())
        .or_insert(0.0) += ledger.initial_cash;
    let movement_start_cursor = previous
        .as_ref()
        .filter(|prior| prior.ledger_epoch == ledger.fill_id_epoch)
        .and_then(|prior| prior.fill_cursor_end.checked_add(1))
        .unwrap_or(1);
    let prior_fill_cursor = previous
        .as_ref()
        .filter(|prior| prior.ledger_epoch == ledger.fill_id_epoch)
        .map(|prior| prior.fill_cursor_end);
    let prior_topup_cursor = previous
        .as_ref()
        .filter(|prior| prior.ledger_epoch == ledger.fill_id_epoch)
        .map(|prior| prior.topup_cursor_end);
    let topup_cursor_start = prior_topup_cursor.unwrap_or(0).min(ledger.topups.len());
    let known_fee_fills = ledger.trades.iter().filter(|trade| trade.fee_known).count();
    let unknown_fee_fills = ledger
        .trades
        .iter()
        .filter(|trade| !trade.fee_known)
        .count();
    let legacy_fill_rows_without_allocator_identity = ledger
        .trades
        .iter()
        .filter(|trade| trade.fill_id.as_deref().is_none_or(str::is_empty))
        .count();
    let fee_currency_missing_fills = ledger
        .trades
        .iter()
        .filter(|trade| trade.fee_known && trade.fee_currency.trim().is_empty())
        .count();
    let unknown_cashflow_fills = ledger
        .trades
        .iter()
        .filter(|trade| {
            !trade.fee_known
                || trade.fee_currency.trim().is_empty()
                || trade
                    .symbol
                    .split_once('-')
                    .is_none_or(|(_, quote)| quote.is_empty())
        })
        .count();
    let invalid_fill_amounts = ledger.trades.iter().any(|trade| {
        !trade.quantity.is_finite()
            || trade.quantity <= 0.0
            || !trade.execution_price.is_finite()
            || trade.execution_price <= 0.0
            || (trade.fee_known && !trade.fee.is_finite())
            || trade
                .symbol
                .split_once('-')
                .is_none_or(|(base, quote)| base.is_empty() || quote.is_empty())
    });
    for trade in &ledger.trades {
        let quote = trade
            .symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or("");
        if quote.is_empty() {
            continue;
        }
        let notional = trade.quantity * trade.execution_price;
        let cash = if matches!(trade.side, TradeSide::Buy) {
            -notional
        } else {
            notional
        };
        let fee = if trade.fee_known {
            -trade.fee.abs()
        } else {
            0.0
        };
        *cashflow_since_origin_by_currency
            .entry(quote.to_owned())
            .or_insert(0.0) += cash;
        *cash_balances_by_currency
            .entry(quote.to_owned())
            .or_insert(0.0) += cash;
        let fill_sequence = trade.fill_id.as_deref().and_then(|id| {
            ledger
                .fill_id_epoch
                .as_deref()
                .and_then(|epoch| canonical_fill_sequence(id, epoch))
        });
        if fill_sequence.is_some_and(|sequence| sequence >= movement_start_cursor) {
            *cash_movements_by_currency
                .entry(quote.to_owned())
                .or_insert(0.0) += cash;
        }
        if trade.fee_known && !trade.fee_currency.trim().is_empty() {
            *cashflow_since_origin_by_currency
                .entry(trade.fee_currency.clone())
                .or_insert(0.0) += fee;
            *cash_balances_by_currency
                .entry(trade.fee_currency.clone())
                .or_insert(0.0) += fee;
            if fill_sequence.is_some_and(|sequence| sequence >= movement_start_cursor) {
                *cash_movements_by_currency
                    .entry(trade.fee_currency.clone())
                    .or_insert(0.0) += fee;
            }
        }
    }
    for topup in &ledger.topups {
        *cashflow_since_origin_by_currency
            .entry("USD".to_owned())
            .or_insert(0.0) += topup.amount;
        *cash_balances_by_currency
            .entry("USD".to_owned())
            .or_insert(0.0) += topup.amount;
    }
    for (cursor, topup) in ledger.topups.iter().enumerate().skip(topup_cursor_start) {
        *cash_movements_by_currency
            .entry("USD".to_owned())
            .or_insert(0.0) += topup.amount;
        *external_contributions_by_currency
            .entry("USD".to_owned())
            .or_insert(0.0) += topup.amount;
        external_contribution_events.push(PortfolioCashContributionV1 {
            contribution_id: format!("pm-topup:v1:{cursor}"),
            source: "position_manager_account_topup_ledger".to_owned(),
            source_cursor: cursor,
            timestamp_utc_ms: topup.timestamp,
            currency: "USD".to_owned(),
            amount: topup.amount,
            value_usd: (topup.amount.is_finite() && topup.amount > 0.0)
                .then_some(topup.amount),
        });
    }
    let (closing_fx_rates_usd_by_currency, closing_fx_rates_updated_at_by_currency) =
        current_portfolio_fx_marks(observed_at_ms);
    let closing_cash_value_usd_by_currency = value_cash_balances_usd(
        &cash_balances_by_currency,
        &closing_fx_rates_usd_by_currency,
    );
    let closing_cash_total_usd = closing_cash_value_usd_by_currency
        .as_ref()
        .map(|values| values.values().sum::<f64>())
        .filter(|value| value.is_finite());
    let opening_cash_balances_by_currency = if explicit_baseline_request {
        Some(cash_balances_by_currency.clone())
    } else {
        previous
            .as_ref()
            .map(|prior| prior.cash_balances_by_currency.clone())
    };
    let opening_cash_value_usd_by_currency = if explicit_baseline_request {
        closing_cash_value_usd_by_currency.clone()
    } else {
        previous
            .as_ref()
            .and_then(|prior| prior.closing_cash_value_usd_by_currency.clone())
    };
    let opening_cash_total_usd = if explicit_baseline_request {
        closing_cash_total_usd
    } else {
        previous
            .as_ref()
            .and_then(|prior| prior.closing_cash_total_usd)
    };
    let opening_fx_rates_usd_by_currency = if explicit_baseline_request {
        Some(closing_fx_rates_usd_by_currency.clone())
    } else {
        previous
            .as_ref()
            .map(|prior| prior.closing_fx_rates_usd_by_currency.clone())
    };
    let opening_fx_rates_updated_at_by_currency = if explicit_baseline_request {
        Some(closing_fx_rates_updated_at_by_currency.clone())
    } else {
        previous
            .as_ref()
            .map(|prior| prior.closing_fx_rates_updated_at_by_currency.clone())
    };
    let heartbeat_gap = previous
        .as_ref()
        .is_some_and(|prior| observed_at_ms.saturating_sub(prior.heartbeat_at_utc_ms) > 60_000);
    let mut completeness_reasons = Vec::new();
    if ledger.fill_id_epoch.is_none() {
        completeness_reasons.push("fill_allocator_epoch_missing".to_owned());
    }
    if unknown_fee_fills > 0 {
        completeness_reasons.push("one_or_more_fill_fees_unknown".to_owned());
    }
    if fee_currency_missing_fills > 0 {
        completeness_reasons.push("one_or_more_fee_currencies_missing".to_owned());
    }
    if invalid_fill_amounts {
        completeness_reasons.push("one_or_more_fill_amounts_or_symbols_invalid".to_owned());
    }
    if !ledger.initial_cash.is_finite() || ledger.initial_cash <= 0.0 {
        completeness_reasons.push("authoritative_usd_initial_cash_unavailable".to_owned());
    }
    if ledger
        .topups
        .iter()
        .any(|topup| !topup.amount.is_finite() || topup.amount <= 0.0)
    {
        completeness_reasons.push("invalid_or_unknown_topup_cash_movement".to_owned());
    }
    let current_epoch_sequences = ledger.fill_id_epoch.as_deref().map(|epoch| {
        ledger
            .trades
            .iter()
            .filter_map(|trade| {
                trade
                    .fill_id
                    .as_deref()
                    .map(|id| canonical_fill_sequence(id, epoch))
            })
            .collect::<Vec<_>>()
    });
    let allocator_cursor_complete = current_epoch_sequences.as_ref().is_some_and(|sequences| {
        sequences.len() == usize::try_from(fill_cursor_end).unwrap_or(usize::MAX)
            && sequences
                .iter()
                .enumerate()
                .all(|(index, sequence)| *sequence == Some(index as u64 + 1))
    }) && ledger.trades.iter().all(|trade| {
        trade
            .fill_id
            .as_deref()
            .is_none_or(|id| id.starts_with("pm-fill:v1:"))
    });
    if !allocator_cursor_complete {
        completeness_reasons.push("fill_identity_or_cursor_coverage_incomplete".to_owned());
    }
    let dated_carry_complete = ledger.dated_carry_positions.iter().all(|position| {
        position.status.starts_with("closed")
            && position.closed_at_ms.is_some()
            && position.entry_fees.is_finite()
            && position.entry_fees >= 0.0
            && ["spot-entry", "future-entry", "spot-exit", "future-exit"]
                .iter()
                .all(|leg| {
                    ledger.trades.iter().any(|trade| {
                        trade.id
                            == format!(
                                "dated-carry:{}:{}:{leg}",
                                position.future_symbol, position.entry_ms
                            )
                    })
                })
    });
    if !dated_carry_complete {
        completeness_reasons.push("dated_carry_position_state_not_fully_reconciled".to_owned());
    }
    let all_flat = summary
        .positions
        .iter()
        .all(|position| position.quantity.abs() <= EPSILON)
        && ledger.dated_carry_positions.iter().all(|position| {
            position.status.starts_with("closed") && position.closed_at_ms.is_some()
        });
    if !all_flat {
        completeness_reasons.push("nonflat_positions_need_point_in_time_valuation".to_owned());
    }
    // Every covered symbol is a symbol represented by this PM-owned paper
    // ledger. This field does not certify coverage of an exchange universe.
    let sequences_complete = allocator_cursor_complete && dated_carry_complete && all_flat;
    if ledger.dated_carry_positions.iter().any(|position| {
        position.status == "open" && (!position.entry_fees.is_finite() || position.entry_fees < 0.0)
    }) {
        completeness_reasons.push("dated_carry_fee_coverage_incomplete".to_owned());
    }
    let epoch_changed = previous.as_ref().is_some_and(|prior| {
        prior.ledger_epoch != ledger.fill_id_epoch
            || (explicit_baseline_request && prior.ledger_epoch_changed)
    });
    if heartbeat_gap && !explicit_baseline_request {
        completeness_reasons.push("persistence_checkpoint_gap_exceeds_60_seconds".to_owned());
    }
    if epoch_changed && !explicit_baseline_request {
        completeness_reasons.push("fill_allocator_epoch_changed".to_owned());
    }
    let source_quality = completeness_reasons.is_empty();
    let baseline_source = if explicit_baseline_request && source_quality {
        Some("position_manager_account_fill_topup_ledger_v1".to_owned())
    } else if !explicit_baseline_request {
        previous
            .as_ref()
            .and_then(|prior| prior.baseline_source.clone())
    } else {
        None
    };
    let is_baseline = explicit_baseline_request && source_quality;
    if is_baseline {
        // A baseline anchors current balances; it is not itself a period of
        // cash movement from the unknown prior state.
        cash_movements_by_currency.clear();
        external_contributions_by_currency.clear();
        external_contribution_events.clear();
    }
    let baseline_evidence_id = if is_baseline {
        Some(evidence_id.clone())
    } else {
        previous
            .as_ref()
            .and_then(|prior| prior.baseline_evidence_id.clone())
    };
    if explicit_baseline_request && !source_quality {
        completeness_reasons
            .push("explicit_baseline_rejected_source_coverage_incomplete".to_owned());
    }
    let checkpoint_contiguous = previous.as_ref().is_some_and(|prior| {
        prior.complete
            && prior.ledger_epoch == ledger.fill_id_epoch
            && fill_cursor_end >= prior.fill_cursor_end
            && prior.trade_row_cursor_end <= ledger.trades.len()
            && ledger.topups.len() >= prior.topup_cursor_end
            && !heartbeat_gap
            && ledger.trades[prior.trade_row_cursor_end..]
                .iter()
                .enumerate()
                .all(|(offset, trade)| {
                    trade.fill_id.as_deref().and_then(|id| {
                        ledger
                            .fill_id_epoch
                            .as_deref()
                            .and_then(|epoch| canonical_fill_sequence(id, epoch))
                    }) == Some(prior.fill_cursor_end + offset as u64 + 1)
                })
            && ledger
                .trades
                .len()
                .saturating_sub(prior.trade_row_cursor_end)
                == usize::try_from(fill_cursor_end.saturating_sub(prior.fill_cursor_end))
                    .unwrap_or(usize::MAX)
    });
    let complete = source_quality && (is_baseline || checkpoint_contiguous);
    if !is_baseline && !checkpoint_contiguous {
        completeness_reasons.push("no_contiguous_complete_baseline_interval".to_owned());
    }
    let mut portfolio_return_completeness_reasons = Vec::new();
    if is_baseline {
        portfolio_return_completeness_reasons
            .push("baseline_anchor_is_not_a_return_period".to_owned());
    }
    if !complete {
        portfolio_return_completeness_reasons
            .push("portfolio_interval_source_incomplete".to_owned());
    }
    if !all_flat {
        portfolio_return_completeness_reasons
            .push("open_positions_not_valued_at_interval_end".to_owned());
    }
    if opening_cash_balances_by_currency.is_none() {
        portfolio_return_completeness_reasons
            .push("opening_per_currency_cash_values_unavailable".to_owned());
    }
    if opening_cash_value_usd_by_currency.is_none() {
        portfolio_return_completeness_reasons
            .push("opening_timestamp_aligned_fx_valuation_unavailable".to_owned());
    }
    if closing_cash_value_usd_by_currency.is_none() {
        portfolio_return_completeness_reasons
            .push("closing_timestamp_aligned_fx_valuation_incomplete".to_owned());
    }
    if opening_cash_total_usd.is_none_or(|value| !value.is_finite() || value <= 0.0) {
        portfolio_return_completeness_reasons
            .push("opening_usd_cash_capital_unavailable_or_nonpositive".to_owned());
    }
    if unknown_cashflow_fills > 0 {
        portfolio_return_completeness_reasons
            .push("fill_cashflows_or_fees_not_fully_known".to_owned());
    }
    if ledger.topups[topup_cursor_start..]
        .iter()
        .any(|topup| !topup.amount.is_finite() || topup.amount <= 0.0)
    {
        portfolio_return_completeness_reasons
            .push("external_contribution_amount_unknown_or_invalid".to_owned());
    }
    let external_contributions_usd = external_contribution_events
        .iter()
        .map(|event| {
            (event.currency == "USD")
                .then_some(event.value_usd)
                .flatten()
        })
        .try_fold(0.0, |sum, amount| {
            amount.filter(|amount| amount.is_finite()).map(|amount| sum + amount)
        })
        .filter(|sum: &f64| sum.is_finite());
    if external_contributions_usd.is_none() {
        portfolio_return_completeness_reasons
            .push("external_contribution_usd_valuation_unavailable".to_owned());
    }
    let portfolio_return_complete = portfolio_return_completeness_reasons.is_empty();
    let flat_cash_period_net_pnl_usd = portfolio_return_complete
        .then(|| {
            Some(
                closing_cash_total_usd? - opening_cash_total_usd? - external_contributions_usd?,
            )
        })
        .flatten()
        .filter(|value| value.is_finite());
    let flat_cash_period_return_pct = flat_cash_period_net_pnl_usd
        .zip(opening_cash_total_usd)
        .map(|(pnl, opening)| pnl / opening * 100.0)
        .filter(|value| value.is_finite());
    let checkpoint = PortfolioEvidenceIntervalV1 {
        schema_version: 1,
        evidence_id: evidence_id.clone(),
        portfolio_id: "paper-default".to_owned(),
        execution_mode: "paper".to_owned(),
        interval_start_utc_ms: if is_baseline {
            Some(observed_at_ms)
        } else {
            previous.as_ref().map(|prior| prior.interval_end_utc_ms)
        },
        interval_end_utc_ms: observed_at_ms,
        ledger_epoch: ledger.fill_id_epoch.clone(),
        fill_cursor_start: if is_baseline {
            Some(fill_cursor_end)
        } else {
            prior_fill_cursor
        },
        fill_cursor_end,
        trade_row_cursor_end: ledger.trades.len(),
        topup_cursor_start: if is_baseline {
            Some(ledger.topups.len())
        } else {
            prior_topup_cursor
        },
        topup_cursor_end: ledger.topups.len(),
        baseline_source,
        baseline_evidence_id,
        is_baseline,
        opening_cash_amount: if is_baseline {
            Some(ledger.initial_cash)
        } else {
            previous
                .as_ref()
                .and_then(|prior| prior.opening_cash_amount)
        },
        opening_cash_currency: if is_baseline {
            Some("USD".to_owned())
        } else {
            previous
                .as_ref()
                .and_then(|prior| prior.opening_cash_currency.clone())
        },
        opening_cash_balances_by_currency,
        opening_cash_value_usd_by_currency,
        opening_cash_total_usd,
        opening_fx_rates_usd_by_currency,
        opening_fx_rates_updated_at_by_currency,
        known_symbols,
        positions: summary
            .positions
            .into_iter()
            .map(serde_json::to_value)
            .chain(
                ledger
                    .dated_carry_positions
                    .iter()
                    .map(serde_json::to_value),
            )
            .collect::<Result<_, _>>()?,
        explicit_flat_symbols,
        all_symbols_covered: sequences_complete,
        symbol_coverage_scope: "all_position_symbols_in_position_manager_ledger".to_owned(),
        cashflow_since_origin_by_currency,
        cash_balances_by_currency,
        cash_movements_by_currency,
        external_contributions_by_currency,
        external_contribution_events,
        closing_cash_value_usd_by_currency,
        closing_fx_rates_usd_by_currency,
        closing_fx_rates_updated_at_by_currency,
        closing_cash_total_usd,
        portfolio_return_complete,
        portfolio_return_completeness_reasons,
        flat_cash_period_net_pnl_usd,
        flat_cash_period_return_pct,
        portfolio_return_scope: "pm_internal_flat_cash_only".to_owned(),
        unknown_cashflow_fills,
        known_fee_fills,
        unknown_fee_fills,
        legacy_fill_rows_without_allocator_identity,
        provenance: vec![
            "position_manager_sqlite_fill_ledger".to_owned(),
            "position_manager_account_api_usd_cash_baseline".to_owned(),
            "position_manager_sqlite_usd_topup_ledger".to_owned(),
            "position_manager_dated_carry_ledger".to_owned(),
        ],
        heartbeat_at_utc_ms: observed_at_ms,
        heartbeat_source: "position_manager_ledger_persistence_only".to_owned(),
        heartbeat_gap: heartbeat_gap && !is_baseline,
        gap_before_baseline: is_baseline
            && (heartbeat_gap
                || epoch_changed
                || previous.as_ref().is_some_and(|prior| !prior.complete)),
        ledger_epoch_changed: epoch_changed,
        valuation_status: if all_flat {
            "flat_cash_only"
        } else {
            "unavailable_for_open_positions"
        }
        .to_owned(),
        market_mark_at_utc_ms: None,
        coverage_gap: !complete,
        truncated: false,
        complete,
        completeness_scope: "position_manager_internal_source".to_owned(),
        independent_execution_ledger_reconciled: false,
        quantitative_eligibility: "blocked_pending_independent_reconciliation".to_owned(),
        completeness_reasons,
    };
    tx.execute(
        "INSERT OR IGNORE INTO portfolio_evidence_intervals(evidence_id,observed_at_ms,fill_cursor_end,payload) VALUES(?1,?2,?3,?4)",
        rusqlite::params![evidence_id, i64::try_from(observed_at_ms)?, i64::try_from(fill_cursor_end)?, serde_json::to_string(&checkpoint)?],
    )?;
    tx.commit()?;
    Ok(())
}

fn canonical_fill_sequence(fill_id: &str, epoch: &str) -> Option<u64> {
    let identity = fill_id.strip_prefix("pm-fill:v1:")?;
    let (fill_epoch, sequence) = identity.rsplit_once(':')?;
    (fill_epoch == epoch)
        .then(|| sequence.parse::<u64>().ok())
        .flatten()
}

#[cfg(test)]
mod ledger_snapshot_tests {
    use super::*;

    fn fill(id: &str, side: &str, sequence: u64, timestamp: u64) -> PositionTrade {
        let mut trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": id,
            "symbol": "BTC-USDT",
            "side": side,
            "quantity": 1.0,
            "execution_price": 100.0,
            "timestamp": timestamp,
            "parent_order_id": format!("order-{id}"),
            "fee": 0.1,
            "fee_known": true,
            "fee_currency": "USDT"
        }))
        .unwrap();
        trade.fill_id = Some(format!("pm-fill:v1:test-epoch:{sequence}"));
        trade
    }

    #[test]
    fn portfolio_intervals_persist_unknown_baseline_flat_period_and_cursor_across_restart() {
        let path = std::env::temp_dir().join(format!(
            "position-management-portfolio-evidence-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let ledger = Ledger {
            fill_id_epoch: Some("test-epoch".to_owned()),
            fill_id_sequence: 1,
            trades: vec![fill("open", "buy", 1, 900)],
            topups: vec![AccountTopup {
                amount: 50.0,
                timestamp: 800,
            }],
            ..Ledger::default()
        };

        save_sqlite_ledger_at(&path, &ledger, 1_000, true).unwrap();
        save_sqlite_ledger_at(&path, &ledger, 1_000, true).unwrap();
        let restarted = load_sqlite_ledger(&path).unwrap();
        assert_eq!(restarted.fill_id_epoch.as_deref(), Some("test-epoch"));
        assert_eq!(restarted.fill_id_sequence, 1);
        let mut ledger = restarted;
        ledger.trades.push(fill("close", "sell", 2, 1_500));
        ledger.fill_id_sequence = 2;
        save_sqlite_ledger_at(&path, &ledger, 2_000, false).unwrap();
        ledger.topups.push(AccountTopup {
            amount: 25.0,
            timestamp: 2_500,
        });
        save_sqlite_ledger_at(&path, &ledger, 3_000, false).unwrap();
        ledger.topups.push(AccountTopup {
            amount: 10.0,
            timestamp: 70_000,
        });
        save_sqlite_ledger_at(&path, &ledger, 70_001, false).unwrap();

        let db = rusqlite::Connection::open(&path).unwrap();
        let rows = {
            let mut statement = db.prepare(
                "SELECT payload FROM portfolio_evidence_intervals ORDER BY observed_at_ms, rowid",
            ).unwrap();
            statement
                .query_map([], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|row| {
                    serde_json::from_str::<PortfolioEvidenceIntervalV1>(&row.unwrap()).unwrap()
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rows.len(), 4, "same checkpoint retry must be idempotent");
        assert_eq!(rows[0].ledger_epoch.as_deref(), Some("test-epoch"));
        assert_eq!(rows[0].fill_cursor_end, 1);
        assert_eq!(rows[0].baseline_source, None);
        assert!(!rows[0].complete, "unknown initial state must fail closed");
        assert_eq!(rows[1].fill_cursor_start, Some(1));
        assert_eq!(rows[1].fill_cursor_end, 2);
        assert!(
            rows[1]
                .explicit_flat_symbols
                .iter()
                .any(|symbol| symbol == "BTC-USDT")
        );
        assert!(rows[1].cash_movements_by_currency.get("USDT").is_some());
        assert!(rows[0].cash_movements_by_currency.get("USD").is_some());
        assert_eq!(rows[2].interval_start_utc_ms, Some(2_000));
        assert!(
            rows[2]
                .explicit_flat_symbols
                .iter()
                .any(|symbol| symbol == "BTC-USDT")
        );
        assert!(rows[3].heartbeat_gap);
        assert!(rows[3].coverage_gap);
        assert!(rows.iter().all(|row| !row.complete));

        drop(db);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn explicit_flat_baseline_heartbeat_topup_and_fill_coverage_survive_restart() {
        let path = std::env::temp_dir().join(format!(
            "position-management-portfolio-baseline-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut ledger = Ledger {
            initial_cash: 10_000.0,
            fill_id_epoch: Some("test-epoch".to_owned()),
            fill_id_sequence: 2,
            trades: vec![fill("open", "buy", 1, 900), fill("close", "sell", 2, 950)],
            topups: vec![AccountTopup {
                amount: 50.0,
                timestamp: 800,
            }],
            ..Ledger::default()
        };
        save_sqlite_ledger_at(&path, &ledger, 1_000, true).unwrap();
        let restarted = load_sqlite_ledger(&path).unwrap();
        let baseline = latest_portfolio_evidence(&path).unwrap();
        assert!(baseline.is_baseline && baseline.complete, "{baseline:?}");
        assert_eq!(
            baseline.baseline_source.as_deref(),
            Some("position_manager_account_fill_topup_ledger_v1")
        );
        assert_eq!(baseline.opening_cash_amount, Some(ledger.initial_cash));
        assert_eq!(baseline.opening_cash_currency.as_deref(), Some("USD"));
        assert_eq!(baseline.ledger_epoch.as_deref(), Some("test-epoch"));
        assert_eq!(baseline.fill_cursor_end, 2);
        assert_eq!(baseline.topup_cursor_end, 1);
        assert_eq!(baseline.unknown_cashflow_fills, 0);
        assert!(
            baseline
                .provenance
                .iter()
                .any(|source| source == "position_manager_account_api_usd_cash_baseline")
        );
        assert!(
            baseline
                .explicit_flat_symbols
                .iter()
                .any(|symbol| symbol == "BTC-USDT")
        );
        assert_eq!(
            baseline.cash_balances_by_currency.get("USD"),
            Some(&10_050.0)
        );

        save_sqlite_ledger_at(&path, &restarted, 30_000, false).unwrap();
        let heartbeat = latest_portfolio_evidence(&path).unwrap();
        assert!(heartbeat.complete && !heartbeat.heartbeat_gap);
        assert_eq!(heartbeat.interval_start_utc_ms, Some(1_000));
        assert_eq!(
            heartbeat.baseline_evidence_id.as_deref(),
            Some(baseline.evidence_id.as_str())
        );

        ledger.topups.push(AccountTopup {
            amount: 25.0,
            timestamp: 31_000,
        });
        save_sqlite_ledger_at(&path, &ledger, 32_000, false).unwrap();
        let after_topup = latest_portfolio_evidence(&path).unwrap();
        assert!(after_topup.complete);
        assert_eq!(after_topup.topup_cursor_start, Some(1));
        assert_eq!(after_topup.topup_cursor_end, 2);
        assert_eq!(
            after_topup.cash_movements_by_currency.get("USD"),
            Some(&25.0)
        );
        assert_eq!(
            after_topup.cash_balances_by_currency.get("USD"),
            Some(&10_075.0)
        );
        assert!(!after_topup.portfolio_return_complete);
        assert!(after_topup.flat_cash_period_net_pnl_usd.is_none());
        assert!(after_topup
            .portfolio_return_completeness_reasons
            .iter()
            .any(|reason| reason == "closing_timestamp_aligned_fx_valuation_incomplete"));

        ledger.trades.push(fill("reopen", "buy", 3, 33_000));
        ledger.fill_id_sequence = 3;
        save_sqlite_ledger_at(&path, &ledger, 34_000, false).unwrap();
        let after_fill = latest_portfolio_evidence(&path).unwrap();
        assert!(!after_fill.complete);
        assert_eq!(after_fill.fill_cursor_end, 3);
        assert!(after_fill.cash_movements_by_currency.contains_key("USDT"));
        assert!(after_fill
            .portfolio_return_completeness_reasons
            .iter()
            .any(|reason| reason == "open_positions_not_valued_at_interval_end"));

        save_sqlite_ledger_at(&path, &ledger, 94_001, false).unwrap();
        let after_gap = latest_portfolio_evidence(&path).unwrap();
        assert!(after_gap.heartbeat_gap && after_gap.coverage_gap);
        assert!(!after_gap.complete);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn unknown_baseline_and_allocator_epoch_change_fail_closed_until_reanchored() {
        let path = std::env::temp_dir().join(format!(
            "position-management-portfolio-epoch-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        save_sqlite_ledger_at(&path, &Ledger::default(), 1_000, true).unwrap();
        let unknown = latest_portfolio_evidence(&path).unwrap();
        assert!(!unknown.complete && unknown.baseline_source.is_none());
        assert!(unknown.coverage_gap);

        let mut ledger = Ledger {
            fill_id_epoch: Some("epoch-a".to_owned()),
            initial_cash: 10_000.0,
            ..Ledger::default()
        };
        save_sqlite_ledger_at(&path, &ledger, 2_000, true).unwrap();
        assert!(latest_portfolio_evidence(&path).unwrap().complete);
        ledger.fill_id_epoch = Some("epoch-b".to_owned());
        save_sqlite_ledger_at(&path, &ledger, 3_000, false).unwrap();
        let changed = latest_portfolio_evidence(&path).unwrap();
        assert!(!changed.complete && changed.coverage_gap);
        assert!(
            changed
                .completeness_reasons
                .iter()
                .any(|reason| reason == "fill_allocator_epoch_changed")
        );
        save_sqlite_ledger_at(&path, &ledger, 4_000, true).unwrap();
        let reanchored = latest_portfolio_evidence(&path).unwrap();
        assert!(reanchored.is_baseline && reanchored.complete);
        assert_eq!(reanchored.ledger_epoch.as_deref(), Some("epoch-b"));
        assert!(reanchored.gap_before_baseline && reanchored.ledger_epoch_changed);

        let mut unknown_fee = fill("unknown-fee", "buy", 1, 5_000);
        unknown_fee.fill_id = Some("pm-fill:v1:epoch-b:1".to_owned());
        unknown_fee.fee_known = false;
        unknown_fee.fee_currency.clear();
        ledger.trades.push(unknown_fee);
        ledger.fill_id_sequence = 1;
        save_sqlite_ledger_at(&path, &ledger, 5_000, false).unwrap();
        let incomplete_fee = latest_portfolio_evidence(&path).unwrap();
        assert!(!incomplete_fee.complete);
        assert_eq!(incomplete_fee.unknown_cashflow_fills, 1);
        assert!(
            incomplete_fee
                .completeness_reasons
                .iter()
                .any(|reason| reason == "one_or_more_fill_fees_unknown")
        );
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    fn latest_portfolio_evidence(
        path: &std::path::Path,
    ) -> anyhow::Result<PortfolioEvidenceIntervalV1> {
        let db = rusqlite::Connection::open(path)?;
        let payload = db.query_row(
            "SELECT payload FROM portfolio_evidence_intervals ORDER BY observed_at_ms DESC, rowid DESC LIMIT 1",
            [],
            |row| row.get::<_, String>(0),
        )?;
        Ok(serde_json::from_str(&payload)?)
    }

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
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "round-trip", "symbol": "BTC-USDT", "side": "buy",
            "quantity": 0.25, "execution_price": 40000.0, "timestamp": 123
        }))
        .unwrap();
        let ledger = Ledger {
            trades: vec![trade],
            initial_cash: 12_345.0,
            topups: vec![AccountTopup {
                amount: 50.0,
                timestamp: 456,
            }],
            max_leverage: Some(3.0),
            ..Ledger::default()
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

    #[test]
    fn sqlite_fill_allocator_and_fill_ids_survive_restart_without_reuse() {
        let path = std::env::temp_dir().join(format!(
            "position-management-fill-id-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let mut ledger = Ledger::default();
        let first_fill_id = allocate_fill_id(&mut ledger, 100);
        let evidence_epoch = ledger_evidence_epoch_id(ledger.fill_id_epoch.as_deref());
        let mut first_fill: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "row-1", "symbol": "BTC-USDT", "side": "buy", "quantity": 0.25,
            "execution_price": 40000.0, "timestamp": 100, "parent_order_id": "parent-1"
        }))
        .unwrap();
        first_fill.fill_id = Some(first_fill_id.clone());
        ledger.trades.push(first_fill);

        save_sqlite_ledger(&path, &ledger).unwrap();
        let mut restarted = load_sqlite_ledger(&path).unwrap();
        assert_eq!(restarted.fill_id_epoch, ledger.fill_id_epoch);
        assert_eq!(
            ledger_evidence_epoch_id(restarted.fill_id_epoch.as_deref()),
            evidence_epoch
        );
        assert_eq!(restarted.fill_id_sequence, 1);
        assert_eq!(
            restarted.trades[0].fill_id.as_deref(),
            Some(first_fill_id.as_str())
        );

        let second_fill_id = allocate_fill_id(&mut restarted, 101);
        assert_ne!(first_fill_id, second_fill_id);
        let mut second_fill: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "row-2", "symbol": "BTC-USDT", "side": "buy", "quantity": 0.25,
            "execution_price": 40001.0, "timestamp": 101, "parent_order_id": "parent-1"
        }))
        .unwrap();
        second_fill.fill_id = Some(second_fill_id.clone());
        restarted.trades.push(second_fill);

        save_sqlite_ledger(&path, &restarted).unwrap();
        let restored = load_sqlite_ledger(&path).unwrap();
        assert_eq!(restored.fill_id_epoch, ledger.fill_id_epoch);
        assert_eq!(
            ledger_evidence_epoch_id(restored.fill_id_epoch.as_deref()),
            evidence_epoch
        );
        assert_eq!(restored.fill_id_sequence, 2);
        assert_eq!(restored.trades.len(), 2);
        assert_eq!(
            restored.trades[0].fill_id.as_deref(),
            Some(first_fill_id.as_str())
        );
        assert_eq!(
            restored.trades[1].fill_id.as_deref(),
            Some(second_fill_id.as_str())
        );
        assert_ne!(restored.trades[0].fill_id, restored.trades[1].fill_id);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn sqlite_legacy_metadata_without_no_fill_registry_loads_with_empty_registry() {
        let path = std::env::temp_dir().join(format!(
            "position-management-legacy-meta-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        save_sqlite_ledger(&path, &Ledger::default()).unwrap();
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute(
            "UPDATE ledger_meta SET payload=?1 WHERE id=1",
            [r#"{"initial_cash":10000.0,"max_leverage":null}"#],
        )
        .unwrap();
        drop(db);

        let restored = load_sqlite_ledger(&path).unwrap();
        assert!(restored.no_fill_order_responses.is_empty());
        assert_eq!(restored.fill_id_epoch, None);
        assert_eq!(restored.fill_id_sequence, 0);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn loading_store_recovers_interrupted_barriers_and_scheduled_runs() {
        let path = std::env::temp_dir().join(format!(
            "position-management-load-{}-{}.sqlite",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
        ));
        let barrier: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id": "triggering", "symbol": "BTC-USDT", "side": "sell",
            "quantity": 1.0, "remaining_quantity": 1.0, "filled_quantity": 0.0,
            "trigger_price": 100.0, "trigger_direction": "below", "created_at": 1,
            "status": "triggering"
        }))
        .unwrap();
        let auto_stop: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id": "legacy-stop", "symbol": "BTC-USDT", "side": "sell",
            "quantity": 1.0, "remaining_quantity": 1.0, "filled_quantity": 0.0,
            "trigger_price": 100.0, "trigger_direction": "below", "created_at": 1,
            "triggered_at": 2, "status": "unfilled", "source": "auto",
            "parent_order_id": "auto-stop-parent"
        }))
        .unwrap();
        let run: VwapRunStatus = serde_json::from_value(serde_json::json!({
            "id": "running", "symbol": "BTC-USDT", "side": "buy", "quantity": 1.0,
            "executed_quantity": 0.4, "strategy": "vwap", "duration_minutes": 5,
            "started_at": 1, "status": "running", "market_vwap": 0.0,
            "execution_vwap": 100.0, "slippage": 0.0,
            "lifecycle": {
                "schema_version": 1,
                "opportunity_id": "opportunity-1",
                "episode_id": "episode-1",
                "trade_id": "trade-1",
                "execution_mode": "paper",
                "entry_policy": "scheduled",
                "source": "auto"
            }
        }))
        .unwrap();
        let legacy_run: VwapRunStatus = serde_json::from_value(serde_json::json!({
            "id": "legacy-completed", "symbol": "ETH-USDT", "side": "sell", "quantity": 2.0,
            "executed_quantity": 2.0, "strategy": "twap", "duration_minutes": 10,
            "started_at": 2, "status": "completed", "market_vwap": 90.0,
            "execution_vwap": 89.0, "slippage": 1.0
        }))
        .unwrap();
        assert!(legacy_run.lifecycle.is_none());
        save_sqlite_ledger(
            &path,
            &Ledger {
                barrier_orders: vec![barrier, auto_stop],
                scheduled_runs: vec![run, legacy_run],
                ..Ledger::default()
            },
        )
        .unwrap();
        let store = PositionStore::load_from_path(path.clone()).await.unwrap();
        let ledger = store.ledger.lock().await;
        assert_eq!(ledger.barrier_orders[0].status, "interrupted");
        assert_eq!(ledger.barrier_orders[1].status, "cancelled");
        assert_eq!(ledger.scheduled_runs[0].status, "interrupted");
        assert_eq!(ledger.scheduled_runs[0].executed_quantity, 0.4);
        assert_eq!(
            ledger.scheduled_runs[0]
                .lifecycle
                .as_ref()
                .unwrap()
                .trade_id,
            "trade-1"
        );
        assert!(
            ledger.scheduled_runs[0]
                .error
                .as_deref()
                .unwrap()
                .contains("automatic replay is disabled")
        );
        assert_eq!(ledger.scheduled_runs[1].status, "completed");
        assert!(ledger.scheduled_runs[1].lifecycle.is_none());
        assert!(
            store.vwap_runs.lock().await["running"]
                .cancelled
                .load(Ordering::Relaxed)
        );
        drop(ledger);

        // The recovery decision is durable. A second process load must not turn
        // the run back into a workerless `running` row or reset its progress.
        store.persist_durable_snapshot(None).await.unwrap();
        drop(store);
        let restarted = PositionStore::load_from_path(path.clone()).await.unwrap();
        let ledger = restarted.ledger.lock().await;
        let recovered = &ledger.scheduled_runs[0];
        assert_eq!(recovered.status, "interrupted");
        assert_eq!(recovered.executed_quantity, 0.4);
        assert_eq!(recovered.lifecycle.as_ref().unwrap().trade_id, "trade-1");
        assert!(
            recovered
                .error
                .as_deref()
                .unwrap()
                .contains("automatic replay is disabled")
        );
        assert!(
            restarted.vwap_runs.lock().await["running"]
                .cancelled
                .load(Ordering::Relaxed)
        );
        drop(ledger);
        drop(restarted);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
