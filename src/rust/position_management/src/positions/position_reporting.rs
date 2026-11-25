
#[derive(Serialize)]
struct FeeSummaryRow {
    source: String,
    liquidity: String,
    fee_currency: String,
    fills: usize,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    traded_notional: f64,
    fees_paid: f64,
    effective_fee_bps: Option<f64>,
    first_fill_ms: u64,
    last_fill_ms: u64,
}
#[derive(Serialize)]
struct FeeSummaryResponse {
    fills: usize,
    rows: Vec<FeeSummaryRow>,
    first_fill_ms: Option<u64>,
    last_fill_ms: Option<u64>,
}

async fn get_fee_summary(
    State(store): State<Arc<PositionStore>>,
) -> Json<FeeSummaryResponse> {
    let ledger = store.ledger.lock().await;
    Json(fee_summary_for_ledger(&ledger))
}

fn fee_summary_for_ledger(ledger: &Ledger) -> FeeSummaryResponse {
    let mut groups = BTreeMap::<(String, String, String), FeeSummaryRow>::new();
    let mut first_fill_ms: Option<u64> = None;
    let mut last_fill_ms: Option<u64> = None;
    for trade in &ledger.trades {
        let source = if trade.source.is_empty() { "unknown" } else { &trade.source }.to_owned();
        let liquidity = if trade.fee_liquidity.is_empty() { "unknown" } else { &trade.fee_liquidity }.to_owned();
        let fee_currency = if !trade.fee_known {
            "unknown".to_owned()
        } else if trade.fee_currency.is_empty() {
            trade.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or("unknown").to_owned()
        } else {
            trade.fee_currency.clone()
        };
        let row = groups.entry((source.clone(), liquidity.clone(), fee_currency.clone())).or_insert(FeeSummaryRow {
            source,
            liquidity,
            fee_currency,
            fills: 0,
            known_fee_fills: 0,
            unknown_fee_fills: 0,
            traded_notional: 0.0,
            fees_paid: 0.0,
            effective_fee_bps: None,
            first_fill_ms: trade.timestamp,
            last_fill_ms: trade.timestamp,
        });
        row.fills += 1;
        row.traded_notional += trade.quantity.abs() * trade.execution_price.abs();
        if trade.fee_known {
            row.known_fee_fills += 1;
            row.fees_paid += trade.fee.abs();
        } else {
            row.unknown_fee_fills += 1;
        }
        row.first_fill_ms = row.first_fill_ms.min(trade.timestamp);
        row.last_fill_ms = row.last_fill_ms.max(trade.timestamp);
        first_fill_ms = Some(first_fill_ms.map_or(trade.timestamp, |current| current.min(trade.timestamp)));
        last_fill_ms = Some(last_fill_ms.map_or(trade.timestamp, |current| current.max(trade.timestamp)));
    }
    let mut rows = groups.into_values().collect::<Vec<_>>();
    for row in &mut rows {
        row.effective_fee_bps = (row.traded_notional > 0.0 && row.unknown_fee_fills == 0)
            .then_some(row.fees_paid / row.traded_notional * 10_000.0);
    }
    rows.sort_by(|left, right| {
        left.source.cmp(&right.source)
            .then_with(|| left.liquidity.cmp(&right.liquidity))
            .then_with(|| left.fee_currency.cmp(&right.fee_currency))
    });
    FeeSummaryResponse {
        fills: ledger.trades.len(),
        rows,
        first_fill_ms,
        last_fill_ms,
    }
}

#[derive(Serialize)]
struct FinancialSnapshotLedgerCoverage {
    trade_rows: usize,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    fee_currency_unknown_rows: usize,
    fee_cashflow_currency_mismatch_rows: usize,
    parent_order_linked_rows: usize,
    rows_without_parent_order_id: usize,
    timestamp_order_inversion_rows: usize,
    independent_execution_ledger_reconciled: bool,
}

