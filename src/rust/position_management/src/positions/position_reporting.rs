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

async fn get_fee_summary(State(store): State<Arc<PositionStore>>) -> Json<FeeSummaryResponse> {
    let ledger = store.ledger.lock().await;
    Json(fee_summary_for_ledger(&ledger))
}

fn fee_summary_for_ledger(ledger: &Ledger) -> FeeSummaryResponse {
    let mut groups = BTreeMap::<(String, String, String), FeeSummaryRow>::new();
    let mut first_fill_ms: Option<u64> = None;
    let mut last_fill_ms: Option<u64> = None;
    for trade in &ledger.trades {
        let source = if trade.source.is_empty() {
            "unknown"
        } else {
            &trade.source
        }
        .to_owned();
        let liquidity = if trade.fee_liquidity.is_empty() {
            "unknown"
        } else {
            &trade.fee_liquidity
        }
        .to_owned();
        let fee_currency = if !trade.fee_known {
            "unknown".to_owned()
        } else if trade.fee_currency.is_empty() {
            trade
                .symbol
                .split_once('-')
                .map(|(_, quote)| quote)
                .unwrap_or("unknown")
                .to_owned()
        } else {
            trade.fee_currency.clone()
        };
        let row = groups
            .entry((source.clone(), liquidity.clone(), fee_currency.clone()))
            .or_insert(FeeSummaryRow {
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
        first_fill_ms =
            Some(first_fill_ms.map_or(trade.timestamp, |current| current.min(trade.timestamp)));
        last_fill_ms =
            Some(last_fill_ms.map_or(trade.timestamp, |current| current.max(trade.timestamp)));
    }
    let mut rows = groups.into_values().collect::<Vec<_>>();
    for row in &mut rows {
        row.effective_fee_bps = (row.traded_notional > 0.0 && row.unknown_fee_fills == 0)
            .then_some(row.fees_paid / row.traded_notional * 10_000.0);
    }
    rows.sort_by(|left, right| {
        left.source
            .cmp(&right.source)
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

fn ledger_evidence_epoch_id(allocator_epoch: Option<&str>) -> Option<String> {
    allocator_epoch.map(|epoch| format!("pm-ledger-evidence:v1:{epoch}"))
}

fn has_canonical_lifecycle_attribution(trade: &PositionTrade) -> bool {
    trade.lifecycle.as_ref().is_some_and(|link| {
        link.schema_version > 0
            && !link.opportunity_id.trim().is_empty()
            && !link.episode_id.trim().is_empty()
            && !link.trade_id.trim().is_empty()
            && !link.execution_mode.trim().is_empty()
            && !link.entry_policy.trim().is_empty()
            && !link.source.trim().is_empty()
    })
}

#[derive(Serialize)]
struct FinancialSnapshotLedgerCoverage {
    trade_rows: usize,
    ledger_evidence_epoch: Option<String>,
    ledger_evidence_epoch_status: &'static str,
    fills_in_evidence_epoch: usize,
    rows_without_fill_identity: usize,
    rows_with_fill_identity_outside_evidence_epoch: usize,
    clean_slate_eligible_rows: usize,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    fee_currency_missing_rows: usize,
    fee_currency_unknown_rows: usize,
    fee_cashflow_currency_mismatch_rows: usize,
    parent_order_linked_rows: usize,
    rows_without_parent_order_id: usize,
    canonical_lifecycle_linked_rows: usize,
    rows_without_canonical_lifecycle: usize,
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

#[derive(Deserialize)]
struct PortfolioEvidenceIntervalsQuery {
    after_ms: Option<u64>,
    after_cursor: Option<i64>,
    limit: Option<usize>,
}

async fn request_portfolio_evidence_baseline(
    State(store): State<Arc<PositionStore>>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let baseline = store
        .persist_portfolio_evidence_baseline()
        .await
        .map_err(internal_error)?;
    Ok(Json(baseline))
}

async fn get_portfolio_evidence_intervals(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PortfolioEvidenceIntervalsQuery>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let path = store.path.clone();
    let after_ms = query.after_ms.unwrap_or(0);
    let limit = query.limit.unwrap_or(250).clamp(1, 1_000);
    let query_limit = limit + 1;
    let result = tokio::task::spawn_blocking(move || -> anyhow::Result<Vec<(i64, PortfolioEvidenceIntervalV1)>> {
        if !path.exists() {
            return Ok(Vec::new());
        }
        let db = rusqlite::Connection::open(path)?;
        let table_exists: bool = db.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='portfolio_evidence_intervals')",
            [],
            |row| row.get(0),
        )?;
        if !table_exists {
            return Ok(Vec::new());
        }
        let after_cursor = query.after_cursor;
        let mut statement = if after_cursor.is_some() {
            db.prepare("SELECT rowid,payload FROM portfolio_evidence_intervals WHERE rowid > ?1 ORDER BY rowid LIMIT ?2")?
        } else {
            db.prepare("SELECT rowid,payload FROM portfolio_evidence_intervals WHERE observed_at_ms > ?1 ORDER BY rowid LIMIT ?2")?
        };
        let rows = statement.query_map(
            rusqlite::params![after_cursor.unwrap_or(i64::try_from(after_ms)?), i64::try_from(query_limit)?],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )?;
        rows.map(|row| {
            let (cursor, payload) = row?;
            Ok((cursor, serde_json::from_str::<PortfolioEvidenceIntervalV1>(&payload)?))
        }).collect()
    })
    .await
    .map_err(internal_error)?
    .map_err(internal_error)?;
    let mut result = result;
    let truncated = result.len() > limit;
    result.truncate(limit);
    let next_cursor = result.last().map(|(cursor, _)| *cursor);
    let intervals = result
        .into_iter()
        .map(|(_, interval)| interval)
        .collect::<Vec<_>>();
    let coverage = if intervals.is_empty() {
        "no_durable_checkpoint"
    } else if !truncated && intervals.iter().all(|interval| interval.complete) {
        "pm_internal_complete"
    } else if truncated {
        "truncated"
    } else {
        "incomplete_or_unknown"
    };
    Ok(Json(serde_json::json!({
        "schema_version": 1,
        "coverage": coverage,
        "quantitative_eligibility": "blocked_pending_independent_reconciliation",
        "truncated": truncated,
        "next_cursor": next_cursor,
        "intervals": intervals,
    })))
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
    let ledger_evidence_epoch = ledger_evidence_epoch_id(ledger.fill_id_epoch.as_deref());
    let in_evidence_epoch = |trade: &PositionTrade| {
        let (Some(epoch), Some(fill_id)) =
            (ledger.fill_id_epoch.as_deref(), trade.fill_id.as_deref())
        else {
            return false;
        };
        fill_id
            .strip_prefix(&format!("pm-fill:v1:{epoch}:"))
            .and_then(|sequence| sequence.parse::<u64>().ok())
            .is_some_and(|sequence| sequence > 0)
    };
    let fills_in_evidence_epoch = ledger
        .trades
        .iter()
        .filter(|trade| in_evidence_epoch(trade))
        .count();
    let rows_without_fill_identity = ledger
        .trades
        .iter()
        .filter(|trade| {
            trade
                .fill_id
                .as_deref()
                .is_none_or(|id| id.trim().is_empty())
        })
        .count();
    let rows_with_fill_identity_outside_evidence_epoch = ledger
        .trades
        .iter()
        .filter(|trade| {
            trade
                .fill_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
        })
        .filter(|trade| !in_evidence_epoch(trade))
        .count();
    let fee_currency_missing_rows = ledger
        .trades
        .iter()
        .filter(|trade| trade.fee_currency.trim().is_empty())
        .count();
    let parent_order_linked_rows = ledger
        .trades
        .iter()
        .filter(|trade| {
            trade
                .parent_order_id
                .as_deref()
                .is_some_and(|id| !id.trim().is_empty())
        })
        .count();
    let canonical_lifecycle_linked_rows = ledger
        .trades
        .iter()
        .filter(|trade| has_canonical_lifecycle_attribution(trade))
        .count();
    let clean_slate_eligible_rows = ledger
        .trades
        .iter()
        .filter(|trade| {
            in_evidence_epoch(trade)
                && trade.fee_known
                && !trade.fee_currency.trim().is_empty()
                && trade
                    .symbol
                    .split_once('-')
                    .is_some_and(|(_, quote)| trade.fee_currency == quote)
                && trade
                    .parent_order_id
                    .as_deref()
                    .is_some_and(|id| !id.trim().is_empty())
                && has_canonical_lifecycle_attribution(trade)
        })
        .count();
    let ledger_evidence_epoch_status = if ledger.trades.is_empty() {
        if ledger.fill_id_epoch.is_some() {
            "initialized_empty"
        } else {
            "uninitialized_empty"
        }
    } else if fills_in_evidence_epoch == 0 {
        "legacy_or_unknown"
    } else if fills_in_evidence_epoch == ledger.trades.len() {
        "clean_slate"
    } else {
        "mixed_legacy_and_current"
    };
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
            ledger_evidence_epoch,
            ledger_evidence_epoch_status,
            fills_in_evidence_epoch,
            rows_without_fill_identity,
            rows_with_fill_identity_outside_evidence_epoch,
            clean_slate_eligible_rows,
            known_fee_fills,
            unknown_fee_fills: ledger.trades.len().saturating_sub(known_fee_fills),
            fee_currency_missing_rows,
            fee_currency_unknown_rows,
            fee_cashflow_currency_mismatch_rows,
            parent_order_linked_rows,
            rows_without_parent_order_id: ledger
                .trades
                .len()
                .saturating_sub(parent_order_linked_rows),
            canonical_lifecycle_linked_rows,
            rows_without_canonical_lifecycle: ledger
                .trades
                .len()
                .saturating_sub(canonical_lifecycle_linked_rows),
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
            *by_parent_order
                .entry(parent_order_id.to_owned())
                .or_default() += realized_delta;
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
            let barrier_created_at = ledger
                .barrier_orders
                .iter()
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
            paper_exchange_schema_version: None,
            paper_exchange_source_epoch: None,
            paper_exchange_source_cursor: None,
            paper_exchange_request_fingerprint: None,
            paper_exchange_execution_id: None,
            paper_exchange_source_quantity: None,
            paper_exchange_source_price: None,
            paper_exchange_source_complete: None,
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
            fee_currency: if fee_known {
                "USDT".to_owned()
            } else {
                String::new()
            },
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

    fn complete_lifecycle_link() -> CanonicalLifecycleLink {
        CanonicalLifecycleLink {
            schema_version: 1,
            opportunity_id: "opportunity-1".to_owned(),
            episode_id: "episode-1".to_owned(),
            trade_id: "trade-1".to_owned(),
            profile_id: Some("profile-1".to_owned()),
            profile_hash: Some("hash-1".to_owned()),
            profile_version: Some(1),
            execution_mode: "paper".to_owned(),
            entry_policy: "limit".to_owned(),
            source: "strategy".to_owned(),
        }
    }

    #[test]
    fn composite_snapshot_uses_one_ledger_copy_and_preserves_fee_integrity() {
        let store = test_store();
        let mut ledger = Ledger::default();
        ledger.trades = vec![
            trade(
                "fill-1",
                TradeSide::Buy,
                1.0,
                100.0,
                2,
                -1.0,
                true,
                Some("order-1"),
            ),
            trade(
                "fill-2",
                TradeSide::Sell,
                1.0,
                120.0,
                1,
                -1.0,
                true,
                Some("order-2"),
            ),
            trade("fill-3", TradeSide::Buy, 0.5, 50.0, 3, 0.0, false, None),
        ];
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let fx_as_of = BTreeMap::from([("USDT".to_owned(), 40)]);

        let snapshot = build_financial_snapshot(&store, &ledger, &rates, fx_as_of, 42).unwrap();

        assert_eq!(snapshot.snapshot_consistency, "single_locked_ledger_clone");
        assert!(snapshot.ledger_snapshot_consistent);
        assert_eq!(
            snapshot.market_marks_consistency,
            "sampled_during_account_projection"
        );
        assert_eq!(snapshot.captured_at_ms, 42);
        assert_eq!(snapshot.execution_mode, "paper");
        assert_eq!(snapshot.account.realized_pnl, 18.0);
        assert_eq!(snapshot.realized_pnl.realized_pnl_all_time_usd, Some(18.0));
        assert_eq!(snapshot.account.fees_paid, 2.0);
        assert_eq!(snapshot.fee_summary.fills, 3);
        assert_eq!(snapshot.ledger_coverage.trade_rows, 3);
        assert_eq!(
            snapshot.ledger_coverage.ledger_evidence_epoch_status,
            "legacy_or_unknown"
        );
        assert_eq!(snapshot.ledger_coverage.fills_in_evidence_epoch, 0);
        assert_eq!(snapshot.ledger_coverage.rows_without_fill_identity, 3);
        assert_eq!(snapshot.ledger_coverage.clean_slate_eligible_rows, 0);
        assert_eq!(snapshot.ledger_coverage.known_fee_fills, 2);
        assert_eq!(snapshot.ledger_coverage.unknown_fee_fills, 1);
        assert_eq!(snapshot.ledger_coverage.parent_order_linked_rows, 2);
        assert_eq!(snapshot.ledger_coverage.rows_without_parent_order_id, 1);
        assert_eq!(snapshot.ledger_coverage.timestamp_order_inversion_rows, 1);
        assert_eq!(snapshot.ledger_coverage.fee_currency_unknown_rows, 0);
        assert_eq!(
            snapshot.ledger_coverage.fee_cashflow_currency_mismatch_rows,
            0
        );
        assert!(
            !snapshot
                .ledger_coverage
                .independent_execution_ledger_reconciled
        );

        let serialized = serde_json::to_value(snapshot).unwrap();
        assert_eq!(serialized["execution_mode"], "paper");
        assert_eq!(serialized["ledger_coverage"]["unknown_fee_fills"], 1);
        assert_eq!(
            serialized["ledger_coverage"]["ledger_evidence_epoch_status"],
            "legacy_or_unknown"
        );
        assert_eq!(
            serialized["realized_pnl"]["realized_pnl_all_time_by_quote"]["USDT"],
            18.0
        );
    }

    #[test]
    fn evidence_epoch_separates_empty_new_legacy_mixed_and_restarted_ledgers() {
        let store = test_store();
        let rates = BTreeMap::from([("USDT".to_owned(), 1.0)]);
        let fx_as_of = BTreeMap::from([("USDT".to_owned(), 40)]);

        let empty =
            build_financial_snapshot(&store, &Ledger::default(), &rates, fx_as_of.clone(), 42)
                .unwrap();
        assert_eq!(
            empty.ledger_coverage.ledger_evidence_epoch_status,
            "uninitialized_empty"
        );
        assert_eq!(empty.ledger_coverage.ledger_evidence_epoch, None);
        assert_eq!(empty.ledger_coverage.clean_slate_eligible_rows, 0);

        let mut current = Ledger::default();
        let current_fill_id = allocate_fill_id(&mut current, 100);
        let mut current_trade = trade(
            "row-current",
            TradeSide::Buy,
            1.0,
            100.0,
            100,
            -0.1,
            true,
            Some("order-current"),
        );
        current_trade.fill_id = Some(current_fill_id.clone());
        current_trade.lifecycle = Some(complete_lifecycle_link());
        current.trades.push(current_trade);

        let fresh =
            build_financial_snapshot(&store, &current, &rates, fx_as_of.clone(), 43).unwrap();
        assert_eq!(
            fresh.ledger_coverage.ledger_evidence_epoch_status,
            "clean_slate"
        );
        assert_eq!(fresh.ledger_coverage.fills_in_evidence_epoch, 1);
        assert_eq!(fresh.ledger_coverage.rows_without_fill_identity, 0);
        assert_eq!(fresh.ledger_coverage.clean_slate_eligible_rows, 1);
        let evidence_epoch = fresh.ledger_coverage.ledger_evidence_epoch.clone().unwrap();
        assert!(evidence_epoch.starts_with("pm-ledger-evidence:v1:"));

        // The allocator epoch and sequence are serialized in the ledger itself.
        // A restart preserves the public evidence epoch and cohort membership.
        let restarted: Ledger =
            serde_json::from_slice(&serde_json::to_vec(&current).unwrap()).unwrap();
        let after_restart =
            build_financial_snapshot(&store, &restarted, &rates, fx_as_of.clone(), 44).unwrap();
        assert_eq!(
            after_restart.ledger_coverage.ledger_evidence_epoch,
            Some(evidence_epoch)
        );
        assert_eq!(after_restart.ledger_coverage.fills_in_evidence_epoch, 1);
        assert_eq!(after_restart.ledger_coverage.clean_slate_eligible_rows, 1);

        let mut legacy = Ledger::default();
        let mut legacy_trade = trade(
            "row-legacy",
            TradeSide::Buy,
            1.0,
            90.0,
            90,
            0.0,
            false,
            None,
        );
        legacy_trade.fee_currency.clear();
        legacy.trades.push(legacy_trade);
        let historical =
            build_financial_snapshot(&store, &legacy, &rates, fx_as_of.clone(), 45).unwrap();
        assert_eq!(
            historical.ledger_coverage.ledger_evidence_epoch_status,
            "legacy_or_unknown"
        );
        assert_eq!(historical.ledger_coverage.rows_without_fill_identity, 1);
        assert_eq!(historical.ledger_coverage.fee_currency_missing_rows, 1);
        assert_eq!(historical.ledger_coverage.clean_slate_eligible_rows, 0);

        let mut mixed = restarted;
        mixed.trades.push(legacy.trades.remove(0));
        let mut foreign_epoch = trade(
            "row-foreign",
            TradeSide::Buy,
            0.5,
            95.0,
            95,
            -0.05,
            true,
            Some("order-foreign"),
        );
        foreign_epoch.fill_id = Some("pm-fill:v1:prior-epoch:7".to_owned());
        foreign_epoch.lifecycle = Some(complete_lifecycle_link());
        mixed.trades.push(foreign_epoch);
        let mixed_snapshot =
            build_financial_snapshot(&store, &mixed, &rates, fx_as_of, 46).unwrap();
        assert_eq!(
            mixed_snapshot.ledger_coverage.ledger_evidence_epoch_status,
            "mixed_legacy_and_current"
        );
        assert_eq!(mixed_snapshot.ledger_coverage.fills_in_evidence_epoch, 1);
        assert_eq!(mixed_snapshot.ledger_coverage.rows_without_fill_identity, 1);
        assert_eq!(
            mixed_snapshot
                .ledger_coverage
                .rows_with_fill_identity_outside_evidence_epoch,
            1
        );
        assert_eq!(mixed_snapshot.ledger_coverage.clean_slate_eligible_rows, 1);
    }

    #[test]
    fn replayed_realized_pnl_matches_account_ledger_append_order() {
        let store = test_store();
        let mut ledger = Ledger::default();
        ledger.trades = vec![
            trade(
                "buy-1",
                TradeSide::Buy,
                1.0,
                100.0,
                1,
                0.0,
                true,
                Some("o1"),
            ),
            trade(
                "buy-2",
                TradeSide::Buy,
                1.0,
                200.0,
                2,
                0.0,
                true,
                Some("o2"),
            ),
            trade(
                "sell-1",
                TradeSide::Sell,
                1.0,
                150.0,
                4,
                0.0,
                true,
                Some("o3"),
            ),
            trade(
                "buy-3",
                TradeSide::Buy,
                1.0,
                300.0,
                3,
                0.0,
                true,
                Some("o4"),
            ),
            trade(
                "sell-2",
                TradeSide::Sell,
                1.0,
                250.0,
                5,
                0.0,
                true,
                Some("o5"),
            ),
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

        assert_eq!(
            snapshot.ledger_coverage.fee_cashflow_currency_mismatch_rows,
            1
        );
        assert_eq!(snapshot.ledger_coverage.fee_currency_unknown_rows, 1);
    }

    #[tokio::test]
    async fn portfolio_interval_cursor_pages_same_millisecond_rows_without_gaps() {
        let path = std::env::temp_dir().join(format!(
            "pm-portfolio-interval-page-{}-{}.sqlite",
            std::process::id(),
            now_ms().unwrap_or_default(),
        ));
        let mut store = test_store();
        store.path = path.clone();
        let store = Arc::new(store);
        let mut ledger = Ledger {
            initial_cash: 10_000.0,
            fill_id_epoch: Some("page-epoch".to_owned()),
            ..Ledger::default()
        };
        save_sqlite_ledger_at(&path, &ledger, 1_000, true).unwrap();
        ledger.topups.push(AccountTopup {
            amount: 25.0,
            timestamp: 900,
            ..AccountTopup::default()
        });
        save_sqlite_ledger_at(&path, &ledger, 1_000, true).unwrap();

        let Json(first) = get_portfolio_evidence_intervals(
            State(store.clone()),
            Query(PortfolioEvidenceIntervalsQuery {
                after_ms: None,
                after_cursor: None,
                limit: Some(1),
            }),
        )
        .await
        .unwrap();
        assert_eq!(first["truncated"], true);
        let first_id = first["intervals"][0]["evidence_id"].as_str().unwrap();
        let cursor = first["next_cursor"].as_i64().unwrap();

        let Json(second) = get_portfolio_evidence_intervals(
            State(store),
            Query(PortfolioEvidenceIntervalsQuery {
                after_ms: None,
                after_cursor: Some(cursor),
                limit: Some(1),
            }),
        )
        .await
        .unwrap();
        assert_eq!(second["truncated"], false);
        assert_ne!(second["intervals"][0]["evidence_id"], first_id);
        assert_eq!(second["intervals"][0]["interval_end_utc_ms"], 1_000);

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[tokio::test]
    async fn portfolio_interval_api_preserves_currency_fx_and_separate_contribution_evidence() {
        let path = std::env::temp_dir().join(format!(
            "pm-portfolio-interval-currency-{}-{}.sqlite",
            std::process::id(),
            now_ms().unwrap_or_default(),
        ));
        let mut store = test_store();
        store.path = path.clone();
        let store = Arc::new(store);
        let ledger = Ledger {
            initial_cash: 10_000.0,
            fill_id_epoch: Some("currency-epoch".to_owned()),
            ..Ledger::default()
        };
        save_sqlite_ledger_at(&path, &ledger, 2_000, true).unwrap();

        // Seed the persisted contract as if the market feed and independent
        // evidence collector had already supplied these sourced FX marks and
        // contribution references. The endpoint must return the exact values,
        // without folding external capital into trading cashflows.
        let db = rusqlite::Connection::open(&path).unwrap();
        let mut payload: serde_json::Value = db
            .query_row(
                "SELECT payload FROM portfolio_evidence_intervals ORDER BY rowid DESC LIMIT 1",
                [],
                |row| row.get::<_, String>(0),
            )
            .map(|payload| serde_json::from_str(&payload).unwrap())
            .unwrap();
        payload["opening_cash_balances_by_currency"] = serde_json::json!({
            "USD": 10_000.0,
            "EUR": 2.0,
        });
        payload["opening_cash_value_usd_by_currency"] = serde_json::json!({
            "USD": 10_000.0,
            "EUR": 2.2,
        });
        payload["opening_fx_rates_usd_by_currency"] = serde_json::json!({
            "USD": 1.0,
            "EUR": 1.1,
        });
        payload["opening_fx_rates_updated_at_by_currency"] = serde_json::json!({
            "USD": 0,
            "EUR": 1_900,
        });
        payload["closing_cash_value_usd_by_currency"] = serde_json::json!({
            "USD": 10_010.0,
            "EUR": 2.4,
        });
        payload["closing_fx_rates_usd_by_currency"] = serde_json::json!({
            "USD": 1.0,
            "EUR": 1.2,
        });
        payload["closing_fx_rates_updated_at_by_currency"] = serde_json::json!({
            "USD": 0,
            "EUR": 1_990,
        });
        payload["cash_movements_by_currency"] = serde_json::json!({"USD": 0.0});
        payload["external_contributions_by_currency"] = serde_json::json!({"USD": 5.0});
        payload["external_contribution_events"] = serde_json::json!([{
            "contribution_id": "pm-topup:v1:0",
            "source": "position_manager_account_topup_ledger",
            "source_cursor": 0,
            "timestamp_utc_ms": 1_950,
            "currency": "USD",
            "amount": 5.0,
            "value_usd": 5.0,
            "independent_source_record_id": "operator-contribution:deposit-1",
            "independent_source_epoch": "pex-epoch-1",
            "independent_source_cursor": 7,
        }]);
        db.execute(
            "UPDATE portfolio_evidence_intervals SET payload = ?1",
            [payload.to_string()],
        )
        .unwrap();
        drop(db);

        let Json(response) = get_portfolio_evidence_intervals(
            State(store),
            Query(PortfolioEvidenceIntervalsQuery {
                after_ms: None,
                after_cursor: None,
                limit: Some(10),
            }),
        )
        .await
        .unwrap();
        let interval = &response["intervals"][0];
        assert_eq!(interval["opening_cash_balances_by_currency"]["EUR"], 2.0);
        assert_eq!(interval["opening_cash_value_usd_by_currency"]["EUR"], 2.2);
        assert_eq!(
            interval["opening_fx_rates_updated_at_by_currency"]["EUR"],
            1_900
        );
        assert_eq!(interval["closing_cash_value_usd_by_currency"]["EUR"], 2.4);
        assert_eq!(
            interval["closing_fx_rates_updated_at_by_currency"]["EUR"],
            1_990
        );
        assert_eq!(interval["cash_movements_by_currency"]["USD"], 0.0);
        assert_eq!(interval["external_contributions_by_currency"]["USD"], 5.0);
        assert_eq!(
            interval["external_contribution_events"][0]["independent_source_record_id"],
            "operator-contribution:deposit-1"
        );
        assert_eq!(
            interval["external_contribution_events"][0]["independent_source_cursor"],
            7
        );
        assert_eq!(
            response["quantitative_eligibility"],
            "blocked_pending_independent_reconciliation"
        );

        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }
}
