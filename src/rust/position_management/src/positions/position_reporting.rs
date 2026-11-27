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

fn net_unrealized_quote(
    gross: Option<f64>,
    entry_fee_cashflow: f64,
    estimated_close_fee: Option<f64>,
    fees_complete: bool,
) -> Option<f64> {
    let (Some(gross), Some(close_fee)) = (gross, estimated_close_fee) else {
        return None;
    };
    (fees_complete && gross.is_finite() && entry_fee_cashflow.is_finite() && close_fee.is_finite())
        .then_some(gross + entry_fee_cashflow - close_fee)
}

fn executable_breakeven_price(
    quantity: f64,
    entry_price: f64,
    entry_fee_cashflow: f64,
    estimated_close_fee_rate: f64,
    fees_complete: bool,
) -> Option<f64> {
    let qty = quantity.abs();
    let rate = estimated_close_fee_rate.abs();
    if !fees_complete
        || !quantity.is_finite()
        || qty <= EPSILON
        || !entry_price.is_finite()
        || entry_price <= 0.0
        || !entry_fee_cashflow.is_finite()
        || !rate.is_finite()
        || rate >= 1.0
    {
        return None;
    }
    let price = if quantity > 0.0 {
        (entry_price * qty - entry_fee_cashflow) / (qty * (1.0 - rate))
    } else {
        (entry_price * qty + entry_fee_cashflow) / (qty * (1.0 + rate))
    };
    (price.is_finite() && price > 0.0).then_some(price)
}

struct ActivePositionIdentity {
    position_identity: Option<String>,
    canonical_trade_ids: Vec<String>,
    parent_order_ids: Vec<String>,
    sources: Vec<String>,
    entry_fill_ids: Vec<String>,
    all_entry_fills_known: bool,
    entry_fee_currency_is_quote: bool,
}

fn barrier_matches_active_position(
    barrier_symbol: &str,
    barrier_source: &str,
    barrier_trade_id: Option<&str>,
    barrier_parent_order_id: Option<&str>,
    position_symbol: &str,
    source_filter: Option<&str>,
    identity: &ActivePositionIdentity,
) -> bool {
    barrier_symbol == position_symbol
        && source_filter.map_or_else(
            || {
                identity
                    .sources
                    .iter()
                    .any(|source| source == barrier_source)
            },
            |source| source == barrier_source,
        )
        && (barrier_trade_id.is_some_and(|id| {
            identity
                .canonical_trade_ids
                .iter()
                .any(|current| current == id)
        }) || barrier_parent_order_id.is_some_and(|id| {
            identity
                .parent_order_ids
                .iter()
                .any(|current| current == id)
        }))
}

