use crate::market_data::{BookSnapshot, MarketData};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::{get, post},
};
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior, params,
};
use serde::{Deserialize, Serialize};
use std::{path::PathBuf, sync::Arc};

const MATCH_SCHEMA_VERSION: &str = "paper_exchange_match:v1";
const MATCH_REQUEST_SCHEMA_VERSION: &str = "paper_exchange_match_request:v2";
const LEDGER_SCHEMA_VERSION: &str = "paper_exchange_match_ledger:v1";
const FINANCIAL_EVIDENCE_SCHEMA_VERSION: &str = "paper_exchange_financial_evidence:v1";
const MAX_LEDGER_PAGE_SIZE: usize = 500;

#[derive(Clone, Deserialize, Serialize)]
pub struct MatchRequest {
    pub symbol: String,
    pub side: String,
    pub quantity: f64,
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub order_id: Option<String>,
    #[serde(default)]
    pub parent_order_id: Option<String>,
    #[serde(default)]
    pub execution_origin: Option<String>,
    #[serde(default)]
    pub execution_price_override: Option<f64>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct Fill {
    pub quantity: f64,
    pub price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cursor: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_cursor_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recorded_at_utc_ms: Option<u64>,
    pub quote_evidence: Option<QuoteEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_currency: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_schedule_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_schedule_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_order_class: Option<String>,
    /// Exact versioned source event that priced this fill. The matching fee
    /// fields above stay separate so legacy readers can ignore the added
    /// provenance without changing their execution contract.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fee_schedule_evidence: Option<FinancialEvidenceEvent>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct QuoteEvidence {
    pub reference_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub worst_ask: Option<f64>,
    pub book_updated_at_ms: u64,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct MatchResponse {
    pub reference_price: Option<f64>,
    pub best_bid: Option<f64>,
    pub best_ask: Option<f64>,
    pub worst_ask: Option<f64>,
    pub book_updated_at_ms: u64,
    pub fills: Vec<Fill>,
    pub remaining_quantity: f64,
    pub filled_quantity: f64,
    pub average_execution_price: Option<f64>,
    pub schema_version: String,
    pub request_schema_version: String,
    pub order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_order_id: Option<String>,
    pub request_fingerprint: String,
    pub source_epoch: Option<String>,
    pub source_cursor_kind: String,
    pub recorded_at_utc_ms: Option<u64>,
    pub source_cursor: Option<u64>,
    pub capture_complete: bool,
    pub capture_status: String,
    pub capture_failure_reason: Option<String>,
    pub evidence_kind: String,
    pub reconciliation_eligible: bool,
}

#[derive(Clone)]
struct OrderApiState {
    market: Arc<MarketData>,
    ledger_path: PathBuf,
}

#[derive(Clone)]
struct SimulatorFeeScheduleConfig {
    fee_schedule_id: String,
    fee_schedule_version: String,
    maker_rate: f64,
    taker_rate: f64,
}

#[derive(Serialize)]
struct LedgerPage {
    schema_version: &'static str,
    evidence_kind: &'static str,
    capture_scope: &'static str,
    reconciliation_eligible: bool,
    source_epoch: String,
    cursor_kind: &'static str,
    history_complete: bool,
    after_cursor: u64,
    next_cursor: Option<u64>,
    max_cursor: u64,
    page_size: usize,
    page_complete: bool,
    identity_complete: bool,
    rows: Vec<LedgerFill>,
}

#[derive(Serialize)]
struct LedgerFill {
    source_cursor: u64,
    source_execution_id: String,
    order_id: String,
    request_fingerprint: String,
    symbol: String,
    side: String,
    quantity: f64,
    price: f64,
    recorded_at_utc_ms: u64,
    quote_evidence: QuoteEvidence,
}

#[derive(Deserialize)]
struct LedgerQuery {
    after_cursor: Option<u64>,
    limit: Option<usize>,
}

/// Source-attributed account evidence imported from the paper account provider.
/// The service stores the provider's stable identity and cursor verbatim; it
/// does not claim that a caller's source attribution is cryptographically
/// authenticated. Reconciliation still requires an independently verified
/// source contract and complete provider coverage.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct FinancialEvidenceEvent {
    pub schema_version: String,
    pub event_id: String,
    pub source_system: String,
    pub source_epoch: String,
    pub source_cursor: u64,
    pub source_record_id: String,
    pub source_timestamp_utc_ms: u64,
    #[serde(flatten)]
    pub payload: FinancialEvidencePayload,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum FinancialEvidencePayload {
    FeeSchedule {
        fee_schedule_id: String,
        fee_schedule_version: String,
        symbol: String,
        effective_from_utc_ms: u64,
        effective_to_utc_ms: Option<u64>,
        maker_rate: f64,
        taker_rate: f64,
    },
    SettledExecution {
        execution_id: String,
        order_id: String,
        symbol: String,
        side: String,
        quantity: f64,
        price: f64,
        fee: f64,
        fee_currency: String,
        fee_schedule_id: String,
        fee_schedule_version: String,
        execution_origin: String,
    },
    AccountBalance {
        portfolio_id: String,
        boundary: String,
        balances_by_currency: std::collections::BTreeMap<String, f64>,
    },
    CashMovement {
        movement_id: String,
        portfolio_id: String,
        movement_kind: String,
        currency: String,
        amount: f64,
    },
    FxMark {
        portfolio_id: String,
        currency: String,
        rate_usd: f64,
    },
    DerivativeSettlementMark {
        portfolio_id: String,
        instrument: String,
        expiry_utc_ms: u64,
        settlement_price: f64,
        settlement_kind: String,
    },
    CoverageAttestation {
        covered_source_epoch: String,
        from_source_cursor: u64,
        through_source_cursor: u64,
        history_complete: bool,
    },
}

#[derive(Serialize)]
struct FinancialEvidenceAppendResponse {
    schema_version: &'static str,
    event_id: String,
    source_epoch: String,
    source_cursor: u64,
    source_timestamp_utc_ms: u64,
    idempotent_replay: bool,
    source_attribution_status: &'static str,
}

#[derive(Serialize)]
struct FinancialEvidencePage {
    schema_version: &'static str,
    evidence_kind: &'static str,
    source_epoch: String,
    cursor_kind: &'static str,
    after_cursor: u64,
    next_cursor: Option<u64>,
    max_cursor: u64,
    observed_source_max_cursor: u64,
    page_size: usize,
    page_complete: bool,
    source_attribution_status: &'static str,
    history_complete: bool,
    current_history_complete: bool,
    execution_history_empty: bool,
    opening_capital_status: String,
    fee_schedule_status: String,
    captured_execution_flow_types: Vec<String>,
    uncovered_execution_flow_types: Vec<String>,
    events: Vec<FinancialEvidenceStoredEvent>,
}

#[derive(Serialize)]
struct FinancialEvidenceStoredEvent {
    source_cursor: u64,
    source_system: String,
    source_epoch: String,
    source_record_id: String,
    source_timestamp_utc_ms: u64,
    event: FinancialEvidenceEvent,
}

#[derive(Deserialize)]
struct FinancialEvidenceQuery {
    after_cursor: Option<u64>,
    limit: Option<usize>,
    through_cursor: Option<u64>,
    through_epoch: Option<String>,
}

#[derive(Deserialize)]
struct EffectiveFeeScheduleQuery {
    symbols: String,
    at_utc_ms: u64,
}

#[derive(Serialize)]
struct EffectiveFeeScheduleSnapshot {
    schema_version: &'static str,
    captured_at_utc_ms: u64,
    ledger_epoch: String,
    observed_max_cursor: u64,
    schedules: Vec<EffectiveFeeScheduleEntry>,
}

#[derive(Serialize)]
struct EffectiveFeeScheduleEntry {
    symbol: String,
    status: &'static str,
    fee_schedule_id: Option<String>,
    fee_schedule_version: Option<String>,
    effective_from_utc_ms: Option<u64>,
    effective_to_utc_ms: Option<u64>,
    maker_rate: Option<f64>,
    taker_rate: Option<f64>,
    source_epoch: Option<String>,
    source_cursor: Option<u64>,
    source_record_id: Option<String>,
    source_timestamp_utc_ms: Option<u64>,
}

async fn match_order(
    State(state): State<OrderApiState>,
    Json(request): Json<MatchRequest>,
) -> Result<Json<MatchResponse>, (StatusCode, String)> {
    if !matches!(request.side.as_str(), "buy" | "sell")
        || request.symbol.trim().is_empty()
        || !request.quantity.is_finite()
        || request.quantity <= 0.0
        || request
            .limit_price
            .is_some_and(|limit| !limit.is_finite() || limit <= 0.0)
        || request.order_id.as_ref().is_some_and(|id| {
            id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control)
        })
        || request.parent_order_id.as_ref().is_some_and(|id| {
            id.trim().is_empty() || id.len() > 256 || id.chars().any(char::is_control)
        })
        || request.execution_origin.as_ref().is_some_and(|origin| {
            !matches!(
                origin.as_str(),
                "market_match"
                    | "match_probe"
                    | "resting_limit"
                    | "protective_barrier"
                    | "vwap"
                    | "twap"
                    | "other"
            )
        })
        || request
            .execution_price_override
            .is_some_and(|price| !price.is_finite() || price <= 0.0)
        || request.execution_price_override.is_some()
            && request.execution_origin.as_deref() != Some("resting_limit")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid order match request".into(),
        ));
    }
    let fingerprint = request_fingerprint(&request);
    let legacy_fingerprint = legacy_request_fingerprint(&request);

    if let Some(order_id) = request.order_id.as_deref() {
        let path = state.ledger_path.clone();
        let order_id = order_id.to_owned();
        let fingerprint_for_lookup = fingerprint.clone();
        let legacy_fingerprint_for_lookup = legacy_fingerprint.clone();
        let existing = tokio::task::spawn_blocking(move || {
            lookup_committed_match(
                &path,
                &order_id,
                &fingerprint_for_lookup,
                legacy_fingerprint_for_lookup.as_deref(),
            )
        })
        .await;
        match existing {
            Ok(Ok(Some(response))) => return Ok(Json(response)),
            Ok(Ok(None)) => {}
            Ok(Err(error)) if is_identity_conflict(&error) => {
                return Err((StatusCode::CONFLICT, error.to_string()));
            }
            Ok(Err(_)) | Err(_) => {
                // Keep matching available if the independent evidence ledger
                // is unavailable. The response is marked uncaptured if the
                // subsequent durable write also fails.
            }
        }
    }

    state
        .market
        .ensure_book_subscription(&request.symbol)
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let book = state
        .market
        .book(&request.symbol)
        .filter(|book| now_ms().is_ok_and(|now| now.saturating_sub(book.updated_at_ms) <= 5_000))
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "Live market book unavailable".into(),
        ))?;

    let mut response = match_book(&request, &book, fingerprint);
    if let Some(execution_price) = request.execution_price_override {
        for fill in &mut response.fills {
            fill.price = execution_price;
        }
        if response.filled_quantity > 1e-12 {
            response.average_execution_price = Some(execution_price);
        }
    }
    let Some(order_id) = request.order_id.as_deref() else {
        // Legacy callers without a stable caller-supplied identity may still
        // match, but this response cannot be durably attributed or retried.
        return Ok(Json(response));
    };

    let path = state.ledger_path;
    let order_id = order_id.to_owned();
    let fingerprint = response.request_fingerprint.clone();
    let response_for_commit = response.clone();
    let request_for_commit = request.clone();
    let committed = tokio::task::spawn_blocking(move || {
        commit_match(
            &path,
            &order_id,
            &fingerprint,
            &request_for_commit,
            response_for_commit,
        )
    })
    .await;
    response = match committed {
        Ok(Ok(response)) => response,
        Ok(Err(error)) if is_identity_conflict(&error) => {
            return Err((StatusCode::CONFLICT, error.to_string()));
        }
        Ok(Err(_)) | Err(_) => {
            return Ok(Json(mark_capture_incomplete(
                response,
                "durable_ledger_write_failed",
            )));
        }
    };
    Ok(Json(response))
}