#[derive(Serialize)]
struct FinancialSnapshotResponse {
    snapshot_id: String,
    captured_at_ms: u64,
    execution_mode: &'static str,
    snapshot_consistency: &'static str,
    fx_rates_updated_at_by_currency: BTreeMap<String, u64>,
    market_marks_consistency: &'static str,
    account: AccountResponse,
    positions: PositionsResponse,
    realized_pnl: RealizedPnlResponse,
    fee_summary: FeeSummaryResponse,
    fee_rates: FeeRatesResponse,
    ledger_snapshot_consistent: bool,
    ledger_coverage: FinancialSnapshotLedgerCoverage,
}

async fn get_financial_snapshot(
    State(store): State<Arc<PositionStore>>,
) -> Result<Json<FinancialSnapshotResponse>, (StatusCode, String)> {
    let fx_rates = get_usd_rates().await?.0;
    let ledger = store.ledger.lock().await.clone();
    let captured_at_ms = now_ms().map_err(internal_error)?;
    Ok(Json(build_financial_snapshot(
        &store,
        &ledger,
        &fx_rates.rates,
        fx_rates.updated_at_by_currency,
        captured_at_ms,
    )?))
}

fn build_financial_snapshot(
    store: &PositionStore,
    ledger: &Ledger,
    rates: &BTreeMap<String, f64>,
    fx_rates_updated_at_by_currency: BTreeMap<String, u64>,
    captured_at_ms: u64,
) -> Result<FinancialSnapshotResponse, (StatusCode, String)> {
    use std::sync::atomic::{AtomicU64, Ordering};

    static SNAPSHOT_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    let account = account_snapshot_with_rates(store, ledger, rates)?;
    let positions = cached_position_summary(store, ledger, None);
    let fee_summary = fee_summary_for_ledger(ledger);
    let realized = replay_realized_pnl(ledger);
    let today_by_quote = realized.today_by_quote.clone();
    let today_by_parent_order = realized.today_by_parent_order.clone();
    let today_by_barrier_created_at = realized.today_by_barrier_created_at.clone();
    let all_time_by_parent_order = realized.all_time_by_parent_order.clone();
    let all_time_by_quote = realized.all_time_by_quote.clone();
    let configured_fees = configured_fee_rates();
    let convert_usd = |totals: &BTreeMap<String, f64>| -> Option<f64> {
        totals.iter().try_fold(0.0, |sum, (quote, amount)| {
            rates.get(quote).map(|rate| sum + amount * rate)
        })
    };
    let realized_pnl = RealizedPnlResponse {
        realized_pnl_by_quote: today_by_quote.clone(),
        realized_pnl_usd: convert_usd(&today_by_quote),
        realized_pnl_by_parent_order: today_by_parent_order,
        realized_pnl_by_barrier_created_at: today_by_barrier_created_at,
        realized_pnl_all_time_by_parent_order: all_time_by_parent_order,
        realized_pnl_all_time_usd: convert_usd(&all_time_by_quote),
        realized_pnl_all_time_by_quote: all_time_by_quote,
    };
    let known_fee_fills = ledger.trades.iter().filter(|trade| trade.fee_known).count();
    let parent_order_linked_rows = ledger
        .trades
        .iter()
        .filter(|trade| trade.parent_order_id.as_deref().is_some_and(|id| !id.is_empty()))
        .count();
    let fee_currency_unknown_rows = ledger
        .trades
        .iter()
        .filter(|trade| trade.fee_known && trade.fee_currency.is_empty())
        .count();
    let fee_cashflow_currency_mismatch_rows = ledger
        .trades
        .iter()
        .filter(|trade| {
            if !trade.fee_known || trade.fee_currency.is_empty() {
                return false;
            }
            let quote = trade
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or("USD");
            trade.fee_currency != quote
        })
        .count();
    let mut timestamp_order_inversion_rows = 0;
    let mut max_timestamp = 0;
    for trade in &ledger.trades {
        if trade.timestamp < max_timestamp {
            timestamp_order_inversion_rows += 1;
        }
        max_timestamp = max_timestamp.max(trade.timestamp);
    }
    let sequence = SNAPSHOT_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let snapshot_id = format!(
        "pm-paper-v1:{}:{captured_at_ms}:{sequence}",
        std::process::id(),
    );

    Ok(FinancialSnapshotResponse {
        snapshot_id,
        captured_at_ms,
        execution_mode: "paper",
        snapshot_consistency: "single_locked_ledger_clone",
        fx_rates_updated_at_by_currency,
        market_marks_consistency: "sampled_during_account_projection",
        ledger_snapshot_consistent: true,
        account,
        positions,
        realized_pnl,
        fee_summary,
        fee_rates: FeeRatesResponse {
            maker: configured_fees.maker.abs(),
            taker: configured_fees.taker.abs(),
        },
        ledger_coverage: FinancialSnapshotLedgerCoverage {
            trade_rows: ledger.trades.len(),
            known_fee_fills,
            unknown_fee_fills: ledger.trades.len().saturating_sub(known_fee_fills),
            fee_currency_unknown_rows,
            fee_cashflow_currency_mismatch_rows,
            parent_order_linked_rows,
            rows_without_parent_order_id: ledger
                .trades
                .len()
                .saturating_sub(parent_order_linked_rows),
            timestamp_order_inversion_rows,
            independent_execution_ledger_reconciled: false,
        },
    })
}