fn active_position_fill_identities(
    trades: &[PositionTrade],
    symbol: &str,
    source: Option<&str>,
) -> ActivePositionIdentity {
    struct ActiveLot<'a> {
        signed_quantity: f64,
        fill_id: Option<String>,
        trade_id: Option<String>,
        parent_order_id: Option<String>,
        source: String,
        fee_known: bool,
        fee_currency: &'a str,
    }
    let mut lots: Vec<ActiveLot<'_>> = Vec::new();
    for trade in trades.iter().filter(|trade| {
        trade.symbol == symbol && source.is_none_or(|source| trade.source == source)
    }) {
        let incoming_sign = if matches!(&trade.side, TradeSide::Buy) {
            1.0
        } else {
            -1.0
        };
        let mut remaining = trade.quantity.abs();
        while remaining > EPSILON
            && lots
                .first()
                .is_some_and(|lot| lot.signed_quantity.signum() != incoming_sign)
        {
            let old_remaining = lots[0].signed_quantity.abs();
            let closed = old_remaining.min(remaining);
            remaining -= closed;
            if old_remaining - closed <= EPSILON {
                lots.remove(0);
            } else {
                lots[0].signed_quantity =
                    lots[0].signed_quantity.signum() * (old_remaining - closed);
            }
        }
        if remaining > EPSILON {
            lots.push(ActiveLot {
                signed_quantity: incoming_sign * remaining,
                fill_id: trade.fill_id.clone(),
                trade_id: trade
                    .lifecycle
                    .as_ref()
                    .map(|lifecycle| lifecycle.trade_id.clone()),
                parent_order_id: trade.parent_order_id.clone(),
                source: trade.source.clone(),
                fee_known: trade.fee_known,
                fee_currency: &trade.fee_currency,
            });
        }
    }
    let fill_ids = lots
        .iter()
        .filter_map(|lot| lot.fill_id.clone())
        .collect::<Vec<_>>();
    let identity = (!lots.is_empty() && fill_ids.len() == lots.len())
        .then(|| format!("pm-position:v1:{}", fill_ids.join(",")));
    let canonical_ids = lots
        .iter()
        .filter_map(|lot| lot.trade_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let parent_order_ids = lots
        .iter()
        .filter_map(|lot| lot.parent_order_id.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let sources = lots
        .iter()
        .map(|lot| lot.source.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let all_entry_fills_known = !lots.is_empty() && lots.iter().all(|lot| lot.fee_known);
    let entry_fee_currency_is_quote = !lots.is_empty()
        && lots.iter().all(|lot| {
            symbol
                .split_once('-')
                .is_some_and(|(_, quote)| lot.fee_currency == quote)
        });
    ActivePositionIdentity {
        position_identity: identity,
        canonical_trade_ids: canonical_ids,
        parent_order_ids,
        sources,
        entry_fill_ids: fill_ids,
        all_entry_fills_known,
        entry_fee_currency_is_quote,
    }
}

#[derive(Clone)]
struct OperationalFeeSchedule {
    symbol: String,
    ledger_epoch: String,
    ledger_high_water_cursor: u64,
    id: String,
    version: String,
    source_epoch: String,
    source_cursor: u64,
    source_record_id: String,
    source_timestamp_ms: u64,
    effective_from_ms: u64,
    effective_to_ms: Option<u64>,
    taker_rate: f64,
}

fn valid_exit_fee_schedule(
    schedule: Option<&OperationalFeeSchedule>,
    symbol: &str,
    at_ms: u64,
) -> bool {
    schedule.is_some_and(|schedule| {
        schedule.symbol == symbol
            && !schedule.ledger_epoch.trim().is_empty()
            && schedule.source_cursor <= schedule.ledger_high_water_cursor
            && !schedule.id.trim().is_empty()
            && !schedule.version.trim().is_empty()
            && !schedule.source_epoch.trim().is_empty()
            && schedule.source_cursor > 0
            && !schedule.source_record_id.trim().is_empty()
            && schedule.source_timestamp_ms <= at_ms
            && schedule.effective_from_ms > 0
            && schedule.effective_from_ms <= at_ms
            && schedule.effective_to_ms.is_none_or(|end| at_ms < end)
            && schedule.taker_rate.is_finite()
            && (0.0..=0.1).contains(&schedule.taker_rate)
    })
}

fn active_entry_fee_evidence_valid(
    trade: &PositionTrade,
    symbol: &str,
    quote_currency: &str,
) -> bool {
    let Some(event) = trade.paper_exchange_fee_schedule_evidence.as_ref() else {
        return false;
    };
    let payload_symbol = event.get("symbol").and_then(serde_json::Value::as_str);
    let schedule_id = event
        .get("fee_schedule_id")
        .and_then(serde_json::Value::as_str);
    let schedule_version = event
        .get("fee_schedule_version")
        .and_then(serde_json::Value::as_str);
    let schedule_time = event
        .get("source_timestamp_utc_ms")
        .and_then(serde_json::Value::as_u64);
    let effective_from = event
        .get("effective_from_utc_ms")
        .and_then(serde_json::Value::as_u64);
    let effective_to = event
        .get("effective_to_utc_ms")
        .and_then(serde_json::Value::as_u64);
    let source_epoch = event
        .get("source_epoch")
        .and_then(serde_json::Value::as_str);
    let source_record_id = event
        .get("source_record_id")
        .and_then(serde_json::Value::as_str);
    let source_cursor = event
        .get("source_cursor")
        .and_then(serde_json::Value::as_u64);
    let order_class = trade.paper_exchange_fee_order_class.as_deref();
    let independent_fee = trade.paper_exchange_fee_amount;
    let independent_currency = trade.paper_exchange_fee_currency.as_deref();
    event.get("event_type").and_then(serde_json::Value::as_str) == Some("fee_schedule")
        && payload_symbol == Some(symbol)
        && schedule_id == trade.paper_exchange_fee_schedule_id.as_deref()
        && schedule_version == trade.paper_exchange_fee_schedule_version.as_deref()
        && schedule_time.is_some_and(|time| time <= trade.timestamp)
        && effective_from.is_some_and(|start| start > 0 && start <= trade.timestamp)
        && effective_to.is_none_or(|end| trade.timestamp < end)
        && source_epoch.is_some_and(|value| !value.trim().is_empty())
        && source_record_id.is_some_and(|value| !value.trim().is_empty())
        && source_cursor.is_some_and(|cursor| cursor > 0)
        && matches!(order_class, Some("maker" | "taker"))
        && trade.fee_known
        && trade.fee_currency == quote_currency
        && independent_currency == Some(quote_currency)
        && independent_fee.is_some_and(|fee| {
            fee.is_finite() && (fee.abs() - trade.fee.abs()).abs() <= 1e-8_f64.max(fee.abs() * 1e-8)
        })
}

#[derive(Clone)]
struct CachedOperationalFeeSchedule {
    cached_at_ms: u64,
    schedule: Option<OperationalFeeSchedule>,
}

static OPERATIONAL_FEE_SCHEDULE_CACHE: std::sync::OnceLock<
    tokio::sync::Mutex<std::collections::HashMap<String, CachedOperationalFeeSchedule>>,
> = std::sync::OnceLock::new();

async fn current_operational_fee_schedules(
    symbols: &[String],
    at_ms: u64,
) -> std::collections::HashMap<String, Option<OperationalFeeSchedule>> {
    const CACHE_TTL_MS: u64 = 10_000;
    const NEGATIVE_CACHE_TTL_MS: u64 = 1_000;
    const MAX_CACHE_SYMBOLS: usize = 256;
    let cache = OPERATIONAL_FEE_SCHEDULE_CACHE
        .get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()));
    let mut result = std::collections::HashMap::new();
    let mut uncached = Vec::new();
    {
        let cache = cache.lock().await;
        for symbol in symbols {
            if let Some(cached) = cache.get(symbol) {
                let ttl = if cached.schedule.is_some() {
                    CACHE_TTL_MS
                } else {
                    NEGATIVE_CACHE_TTL_MS
                };
                if at_ms.saturating_sub(cached.cached_at_ms) <= ttl
                    && cached.schedule.as_ref().is_none_or(|schedule| {
                        valid_exit_fee_schedule(Some(schedule), symbol, at_ms)
                    })
                {
                    result.insert(symbol.clone(), cached.schedule.clone());
                    continue;
                }
            }
            uncached.push(symbol.clone());
        }
    }
    let fetched = if uncached.is_empty() {
        Some(std::collections::HashMap::new())
    } else {
        let path = format!(
            "/internal/financial/fee-schedules?symbols={}&at_utc_ms={at_ms}",
            uncached.join(",")
        );
        let snapshot = paper_exchange_client::paper_exchange_get(&path).await.ok();
        snapshot.and_then(|snapshot| {
            if snapshot
                .get("captured_at_utc_ms")
                .and_then(serde_json::Value::as_u64)
                != Some(at_ms)
            {
                return None;
            }
            let ledger_epoch = snapshot
                .get("ledger_epoch")
                .and_then(serde_json::Value::as_str)?;
            if ledger_epoch.trim().is_empty() {
                return None;
            }
            let high_water = snapshot
                .get("observed_max_cursor")
                .and_then(serde_json::Value::as_u64)?;
            let entries = snapshot
                .get("schedules")
                .and_then(serde_json::Value::as_array)?;
            let mut fetched = std::collections::HashMap::new();
            for entry in entries {
                let symbol = entry.get("symbol").and_then(serde_json::Value::as_str)?;
                if !uncached.iter().any(|requested| requested == symbol) {
                    continue;
                }
                let schedule =
                    if entry.get("status").and_then(serde_json::Value::as_str) != Some("matched") {
                        None
                    } else {
                        let (
                            Some(id),
                            Some(version),
                            Some(source_epoch),
                            Some(source_cursor),
                            Some(source_record_id),
                            Some(source_timestamp_ms),
                            Some(effective_from_ms),
                            Some(taker_rate),
                        ) = (
                            entry
                                .get("fee_schedule_id")
                                .and_then(serde_json::Value::as_str),
                            entry
                                .get("fee_schedule_version")
                                .and_then(serde_json::Value::as_str),
                            entry
                                .get("source_epoch")
                                .and_then(serde_json::Value::as_str),
                            entry
                                .get("source_cursor")
                                .and_then(serde_json::Value::as_u64),
                            entry
                                .get("source_record_id")
                                .and_then(serde_json::Value::as_str),
                            entry
                                .get("source_timestamp_utc_ms")
                                .and_then(serde_json::Value::as_u64),
                            entry
                                .get("effective_from_utc_ms")
                                .and_then(serde_json::Value::as_u64),
                            entry.get("taker_rate").and_then(serde_json::Value::as_f64),
                        )
                        else {
                            fetched.insert(symbol.to_owned(), None);
                            continue;
                        };
                        let effective_to_ms = entry
                            .get("effective_to_utc_ms")
                            .and_then(serde_json::Value::as_u64);
                        if source_epoch.trim().is_empty()
                            || source_cursor == 0
                            || source_cursor > high_water
                            || id.trim().is_empty()
                            || version.trim().is_empty()
                            || source_record_id.trim().is_empty()
                            || source_timestamp_ms == 0
                            || source_timestamp_ms > at_ms
                            || effective_from_ms == 0
                            || !taker_rate.is_finite()
                            || !(0.0..=0.1).contains(&taker_rate.abs())
                        {
                            None
                        } else {
                            Some(OperationalFeeSchedule {
                                symbol: symbol.to_owned(),
                                ledger_epoch: ledger_epoch.to_owned(),
                                ledger_high_water_cursor: high_water,
                                id: id.to_owned(),
                                version: version.to_owned(),
                                source_epoch: source_epoch.to_owned(),
                                source_cursor,
                                source_record_id: source_record_id.to_owned(),
                                source_timestamp_ms,
                                effective_from_ms,
                                effective_to_ms,
                                taker_rate: taker_rate.abs(),
                            })
                        }
                    };
                fetched.insert(symbol.to_owned(), schedule);
            }
            for symbol in &uncached {
                fetched.entry(symbol.clone()).or_insert(None);
            }
            Some(fetched)
        })
    };
    let fetched = fetched.unwrap_or_else(|| {
        uncached
            .iter()
            .cloned()
            .map(|symbol| (symbol, None))
            .collect()
    });
    let mut cache = cache.lock().await;
    for symbol in &uncached {
        let schedule = fetched.get(symbol).cloned().flatten();
        if cache.len() >= MAX_CACHE_SYMBOLS && !cache.contains_key(symbol) {
            if let Some(oldest) = cache
                .iter()
                .min_by_key(|(_, value)| value.cached_at_ms)
                .map(|(key, _)| key.clone())
            {
                cache.remove(&oldest);
            }
        }
        cache.insert(
            symbol.clone(),
            CachedOperationalFeeSchedule {
                cached_at_ms: at_ms,
                schedule: schedule.clone(),
            },
        );
        result.insert(symbol.clone(), schedule);
    }
    result
}