async fn ledger_history(
    State(state): State<OrderApiState>,
    Query(query): Query<LedgerQuery>,
) -> Result<Json<LedgerPage>, (StatusCode, String)> {
    let after_cursor = query.after_cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    if i64::try_from(after_cursor).is_err() || !(1..=MAX_LEDGER_PAGE_SIZE).contains(&limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "after_cursor must fit SQLite's cursor range and limit must be 1..={MAX_LEDGER_PAGE_SIZE}"
            ),
        ));
    }
    let path = state.ledger_path;
    let page = tokio::task::spawn_blocking(move || read_ledger_page(&path, after_cursor, limit))
        .await
        .map_err(database_join_error)?
        .map_err(database_error)?;
    Ok(Json(page))
}

async fn append_financial_evidence(
    State(state): State<OrderApiState>,
    Json(event): Json<FinancialEvidenceEvent>,
) -> Result<Json<FinancialEvidenceAppendResponse>, (StatusCode, String)> {
    validate_financial_evidence(&event).map_err(|message| (StatusCode::BAD_REQUEST, message))?;
    let path = state.ledger_path;
    let response =
        tokio::task::spawn_blocking(move || append_financial_evidence_sync(&path, &event))
            .await
            .map_err(database_join_error)?
            .map_err(database_error)?;
    Ok(Json(response))
}

async fn financial_evidence_history(
    State(state): State<OrderApiState>,
    Query(query): Query<FinancialEvidenceQuery>,
) -> Result<Json<FinancialEvidencePage>, (StatusCode, String)> {
    let after_cursor = query.after_cursor.unwrap_or(0);
    let limit = query.limit.unwrap_or(100);
    if query.through_cursor.is_some() != query.through_epoch.is_some() {
        return Err((
            StatusCode::BAD_REQUEST,
            "through_cursor and through_epoch must be supplied together".to_owned(),
        ));
    }
    if i64::try_from(after_cursor).is_err() || !(1..=MAX_LEDGER_PAGE_SIZE).contains(&limit) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "after_cursor must fit SQLite's cursor range and limit must be 1..={MAX_LEDGER_PAGE_SIZE}"
            ),
        ));
    }
    if query
        .through_cursor
        .is_some_and(|through_cursor| after_cursor > through_cursor)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "after_cursor cannot exceed through_cursor".to_owned(),
        ));
    }
    let path = state.ledger_path;
    let page = tokio::task::spawn_blocking(move || {
        read_financial_evidence_page(
            &path,
            after_cursor,
            limit,
            query.through_cursor,
            query.through_epoch.as_deref(),
        )
    })
    .await
    .map_err(database_join_error)?
    .map_err(|error| {
        if error
            .to_string()
            .contains("financial evidence source epoch mismatch")
            || error
                .to_string()
                .contains("through_cursor exceeds current source maximum")
        {
            (StatusCode::CONFLICT, error.to_string())
        } else {
            database_error(error)
        }
    })?;
    Ok(Json(page))
}

async fn effective_fee_schedules(
    State(state): State<OrderApiState>,
    Query(query): Query<EffectiveFeeScheduleQuery>,
) -> Result<Json<EffectiveFeeScheduleSnapshot>, (StatusCode, String)> {
    let symbols = query
        .symbols
        .split(',')
        .map(str::trim)
        .map(str::to_ascii_uppercase)
        .filter(|symbol| !symbol.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if query.at_utc_ms == 0
        || query.symbols.len() > 8_192
        || symbols.is_empty()
        || symbols.len() > 256
        || symbols.iter().any(|symbol| {
            !symbol.is_ascii()
                || symbol.len() > 32
                || !symbol.split_once('-').is_some_and(|(base, quote)| {
                    !base.is_empty()
                        && !quote.is_empty()
                        && base.bytes().all(|b| b.is_ascii_alphanumeric())
                        && quote.bytes().all(|b| b.is_ascii_alphanumeric())
                })
        })
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid fee-schedule symbols or timestamp".to_owned(),
        ));
    }
    let path = state.ledger_path;
    let at_utc_ms = query.at_utc_ms;
    let snapshot = tokio::task::spawn_blocking(move || {
        effective_fee_schedule_snapshot(&path, &symbols.into_iter().collect::<Vec<_>>(), at_utc_ms)
    })
    .await
    .map_err(database_join_error)?
    .map_err(database_error)?;
    Ok(Json(snapshot))
}

fn effective_fee_schedule_snapshot(
    path: &PathBuf,
    symbols: &[String],
    at_utc_ms: u64,
) -> anyhow::Result<EffectiveFeeScheduleSnapshot> {
    let mut connection = Connection::open(path)?;
    let transaction = connection.transaction()?;
    let source_epoch = financial_evidence_source_epoch(&transaction)?;
    let observed_max_cursor = transaction.query_row(
        "SELECT COALESCE(MAX(source_cursor), 0) FROM financial_evidence_events",
        [],
        |row| row.get::<_, i64>(0),
    )? as u64;
    let mut schedules = Vec::with_capacity(symbols.len());
    for symbol in symbols {
        let mut statement = transaction.prepare(
            "SELECT s.fee_schedule_id,s.fee_schedule_version,s.effective_from_utc_ms,
                    s.effective_to_utc_ms,s.maker_rate,s.taker_rate,e.source_epoch,
                    e.source_cursor,e.source_record_id,e.source_timestamp_utc_ms,e.event_json
             FROM financial_fee_schedules s
             JOIN financial_evidence_events e ON e.source_cursor=s.event_cursor
             WHERE s.symbol=?1 AND s.effective_from_utc_ms<=?2
               AND (s.effective_to_utc_ms IS NULL OR ?2<s.effective_to_utc_ms)
               AND e.source_timestamp_utc_ms<=?2
             ORDER BY s.effective_from_utc_ms DESC,s.event_cursor DESC LIMIT 2",
        )?;
        let rows = statement
            .query_map(
                rusqlite::params![symbol, i64::try_from(at_utc_ms)?],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, f64>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, String>(10)?,
                    ))
                },
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let entry = if rows.len() > 1 {
            EffectiveFeeScheduleEntry {
                symbol: symbol.clone(),
                status: "ambiguous",
                fee_schedule_id: None,
                fee_schedule_version: None,
                effective_from_utc_ms: None,
                effective_to_utc_ms: None,
                maker_rate: None,
                taker_rate: None,
                source_epoch: None,
                source_cursor: None,
                source_record_id: None,
                source_timestamp_utc_ms: None,
            }
        } else if let Some((
            id,
            version,
            effective_from,
            effective_to,
            maker,
            taker,
            epoch,
            cursor,
            record_id,
            timestamp,
            event_json,
        )) = rows.into_iter().next()
        {
            let event: serde_json::Value = serde_json::from_str(&event_json)?;
            let matches = event.get("event_type").and_then(serde_json::Value::as_str)
                == Some("fee_schedule")
                && event.get("symbol").and_then(serde_json::Value::as_str) == Some(symbol.as_str())
                && event
                    .get("fee_schedule_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(id.as_str())
                && event
                    .get("fee_schedule_version")
                    .and_then(serde_json::Value::as_str)
                    == Some(version.as_str())
                && cursor > 0
                && !record_id.trim().is_empty()
                && timestamp > 0
                && event.get("source_epoch").and_then(serde_json::Value::as_str)
                    == Some(epoch.as_str())
                && event.get("source_cursor").and_then(serde_json::Value::as_u64)
                    == u64::try_from(cursor).ok()
                && event.get("source_record_id").and_then(serde_json::Value::as_str)
                    == Some(record_id.as_str())
                && event.get("source_timestamp_utc_ms").and_then(serde_json::Value::as_u64)
                    == u64::try_from(timestamp).ok()
                && effective_from > 0
                && maker.is_finite()
                && taker.is_finite();
            if matches {
                EffectiveFeeScheduleEntry {
                    symbol: symbol.clone(),
                    status: "matched",
                    fee_schedule_id: Some(id),
                    fee_schedule_version: Some(version),
                    effective_from_utc_ms: Some(u64::try_from(effective_from)?),
                    effective_to_utc_ms: effective_to.map(u64::try_from).transpose()?,
                    maker_rate: Some(maker),
                    taker_rate: Some(taker),
                    source_epoch: Some(epoch),
                    source_cursor: Some(u64::try_from(cursor)?),
                    source_record_id: Some(record_id),
                    source_timestamp_utc_ms: Some(u64::try_from(timestamp)?),
                }
            } else {
                EffectiveFeeScheduleEntry {
                    symbol: symbol.clone(),
                    status: "mismatched_source_record",
                    fee_schedule_id: None,
                    fee_schedule_version: None,
                    effective_from_utc_ms: None,
                    effective_to_utc_ms: None,
                    maker_rate: None,
                    taker_rate: None,
                    source_epoch: None,
                    source_cursor: None,
                    source_record_id: None,
                    source_timestamp_utc_ms: None,
                }
            }
        } else {
            EffectiveFeeScheduleEntry {
                symbol: symbol.clone(),
                status: "missing",
                fee_schedule_id: None,
                fee_schedule_version: None,
                effective_from_utc_ms: None,
                effective_to_utc_ms: None,
                maker_rate: None,
                taker_rate: None,
                source_epoch: None,
                source_cursor: None,
                source_record_id: None,
                source_timestamp_utc_ms: None,
            }
        };
        schedules.push(entry);
    }
    transaction.commit()?;
    Ok(EffectiveFeeScheduleSnapshot {
        schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION,
        captured_at_utc_ms: at_utc_ms,
        ledger_epoch: source_epoch,
        observed_max_cursor,
        schedules,
    })
}

fn validate_financial_evidence(event: &FinancialEvidenceEvent) -> Result<(), String> {
    let valid_id = |value: &str| {
        !value.trim().is_empty() && value.len() <= 256 && !value.chars().any(char::is_control)
    };
    let normalized_source = event
        .source_system
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_");
    if event.schema_version != FINANCIAL_EVIDENCE_SCHEMA_VERSION
        || !valid_id(&event.event_id)
        || !valid_id(&event.source_system)
        || matches!(
            normalized_source.as_str(),
            "pm" | "position_management" | "position_manager" | "auto_trader"
        )
        || !valid_id(&event.source_epoch)
        || !valid_id(&event.source_record_id)
        || event.source_cursor == 0
        || i64::try_from(event.source_cursor).is_err()
        || event.source_timestamp_utc_ms == 0
    {
        return Err("Invalid versioned financial evidence identity or source cursor".into());
    }
    let valid_currency = |currency: &str| {
        !currency.is_empty()
            && currency.len() <= 16
            && currency
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit())
    };
    let valid_finite = |value: f64| value.is_finite();
    match &event.payload {
        FinancialEvidencePayload::FeeSchedule {
            fee_schedule_id,
            fee_schedule_version,
            symbol,
            effective_from_utc_ms,
            effective_to_utc_ms,
            maker_rate,
            taker_rate,
        } => {
            if !valid_id(fee_schedule_id)
                || !valid_id(fee_schedule_version)
                || !valid_id(symbol)
                || *effective_from_utc_ms == 0
                || effective_to_utc_ms.is_some_and(|end| end <= *effective_from_utc_ms)
                || !valid_finite(*maker_rate)
                || !valid_finite(*taker_rate)
            {
                return Err("Invalid authoritative fee schedule evidence".into());
            }
        }
        FinancialEvidencePayload::SettledExecution {
            execution_id,
            order_id,
            symbol,
            side,
            quantity,
            price,
            fee,
            fee_currency,
            fee_schedule_id,
            fee_schedule_version,
            execution_origin,
        } => {
            if !valid_id(execution_id)
                || !valid_id(order_id)
                || !valid_id(symbol)
                || !matches!(side.as_str(), "buy" | "sell")
                || !valid_finite(*quantity)
                || *quantity <= 0.0
                || !valid_finite(*price)
                || *price <= 0.0
                || !valid_finite(*fee)
                || !valid_currency(fee_currency)
                || !valid_id(fee_schedule_id)
                || !valid_id(fee_schedule_version)
                || !matches!(
                    execution_origin.as_str(),
                    "market_match"
                        | "resting_limit"
                        | "protective_barrier"
                        | "vwap"
                        | "twap"
                        | "other"
                )
            {
                return Err("Invalid settled execution, fee or schedule identity".into());
            }
        }
        FinancialEvidencePayload::AccountBalance {
            portfolio_id,
            boundary,
            balances_by_currency,
        } => {
            if !valid_id(portfolio_id)
                || !matches!(boundary.as_str(), "opening" | "closing")
                || balances_by_currency.is_empty()
                || balances_by_currency
                    .iter()
                    .any(|(currency, amount)| !valid_currency(currency) || !valid_finite(*amount))
            {
                return Err("Invalid sourced account balance snapshot".into());
            }
        }
        FinancialEvidencePayload::CashMovement {
            movement_id,
            portfolio_id,
            movement_kind,
            currency,
            amount,
        } => {
            if !valid_id(movement_id)
                || !valid_id(portfolio_id)
                || !matches!(
                    movement_kind.as_str(),
                    "contribution" | "withdrawal" | "adjustment"
                )
                || !valid_currency(currency)
                || !valid_finite(*amount)
                || *amount == 0.0
            {
                return Err("Invalid sourced cash movement".into());
            }
        }
        FinancialEvidencePayload::FxMark {
            portfolio_id,
            currency,
            rate_usd,
        } => {
            if !valid_id(portfolio_id)
                || !valid_currency(currency)
                || !valid_finite(*rate_usd)
                || *rate_usd <= 0.0
                || (currency == "USD" && *rate_usd != 1.0)
            {
                return Err("Invalid sourced timestamped FX mark".into());
            }
        }
        FinancialEvidencePayload::DerivativeSettlementMark {
            portfolio_id,
            instrument,
            expiry_utc_ms,
            settlement_price,
            settlement_kind,
        } => {
            if !valid_id(portfolio_id)
                || !valid_id(instrument)
                || *expiry_utc_ms == 0
                || !valid_finite(*settlement_price)
                || *settlement_price <= 0.0
                || settlement_kind != "official_delivery_price"
                || !event
                    .source_system
                    .starts_with("binance_futures_delivery_price")
                || event.source_timestamp_utc_ms != *expiry_utc_ms
            {
                return Err("Invalid or non-official derivative settlement reference".into());
            }
        }
        FinancialEvidencePayload::CoverageAttestation {
            covered_source_epoch,
            from_source_cursor,
            through_source_cursor,
            history_complete: _,
        } => {
            if !valid_id(covered_source_epoch)
                || covered_source_epoch != &event.source_epoch
                || *from_source_cursor == 0
                || from_source_cursor > through_source_cursor
                || *through_source_cursor >= event.source_cursor
                || i64::try_from(*through_source_cursor).is_err()
            {
                return Err("Invalid source coverage attestation bounds".into());
            }
        }
    }
    Ok(())
}

