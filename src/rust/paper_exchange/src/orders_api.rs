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
const MATCH_REQUEST_SCHEMA_VERSION: &str = "paper_exchange_match_request:v1";
const LEDGER_SCHEMA_VERSION: &str = "paper_exchange_match_ledger:v1";
const MAX_LEDGER_PAGE_SIZE: usize = 500;

#[derive(Clone, Deserialize, Serialize)]
pub struct MatchRequest {
    pub symbol: String,
    pub side: String,
    pub quantity: f64,
    pub limit_price: Option<f64>,
    #[serde(default)]
    pub order_id: Option<String>,
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
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid order match request".into(),
        ));
    }
    let fingerprint = request_fingerprint(&request);

    if let Some(order_id) = request.order_id.as_deref() {
        let path = state.ledger_path.clone();
        let order_id = order_id.to_owned();
        let fingerprint_for_lookup = fingerprint.clone();
        let existing = tokio::task::spawn_blocking(move || {
            lookup_committed_match(&path, &order_id, &fingerprint_for_lookup)
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
    format!(
        "v1:{}:{}:{:016x}:{}",
        request.symbol,
        request.side,
        request.quantity.to_bits(),
        limit.unwrap_or_else(|| "none".to_owned())
    )
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
             recorded_at_utc_ms INTEGER NOT NULL
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
         CREATE INDEX IF NOT EXISTS match_fills_order ON match_fills(order_id, source_cursor);",
    )?;
    Ok(connection)
}

fn lookup_committed_match(
    path: &PathBuf,
    order_id: &str,
    fingerprint: &str,
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
        Some((stored_fingerprint, payload)) if stored_fingerprint == fingerprint => {
            Ok(Some(serde_json::from_str(&payload)?))
        }
        Some(_) => Err(anyhow::anyhow!(
            "order_id was already used for a different match request"
        )),
    }
}

fn commit_match(
    path: &PathBuf,
    order_id: &str,
    fingerprint: &str,
    request: &MatchRequest,
    mut response: MatchResponse,
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
        if stored_fingerprint != fingerprint {
            anyhow::bail!("order_id was already used for a different match request");
        }
        return Ok(serde_json::from_str(&payload)?);
    }

    let recorded_at_utc_ms = now_ms()?;
    transaction.execute(
        "INSERT INTO match_requests(order_id, request_fingerprint, symbol, side, requested_quantity,
                                   limit_price, response_json, request_schema_version, recorded_at_utc_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, '', ?7, ?8)",
        params![
            order_id,
            fingerprint,
            request.symbol,
            request.side,
            request.quantity,
            request.limit_price,
            MATCH_REQUEST_SCHEMA_VERSION,
            i64::try_from(recorded_at_utc_ms)?
        ],
    )?;
    let request_cursor = transaction.last_insert_rowid();
    response.order_id = Some(order_id.to_owned());
    response.source_epoch = Some(ledger_source_epoch(&transaction)?);
    response.recorded_at_utc_ms = Some(recorded_at_utc_ms);
    response.source_cursor = Some(u64::try_from(request_cursor)?);
    response.capture_complete = true;
    response.capture_status = "durably_captured".to_owned();
    response.capture_failure_reason = None;
    for (index, fill) in response.fills.iter_mut().enumerate() {
        let execution_id = format!("paper-match-{request_cursor}-{index}");
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
                order_id,
                &request.symbol,
                &request.side,
                fill.quantity,
                fill.price,
                i64::try_from(recorded_at_utc_ms)?,
                quote_json
            ],
        )?;
        fill.source_execution_id = Some(execution_id);
        fill.source_cursor = Some(u64::try_from(transaction.last_insert_rowid())?);
        fill.source_cursor_kind = Some("match_fill".to_owned());
        fill.recorded_at_utc_ms = Some(recorded_at_utc_ms);
        fill.quote_evidence = Some(quote);
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
    error
        .to_string()
        .contains("already used for a different match request")
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

pub fn router(market: Arc<MarketData>) -> Router {
    let ledger_path = std::env::var_os("PAPER_EXCHANGE_EXECUTION_LEDGER_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("paper_exchange_matches.sqlite")
        });
    router_with_path(market, ledger_path)
}

fn router_with_path(market: Arc<MarketData>, ledger_path: PathBuf) -> Router {
    let state = OrderApiState {
        market,
        ledger_path,
    };
    Router::new()
        .route("/internal/orders/match", post(match_order))
        .route("/internal/orders/match/ledger", get(ledger_history))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::market_data::BookLevel;

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
        }
    }

    fn request(order_id: Option<&str>, quantity: f64) -> MatchRequest {
        MatchRequest {
            symbol: "BTC-USDT".into(),
            side: "buy".into(),
            quantity,
            limit_price: Some(11.0),
            order_id: order_id.map(str::to_owned),
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