#[derive(Serialize)]
struct OperationalPositionResponse {
    position_identity: Option<String>,
    canonical_trade_ids: Vec<String>,
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    realized_pnl: f64,
    entry_fees: f64,
    fees_paid: f64,
    last_trade_timestamp: u64,
    quote_currency: Option<String>,
    entry_value_quote: Option<f64>,
    entry_value_usd: Option<f64>,
    quote_fx_rate: Option<f64>,
    quote_fx_updated_at_ms: Option<u64>,
    quote_fx_source_record_id: Option<String>,
    executable_close_side: Option<String>,
    executable_mark: Option<f64>,
    mark_age_ms: Option<u64>,
    mark_source_system: Option<String>,
    mark_source_epoch: Option<String>,
    mark_source_cursor: Option<u64>,
    mark_source_record_id: Option<String>,
    mark_updated_at_ms: Option<u64>,
    gross_unrealized_quote: Option<f64>,
    net_unrealized_quote: Option<f64>,
    net_unrealized_usd: Option<f64>,
    net_pnl_cost_scope: &'static str,
    entry_fees_quote: Option<f64>,
    estimated_exit_fee_rate: f64,
    projected_exit_fee_rate: Option<f64>,
    fee_schedule_status: &'static str,
    exit_fee_schedule_id: Option<String>,
    exit_fee_schedule_version: Option<String>,
    exit_fee_schedule_ledger_epoch: Option<String>,
    exit_fee_schedule_ledger_high_water_cursor: Option<u64>,
    exit_fee_schedule_source_epoch: Option<String>,
    exit_fee_schedule_source_cursor: Option<u64>,
    exit_fee_schedule_source_record_id: Option<String>,
    exit_fee_schedule_source_timestamp_ms: Option<u64>,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    signed_exposure_quote: Option<f64>,
    gross_exposure_quote: Option<f64>,
    signed_exposure_usd: Option<f64>,
    gross_exposure_usd: Option<f64>,
    breakeven_price: Option<f64>,
    net_edge_vs_breakeven_quote: Option<f64>,
    net_edge_vs_breakeven_percent: Option<f64>,
    valuation_status: &'static str,
    valuation_reasons: Vec<&'static str>,
    protection_status: &'static str,
    protection_exception: Option<bool>,
    protection_orders: Vec<BarrierOrder>,
    protection_markers: Vec<OperationalProtectionMarker>,
}