fn append_financial_evidence_sync(
    path: &PathBuf,
    event: &FinancialEvidenceEvent,
) -> anyhow::Result<FinancialEvidenceAppendResponse> {
    let mut connection = open_ledger(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let (local_cursor, idempotent_replay, source_timestamp_utc_ms) =
        insert_financial_evidence_transaction(&transaction, event)?;
    let source_epoch = financial_evidence_source_epoch(&transaction)?;
    transaction.commit()?;
    Ok(FinancialEvidenceAppendResponse {
        schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION,
        event_id: event.event_id.clone(),
        source_epoch,
        source_cursor: local_cursor,
        source_timestamp_utc_ms,
        idempotent_replay,
        source_attribution_status: "source_attributed_not_cryptographically_verified",
    })
}

fn insert_financial_evidence_transaction(
    transaction: &Transaction<'_>,
    event: &FinancialEvidenceEvent,
) -> anyhow::Result<(u64, bool, u64)> {
    let payload = serde_json::to_string(event)?;
    let existing: Option<(i64, String)> = transaction
        .query_row(
            "SELECT source_cursor, event_json FROM financial_evidence_events WHERE event_id = ?1",
            [&event.event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((cursor, stored)) = existing {
        let stored_event = serde_json::from_str::<FinancialEvidenceEvent>(&stored)?;
        let same_contribution = match (&stored_event.payload, &event.payload) {
            (
                FinancialEvidencePayload::CashMovement {
                    movement_id: old_id,
                    portfolio_id: old_portfolio,
                    movement_kind: old_kind,
                    currency: old_currency,
                    amount: old_amount,
                },
                FinancialEvidencePayload::CashMovement {
                    movement_id: new_id,
                    portfolio_id: new_portfolio,
                    movement_kind: new_kind,
                    currency: new_currency,
                    amount: new_amount,
                },
            ) => {
                stored_event.schema_version == event.schema_version
                    && stored_event.event_id == event.event_id
                    && stored_event.source_system == event.source_system
                    && stored_event.source_epoch == event.source_epoch
                    && stored_event.source_record_id == event.source_record_id
                    && old_id == new_id
                    && old_portfolio == new_portfolio
                    && old_kind == new_kind
                    && old_currency == new_currency
                    && old_amount == new_amount
            }
            _ => false,
        };
        if stored_event != *event && !same_contribution {
            anyhow::bail!("financial evidence event_id was reused with different source data");
        }
        return Ok((
            u64::try_from(cursor)?,
            true,
            stored_event.source_timestamp_utc_ms,
        ));
    }
    if let FinancialEvidencePayload::SettledExecution {
        symbol,
        fee_schedule_id,
        fee_schedule_version,
        ..
    } = &event.payload
    {
        let schedule_is_valid: bool = transaction.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM financial_fee_schedules
                 WHERE fee_schedule_id = ?1 AND fee_schedule_version = ?2
                   AND symbol = ?3 AND effective_from_utc_ms <= ?4
                   AND (effective_to_utc_ms IS NULL OR ?4 < effective_to_utc_ms)
             )",
            params![
                fee_schedule_id,
                fee_schedule_version,
                symbol,
                i64::try_from(event.source_timestamp_utc_ms)?
            ],
            |row| row.get(0),
        )?;
        if !schedule_is_valid {
            anyhow::bail!("settled execution lacks its matching effective versioned fee schedule");
        }
    }
    transaction.execute(
        "INSERT INTO financial_evidence_events(event_id, event_type, source_system, source_epoch,
             provider_cursor, source_record_id, source_timestamp_utc_ms, event_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            &event.event_id,
            financial_event_type(&event.payload),
            &event.source_system,
            &event.source_epoch,
            i64::try_from(event.source_cursor)?,
            &event.source_record_id,
            i64::try_from(event.source_timestamp_utc_ms)?,
            payload,
        ],
    )?;
    let local_cursor = transaction.last_insert_rowid();
    if let FinancialEvidencePayload::FeeSchedule {
        fee_schedule_id,
        fee_schedule_version,
        symbol,
        effective_from_utc_ms,
        effective_to_utc_ms,
        maker_rate,
        taker_rate,
    } = &event.payload
    {
        transaction.execute(
            "INSERT INTO financial_fee_schedules(fee_schedule_id, fee_schedule_version,
                 symbol, effective_from_utc_ms, effective_to_utc_ms, maker_rate, taker_rate,
                 event_cursor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                fee_schedule_id,
                fee_schedule_version,
                symbol,
                i64::try_from(*effective_from_utc_ms)?,
                effective_to_utc_ms.map(i64::try_from).transpose()?,
                maker_rate,
                taker_rate,
                local_cursor,
            ],
        )?;
    }
    Ok((
        u64::try_from(local_cursor)?,
        false,
        event.source_timestamp_utc_ms,
    ))
}

fn financial_event_type(payload: &FinancialEvidencePayload) -> &'static str {
    match payload {
        FinancialEvidencePayload::FeeSchedule { .. } => "fee_schedule",
        FinancialEvidencePayload::SettledExecution { .. } => "settled_execution",
        FinancialEvidencePayload::AccountBalance { .. } => "account_balance",
        FinancialEvidencePayload::CashMovement { .. } => "cash_movement",
        FinancialEvidencePayload::FxMark { .. } => "fx_mark",
        FinancialEvidencePayload::DerivativeSettlementMark { .. } => "derivative_settlement_mark",
        FinancialEvidencePayload::CoverageAttestation { .. } => "coverage_attestation",
    }
}

fn read_financial_evidence_page(
    path: &PathBuf,
    after_cursor: u64,
    limit: usize,
    through_cursor: Option<u64>,
    through_epoch: Option<&str>,
) -> anyhow::Result<FinancialEvidencePage> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let observed_max_cursor: i64 = transaction.query_row(
        "SELECT COALESCE(MAX(source_cursor), 0) FROM financial_evidence_events",
        [],
        |row| row.get(0),
    )?;
    let source_epoch = financial_evidence_source_epoch(&transaction)?;
    if through_epoch.is_some_and(|epoch| epoch != source_epoch) {
        anyhow::bail!("financial evidence source epoch mismatch");
    }
    let max_cursor = through_cursor
        .map(i64::try_from)
        .transpose()?
        .unwrap_or(observed_max_cursor);
    if max_cursor > observed_max_cursor {
        anyhow::bail!("through_cursor exceeds current source maximum");
    }
    if i64::try_from(after_cursor)? > max_cursor {
        anyhow::bail!("after_cursor exceeds the pinned through_cursor");
    }
    let opening_capital_status = simulator_opening_capital_status(&transaction, max_cursor)?;
    let fee_schedule_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM financial_fee_schedules WHERE event_cursor <= ?1",
        [max_cursor],
        |row| row.get(0),
    )?;
    let fee_schedule_status = if fee_schedule_count > 0 {
        "versioned_simulator_fee_schedule_captured".to_owned()
    } else if configured_simulator_fee_schedule().is_none() {
        "missing_explicit_versioned_fee_schedule_configuration".to_owned()
    } else {
        "fee_schedule_capture_pending_first_match".to_owned()
    };
    let history_complete = financial_source_history_complete(
        &transaction,
        &opening_capital_status,
        fee_schedule_count,
        max_cursor,
    )?;
    let current_opening_status =
        simulator_opening_capital_status(&transaction, observed_max_cursor)?;
    let current_fee_schedule_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM financial_fee_schedules WHERE event_cursor <= ?1",
        [observed_max_cursor],
        |row| row.get(0),
    )?;
    let current_history_complete = financial_source_history_complete(
        &transaction,
        &current_opening_status,
        current_fee_schedule_count,
        observed_max_cursor,
    )?;
    // Preserve the established meaning of this safety flag: whether the
    // matcher ledger has any real execution history (probes are excluded).
    // It is intentionally a live ledger-level fact rather than a claim about
    // the pinned financial-event page; the latter is represented by `events`.
    let execution_history_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM match_fills f
         JOIN match_requests r ON r.source_cursor = f.request_cursor
         WHERE r.execution_origin != 'match_probe'",
        [],
        |row| row.get(0),
    )?;
    let execution_history_empty = execution_history_count == 0;
    let mut captured_execution_flow_types = Vec::new();
    {
        let mut statement = transaction.prepare(
            "SELECT event_json FROM financial_evidence_events
             WHERE source_cursor <= ?1 AND event_type = 'settled_execution'",
        )?;
        let mut rows = statement.query([max_cursor])?;
        while let Some(row) = rows.next()? {
            let event: FinancialEvidenceEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
            if let FinancialEvidencePayload::SettledExecution {
                execution_origin, ..
            } = event.payload
            {
                if !captured_execution_flow_types.contains(&execution_origin) {
                    captured_execution_flow_types.push(execution_origin);
                }
            }
        }
    }
    captured_execution_flow_types.sort();
    let all_execution_flow_types = [
        "market_match",
        "resting_limit",
        "protective_barrier",
        "vwap",
        "twap",
        "dated_carry",
    ];
    let uncovered_execution_flow_types = all_execution_flow_types
        .iter()
        .filter(|flow| {
            !captured_execution_flow_types
                .iter()
                .any(|captured| captured == **flow)
        })
        .map(|flow| (*flow).to_owned())
        .collect();
    let mut events = Vec::with_capacity(limit + 1);
    {
        let mut statement = transaction.prepare(
            "SELECT source_cursor, source_system, source_epoch, source_record_id,
                source_timestamp_utc_ms, event_json
         FROM financial_evidence_events WHERE source_cursor > ?1 AND source_cursor <= ?2
         ORDER BY source_cursor LIMIT ?3",
        )?;
        let mut rows = statement.query(params![
            i64::try_from(after_cursor)?,
            max_cursor,
            i64::try_from(limit + 1)?
        ])?;
        while let Some(row) = rows.next()? {
            events.push(FinancialEvidenceStoredEvent {
                source_cursor: u64::try_from(row.get::<_, i64>(0)?)?,
                source_system: row.get(1)?,
                source_epoch: row.get(2)?,
                source_record_id: row.get(3)?,
                source_timestamp_utc_ms: u64::try_from(row.get::<_, i64>(4)?)?,
                event: serde_json::from_str(&row.get::<_, String>(5)?)?,
            });
        }
    }
    let page_complete = events.len() <= limit;
    events.truncate(limit);
    let next_cursor = events.last().map(|event| event.source_cursor);
    transaction.commit()?;
    Ok(FinancialEvidencePage {
        schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION,
        evidence_kind: "paper_exchange_source_attributed_account_evidence",
        source_epoch,
        cursor_kind: "financial_event",
        after_cursor,
        next_cursor,
        max_cursor: u64::try_from(max_cursor)?,
        observed_source_max_cursor: u64::try_from(observed_max_cursor)?,
        page_size: events.len(),
        page_complete,
        source_attribution_status: "source_attributed_not_cryptographically_verified",
        // This describes the durable Paper Exchange source ledger only. The
        // portfolio reconciler still cross-checks PM fill/cash coverage.
        history_complete,
        current_history_complete,
        execution_history_empty,
        opening_capital_status,
        fee_schedule_status,
        captured_execution_flow_types,
        uncovered_execution_flow_types,
        events,
    })
}

