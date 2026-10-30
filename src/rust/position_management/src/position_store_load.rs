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
        let path = json_path.with_extension("msgpack");
        let binary_snapshot = tokio::fs::read(&path).await;
        let loaded_binary = binary_snapshot.is_ok();
        let legacy_json_path = (!loaded_binary).then(|| json_path.clone());
        let snapshot = match binary_snapshot {
            Ok(bytes) => Some((bytes, true)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                match tokio::fs::read(&json_path).await {
                    Ok(bytes) => Some((bytes, false)),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
                    Err(error) => return Err(error.into()),
                }
            }
            Err(error) => return Err(error.into()),
        };
        let mut ledger = match snapshot {
            Some((bytes, true)) => rmp_serde::from_slice::<Ledger>(&bytes)?,
            Some((bytes, false)) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value.is_array() {
                    Ledger {
                        trades: serde_json::from_value(value)?,
                        initial_cash: default_account_cash(),
                        topups: Vec::new(),
                        max_leverage: None,
                        orders: Vec::new(),
                        barrier_orders: Vec::new(),
                        scheduled_runs: Vec::new(),
                    }
                } else {
                    serde_json::from_value(value)?
                }
            }
            None => Ledger {
                initial_cash: default_account_cash(),
                ..Ledger::default()
            },
        };
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
                && order.parent_order_id.as_deref().is_some_and(|id| id.starts_with("auto-stop-"))
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
            account_ledger_cache: StdMutex::new(None),
            realized_pnl_cache: StdMutex::new(None),
            positions_cache: StdMutex::new(None),
            books: StdRwLock::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols,
            vwap_runs: Mutex::new(vwap_runs),
        };
        let writer_ledger = Arc::clone(&store.ledger);
        let writer_notify = Arc::clone(&store.persistence_notify);
        let writer_timings = Arc::clone(&store.persistence_timings_ms);
        let writer_path = store.path.clone();
        tokio::spawn(async move {
            let mut legacy_json_path = legacy_json_path;
            loop {
                writer_notify.notified().await;
                // Let a burst of fills and book updates collapse into one current snapshot.
                tokio::time::sleep(Duration::from_millis(10)).await;
                let ledger_guard = Arc::clone(&writer_ledger).lock_owned().await;
                let serialization_started = Instant::now();
                let snapshot = match tokio::task::spawn_blocking(move || {
                    rmp_serde::to_vec_named(&*ledger_guard)
                })
                .await
                {
                    Ok(Ok(snapshot)) => snapshot,
                    Ok(Err(error)) => {
                        crate::service_log!("ERROR", "Position ledger serialization failed: {error}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        writer_notify.notify_one();
                        continue;
                    }
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
                match tokio::task::spawn_blocking(move || persist_ledger_bytes(&path, snapshot))
                    .await
                {
                    Ok(Ok(())) => {
                        record_position_persistence_sample(
                            &writer_timings,
                            "snapshot_write",
                            write_started.elapsed().as_secs_f64() * 1_000.0,
                        );
                        if let Some(legacy_path) = legacy_json_path.take() {
                            if let Err(error) = std::fs::remove_file(&legacy_path) {
                                if error.kind() != std::io::ErrorKind::NotFound {
                                    crate::service_log!(
                                        "WARN",
                                        "Could not remove migrated JSON position ledger {}: {error}",
                                        legacy_path.display()
                                    );
                                }
                            }
                        }
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
        if ledger_migrated || !loaded_binary {
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

fn persist_ledger_bytes(path: &std::path::Path, bytes: Vec<u8>) -> anyhow::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let temp = path.with_extension("msgpack.tmp");
    std::fs::write(&temp, bytes)?;
    std::fs::rename(temp, path)?;
    Ok(())
}