#[derive(Serialize)]
struct OperationalProtectionMarker {
    barrier_id: String,
    order_kind: String,
    status: String,
    trigger_price: f64,
    projected_net_return_percent: Option<f64>,
}

#[derive(Serialize)]
struct OperationalPositionsResponse {
    schema_version: u32,
    captured_at_ms: u64,
    trade_count: usize,
    gross_exposure_usd: Option<f64>,
    net_unrealized_usd: Option<f64>,
    protection_exception_count: Option<usize>,
    aggregate_valuation_status: &'static str,
    aggregate_valuation_reasons: Vec<&'static str>,
    positions: Vec<OperationalPositionResponse>,
}

async fn get_operational_positions(
    State(store): State<Arc<PositionStore>>,
    Query(query): Query<PositionSourceQuery>,
) -> Result<Json<OperationalPositionsResponse>, (StatusCode, String)> {
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
    let captured_at_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(internal_error)?
        .as_millis() as u64;
    let rates = get_usd_rates().await.ok().map(|response| response.0);
    let (summary, trades, barrier_ledger) = {
        let ledger = store.ledger.lock().await;
        (
            cached_position_summary(&store, &ledger, query.source.as_deref()),
            ledger.trades.clone(),
            ledger.barrier_orders.clone(),
        )
    };
    let trade_count = summary.trade_count;
    let marks = store.market_marks_snapshot();
    let active_symbols = summary
        .positions
        .iter()
        .filter(|position| position.quantity.abs() > EPSILON)
        .map(|position| position.symbol.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let fee_schedules = current_operational_fee_schedules(&active_symbols, captured_at_ms).await;
    let positions: Vec<OperationalPositionResponse> = summary.positions.into_iter().filter(|position| position.quantity.abs() > EPSILON).map(|position| {
        const MAX_MARK_AGE_MS: u64 = 5_000;
        let mut reasons = Vec::new();
        let parsed_symbol = position.symbol.split_once('-');
        let quote = parsed_symbol.map(|(_, quote)| quote.to_owned());
        let valid_quantity = position.quantity.is_finite() && position.quantity.abs() > EPSILON;
        let entry_value_quote = (valid_quantity && position.average_entry_price.is_finite()
            && position.average_entry_price > 0.0)
            .then_some(position.quantity.abs() * position.average_entry_price);
        let mark = marks.get(&position.symbol);
        let close_side = if position.quantity > 0.0 { "bid" } else { "ask" };
        let close_mark = mark.and_then(|mark| if position.quantity > 0.0 { mark.bid } else { mark.ask });
        let mark_age_ms = mark.map(|mark| captured_at_ms.saturating_sub(mark.updated_at_ms));
        let mark_is_valid = mark.zip(close_mark).is_some_and(|(mark, price)| {
            price.is_finite() && price > 0.0 && mark.updated_at_ms > 0
                && mark.bid.is_some_and(|bid| bid.is_finite() && bid > 0.0)
                && mark.ask.is_some_and(|ask| ask.is_finite() && ask > 0.0)
                && mark.bid.zip(mark.ask).is_some_and(|(bid, ask)| ask >= bid)
                && mark.updated_at_ms <= captured_at_ms
                && captured_at_ms.saturating_sub(mark.updated_at_ms) <= MAX_MARK_AGE_MS
                && mark.source_system.as_deref() == Some("paper_exchange.market_data_cache")
                && mark.source_epoch.as_deref().is_some_and(|value| !value.is_empty())
                && mark.source_cursor.is_some_and(|cursor| cursor > 0)
                && mark.source_record_id.as_deref().is_some_and(|value| !value.is_empty())
        });
        if !valid_quantity { reasons.push("position_quantity_invalid_or_flat"); }
        if entry_value_quote.is_none() { reasons.push("entry_value_unavailable"); }
        if mark.is_none() { reasons.push("executable_mark_missing"); }
        else if !mark_is_valid { reasons.push("executable_mark_stale_or_unproven"); }
        let quote_currency = quote.as_deref();
        let identity = active_position_fill_identities(&trades, &position.symbol, query.source.as_deref());
        let position_identity = identity.position_identity.clone();
        let canonical_trade_ids = identity.canonical_trade_ids.clone();
        let active_fill_ids = identity.entry_fill_ids.iter().cloned().collect::<std::collections::BTreeSet<_>>();
        let active_fills = trades.iter().filter(|trade| trade.fill_id.as_ref().is_some_and(|id| active_fill_ids.contains(id))).collect::<Vec<_>>();
        let fee_currency_is_quote = quote_currency.is_some() && identity.position_identity.is_some()
            && identity.all_entry_fills_known && identity.entry_fee_currency_is_quote
            && !active_fills.is_empty() && active_fills.len() == identity.entry_fill_ids.len();
        let entry_fee_evidence_valid = fee_currency_is_quote && active_fills.iter()
            .all(|trade| active_entry_fee_evidence_valid(trade, &position.symbol, quote_currency.unwrap()));
        if !identity.all_entry_fills_known { reasons.push("active_entry_fee_unknown_or_identity_missing"); }
        if !fee_currency_is_quote { reasons.push("fee_currency_not_proven_in_quote_currency"); }
        if !entry_fee_evidence_valid { reasons.push("active_entry_fee_source_evidence_missing_or_mismatched"); }
        let exit_fee_schedule = fee_schedules.get(&position.symbol).and_then(Clone::clone)
            .filter(|schedule| valid_exit_fee_schedule(Some(schedule), &position.symbol, captured_at_ms));
        if exit_fee_schedule.is_none() { reasons.push("authoritative_exit_fee_schedule_missing_ambiguous_or_stale"); }
        let fx = if quote_currency == Some("USD") { Some(1.0) } else {
            quote_currency.and_then(|quote| rates.as_ref().and_then(|rates| rates.rates.get(quote).copied()))
        };
        let fx_at = quote_currency.and_then(|quote| rates.as_ref().and_then(|rates| rates.updated_at_by_currency.get(quote).copied()));
        let fx_id = quote_currency.and_then(|quote| rates.as_ref().and_then(|rates| rates.source_record_id_by_currency.get(quote)));
        let fx_valid = quote_currency == Some("USD") || (fx.zip(fx_at).zip(fx_id).is_some_and(|((rate, at), id)|
            rate.is_finite() && rate > 0.0 && at > 0 && at <= captured_at_ms
                && captured_at_ms.saturating_sub(at) <= USD_RATE_STREAM_MAX_AGE.as_millis() as u64
                && !id.trim().is_empty()));
        if !fx_valid { reasons.push("quote_fx_missing_stale_or_unproven"); }
        let valid_fx = fx.filter(|_| fx_valid);
        let gross = (mark_is_valid && position.average_entry_price.is_finite() && position.average_entry_price > 0.0)
            .then(|| position.quantity * (close_mark.unwrap() - position.average_entry_price));
        let exit_fee_rate = exit_fee_schedule.as_ref().map(|schedule| schedule.taker_rate);
        let exit_fee_rate_valid = exit_fee_rate.is_some_and(|rate| rate.is_finite() && (0.0..1.0).contains(&rate));
        if !exit_fee_rate_valid { reasons.push("estimated_exit_fee_rate_unavailable_or_invalid"); }
        let exit_fee = (mark_is_valid && exit_fee_rate_valid)
            .then(|| position.quantity.abs() * close_mark.unwrap() * exit_fee_rate.unwrap());
        let fees_complete = entry_fee_evidence_valid && exit_fee_schedule.is_some();
        let net = net_unrealized_quote(gross, position.entry_fees, exit_fee, fees_complete);
        let breakeven = executable_breakeven_price(position.quantity, position.average_entry_price,
            position.entry_fees, exit_fee_rate.unwrap_or_default(), fees_complete);
        let edge = net;
        let edge_percent = edge.zip(entry_value_quote)
            .map(|(edge, entry_value)| edge / entry_value * 100.0);
        let exposure_quote = mark_is_valid.then(|| position.quantity * close_mark.unwrap());
        let net_usd = net.zip(valid_fx).map(|(value, rate)| value * rate);
        let exposure_usd = exposure_quote.zip(valid_fx).map(|(value, rate)| value * rate);
        let entry_value_usd = entry_value_quote.zip(valid_fx).map(|(value, rate)| value * rate);
        let protection_orders = barrier_ledger.iter().filter(|barrier| {
            barrier_matches_active_position(
                &barrier.symbol,
                &barrier.source,
                barrier.lifecycle.as_ref().map(|lifecycle| lifecycle.trade_id.as_str()),
                barrier.parent_order_id.as_deref(),
                &position.symbol,
                query.source.as_deref(),
                &identity,
            )
                && matches!(barrier.status.as_str(), "armed" | "triggering" | "interrupted" | "failed" | "rejected" | "error")
        }).cloned().collect::<Vec<_>>();
        let protection_markers = protection_orders.iter().map(|barrier| {
            let projected_net_return_percent = if fees_complete && exit_fee_rate_valid
                && barrier.trigger_price.is_finite() && barrier.trigger_price > 0.0 {
                entry_value_quote.map(|entry_value| {
                    let gross = position.quantity * (barrier.trigger_price - position.average_entry_price);
                    let close_fee = position.quantity.abs() * barrier.trigger_price * exit_fee_rate.unwrap();
                    if entry_value.is_finite() && entry_value > 0.0 {
                        net_unrealized_quote(Some(gross), position.entry_fees, Some(close_fee), fees_complete)
                            .map(|net| net / entry_value * 100.0)
                    } else { None }
                }).flatten().filter(|value| value.is_finite())
            } else { None };
            OperationalProtectionMarker {
                barrier_id: barrier.id.clone(), order_kind: barrier.order_kind.clone(),
                status: barrier.status.clone(), trigger_price: barrier.trigger_price,
                projected_net_return_percent,
            }
        }).collect::<Vec<_>>();
        let protection_exception = protection_orders.iter().any(|barrier|
            matches!(barrier.status.as_str(), "interrupted" | "failed" | "rejected" | "error"));
        let protection_status = if protection_exception { "exception" }
            else if protection_orders.iter().any(|barrier| barrier.status == "armed" || barrier.status == "triggering") { "active" }
            else { "unknown_no_exact_barrier_link" };
        let protection_exception = if protection_orders.is_empty() { None } else { Some(protection_exception) };
        let complete = reasons.is_empty() && net.is_some() && exposure_usd.is_some();
        OperationalPositionResponse {
            position_identity, canonical_trade_ids,
            symbol: position.symbol, quantity: position.quantity, average_entry_price: position.average_entry_price,
            realized_pnl: position.realized_pnl, entry_fees: position.entry_fees,
            fees_paid: position.fees_paid, last_trade_timestamp: position.last_trade_timestamp,
            quote_currency: quote, entry_value_quote, entry_value_usd,
            quote_fx_rate: valid_fx, quote_fx_updated_at_ms: fx_at.filter(|_| fx_valid),
            quote_fx_source_record_id: fx_id.filter(|_| fx_valid).cloned(),
            executable_close_side: mark_is_valid.then(|| close_side.to_owned()),
            executable_mark: mark_is_valid.then_some(close_mark.unwrap()), mark_age_ms,
            mark_source_system: mark.and_then(|mark| mark.source_system.clone()),
            mark_source_epoch: mark.and_then(|mark| mark.source_epoch.clone()),
            mark_source_cursor: mark.and_then(|mark| mark.source_cursor),
            mark_source_record_id: mark.and_then(|mark| mark.source_record_id.clone()),
            mark_updated_at_ms: mark.map(|mark| mark.updated_at_ms), gross_unrealized_quote: gross,
            net_unrealized_quote: net, net_unrealized_usd: net_usd,
            net_pnl_cost_scope: "gross mark-to-entry less known quote-denominated entry fees and estimated taker close fee; excludes FX/reconciliation adjustments",
            entry_fees_quote: entry_fee_evidence_valid.then_some(position.entry_fees),
            estimated_exit_fee_rate: position.estimated_exit_fee_rate,
            projected_exit_fee_rate: exit_fee_rate,
            fee_schedule_status: if exit_fee_schedule.is_some() { "authoritative_versioned_taker_schedule" } else { "unavailable_or_unproven" },
            exit_fee_schedule_id: exit_fee_schedule.as_ref().map(|schedule| schedule.id.clone()),
            exit_fee_schedule_version: exit_fee_schedule.as_ref().map(|schedule| schedule.version.clone()),
            exit_fee_schedule_ledger_epoch: exit_fee_schedule.as_ref().map(|schedule| schedule.ledger_epoch.clone()),
            exit_fee_schedule_ledger_high_water_cursor: exit_fee_schedule.as_ref().map(|schedule| schedule.ledger_high_water_cursor),
            exit_fee_schedule_source_epoch: exit_fee_schedule.as_ref().map(|schedule| schedule.source_epoch.clone()),
            exit_fee_schedule_source_cursor: exit_fee_schedule.as_ref().map(|schedule| schedule.source_cursor),
            exit_fee_schedule_source_record_id: exit_fee_schedule.as_ref().map(|schedule| schedule.source_record_id.clone()),
            exit_fee_schedule_source_timestamp_ms: exit_fee_schedule.as_ref().map(|schedule| schedule.source_timestamp_ms),
            known_fee_fills: position.known_fee_fills, unknown_fee_fills: position.unknown_fee_fills,
            signed_exposure_quote: exposure_quote,
            gross_exposure_quote: exposure_quote.map(f64::abs), signed_exposure_usd: exposure_usd,
            gross_exposure_usd: exposure_usd.map(f64::abs), breakeven_price: breakeven,
            net_edge_vs_breakeven_quote: edge, net_edge_vs_breakeven_percent: edge_percent,
            valuation_status: if complete { "complete" } else { "incomplete" },
            valuation_reasons: reasons, protection_status, protection_exception, protection_orders, protection_markers,
        }
    }).collect();
    let aggregate_valuation_reasons = positions
        .iter()
        .flat_map(|position| position.valuation_reasons.iter().copied())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let aggregate_valuation_complete = positions
        .iter()
        .all(|position| position.valuation_status == "complete");
    let gross_exposure_usd = positions
        .iter()
        .map(|position| position.gross_exposure_usd)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum());
    let net_unrealized_usd = positions
        .iter()
        .map(|position| position.net_unrealized_usd)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().sum());
    let protection_exception_count = positions
        .iter()
        .map(|position| position.protection_exception)
        .collect::<Option<Vec<_>>>()
        .map(|values| values.into_iter().filter(|exception| *exception).count());
    Ok(Json(OperationalPositionsResponse {
        schema_version: 1,
        captured_at_ms,
        trade_count,
        gross_exposure_usd,
        net_unrealized_usd,
        protection_exception_count,
        aggregate_valuation_status: if aggregate_valuation_complete {
            "complete"
        } else {
            "incomplete"
        },
        aggregate_valuation_reasons,
        positions,
    }))
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

    #[test]
    fn operational_net_pnl_and_breakeven_fail_closed_without_complete_fee_inputs() {
        assert_eq!(
            net_unrealized_quote(Some(12.0), -2.0, Some(1.0), true),
            Some(9.0)
        );
        assert_eq!(
            net_unrealized_quote(Some(12.0), -2.0, Some(1.0), false),
            None
        );
        assert_eq!(net_unrealized_quote(Some(12.0), -2.0, None, true), None);
        let long = executable_breakeven_price(2.0, 100.0, -2.0, 0.01, true).unwrap();
        assert!((long - 102.020_202_020_202_02).abs() < 1e-10);
        let short = executable_breakeven_price(-2.0, 100.0, -2.0, 0.01, true).unwrap();
        assert!((short - 98.019_801_980_198_02).abs() < 1e-10);
        assert_eq!(
            executable_breakeven_price(2.0, 100.0, -2.0, 0.01, false),
            None
        );
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
            paper_exchange_fee_amount: None,
            paper_exchange_fee_currency: None,
            paper_exchange_fee_schedule_id: None,
            paper_exchange_fee_schedule_version: None,
            paper_exchange_fee_order_class: None,
            paper_exchange_fee_schedule_evidence: None,
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

    #[test]
    fn operational_position_identity_tracks_remaining_open_fill_lineage() {
        let mut opening = trade(
            "open",
            TradeSide::Buy,
            2.0,
            100.0,
            1,
            0.0,
            true,
            Some("open"),
        );
        opening.fill_id = Some("fill-open".into());
        let mut partial_close = trade(
            "close",
            TradeSide::Sell,
            0.5,
            101.0,
            2,
            0.0,
            true,
            Some("close"),
        );
        partial_close.fill_id = Some("fill-close".into());
        let mut scale_in = trade("add", TradeSide::Buy, 1.0, 102.0, 3, 0.0, true, Some("add"));
        scale_in.fill_id = Some("fill-add".into());
        let ledger = Ledger {
            trades: vec![opening, partial_close, scale_in],
            ..Ledger::default()
        };
        let identity = active_position_fill_identities(&ledger.trades, "BTC-USDT", Some("auto"));
        assert_eq!(
            identity.position_identity.as_deref(),
            Some("pm-position:v1:fill-open,fill-add")
        );
        assert!(
            identity.canonical_trade_ids.is_empty(),
            "missing canonical lifecycle IDs must remain explicit"
        );
    }

    #[test]
    fn closed_same_symbol_fill_does_not_poison_active_fee_currency_identity() {
        let mut closed_open = trade(
            "old-open",
            TradeSide::Buy,
            1.0,
            100.0,
            1,
            0.0,
            true,
            Some("old"),
        );
        closed_open.fill_id = Some("old-open-fill".into());
        closed_open.fee_currency = "BTC".into();
        let mut closed_close = trade(
            "old-close",
            TradeSide::Sell,
            1.0,
            101.0,
            2,
            0.0,
            true,
            Some("old"),
        );
        closed_close.fill_id = Some("old-close-fill".into());
        let mut active_open = trade(
            "current",
            TradeSide::Buy,
            2.0,
            102.0,
            3,
            0.0,
            true,
            Some("current"),
        );
        active_open.fill_id = Some("active-fill".into());
        let trades = vec![closed_open, closed_close, active_open];
        let identity = active_position_fill_identities(&trades, "BTC-USDT", Some("auto"));
        assert_eq!(identity.entry_fill_ids, vec!["active-fill"]);
        assert!(identity.entry_fee_currency_is_quote);
        assert!(identity.all_entry_fills_known);
    }

    #[test]
    fn same_symbol_barrier_requires_exact_active_identity_and_source() {
        let identity = ActivePositionIdentity {
            position_identity: Some("pm-position:v1:fill".into()),
            canonical_trade_ids: vec!["trade-1".into()],
            parent_order_ids: vec!["entry-order-1".into()],
            sources: vec!["auto".into()],
            entry_fill_ids: vec!["fill".into()],
            all_entry_fills_known: true,
            entry_fee_currency_is_quote: true,
        };
        assert!(barrier_matches_active_position(
            "BTC-USDT",
            "auto",
            Some("trade-1"),
            None,
            "BTC-USDT",
            Some("auto"),
            &identity
        ));
        assert!(barrier_matches_active_position(
            "BTC-USDT",
            "auto",
            None,
            Some("entry-order-1"),
            "BTC-USDT",
            Some("auto"),
            &identity
        ));
        assert!(!barrier_matches_active_position(
            "BTC-USDT",
            "manual",
            Some("trade-1"),
            None,
            "BTC-USDT",
            Some("auto"),
            &identity
        ));
        assert!(!barrier_matches_active_position(
            "BTC-USDT",
            "auto",
            Some("trade-other"),
            None,
            "BTC-USDT",
            Some("auto"),
            &identity
        ));
    }

    #[test]
    fn entry_fee_evidence_requires_matching_version_and_source_identity() {
        let mut fill = trade(
            "fee",
            TradeSide::Buy,
            1.0,
            100.0,
            10,
            -0.1,
            true,
            Some("entry"),
        );
        fill.paper_exchange_fee_amount = Some(0.1);
        fill.paper_exchange_fee_currency = Some("USDT".into());
        fill.paper_exchange_fee_schedule_id = Some("spot-fees".into());
        fill.paper_exchange_fee_schedule_version = Some("v1".into());
        fill.paper_exchange_fee_order_class = Some("taker".into());
        fill.paper_exchange_fee_schedule_evidence = Some(serde_json::json!({
            "schema_version":"paper_exchange_financial_evidence.v1", "event_id":"fees-1",
            "source_system":"simulator", "source_epoch":"epoch-1", "source_cursor":7,
            "source_record_id":"record-7", "source_timestamp_utc_ms":9,
            "event_type":"fee_schedule", "fee_schedule_id":"spot-fees", "fee_schedule_version":"v1",
            "symbol":"BTC-USDT", "effective_from_utc_ms":1, "effective_to_utc_ms":null,
            "maker_rate":0.0, "taker_rate":0.001
        }));
        assert!(active_entry_fee_evidence_valid(&fill, "BTC-USDT", "USDT"));
        fill.paper_exchange_fee_schedule_version = Some("v2".into());
        assert!(!active_entry_fee_evidence_valid(&fill, "BTC-USDT", "USDT"));
        fill.paper_exchange_fee_schedule_version = Some("v1".into());
        fill.paper_exchange_fee_schedule_evidence = None;
        assert!(!active_entry_fee_evidence_valid(&fill, "BTC-USDT", "USDT"));
    }

    #[test]
    fn exit_fee_schedule_rejects_symbol_mismatch_expired_and_absent_identity() {
        let schedule = OperationalFeeSchedule {
            symbol: "BTC-USDT".into(),
            ledger_epoch: "ledger-epoch".into(),
            ledger_high_water_cursor: 7,
            id: "spot-fees".into(),
            version: "v1".into(),
            source_epoch: "epoch".into(),
            source_cursor: 2,
            source_record_id: "record".into(),
            source_timestamp_ms: 10,
            effective_from_ms: 1,
            effective_to_ms: Some(20),
            taker_rate: 0.001,
        };
        assert!(valid_exit_fee_schedule(Some(&schedule), "BTC-USDT", 15));
        assert!(!valid_exit_fee_schedule(Some(&schedule), "ETH-USDT", 15));
        assert!(!valid_exit_fee_schedule(Some(&schedule), "BTC-USDT", 20));
        assert!(!valid_exit_fee_schedule(None, "BTC-USDT", 15));
    }

    #[tokio::test]
    async fn operational_positions_serialize_executable_mark_exposure_and_protection_state() {
        let store = Arc::new(test_store());
        let mut opening = trade(
            "open",
            TradeSide::Buy,
            2.0,
            100.0,
            1,
            -0.2,
            true,
            Some("open"),
        );
        opening.symbol = "BTC-USD".into();
        opening.fill_id = Some("pm-fill:v1:epoch:1".into());
        opening.lifecycle = Some(complete_lifecycle_link());
        opening.fee_currency = "USD".into();
        let mark_time = now_ms().unwrap();
        opening.paper_exchange_fee_amount = Some(0.2);
        opening.paper_exchange_fee_currency = Some("USD".into());
        opening.paper_exchange_fee_schedule_id = Some("spot-fees".into());
        opening.paper_exchange_fee_schedule_version = Some("v1".into());
        opening.paper_exchange_fee_order_class = Some("taker".into());
        opening.paper_exchange_fee_schedule_evidence = Some(serde_json::json!({
            "schema_version":"paper_exchange_financial_evidence.v1", "event_id":"fee-schedule-1",
            "source_system":"simulator", "source_epoch":"fee-epoch", "source_cursor":1,
            "source_record_id":"fee-schedule-record-1", "source_timestamp_utc_ms":1,
            "event_type":"fee_schedule", "fee_schedule_id":"spot-fees", "fee_schedule_version":"v1",
            "symbol":"BTC-USD", "effective_from_utc_ms":1, "effective_to_utc_ms":null,
            "maker_rate":0.0, "taker_rate":0.001
        }));
        OPERATIONAL_FEE_SCHEDULE_CACHE
            .get_or_init(|| tokio::sync::Mutex::new(std::collections::HashMap::new()))
            .lock()
            .await
            .insert(
                "BTC-USD".into(),
                CachedOperationalFeeSchedule {
                    cached_at_ms: mark_time,
                    schedule: Some(OperationalFeeSchedule {
                        symbol: "BTC-USD".into(),
                        ledger_epoch: "ledger-epoch".into(),
                        ledger_high_water_cursor: 1,
                        id: "spot-fees".into(),
                        version: "v1".into(),
                        source_epoch: "fee-epoch".into(),
                        source_cursor: 1,
                        source_record_id: "fee-schedule-record-1".into(),
                        source_timestamp_ms: mark_time.saturating_sub(1),
                        effective_from_ms: 1,
                        effective_to_ms: None,
                        taker_rate: 0.001,
                    }),
                },
            );
        store.ledger.lock().await.trades.push(opening);
        store.market_marks.write().unwrap().insert(
            "BTC-USD".into(),
            MarketMark {
                bid: Some(104.0),
                ask: Some(105.0),
                mid: Some(104.5),
                worst_ask: None,
                updated_at_ms: mark_time,
                source_system: Some("paper_exchange.market_data_cache".into()),
                source_epoch: Some("pex-epoch".into()),
                source_cursor: Some(9),
                source_record_id: Some("pex-mark:9".into()),
            },
        );
        let response = get_operational_positions(
            State(store),
            Query(PositionSourceQuery {
                source: Some("auto".into()),
            }),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(response.schema_version, 1);
        assert_eq!(response.positions.len(), 1);
        assert_eq!(response.gross_exposure_usd, Some(208.0));
        assert_eq!(response.protection_exception_count, None);
        assert_eq!(response.aggregate_valuation_status, "complete");
        let position = &response.positions[0];
        assert_eq!(position.executable_close_side.as_deref(), Some("bid"));
        assert_eq!(position.executable_mark, Some(104.0));
        assert_eq!(position.entry_value_quote, Some(200.0));
        assert_eq!(position.gross_unrealized_quote, Some(8.0));
        assert!((position.net_unrealized_quote.unwrap() - 7.592).abs() < 1e-12);
        assert!(
            (position.net_edge_vs_breakeven_percent.unwrap() - (7.592 / 200.0 * 100.0)).abs()
                < 1e-12
        );
        assert_eq!(
            position.mark_source_record_id.as_deref(),
            Some("pex-mark:9")
        );
        assert_eq!(position.canonical_trade_ids, vec!["trade-1"]);
        assert_eq!(position.protection_status, "unknown_no_exact_barrier_link");
        assert_eq!(position.protection_exception, None);
        assert_eq!(position.exit_fee_schedule_id.as_deref(), Some("spot-fees"));
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