fn replay_realized_pnl(ledger: &Ledger) -> RealizedPnlLedgerSnapshot {
    let trades = &ledger.trades;
    let barrier_created_at = ledger
        .barrier_orders
        .iter()
        .map(|barrier| (barrier.id.clone(), barrier.created_at.to_string()))
        .collect::<HashMap<_, _>>();
    let mut positions = BTreeMap::<String, Position>::new();
    let mut by_quote = BTreeMap::<String, f64>::new();
    let mut by_parent_order = BTreeMap::<String, f64>::new();
    let mut by_barrier_created_at = BTreeMap::<String, f64>::new();
    for trade in trades {
        let position = positions.entry(trade.symbol.clone()).or_insert(Position {
            symbol: trade.symbol.clone(),
            quantity: 0.0,
            average_entry_price: 0.0,
            realized_pnl: 0.0,
            entry_fees: 0.0,
            fees_paid: 0.0,
            known_fee_fills: 0,
            unknown_fee_fills: 0,
            estimated_exit_fee_rate: 0.0,
            last_trade_timestamp: trade.timestamp,
        });
        let prior_realized = position.realized_pnl;
        apply_trade(position, trade);
        let realized_delta = position.realized_pnl - prior_realized;
        if realized_delta.abs() <= EPSILON {
            continue;
        }
        let quote = trade
            .symbol
            .split_once('-')
            .map(|(_, quote)| quote)
            .unwrap_or("USD")
            .to_owned();
        *by_quote.entry(quote).or_default() += realized_delta;
        if let Some(parent_order_id) = trade.parent_order_id.as_deref() {
            *by_parent_order.entry(parent_order_id.to_owned()).or_default() += realized_delta;
            if let Some(created_at) = barrier_created_at.get(parent_order_id) {
                *by_barrier_created_at.entry(created_at.clone()).or_default() += realized_delta;
            }
        }
    }
    RealizedPnlLedgerSnapshot {
        trade_count: ledger.trades.len(),
        barrier_count: ledger.barrier_orders.len(),
        source: None,
        since_ms: 0,
        today_by_quote: by_quote.clone(),
        today_by_parent_order: by_parent_order.clone(),
        today_by_barrier_created_at: by_barrier_created_at,
        all_time_by_parent_order: by_parent_order,
        all_time_by_quote: by_quote,
    }
}

#[derive(Deserialize)]

struct PositionSourceQuery {
    source: Option<String>,
}
#[derive(Clone)]
struct RealizedPnlLedgerSnapshot {
    trade_count: usize,
    barrier_count: usize,
    source: Option<String>,
    since_ms: u64,
    today_by_quote: BTreeMap<String, f64>,
    today_by_parent_order: BTreeMap<String, f64>,
    today_by_barrier_created_at: BTreeMap<String, f64>,
    all_time_by_parent_order: BTreeMap<String, f64>,
    all_time_by_quote: BTreeMap<String, f64>,
}

