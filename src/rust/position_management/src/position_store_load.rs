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
        let path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("positions.json"));
        let mut ledger = match tokio::fs::read(&path).await {
            Ok(bytes) => {
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
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ledger {
                initial_cash: default_account_cash(),
                ..Ledger::default()
            },
            Err(error) => return Err(error.into()),
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
        let store = Self {
            path,
            ledger: Mutex::new(ledger),
            books: StdRwLock::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols,
            vwap_runs: Mutex::new(vwap_runs),
        };
        if ledger_migrated {
            let ledger = store.ledger.lock().await;
            store.persist(&ledger).await?;
        }
        Ok(store)
    }
    async fn persist(&self, ledger: &Ledger) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(ledger)?;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temp = self.path.with_extension("positions.tmp");
        tokio::fs::write(&temp, bytes).await?;
        tokio::fs::rename(temp, &self.path).await?;
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