fn financial_source_history_complete(
    connection: &Connection,
    opening_capital_status: &str,
    fee_schedule_count: i64,
    through_cursor: i64,
) -> anyhow::Result<bool> {
    if opening_capital_status != "explicit_simulator_opening_capital_captured"
        || fee_schedule_count <= 0
    {
        return Ok(false);
    }
    let (event_count, max_cursor, min_cursor): (i64, i64, i64) = connection.query_row(
        "SELECT COUNT(*), COALESCE(MAX(source_cursor), 0), COALESCE(MIN(source_cursor), 0)
         FROM financial_evidence_events WHERE source_cursor <= ?1",
        [through_cursor],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if event_count != through_cursor || max_cursor != through_cursor || min_cursor != 1 {
        return Ok(false);
    }
    let unsettled_match_fills: i64 = connection.query_row(
        "SELECT COUNT(*) FROM match_fills f
         JOIN match_requests r ON r.source_cursor = f.request_cursor
         WHERE r.execution_origin != 'match_probe'
           AND NOT EXISTS (
               SELECT 1 FROM financial_evidence_events e
               WHERE e.event_type = 'settled_execution'
                 AND e.source_system = 'paper_exchange_simulator'
                 AND e.source_record_id = f.source_execution_id
           )",
        [],
        |row| row.get(0),
    )?;
    Ok(unsettled_match_fills == 0)
}

fn simulator_opening_capital_status(
    connection: &Connection,
    through_cursor: i64,
) -> anyhow::Result<String> {
    let configured = configured_opening_capital();
    let mut statement = connection.prepare(
        "SELECT event_json FROM financial_evidence_events
         WHERE source_cursor <= ?1 AND event_type = 'account_balance'",
    )?;
    let mut rows = statement.query([through_cursor])?;
    while let Some(row) = rows.next()? {
        let event: FinancialEvidenceEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
        if let FinancialEvidencePayload::AccountBalance { boundary, .. } = &event.payload {
            if boundary == "opening" {
                return Ok(match configured.as_ref() {
                    Some((configured_portfolio, configured_amount))
                        if opening_capital_event_matches(
                            &event,
                            configured_portfolio,
                            *configured_amount,
                        ) =>
                    {
                        "explicit_simulator_opening_capital_captured".to_owned()
                    }
                    Some(_) => "existing_opening_capital_mismatches_configuration".to_owned(),
                    None if event.source_system
                        == "paper_exchange_simulator_account_configuration"
                        && event.source_record_id.starts_with("opening-capital:") =>
                    {
                        "explicit_simulator_opening_capital_captured".to_owned()
                    }
                    None => "opening_capital_present_but_explicit_configuration_missing".to_owned(),
                });
            }
        }
    }
    let existing_match_count: i64 = connection.query_row(
        "SELECT COUNT(*) FROM match_fills f
         JOIN match_requests r ON r.source_cursor = f.request_cursor
         WHERE r.execution_origin != 'match_probe'
           AND NOT EXISTS (
               SELECT 1 FROM financial_evidence_events e
               WHERE e.source_cursor <= ?1
                 AND e.event_type = 'settled_execution'
                 AND e.source_system = 'paper_exchange_simulator'
                 AND e.source_record_id = f.source_execution_id
           )",
        [through_cursor],
        |row| row.get(0),
    )?;
    if existing_match_count > 0 {
        return Ok("existing_execution_history_needs_new_account_baseline".to_owned());
    }
    if configured.is_none() {
        return Ok("missing_explicit_opening_capital_configuration".to_owned());
    }
    Ok("opening_capital_pending_initialization".to_owned())
}

fn financial_evidence_source_epoch(connection: &Connection) -> anyhow::Result<String> {
    Ok(connection.query_row(
        "SELECT source_epoch FROM financial_evidence_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

fn match_book(request: &MatchRequest, book: &BookSnapshot, fingerprint: String) -> MatchResponse {
    let buy = request.side == "buy";
    let levels = if buy { &book.asks } else { &book.bids };
    let mut remaining = request.quantity;
    let reference_price = levels.first().map(|level| level.price);
    let mut fills = Vec::with_capacity(levels.len());
    for level in levels {
        let crosses = match request.limit_price {
            Some(limit) if buy => level.price <= limit,
            Some(limit) => level.price >= limit,
            None => true,
        };
        if !crosses || remaining <= 1e-12 {
            continue;
        }
        let quantity = remaining.min(level.size);
        if quantity > 1e-12 {
            fills.push(Fill {
                quantity,
                price: level.price,
                source_execution_id: None,
                source_cursor: None,
                source_cursor_kind: None,
                recorded_at_utc_ms: None,
                quote_evidence: None,
                fee: None,
                fee_currency: None,
                fee_schedule_id: None,
                fee_schedule_version: None,
                fee_order_class: None,
                fee_schedule_evidence: None,
            });
            remaining -= quantity;
        }
    }
    let filled = request.quantity - remaining;
    let average = (filled > 1e-12).then(|| {
        fills
            .iter()
            .map(|fill| fill.quantity * fill.price)
            .sum::<f64>()
            / filled
    });
    MatchResponse {
        reference_price,
        best_bid: book.bids.first().map(|level| level.price),
        best_ask: book.asks.first().map(|level| level.price),
        worst_ask: book.asks.last().map(|level| level.price),
        book_updated_at_ms: book.updated_at_ms,
        fills,
        remaining_quantity: remaining,
        filled_quantity: filled,
        average_execution_price: average,
        schema_version: MATCH_SCHEMA_VERSION.to_owned(),
        request_schema_version: MATCH_REQUEST_SCHEMA_VERSION.to_owned(),
        order_id: request.order_id.clone(),
        parent_order_id: request.parent_order_id.clone(),
        request_fingerprint: fingerprint,
        source_epoch: None,
        source_cursor_kind: "match_request".to_owned(),
        recorded_at_utc_ms: None,
        source_cursor: None,
        capture_complete: false,
        capture_status: if request.order_id.is_some() {
            "pending".into()
        } else {
            "uncaptured_missing_order_id".into()
        },
        capture_failure_reason: request
            .order_id
            .is_none()
            .then(|| "stable_order_id_required".to_owned()),
        evidence_kind: "paper_exchange_match_evidence".into(),
        reconciliation_eligible: false,
    }
}

fn request_fingerprint(request: &MatchRequest) -> String {
    let limit = request
        .limit_price
        .map(f64::to_bits)
        .map(|bits| format!("{bits:016x}"));
    let execution_origin = request
        .execution_origin
        .as_deref()
        .unwrap_or("market_match");
    let parent_order_id = request.parent_order_id.as_deref().unwrap_or_default();
    let execution_price = request
        .execution_price_override
        .map(f64::to_bits)
        .map(|bits| format!("{bits:016x}"))
        .unwrap_or_else(|| "none".to_owned());
    format!(
        "v2:{}:{}:{:016x}:{}:{}:{}:{}",
        request.symbol,
        request.side,
        request.quantity.to_bits(),
        limit.unwrap_or_else(|| "none".to_owned()),
        execution_origin,
        execution_price,
        parent_order_id,
    )
}

fn request_fingerprint_v1(request: &MatchRequest) -> String {
    let limit = request
        .limit_price
        .map(f64::to_bits)
        .map(|bits| format!("{bits:016x}"));
    format!(
        "v1:{}:{}:{:016x}:{}",
        request.symbol,
        request.side,
        request.quantity.to_bits(),
        limit.unwrap_or_else(|| "none".to_owned())
    )
}

fn legacy_request_fingerprint(request: &MatchRequest) -> Option<String> {
    (request.parent_order_id.is_none()
        && request.execution_price_override.is_none()
        && request
            .execution_origin
            .as_deref()
            .is_none_or(|origin| origin == "market_match"))
    .then(|| request_fingerprint_v1(request))
}

fn mark_capture_incomplete(mut response: MatchResponse, reason: &str) -> MatchResponse {
    response.source_epoch = None;
    response.recorded_at_utc_ms = None;
    response.source_cursor = None;
    response.capture_complete = false;
    response.capture_status = "incomplete".to_owned();
    response.capture_failure_reason = Some(reason.to_owned());
    for fill in &mut response.fills {
        fill.source_execution_id = None;
        fill.source_cursor = None;
        fill.source_cursor_kind = None;
        fill.recorded_at_utc_ms = None;
        fill.quote_evidence = None;
    }
    response
}

fn open_ledger(path: &PathBuf) -> anyhow::Result<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let connection = Connection::open(path)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    connection.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         CREATE TABLE IF NOT EXISTS match_ledger_meta (
             id INTEGER PRIMARY KEY CHECK(id = 1),
             source_epoch TEXT NOT NULL
         );
         INSERT OR IGNORE INTO match_ledger_meta(id, source_epoch)
             VALUES (1, lower(hex(randomblob(16))));
         CREATE TABLE IF NOT EXISTS match_requests (
             source_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
             order_id TEXT NOT NULL UNIQUE,
             request_fingerprint TEXT NOT NULL,
             symbol TEXT NOT NULL,
             side TEXT NOT NULL,
             requested_quantity REAL NOT NULL,
             limit_price REAL,
             response_json TEXT NOT NULL,
             request_schema_version TEXT NOT NULL,
             recorded_at_utc_ms INTEGER NOT NULL,
             execution_origin TEXT NOT NULL DEFAULT 'legacy_match_only'
         );
         CREATE TABLE IF NOT EXISTS match_fills (
             source_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
             source_execution_id TEXT NOT NULL UNIQUE,
             request_cursor INTEGER NOT NULL REFERENCES match_requests(source_cursor),
             order_id TEXT NOT NULL,
             symbol TEXT NOT NULL,
             side TEXT NOT NULL,
             quantity REAL NOT NULL,
             price REAL NOT NULL,
             recorded_at_utc_ms INTEGER NOT NULL,
             quote_json TEXT NOT NULL
         );
         CREATE INDEX IF NOT EXISTS match_fills_order ON match_fills(order_id, source_cursor);
         CREATE TABLE IF NOT EXISTS financial_evidence_meta (
             id INTEGER PRIMARY KEY CHECK(id = 1),
             source_epoch TEXT NOT NULL
         );
         INSERT OR IGNORE INTO financial_evidence_meta(id, source_epoch)
             VALUES (1, lower(hex(randomblob(16))));
         CREATE TABLE IF NOT EXISTS financial_evidence_events (
             source_cursor INTEGER PRIMARY KEY AUTOINCREMENT,
             event_id TEXT NOT NULL UNIQUE,
             event_type TEXT NOT NULL,
             source_system TEXT NOT NULL,
             source_epoch TEXT NOT NULL,
             provider_cursor INTEGER NOT NULL,
             source_record_id TEXT NOT NULL,
             source_timestamp_utc_ms INTEGER NOT NULL,
             event_json TEXT NOT NULL,
             UNIQUE(source_system, source_epoch, source_record_id)
         );
         CREATE INDEX IF NOT EXISTS financial_evidence_source_page
             ON financial_evidence_events(source_system, source_epoch, provider_cursor);
         CREATE TABLE IF NOT EXISTS financial_fee_schedules (
             fee_schedule_id TEXT NOT NULL,
             fee_schedule_version TEXT NOT NULL,
             symbol TEXT NOT NULL,
             effective_from_utc_ms INTEGER NOT NULL,
             effective_to_utc_ms INTEGER,
             maker_rate REAL NOT NULL,
             taker_rate REAL NOT NULL,
             event_cursor INTEGER NOT NULL REFERENCES financial_evidence_events(source_cursor),
             PRIMARY KEY(fee_schedule_id, fee_schedule_version, symbol)
         );
         CREATE INDEX IF NOT EXISTS financial_fee_schedules_effective_lookup
             ON financial_fee_schedules(symbol, effective_from_utc_ms, effective_to_utc_ms, event_cursor);",
    )?;
    let mut columns = connection.prepare("PRAGMA table_info(match_requests)")?;
    let mut rows = columns.query([])?;
    let mut has_execution_origin = false;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == "execution_origin" {
            has_execution_origin = true;
            break;
        }
    }
    drop(rows);
    drop(columns);
    if !has_execution_origin {
        connection.execute_batch(
            "ALTER TABLE match_requests ADD COLUMN execution_origin TEXT NOT NULL DEFAULT 'legacy_match_only';",
        )?;
    }
    Ok(connection)
}

fn lookup_committed_match(
    path: &PathBuf,
    order_id: &str,
    fingerprint: &str,
    legacy_fingerprint: Option<&str>,
) -> anyhow::Result<Option<MatchResponse>> {
    let connection = open_ledger(path)?;
    let existing: Option<(String, String)> = connection
        .query_row(
            "SELECT request_fingerprint, response_json FROM match_requests WHERE order_id = ?1",
            [order_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match existing {
        None => Ok(None),
        Some((stored_fingerprint, payload))
            if stored_fingerprint == fingerprint
                || legacy_fingerprint == Some(stored_fingerprint.as_str()) =>
        {
            Ok(Some(serde_json::from_str(&payload)?))
        }
        Some(_) => Err(anyhow::anyhow!(
            "order_id was already used for a different match request"
        )),
    }
}

fn configured_simulator_fee_schedule() -> Option<SimulatorFeeScheduleConfig> {
    let fee_schedule_id = std::env::var("PAPER_EXCHANGE_FEE_SCHEDULE_ID").ok()?;
    let fee_schedule_version = std::env::var("PAPER_EXCHANGE_FEE_SCHEDULE_VERSION").ok()?;
    let maker_rate = std::env::var("PAPER_OKX_SPOT_MAKER_FEE_RATE")
        .ok()?
        .parse::<f64>()
        .ok()?;
    let taker_rate = std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE")
        .ok()?
        .parse::<f64>()
        .ok()?;
    if fee_schedule_id.trim().is_empty()
        || fee_schedule_version.trim().is_empty()
        || !maker_rate.is_finite()
        || !(0.0..=0.1).contains(&maker_rate.abs())
        || !taker_rate.is_finite()
        || !(0.0..=0.1).contains(&taker_rate.abs())
    {
        return None;
    }
    Some(SimulatorFeeScheduleConfig {
        fee_schedule_id,
        fee_schedule_version,
        maker_rate: maker_rate.abs(),
        taker_rate: taker_rate.abs(),
    })
}

fn configured_opening_capital() -> Option<(String, f64)> {
    let portfolio_id = std::env::var("PAPER_EXCHANGE_PORTFOLIO_ID").ok()?;
    let amount = std::env::var("PAPER_ACCOUNT_INITIAL_CASH")
        .ok()?
        .parse::<f64>()
        .ok()?;
    if portfolio_id.trim().is_empty() || !amount.is_finite() || amount <= 0.0 {
        return None;
    }
    Some((portfolio_id, amount))
}

fn opening_capital_event_matches(
    event: &FinancialEvidenceEvent,
    expected_portfolio_id: &str,
    expected_amount_usd: f64,
) -> bool {
    matches!(
        &event.payload,
        FinancialEvidencePayload::AccountBalance {
            portfolio_id,
            boundary,
            balances_by_currency,
        } if portfolio_id == expected_portfolio_id
            && boundary == "opening"
            && balances_by_currency.len() == 1
            && balances_by_currency.get("USD") == Some(&expected_amount_usd)
    )
}

fn ensure_simulator_opening_capital(
    transaction: &Transaction<'_>,
) -> anyhow::Result<Option<&'static str>> {
    ensure_simulator_opening_capital_with(transaction, configured_opening_capital())
}

fn ensure_simulator_opening_capital_with(
    transaction: &Transaction<'_>,
    opening_capital: Option<(String, f64)>,
) -> anyhow::Result<Option<&'static str>> {
    let mut statement = transaction.prepare(
        "SELECT event_json FROM financial_evidence_events WHERE event_type = 'account_balance'",
    )?;
    let mut rows = statement.query([])?;
    while let Some(row) = rows.next()? {
        let event: FinancialEvidenceEvent = serde_json::from_str(&row.get::<_, String>(0)?)?;
        if let FinancialEvidencePayload::AccountBalance { boundary, .. } = &event.payload {
            if boundary == "opening" {
                let matches_configuration =
                    opening_capital
                        .as_ref()
                        .is_some_and(|(portfolio_id, amount)| {
                            opening_capital_event_matches(&event, portfolio_id, *amount)
                        });
                let persisted_configuration = event.source_system
                    == "paper_exchange_simulator_account_configuration"
                    && event.source_record_id.starts_with("opening-capital:");
                return Ok(if matches_configuration {
                    None
                } else if opening_capital.is_none() && persisted_configuration {
                    None
                } else {
                    Some("existing_opening_capital_mismatches_configuration")
                });
            }
        }
    }
    let existing_match_count: i64 = transaction.query_row(
        "SELECT COUNT(*) FROM match_fills f
         JOIN match_requests r ON r.source_cursor = f.request_cursor
         WHERE r.execution_origin != 'match_probe'",
        [],
        |row| row.get(0),
    )?;
    if existing_match_count > 0 {
        return Ok(Some(
            "existing_execution_history_requires_new_account_baseline",
        ));
    }
    let Some((portfolio_id, amount)) = opening_capital else {
        return Ok(Some("missing_explicit_opening_capital_configuration"));
    };
    let timestamp = now_ms()?;
    let source_epoch = ledger_source_epoch(transaction)?;
    let event = FinancialEvidenceEvent {
        schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION.to_owned(),
        event_id: format!("paper-opening-capital:{portfolio_id}:{source_epoch}"),
        source_system: "paper_exchange_simulator_account_configuration".to_owned(),
        source_epoch,
        source_cursor: 1,
        source_record_id: format!("opening-capital:{portfolio_id}"),
        source_timestamp_utc_ms: timestamp,
        payload: FinancialEvidencePayload::AccountBalance {
            portfolio_id,
            boundary: "opening".to_owned(),
            balances_by_currency: std::collections::BTreeMap::from([("USD".to_owned(), amount)]),
        },
    };
    validate_financial_evidence(&event).map_err(anyhow::Error::msg)?;
    insert_financial_evidence_transaction(transaction, &event)?;
    Ok(None)
}

fn ensure_simulator_fee_schedule(
    transaction: &Transaction<'_>,
    symbol: &str,
    effective_from_utc_ms: u64,
    request_cursor: i64,
    config: &SimulatorFeeScheduleConfig,
) -> anyhow::Result<FinancialEvidenceEvent> {
    let Some((_, _quote_currency)) = symbol.rsplit_once('-') else {
        anyhow::bail!("symbol has no quote currency for fee schedule attribution");
    };
    let existing_rates: Option<(f64, f64)> = transaction
        .query_row(
            "SELECT maker_rate, taker_rate FROM financial_fee_schedules
             WHERE fee_schedule_id = ?1 AND fee_schedule_version = ?2 AND symbol = ?3",
            params![config.fee_schedule_id, config.fee_schedule_version, symbol],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((maker_rate, taker_rate)) = existing_rates {
        if maker_rate == config.maker_rate && taker_rate == config.taker_rate {
            let event_id = format!(
                "paper-fee-schedule:{}:{}:{symbol}",
                config.fee_schedule_id, config.fee_schedule_version
            );
            let event_json: String = transaction.query_row(
                "SELECT event_json FROM financial_evidence_events WHERE event_id = ?1",
                [event_id],
                |row| row.get(0),
            )?;
            return Ok(serde_json::from_str(&event_json)?);
        }
        anyhow::bail!("configured fee schedule version was reused with changed rates");
    }
    let source_epoch = ledger_source_epoch(transaction)?;
    let event = FinancialEvidenceEvent {
        schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION.to_owned(),
        event_id: format!(
            "paper-fee-schedule:{}:{}:{symbol}",
            config.fee_schedule_id, config.fee_schedule_version
        ),
        source_system: "paper_exchange_simulator_configuration".to_owned(),
        source_epoch,
        source_cursor: u64::try_from(request_cursor)?,
        source_record_id: format!(
            "configured-fee-schedule:{}:{}:{symbol}",
            config.fee_schedule_id, config.fee_schedule_version
        ),
        source_timestamp_utc_ms: effective_from_utc_ms,
        payload: FinancialEvidencePayload::FeeSchedule {
            fee_schedule_id: config.fee_schedule_id.clone(),
            fee_schedule_version: config.fee_schedule_version.clone(),
            symbol: symbol.to_owned(),
            effective_from_utc_ms,
            effective_to_utc_ms: None,
            maker_rate: config.maker_rate,
            taker_rate: config.taker_rate,
        },
    };
    validate_financial_evidence(&event).map_err(anyhow::Error::msg)?;
    insert_financial_evidence_transaction(transaction, &event)?;
    Ok(event)
}

fn commit_match(
    path: &PathBuf,
    order_id: &str,
    fingerprint: &str,
    request: &MatchRequest,
    response: MatchResponse,
) -> anyhow::Result<MatchResponse> {
    commit_match_with_config(
        path,
        order_id,
        fingerprint,
        request,
        response,
        configured_simulator_fee_schedule(),
        configured_opening_capital(),
    )
}

fn commit_match_with_config(
    path: &PathBuf,
    order_id: &str,
    fingerprint: &str,
    request: &MatchRequest,
    mut response: MatchResponse,
    fee_schedule_config: Option<SimulatorFeeScheduleConfig>,
    opening_capital: Option<(String, f64)>,
) -> anyhow::Result<MatchResponse> {
    let mut connection = open_ledger(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let existing: Option<(String, String)> = transaction
        .query_row(
            "SELECT request_fingerprint, response_json FROM match_requests WHERE order_id = ?1",
            [order_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    if let Some((stored_fingerprint, payload)) = existing {
        let legacy_fingerprint = legacy_request_fingerprint(request);
        if stored_fingerprint != fingerprint
            && legacy_fingerprint.as_deref() != Some(stored_fingerprint.as_str())
        {
            anyhow::bail!("order_id was already used for a different match request");
        }
        return Ok(serde_json::from_str(&payload)?);
    }

    let _opening_capital_status =
        ensure_simulator_opening_capital_with(&transaction, opening_capital)?;
    let recorded_at_utc_ms = now_ms()?;
    transaction.execute(
        "INSERT INTO match_requests(order_id, request_fingerprint, symbol, side, requested_quantity,
                                   limit_price, response_json, request_schema_version, recorded_at_utc_ms,
                                   execution_origin)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7, ?8, ?9)",
        params![
            order_id,
            fingerprint,
            request.symbol,
            request.side,
            request.quantity,
            request.limit_price,
            MATCH_REQUEST_SCHEMA_VERSION,
            i64::try_from(recorded_at_utc_ms)?,
            request.execution_origin.as_deref().unwrap_or("market_match"),
        ],
    )?;
    let request_cursor = transaction.last_insert_rowid();
    let fee_schedule_evidence = if let Some(config) = &fee_schedule_config {
        Some(ensure_simulator_fee_schedule(
            &transaction,
            &request.symbol,
            recorded_at_utc_ms,
            request_cursor,
            config,
        )?)
    } else {
        None
    };
    response.order_id = Some(order_id.to_owned());
    response.parent_order_id = request.parent_order_id.clone();
    response.source_epoch = Some(ledger_source_epoch(&transaction)?);
    response.recorded_at_utc_ms = Some(recorded_at_utc_ms);
    response.source_cursor = Some(u64::try_from(request_cursor)?);
    response.capture_complete = true;
    response.capture_status = "durably_captured".to_owned();
    response.capture_failure_reason = None;
    for (index, fill) in response.fills.iter_mut().enumerate() {
        let execution_id = format!("paper-match-{request_cursor}-{index}");
        let settled_parent_order_id = request.parent_order_id.as_deref().unwrap_or(order_id);
        let quote = QuoteEvidence {
            reference_price: response.reference_price,
            best_bid: response.best_bid,
            best_ask: response.best_ask,
            worst_ask: response.worst_ask,
            book_updated_at_ms: response.book_updated_at_ms,
        };
        let quote_json = serde_json::to_string(&quote)?;
        transaction.execute(
            "INSERT INTO match_fills(source_execution_id, request_cursor, order_id, symbol, side,
                                     quantity, price, recorded_at_utc_ms, quote_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                execution_id,
                request_cursor,
                settled_parent_order_id,
                &request.symbol,
                &request.side,
                fill.quantity,
                fill.price,
                i64::try_from(recorded_at_utc_ms)?,
                quote_json
            ],
        )?;
        fill.source_execution_id = Some(execution_id.clone());
        let fill_cursor = transaction.last_insert_rowid();
        fill.source_cursor = Some(u64::try_from(fill_cursor)?);
        fill.source_cursor_kind = Some("match_fill".to_owned());
        fill.recorded_at_utc_ms = Some(recorded_at_utc_ms);
        fill.quote_evidence = Some(quote);
        let execution_origin = request
            .execution_origin
            .as_deref()
            .unwrap_or("market_match");
        if execution_origin != "match_probe" {
            if let (Some(config), Some((_, quote_currency))) = (
                fee_schedule_config.as_ref(),
                request.symbol.rsplit_once('-'),
            ) {
                let fee_rate = if execution_origin == "resting_limit" {
                    config.maker_rate
                } else {
                    config.taker_rate
                };
                let fee = -(fill.quantity * fill.price * fee_rate);
                if !fee.is_finite() {
                    anyhow::bail!("simulated settled fee amount is non-finite");
                }
                fill.fee = Some(fee);
                fill.fee_currency = Some(quote_currency.to_owned());
                fill.fee_schedule_id = Some(config.fee_schedule_id.clone());
                fill.fee_schedule_version = Some(config.fee_schedule_version.clone());
                fill.fee_order_class = Some(if execution_origin == "resting_limit" {
                    "maker".to_owned()
                } else {
                    "taker".to_owned()
                });
                fill.fee_schedule_evidence = fee_schedule_evidence.clone();
                let execution_event = FinancialEvidenceEvent {
                    schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION.to_owned(),
                    event_id: format!("paper-settlement:{execution_id}"),
                    source_system: "paper_exchange_simulator".to_owned(),
                    source_epoch: ledger_source_epoch(&transaction)?,
                    source_cursor: u64::try_from(fill_cursor)?,
                    source_record_id: execution_id.clone(),
                    source_timestamp_utc_ms: recorded_at_utc_ms,
                    payload: FinancialEvidencePayload::SettledExecution {
                        execution_id: execution_id.clone(),
                        order_id: settled_parent_order_id.to_owned(),
                        symbol: request.symbol.clone(),
                        side: request.side.clone(),
                        quantity: fill.quantity,
                        price: fill.price,
                        fee,
                        fee_currency: quote_currency.to_owned(),
                        fee_schedule_id: config.fee_schedule_id.clone(),
                        fee_schedule_version: config.fee_schedule_version.clone(),
                        execution_origin: execution_origin.to_owned(),
                    },
                };
                validate_financial_evidence(&execution_event).map_err(anyhow::Error::msg)?;
                insert_financial_evidence_transaction(&transaction, &execution_event)?;
            }
        }
    }
    let serialized = serde_json::to_string(&response)?;
    transaction.execute(
        "UPDATE match_requests SET response_json = ?1 WHERE source_cursor = ?2",
        params![serialized, request_cursor],
    )?;
    transaction.commit()?;
    Ok(response)
}

fn read_ledger_page(path: &PathBuf, after_cursor: u64, limit: usize) -> anyhow::Result<LedgerPage> {
    let mut connection = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Deferred)?;
    let page = read_ledger_page_snapshot(&transaction, after_cursor, limit)?;
    transaction.commit()?;
    Ok(page)
}

fn read_ledger_page_snapshot(
    snapshot: &Transaction<'_>,
    after_cursor: u64,
    limit: usize,
) -> anyhow::Result<LedgerPage> {
    let max_cursor: i64 = snapshot.query_row(
        "SELECT COALESCE(MAX(source_cursor), 0) FROM match_fills",
        [],
        |row| row.get(0),
    )?;
    let mut statement = snapshot.prepare(
        "SELECT f.source_cursor, f.source_execution_id, f.order_id, r.request_fingerprint,
                f.symbol, f.side, f.quantity, f.price, f.recorded_at_utc_ms, f.quote_json
         FROM match_fills f JOIN match_requests r ON r.source_cursor = f.request_cursor
         WHERE f.source_cursor > ?1 AND f.source_cursor <= ?2
         ORDER BY f.source_cursor LIMIT ?3",
    )?;
    let mut rows = statement.query(params![
        i64::try_from(after_cursor)?,
        max_cursor,
        i64::try_from(limit + 1)?
    ])?;
    let mut result = Vec::with_capacity(limit + 1);
    while let Some(row) = rows.next()? {
        let timestamp: i64 = row.get(8)?;
        result.push(LedgerFill {
            source_cursor: u64::try_from(row.get::<_, i64>(0)?)?,
            source_execution_id: row.get(1)?,
            order_id: row.get(2)?,
            request_fingerprint: row.get(3)?,
            symbol: row.get(4)?,
            side: row.get(5)?,
            quantity: row.get(6)?,
            price: row.get(7)?,
            recorded_at_utc_ms: u64::try_from(timestamp)?,
            quote_evidence: serde_json::from_str(&row.get::<_, String>(9)?)?,
        });
    }
    let complete = result.len() <= limit;
    result.truncate(limit);
    let identity_complete = result.iter().all(|row| !row.order_id.is_empty());
    let next_cursor = result.last().map(|row| row.source_cursor);
    Ok(LedgerPage {
        schema_version: LEDGER_SCHEMA_VERSION,
        evidence_kind: "paper_exchange_match_evidence",
        capture_scope: "matches_with_caller_supplied_order_id_only",
        reconciliation_eligible: false,
        source_epoch: ledger_source_epoch(snapshot)?,
        cursor_kind: "match_fill",
        history_complete: false,
        after_cursor,
        next_cursor,
        max_cursor: u64::try_from(max_cursor)?,
        page_size: result.len(),
        page_complete: complete,
        identity_complete,
        rows: result,
    })
}

fn ledger_source_epoch(connection: &Connection) -> anyhow::Result<String> {
    Ok(connection.query_row(
        "SELECT source_epoch FROM match_ledger_meta WHERE id = 1",
        [],
        |row| row.get(0),
    )?)
}

fn database_error(error: anyhow::Error) -> (StatusCode, String) {
    let message = error.to_string();
    if is_identity_conflict(&error) {
        (StatusCode::CONFLICT, message)
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            format!("Matching evidence unavailable: {message}"),
        )
    }
}