async fn get_realized_pnl(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<RealizedPnlQuery>,
) -> Result<Json<RealizedPnlResponse>, (StatusCode, String)> {
    if query
        .source
        .as_deref()
        .is_some_and(|source| !matches!(source, "manual" | "auto"))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Position source must be manual or auto".into(),
        ));
    }
    let since_ms = query.since_ms.unwrap_or(0);
    let (ledger, _, _, _) = snapshot_position_ledger(&store.ledger).await;
    let snapshot = {
        let mut cache = store
            .realized_pnl_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let is_current = cache.as_ref().is_some_and(|snapshot| {
            snapshot.trade_count == ledger.trades.len()
                && snapshot.barrier_count == ledger.barrier_orders.len()
                && snapshot.source == query.source
                && snapshot.since_ms == since_ms
        });
        if !is_current {
            let mut trades = ledger
                .trades
                .iter()
                .filter(|trade| {
                    query
                        .source
                        .as_deref()
                        .is_none_or(|source| trade.source == source)
                })
                .cloned()
                .collect::<Vec<_>>();
            let trade_count = ledger.trades.len();
            let barrier_count = ledger.barrier_orders.len();
            let barrier_created_at = ledger.barrier_orders.iter()
                .map(|barrier| (barrier.id.clone(), barrier.created_at.to_string()))
                .collect::<HashMap<_, _>>();
            drop(ledger);
            trades.sort_by_key(|trade| trade.timestamp);
            let mut positions = BTreeMap::<String, Position>::new();
            let mut today_by_quote = BTreeMap::<String, f64>::new();
            let mut today_by_parent_order = BTreeMap::<String, f64>::new();
            let mut today_by_barrier_created_at = BTreeMap::<String, f64>::new();
            let mut all_time_by_parent_order = BTreeMap::<String, f64>::new();
            let mut all_time_by_quote = BTreeMap::<String, f64>::new();
            for trade in &trades {
                let position = positions.entry(trade.symbol.clone()).or_insert(Position {
                    symbol: trade.symbol.clone(),
                    quantity: 0.0,
                    average_entry_price: 0.0,
                    realized_pnl: 0.0,
                    entry_fees: 0.0,
                    fees_paid: 0.0,
                    known_fee_fills: 0,
                    unknown_fee_fills: 0,
                    estimated_exit_fee_rate: 0.0,
                    last_trade_timestamp: trade.timestamp,
                });
                let prior_realized = position.realized_pnl;
                apply_trade(position, trade);
                let realized_delta = position.realized_pnl - prior_realized;
                if realized_delta.abs() <= EPSILON {
                    continue;
                }
                let quote = trade
                    .symbol
                    .split_once('-')
                    .map(|(_, quote)| quote)
                    .unwrap_or("USD");
                *all_time_by_quote.entry(quote.to_owned()).or_default() += realized_delta;
                if let Some(parent_order_id) = trade.parent_order_id.as_deref() {
                    *all_time_by_parent_order
                        .entry(parent_order_id.to_owned())
                        .or_default() += realized_delta;
                }
                if trade.timestamp >= since_ms {
                    *today_by_quote.entry(quote.to_owned()).or_default() += realized_delta;
                    if let Some(parent_order_id) = trade.parent_order_id.as_deref() {
                        *today_by_parent_order
                            .entry(parent_order_id.to_owned())
                            .or_default() += realized_delta;
                        if let Some(created_at) = barrier_created_at.get(parent_order_id) {
                            *today_by_barrier_created_at
                                .entry(created_at.clone())
                                .or_default() += realized_delta;
                        }
                    }
                }
            }
            *cache = Some(RealizedPnlLedgerSnapshot {
                trade_count,
                barrier_count,
                source: query.source.clone(),
                since_ms,
                today_by_quote,
                today_by_parent_order,
                today_by_barrier_created_at,
                all_time_by_parent_order,
                all_time_by_quote,
            });
        } else {
            drop(ledger);
        }
        cache
            .as_ref()
            .expect("realized P&L cache populated")
            .clone()
    };
    let rates = get_usd_rates().await?.0.rates;
    let convert_usd = |totals: &BTreeMap<String, f64>| -> Option<f64> {
        totals.iter().try_fold(0.0, |sum, (quote, amount)| {
            rates.get(quote).map(|rate| sum + amount * rate)
        })
    };
    Ok(Json(RealizedPnlResponse {
        realized_pnl_usd: convert_usd(&snapshot.today_by_quote),
        realized_pnl_by_quote: snapshot.today_by_quote,
        realized_pnl_by_parent_order: snapshot.today_by_parent_order,
        realized_pnl_by_barrier_created_at: snapshot.today_by_barrier_created_at,
        realized_pnl_all_time_by_parent_order: snapshot.all_time_by_parent_order,
        realized_pnl_all_time_usd: convert_usd(&snapshot.all_time_by_quote),
        realized_pnl_all_time_by_quote: snapshot.all_time_by_quote,
    }))
}


fn cached_position_summary(
    store: &PositionStore,
    ledger: &Ledger,
    source: Option<&str>,
) -> PositionsResponse {
    indexed_position_summary(store, ledger, source)
}

async fn get_positions(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PositionSourceQuery>,
) -> Result<Json<PositionsResponse>, (StatusCode, String)> {
    if query
        .source
        .as_deref()
        .is_some_and(|source| !matches!(source, "manual" | "auto"))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Position source must be manual or auto".into(),
        ));
    }
    let ledger = store.ledger.lock().await;
    let positions = cached_position_summary(&store, &ledger, query.source.as_deref());
    Ok(Json(positions))
}

#[cfg(test)]
mod financial_snapshot_tests {
    use super::*;

    fn test_store() -> PositionStore {
        let (state_updates, _) = broadcast::channel(8);
        PositionStore {
            path: PathBuf::new(),
            ledger: Arc::new(Mutex::new(Ledger::default())),
            persistence_notify: Arc::new(Notify::new()),
            persistence_write_lock: Arc::new(Mutex::new(())),
            pending_client_order_commits: StdMutex::new(HashSet::new()),
            state_updates,
            persistence_timings_ms: Arc::new(StdMutex::new(BTreeMap::new())),
            trade_index: StdMutex::new(LedgerTradeIndex::default()),
            realized_pnl_cache: StdMutex::new(None),
            market_marks: StdRwLock::new(HashMap::new()),
            market_mark_pollers: StdMutex::new(HashSet::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        }
    }

    fn trade(
        id: &str,
        side: TradeSide,
        quantity: f64,
        price: f64,
        timestamp: u64,
        fee: f64,
        fee_known: bool,
        parent_order_id: Option<&str>,
    ) -> PositionTrade {
        PositionTrade {
            id: id.to_owned(),
            fill_id: None,
            symbol: "BTC-USDT".to_owned(),
            side,
            quantity,
            execution_price: price,
            reference_price: None,
            timestamp,
            source: "auto".to_owned(),
            parent_order_id: parent_order_id.map(str::to_owned),
            client_order_id: None,
            lifecycle: None,
            fee,
            fee_known,
            fee_currency: if fee_known { "USDT".to_owned() } else { String::new() },
            fee_rate: 0.001,
            fee_liquidity: "taker".to_owned(),
            resting_order_id: None,
            decision_timestamp: None,
            decision_price: None,
            decision_mid_price: None,
            slippage: None,
            slippage_bps: None,
        }
    }

    #[test]
    fn composite_snapshot_uses_one_ledger_copy_and_preserves_fee_integrity() {
        let store = test_store();
        let mut ledger = Ledger::default();
        ledger.trades = vec![
            trade("fill-1", TradeSide::Buy, 1.0, 100.0, 2, -1.0, true, Some("order-1")),
            trade("fill-2", TradeSide::Sell, 1.0, 120.0, 1, -1.0, true, Some("order-2")),
            trade("fill-3", TradeSide::Buy, 0.5, 50.0, 3, 0.0, false, None),
        ];
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let fx_as_of = BTreeMap::from([("USDT".to_owned(), 40)]);

        let snapshot = build_financial_snapshot(&store, &ledger, &rates, fx_as_of, 42).unwrap();

        assert_eq!(snapshot.snapshot_consistency, "single_locked_ledger_clone");
        assert!(snapshot.ledger_snapshot_consistent);
        assert_eq!(snapshot.market_marks_consistency, "sampled_during_account_projection");
        assert_eq!(snapshot.captured_at_ms, 42);
        assert_eq!(snapshot.execution_mode, "paper");
        assert_eq!(snapshot.account.realized_pnl, 18.0);
        assert_eq!(snapshot.realized_pnl.realized_pnl_all_time_usd, Some(18.0));
        assert_eq!(snapshot.account.fees_paid, 2.0);
        assert_eq!(snapshot.fee_summary.fills, 3);
        assert_eq!(snapshot.ledger_coverage.trade_rows, 3);
        assert_eq!(snapshot.ledger_coverage.known_fee_fills, 2);
        assert_eq!(snapshot.ledger_coverage.unknown_fee_fills, 1);
        assert_eq!(snapshot.ledger_coverage.parent_order_linked_rows, 2);
        assert_eq!(snapshot.ledger_coverage.rows_without_parent_order_id, 1);
        assert_eq!(snapshot.ledger_coverage.timestamp_order_inversion_rows, 1);
        assert_eq!(snapshot.ledger_coverage.fee_currency_unknown_rows, 0);
        assert_eq!(snapshot.ledger_coverage.fee_cashflow_currency_mismatch_rows, 0);
        assert!(!snapshot.ledger_coverage.independent_execution_ledger_reconciled);

        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized["execution_mode"], "paper");
        assert_eq!(serialized["ledger_coverage"]["unknown_fee_fills"], 1);
        assert_eq!(serialized["realized_pnl"]["realized_pnl_all_time_by_quote"]["USDT"], 18.0);
    }