fn is_identity_conflict(error: &anyhow::Error) -> bool {
    let message = error.to_string();
    message.contains("already used for a different match request")
        || message.contains("financial evidence event_id was reused with different source data")
}

fn database_join_error(error: tokio::task::JoinError) -> (StatusCode, String) {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        format!("Matching evidence worker failed: {error}"),
    )
}

fn now_ms() -> anyhow::Result<u64> {
    Ok(std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64)
}

pub fn router(market: Arc<MarketData>) -> anyhow::Result<Router> {
    let ledger_path = std::env::var_os("PAPER_EXCHANGE_EXECUTION_LEDGER_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("paper_exchange_matches.sqlite")
        });
    initialize_simulator_account_evidence(&ledger_path)?;
    Ok(router_with_path(market, ledger_path))
}

fn initialize_simulator_account_evidence(path: &PathBuf) -> anyhow::Result<()> {
    let mut connection = open_ledger(path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if let Some(reason) = ensure_simulator_opening_capital(&transaction)? {
        crate::service_log!(
            "WARN",
            "Independent opening capital evidence unavailable: {reason}"
        );
    }
    transaction.commit()?;
    Ok(())
}

fn router_with_path(market: Arc<MarketData>, ledger_path: PathBuf) -> Router {
    let state = OrderApiState {
        market,
        ledger_path,
    };
    Router::new()
        .route("/internal/orders/match", post(match_order))
        .route("/internal/orders/match/ledger", get(ledger_history))
        .route(
            "/internal/financial/evidence",
            post(append_financial_evidence).get(financial_evidence_history),
        )
        .route(
            "/internal/financial/fee-schedules",
            get(effective_fee_schedules),
        )
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::BookLevel;
    use std::collections::BTreeMap;

    fn book() -> BookSnapshot {
        BookSnapshot {
            bids: vec![BookLevel {
                price: 10.0,
                size: 2.0,
            }],
            asks: vec![
                BookLevel {
                    price: 10.5,
                    size: 1.0,
                },
                BookLevel {
                    price: 11.0,
                    size: 2.0,
                },
            ],
            updated_at_ms: 1,
            source_epoch: None,
            source_cursor: None,
            source_record_id: None,
        }
    }

    fn request(order_id: Option<&str>, quantity: f64) -> MatchRequest {
        MatchRequest {
            symbol: "BTC-USDT".into(),
            side: "buy".into(),
            quantity,
            limit_price: Some(11.0),
            order_id: order_id.map(str::to_owned),
            parent_order_id: None,
            execution_origin: None,
            execution_price_override: None,
        }
    }

    fn temp_db(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "paper-exchange-ledger-{}-{}-{name}",
            std::process::id(),
            now_ms().unwrap()
        ));
        std::fs::create_dir_all(&root).unwrap();
        root.join("ledger.sqlite")
    }

    fn financial_event(
        event_id: &str,
        source_cursor: u64,
        payload: FinancialEvidencePayload,
    ) -> FinancialEvidenceEvent {
        FinancialEvidenceEvent {
            schema_version: FINANCIAL_EVIDENCE_SCHEMA_VERSION.to_owned(),
            event_id: event_id.to_owned(),
            source_system: "paper-account-provider-fixture".to_owned(),
            source_epoch: "provider-epoch-fixture".to_owned(),
            source_cursor,
            source_record_id: format!("provider-record-{source_cursor}"),
            source_timestamp_utc_ms: 1_000 + source_cursor,
            payload,
        }
    }

    fn fixture_fee_schedule() -> FinancialEvidenceEvent {
        financial_event(
            "fee-schedule-1",
            1,
            FinancialEvidencePayload::FeeSchedule {
                fee_schedule_id: "account-fees".to_owned(),
                fee_schedule_version: "2026-01".to_owned(),
                symbol: "BTC-USDT".to_owned(),
                effective_from_utc_ms: 1_000,
                effective_to_utc_ms: Some(10_000),
                maker_rate: 0.001,
                taker_rate: 0.002,
            },
        )
    }

    #[test]
    fn effective_fee_schedule_lookup_is_bounded_and_fails_closed_for_overlap() {
        let path = temp_db("effective-fee-schedule");
        append_financial_evidence_sync(&path, &fixture_fee_schedule()).unwrap();
        let mut db = open_ledger(&path).unwrap();
        let tx = db.transaction().unwrap();
        for cursor in 2..=5_001_u64 {
            tx.execute(
                "INSERT INTO financial_evidence_events(event_id,event_type,source_system,source_epoch,
                     provider_cursor,source_record_id,source_timestamp_utc_ms,event_json)
                 VALUES(?1,'unrelated_fixture','paper-account-provider-fixture','provider-epoch-fixture',?2,?3,1001,'{}')",
                rusqlite::params![format!("unrelated-{cursor}"), i64::try_from(cursor).unwrap(), format!("unrelated-record-{cursor}")],
            ).unwrap();
        }
        tx.commit().unwrap();
        let matched =
            effective_fee_schedule_snapshot(&path, &["BTC-USDT".into(), "ETH-USDT".into()], 1_500)
                .unwrap();
        assert_eq!(matched.observed_max_cursor, 5_001);
        assert_eq!(
            matched.schedules.len(),
            2,
            "a single batch lookup returns every requested symbol"
        );
        assert_eq!(matched.schedules[0].status, "matched");
        assert_eq!(
            matched.schedules[0].fee_schedule_id.as_deref(),
            Some("account-fees")
        );
        assert_eq!(matched.schedules[0].source_cursor, Some(1));
        assert_eq!(
            matched.schedules[0].source_record_id.as_deref(),
            Some("provider-record-1")
        );
        assert_eq!(matched.schedules[1].status, "missing");

        let missing = effective_fee_schedule_snapshot(&path, &["BTC-USDT".into()], 10_000).unwrap();
        assert_eq!(missing.schedules[0].status, "missing");

        append_financial_evidence_sync(
            &path,
            &financial_event(
                "overlapping-fee-schedule",
                2,
                FinancialEvidencePayload::FeeSchedule {
                    fee_schedule_id: "second-fees".into(),
                    fee_schedule_version: "v2".into(),
                    symbol: "BTC-USDT".into(),
                    effective_from_utc_ms: 1_400,
                    effective_to_utc_ms: None,
                    maker_rate: 0.0,
                    taker_rate: 0.003,
                },
            ),
        )
        .unwrap();
        let ambiguous =
            effective_fee_schedule_snapshot(&path, &["BTC-USDT".into()], 1_500).unwrap();
        assert_eq!(ambiguous.schedules[0].status, "ambiguous");
        let _ = std::fs::remove_file(&path);
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir_all(parent);
        }
    }

    fn fixture_settled_fill() -> FinancialEvidenceEvent {
        financial_event(
            "settlement-1",
            2,
            FinancialEvidencePayload::SettledExecution {
                execution_id: "provider-fill-1".to_owned(),
                order_id: "order-1".to_owned(),
                symbol: "BTC-USDT".to_owned(),
                side: "buy".to_owned(),
                quantity: 0.5,
                price: 100.0,
                fee: -0.1,
                fee_currency: "USDT".to_owned(),
                fee_schedule_id: "account-fees".to_owned(),
                fee_schedule_version: "2026-01".to_owned(),
                execution_origin: "market_match".to_owned(),
            },
        )
    }

    #[test]
    fn financial_evidence_is_versioned_idempotent_restart_safe_and_keeps_provenance() {
        let path = temp_db("financial-evidence-restart");
        let schedule = fixture_fee_schedule();
        validate_financial_evidence(&schedule).unwrap();
        let schedule_saved = append_financial_evidence_sync(&path, &schedule).unwrap();
        assert!(!schedule_saved.idempotent_replay);

        let settlement = fixture_settled_fill();
        let fill_saved = append_financial_evidence_sync(&path, &settlement).unwrap();
        assert!(!fill_saved.idempotent_replay);
        let replayed = append_financial_evidence_sync(&path, &settlement).unwrap();
        assert!(replayed.idempotent_replay);
        assert_eq!(replayed.source_cursor, fill_saved.source_cursor);
        assert_eq!(replayed.source_epoch, fill_saved.source_epoch);

        let mut changed_replay = settlement.clone();
        if let FinancialEvidencePayload::SettledExecution { fee, .. } = &mut changed_replay.payload
        {
            *fee = -0.2;
        }
        assert!(append_financial_evidence_sync(&path, &changed_replay).is_err());

        let mut balance = financial_event(
            "opening-balance-1",
            3,
            FinancialEvidencePayload::AccountBalance {
                portfolio_id: "paper-default".to_owned(),
                boundary: "opening".to_owned(),
                balances_by_currency: BTreeMap::from([("USD".to_owned(), 10_000.0)]),
            },
        );
        balance.source_system = "paper_exchange_simulator_account_configuration".to_owned();
        balance.source_record_id = "opening-capital:paper-default".to_owned();
        let contribution = financial_event(
            "contribution-1",
            4,
            FinancialEvidencePayload::CashMovement {
                movement_id: "deposit-1".to_owned(),
                portfolio_id: "paper-default".to_owned(),
                movement_kind: "contribution".to_owned(),
                currency: "USD".to_owned(),
                amount: 250.0,
            },
        );
        let fx = financial_event(
            "fx-usdt-1",
            5,
            FinancialEvidencePayload::FxMark {
                portfolio_id: "paper-default".to_owned(),
                currency: "USDT".to_owned(),
                rate_usd: 0.9998,
            },
        );
        for event in [&balance, &contribution, &fx] {
            validate_financial_evidence(event).unwrap();
            append_financial_evidence_sync(&path, event).unwrap();
        }

        // Reopen the database to exercise the same code path used after process restart.
        let page = read_financial_evidence_page(&path, 0, 10, None, None).unwrap();
        assert_eq!(page.schema_version, FINANCIAL_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(
            page.evidence_kind,
            "paper_exchange_source_attributed_account_evidence"
        );
        assert_eq!(page.cursor_kind, "financial_event");
        assert_eq!(page.events.len(), 5);
        assert!(page.page_complete);
        assert!(page.history_complete);
        assert_eq!(page.source_epoch, schedule_saved.source_epoch);
        assert_eq!(page.events[1].source_record_id, settlement.source_record_id);
        assert_eq!(page.events[1].event, settlement);
        assert_eq!(page.events[2].event, balance);
        assert_eq!(page.events[3].event, contribution);
        assert_eq!(page.events[4].event, fx);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn financial_evidence_rejects_missing_schedule_identity_bad_fx_and_cursor_reuse() {
        let path = temp_db("financial-evidence-invalid");
        let settlement = fixture_settled_fill();
        validate_financial_evidence(&settlement).unwrap();
        assert!(append_financial_evidence_sync(&path, &settlement).is_err());

        let invalid_fx = financial_event(
            "fx-invalid",
            1,
            FinancialEvidencePayload::FxMark {
                portfolio_id: "paper-default".to_owned(),
                currency: "USD".to_owned(),
                rate_usd: 1.01,
            },
        );
        assert!(validate_financial_evidence(&invalid_fx).is_err());
        let mut pm_copied_event = invalid_fx.clone();
        pm_copied_event.source_system = "position_management".to_owned();
        assert!(validate_financial_evidence(&pm_copied_event).is_err());
        let mut missing_source_record = invalid_fx.clone();
        missing_source_record.source_record_id.clear();
        assert!(validate_financial_evidence(&missing_source_record).is_err());

        let schedule = fixture_fee_schedule();
        append_financial_evidence_sync(&path, &schedule).unwrap();
        append_financial_evidence_sync(&path, &settlement).unwrap();
        let cursor_conflict = financial_event(
            "different-event-same-provider-cursor",
            2,
            FinancialEvidencePayload::CashMovement {
                movement_id: "movement-2".to_owned(),
                portfolio_id: "paper-default".to_owned(),
                movement_kind: "withdrawal".to_owned(),
                currency: "USD".to_owned(),
                amount: -10.0,
            },
        );
        assert!(append_financial_evidence_sync(&path, &cursor_conflict).is_err());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn financial_evidence_pinned_cursor_stays_stable_across_appends_and_rejects_invalid_bounds() {
        let path = temp_db("financial-evidence-pinned-cursor");
        let schedule = fixture_fee_schedule();
        let mut balance = financial_event(
            "opening-balance-pinned",
            2,
            FinancialEvidencePayload::AccountBalance {
                portfolio_id: "paper-default".to_owned(),
                boundary: "opening".to_owned(),
                balances_by_currency: BTreeMap::from([("USD".to_owned(), 10_000.0)]),
            },
        );
        balance.source_system = "paper_exchange_simulator_account_configuration".to_owned();
        balance.source_record_id = "opening-capital:paper-default".to_owned();
        let contribution = financial_event(
            "contribution-pinned",
            3,
            FinancialEvidencePayload::CashMovement {
                movement_id: "deposit-pinned".to_owned(),
                portfolio_id: "paper-default".to_owned(),
                movement_kind: "contribution".to_owned(),
                currency: "USD".to_owned(),
                amount: 250.0,
            },
        );
        let fx = financial_event(
            "fx-pinned",
            4,
            FinancialEvidencePayload::FxMark {
                portfolio_id: "paper-default".to_owned(),
                currency: "USDT".to_owned(),
                rate_usd: 0.9998,
            },
        );
        for event in [&schedule, &balance, &contribution, &fx] {
            append_financial_evidence_sync(&path, event).unwrap();
        }

        let first = read_financial_evidence_page(&path, 0, 2, None, None).unwrap();
        assert_eq!(first.max_cursor, 4);
        assert_eq!(first.observed_source_max_cursor, 4);
        assert!(!first.page_complete);
        let pinned_cursor = first.max_cursor;
        let pinned_epoch = first.source_epoch.clone();

        let appended = financial_event(
            "contribution-appended-after-first-page",
            5,
            FinancialEvidencePayload::CashMovement {
                movement_id: "deposit-after-snapshot".to_owned(),
                portfolio_id: "paper-default".to_owned(),
                movement_kind: "contribution".to_owned(),
                currency: "USD".to_owned(),
                amount: 125.0,
            },
        );
        append_financial_evidence_sync(&path, &appended).unwrap();

        let second = read_financial_evidence_page(
            &path,
            first.next_cursor.unwrap(),
            2,
            Some(pinned_cursor),
            Some(&pinned_epoch),
        )
        .unwrap();
        assert_eq!(second.source_epoch, pinned_epoch);
        assert_eq!(second.max_cursor, pinned_cursor);
        assert_eq!(second.observed_source_max_cursor, 5);
        assert!(second.page_complete);
        assert!(second.history_complete);
        assert!(second.current_history_complete);
        assert!(
            second
                .events
                .iter()
                .all(|event| event.source_cursor <= pinned_cursor)
        );
        assert!(
            !second
                .events
                .iter()
                .any(|event| event.event.event_id == appended.event_id)
        );

        assert!(read_financial_evidence_page(&path, 0, 2, Some(6), Some(&pinned_epoch)).is_err());
        assert!(
            read_financial_evidence_page(&path, 5, 2, Some(pinned_cursor), Some(&pinned_epoch))
                .is_err()
        );
        assert!(
            read_financial_evidence_page(&path, 0, 2, Some(pinned_cursor), Some("wrong-epoch"))
                .is_err()
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn existing_opening_capital_mismatch_is_not_reported_as_captured() {
        let event = financial_event(
            "opening-mismatch-test",
            1,
            FinancialEvidencePayload::AccountBalance {
                portfolio_id: "paper-default".to_owned(),
                boundary: "opening".to_owned(),
                balances_by_currency: BTreeMap::from([("USD".to_owned(), 10_000.0)]),
            },
        );
        assert!(opening_capital_event_matches(
            &event,
            "paper-default",
            10_000.0
        ));
        assert!(!opening_capital_event_matches(
            &event,
            "paper-default",
            9_999.0
        ));
        assert!(!opening_capital_event_matches(
            &event,
            "other-paper-account",
            10_000.0
        ));
    }

    #[test]
    fn financial_evidence_tables_migrate_additively_from_existing_match_ledger() {
        let path = temp_db("financial-evidence-migration");
        let connection = Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE match_ledger_meta (id INTEGER PRIMARY KEY, source_epoch TEXT NOT NULL);
                 CREATE TABLE match_requests (source_cursor INTEGER PRIMARY KEY, order_id TEXT);
                 CREATE TABLE match_fills (source_cursor INTEGER PRIMARY KEY, order_id TEXT);",
            )
            .unwrap();
        drop(connection);
        open_ledger(&path).unwrap();
        let table_count: i64 = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table'
                 AND name IN ('match_fills', 'financial_evidence_events', 'financial_fee_schedules')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 3);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn market_match_commits_versioned_simulator_fees_and_opening_capital_atomically() {
        let path = temp_db("simulator-financial-capture");
        let order = request(Some("simulator-order-1"), 1.0);
        let response = commit_match_with_config(
            &path,
            "simulator-order-1",
            &request_fingerprint(&order),
            &order,
            match_book(&order, &book(), request_fingerprint(&order)),
            Some(SimulatorFeeScheduleConfig {
                fee_schedule_id: "paper-configured-fees".to_owned(),
                fee_schedule_version: "paper-config-v1".to_owned(),
                maker_rate: 0.001,
                taker_rate: 0.002,
            }),
            Some(("paper-default".to_owned(), 10_000.0)),
        )
        .unwrap();

        assert!(response.capture_complete);
        assert_eq!(response.fills.len(), 1);
        assert_eq!(response.fills[0].fee, Some(-0.021));
        assert_eq!(response.fills[0].fee_currency.as_deref(), Some("USDT"));
        assert_eq!(
            response.fills[0].fee_schedule_version.as_deref(),
            Some("paper-config-v1")
        );
        assert_eq!(response.fills[0].fee_order_class.as_deref(), Some("taker"));
        let schedule = response.fills[0]
            .fee_schedule_evidence
            .as_ref()
            .expect("settled fill carries the exact fee-schedule source event");
        assert_eq!(
            schedule.source_system,
            "paper_exchange_simulator_configuration"
        );
        assert!(!schedule.source_epoch.trim().is_empty());
        assert!(!schedule.source_record_id.trim().is_empty());
        assert!(schedule.source_cursor > 0);
        assert!(matches!(
            &schedule.payload,
            FinancialEvidencePayload::FeeSchedule {
                fee_schedule_version,
                taker_rate,
                ..
            } if fee_schedule_version == "paper-config-v1" && *taker_rate == 0.002
        ));
        let page = read_financial_evidence_page(&path, 0, 20, None, None).unwrap();
        assert_eq!(page.events.len(), 3);
        assert_eq!(
            page.opening_capital_status,
            "explicit_simulator_opening_capital_captured"
        );
        assert_eq!(
            page.fee_schedule_status,
            "versioned_simulator_fee_schedule_captured"
        );
        assert_eq!(page.captured_execution_flow_types, ["market_match"]);
        assert_eq!(
            page.uncovered_execution_flow_types,
            [
                "resting_limit",
                "protective_barrier",
                "vwap",
                "twap",
                "dated_carry"
            ]
        );
        assert!(page.history_complete);
        assert!(matches!(
            &page.events[0].event.payload,
            FinancialEvidencePayload::AccountBalance { boundary, balances_by_currency, .. }
                if boundary == "opening" && balances_by_currency["USD"] == 10_000.0
        ));
        assert!(matches!(
            &page.events[1].event.payload,
            FinancialEvidencePayload::FeeSchedule { fee_schedule_version, .. }
                if fee_schedule_version == "paper-config-v1"
        ));
        assert!(matches!(
            &page.events[2].event.payload,
            FinancialEvidencePayload::SettledExecution { fee, execution_origin, .. }
                if *fee == -0.021 && execution_origin == "market_match"
        ));
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn opening_capital_is_not_backfilled_after_legacy_match_history() {
        let path = temp_db("legacy-history-baseline");
        let order = request(Some("legacy-order-1"), 1.0);
        commit_match_with_config(
            &path,
            "legacy-order-1",
            &request_fingerprint(&order),
            &order,
            match_book(&order, &book(), request_fingerprint(&order)),
            None,
            None,
        )
        .unwrap();

        let mut connection = open_ledger(&path).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let status = ensure_simulator_opening_capital_with(
            &transaction,
            Some(("paper-default".to_owned(), 10_000.0)),
        )
        .unwrap();
        assert_eq!(
            status,
            Some("existing_execution_history_requires_new_account_baseline")
        );
        transaction.commit().unwrap();
        let page = read_financial_evidence_page(&path, 0, 20, None, None).unwrap();
        assert_eq!(page.events.len(), 0);
        assert!(!page.execution_history_empty);
        assert_eq!(
            page.opening_capital_status,
            "existing_execution_history_needs_new_account_baseline"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn matcher_probe_fills_do_not_count_as_execution_history_or_block_opening_capital() {
        let path = temp_db("probe-only-history");
        let mut probe = request(Some("probe-only-order-1"), 1.0);
        probe.execution_origin = Some("match_probe".to_owned());
        let response = match_book(&probe, &book(), request_fingerprint(&probe));
        assert_eq!(response.filled_quantity, 1.0);

        commit_match_with_config(
            &path,
            "probe-only-order-1",
            &request_fingerprint(&probe),
            &probe,
            response,
            None,
            None,
        )
        .unwrap();

        let page = read_financial_evidence_page(&path, 0, 20, None, None).unwrap();
        assert!(page.execution_history_empty);
        assert_eq!(page.events.len(), 0);

        let mut connection = open_ledger(&path).unwrap();
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        let status = ensure_simulator_opening_capital_with(
            &transaction,
            Some(("paper-default".to_owned(), 10_000.0)),
        )
        .unwrap();
        assert_eq!(status, None);
        transaction.commit().unwrap();

        let page = read_financial_evidence_page(&path, 0, 20, None, None).unwrap();
        assert!(page.execution_history_empty);
        assert_eq!(
            page.opening_capital_status,
            "explicit_simulator_opening_capital_captured"
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn matches_buy_across_levels_respects_limit_and_returns_book_marks() {
        let result = match_book(
            &request(None, 3.0),
            &book(),
            request_fingerprint(&request(None, 3.0)),
        );
        assert_eq!(result.reference_price, Some(10.5));
        assert_eq!(result.best_bid, Some(10.0));
        assert_eq!(result.best_ask, Some(10.5));
        assert_eq!(result.worst_ask, Some(11.0));
        assert_eq!(result.filled_quantity, 3.0);
        assert_eq!(result.remaining_quantity, 0.0);
        assert_eq!(result.fills.len(), 2);
    }

    #[test]
    fn respects_sell_limits_and_returns_unfilled_quantity() {
        let mut sell = request(None, 3.0);
        sell.side = "sell".into();
        sell.limit_price = Some(10.0);
        let result = match_book(&sell, &book(), request_fingerprint(&sell));
        assert_eq!(result.filled_quantity, 2.0);
        assert_eq!(result.remaining_quantity, 1.0);
    }

    #[test]
    fn keyed_match_replays_exact_durable_response_and_conflicts_on_changed_request() {
        let path = temp_db("replay");
        let first_request = request(Some("client-order-7"), 3.0);
        let first = commit_match(
            &path,
            "client-order-7",
            &request_fingerprint(&first_request),
            &first_request,
            match_book(&first_request, &book(), request_fingerprint(&first_request)),
        )
        .unwrap();
        assert_eq!(first.schema_version, MATCH_SCHEMA_VERSION);
        assert_eq!(first.request_schema_version, MATCH_REQUEST_SCHEMA_VERSION);
        assert!(first.capture_complete);
        assert_eq!(first.capture_status, "durably_captured");
        assert_eq!(first.source_cursor_kind, "match_request");
        assert!(
            first
                .source_epoch
                .as_ref()
                .is_some_and(|epoch| epoch.len() == 32)
        );
        assert!(first.fills.iter().all(|fill| {
            fill.source_execution_id.is_some()
                && fill.source_cursor.is_some()
                && fill.source_cursor_kind.as_deref() == Some("match_fill")
                && fill.recorded_at_utc_ms.is_some()
                && fill.quote_evidence.is_some()
        }));
        let replay = lookup_committed_match(
            &path,
            "client-order-7",
            &request_fingerprint(&first_request),
            Some(&request_fingerprint_v1(&first_request)),
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            serde_json::to_value(first).unwrap(),
            serde_json::to_value(replay).unwrap()
        );
        assert!(
            lookup_committed_match(
                &path,
                "client-order-7",
                &request_fingerprint(&request(Some("client-order-7"), 2.0)),
                None,
            )
            .is_err()
        );
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn durable_rows_keep_order_identity_quote_and_bounded_cursor_evidence() {
        let path = temp_db("history");
        for order_id in ["client-1", "client-2"] {
            let request = request(Some(order_id), 1.0);
            commit_match(
                &path,
                order_id,
                &request_fingerprint(&request),
                &request,
                match_book(&request, &book(), request_fingerprint(&request)),
            )
            .unwrap();
        }
        let first_page = read_ledger_page(&path, 0, 1).unwrap();
        assert!(!first_page.page_complete);
        assert!(first_page.identity_complete);
        assert!(!first_page.reconciliation_eligible);
        assert_eq!(first_page.rows.len(), 1);
        assert_eq!(first_page.rows[0].order_id, "client-1");
        assert_eq!(first_page.rows[0].quote_evidence.best_ask, Some(10.5));
        assert!(!first_page.history_complete);
        assert_eq!(first_page.source_epoch.len(), 32);
        assert_eq!(first_page.cursor_kind, "match_fill");
        let second_page = read_ledger_page(&path, first_page.next_cursor.unwrap(), 1).unwrap();
        assert!(second_page.page_complete);
        assert_eq!(second_page.rows[0].order_id, "client-2");
        assert_eq!(first_page.source_epoch, second_page.source_epoch);
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn page_rows_stay_within_high_water_mark_when_a_concurrent_fill_commits() {
        let path = temp_db("snapshot-race");
        let first_request = request(Some("snapshot-1"), 1.0);
        commit_match(
            &path,
            "snapshot-1",
            &request_fingerprint(&first_request),
            &first_request,
            match_book(&first_request, &book(), request_fingerprint(&first_request)),
        )
        .unwrap();

        let mut reader =
            Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let snapshot = reader
            .transaction_with_behavior(TransactionBehavior::Deferred)
            .unwrap();
        let high_water: i64 = snapshot
            .query_row(
                "SELECT COALESCE(MAX(source_cursor), 0) FROM match_fills",
                [],
                |row| row.get(0),
            )
            .unwrap();

        let second_request = request(Some("snapshot-2"), 1.0);
        commit_match(
            &path,
            "snapshot-2",
            &request_fingerprint(&second_request),
            &second_request,
            match_book(
                &second_request,
                &book(),
                request_fingerprint(&second_request),
            ),
        )
        .unwrap();

        let snapshot_page = read_ledger_page_snapshot(&snapshot, 0, 10).unwrap();
        assert_eq!(snapshot_page.max_cursor, u64::try_from(high_water).unwrap());
        assert_eq!(snapshot_page.rows.len(), 1);
        assert!(
            snapshot_page
                .rows
                .iter()
                .all(|row| row.source_cursor <= snapshot_page.max_cursor)
        );
        snapshot.commit().unwrap();

        let next_page = read_ledger_page(&path, snapshot_page.max_cursor, 10).unwrap();
        assert_eq!(next_page.rows.len(), 1);
        assert_eq!(next_page.rows[0].order_id, "snapshot-2");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn history_read_does_not_create_or_initialize_a_missing_ledger() {
        let path = temp_db("readonly-missing");
        assert!(!path.exists());
        assert!(read_ledger_page(&path, 0, 10).is_err());
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_order_identity_remains_explicitly_uncaptured() {
        let request = request(None, 1.0);
        let result = match_book(&request, &book(), request_fingerprint(&request));
        assert_eq!(result.order_id, None);
        assert_eq!(result.recorded_at_utc_ms, None);
        assert_eq!(result.source_cursor, None);
        assert!(!result.capture_complete);
        assert_eq!(result.capture_status, "uncaptured_missing_order_id");
        assert_eq!(
            result.capture_failure_reason.as_deref(),
            Some("stable_order_id_required")
        );
        assert!(!result.reconciliation_eligible);
        assert_eq!(result.source_epoch, None);
        assert!(
            result
                .fills
                .iter()
                .all(|fill| fill.source_execution_id.is_none())
        );
    }

    #[test]
    fn evidence_failure_marks_capture_incomplete_without_suppressing_match_result() {
        let request = request(Some("client-order-outage"), 1.0);
        let result = match_book(&request, &book(), request_fingerprint(&request));
        let incomplete = mark_capture_incomplete(result, "durable_ledger_write_failed");
        assert_eq!(incomplete.filled_quantity, 1.0);
        assert!(!incomplete.capture_complete);
        assert_eq!(incomplete.capture_status, "incomplete");
        assert_eq!(
            incomplete.capture_failure_reason.as_deref(),
            Some("durable_ledger_write_failed")
        );
        assert!(
            incomplete
                .fills
                .iter()
                .all(|fill| fill.source_cursor.is_none())
        );
    }

    #[test]
    fn durable_write_failure_returns_error_without_a_committed_match() {
        let root = std::env::temp_dir().join(format!("paper-ledger-file-{}", now_ms().unwrap()));
        std::fs::write(&root, b"not a directory").unwrap();
        let path = root.join("cannot-open.sqlite");
        let request = request(Some("client-fail"), 1.0);
        let result = commit_match(
            &path,
            "client-fail",
            &request_fingerprint(&request),
            &request,
            match_book(&request, &book(), request_fingerprint(&request)),
        );
        assert!(result.is_err());
        let _ = std::fs::remove_file(root);
    }
}