    #[test]
    fn replayed_realized_pnl_matches_account_ledger_append_order() {
        let store = test_store();
        let mut ledger = Ledger::default();
        ledger.trades = vec![
            trade("buy-1", TradeSide::Buy, 1.0, 100.0, 1, 0.0, true, Some("o1")),
            trade("buy-2", TradeSide::Buy, 1.0, 200.0, 2, 0.0, true, Some("o2")),
            trade("sell-1", TradeSide::Sell, 1.0, 150.0, 4, 0.0, true, Some("o3")),
            trade("buy-3", TradeSide::Buy, 1.0, 300.0, 3, 0.0, true, Some("o4")),
            trade("sell-2", TradeSide::Sell, 1.0, 250.0, 5, 0.0, true, Some("o5")),
        ];
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let snapshot = build_financial_snapshot(
            &store,
            &ledger,
            &rates,
            BTreeMap::from([("USDT".to_owned(), 40)]),
            42,
        )
        .unwrap();

        assert_eq!(snapshot.account.realized_pnl, 25.0);
        assert_eq!(snapshot.realized_pnl.realized_pnl_all_time_usd, Some(25.0));
        assert_eq!(snapshot.ledger_coverage.timestamp_order_inversion_rows, 1);
    }

    #[test]
    fn snapshot_counts_known_fee_rows_with_missing_or_mismatched_currency() {
        let store = test_store();
        let mut ledger = Ledger::default();
        let mut mismatch = trade("mismatch", TradeSide::Buy, 1.0, 100.0, 1, -1.0, true, None);
        mismatch.fee_currency = "BNB".to_owned();
        let mut unknown = trade("unknown", TradeSide::Buy, 1.0, 100.0, 2, -1.0, true, None);
        unknown.fee_currency.clear();
        ledger.trades = vec![mismatch, unknown];
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let snapshot = build_financial_snapshot(
            &store,
            &ledger,
            &rates,
            BTreeMap::from([("USDT".to_owned(), 40)]),
            42,
        )
        .unwrap();

        assert_eq!(snapshot.ledger_coverage.fee_cashflow_currency_mismatch_rows, 1);
        assert_eq!(snapshot.ledger_coverage.fee_currency_unknown_rows, 1);
    }
}
