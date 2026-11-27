#[derive(Clone, Deserialize)]
struct MarketMark {
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    worst_ask: Option<f64>,
    updated_at_ms: u64,
    #[serde(default)]
    source_system: Option<String>,
    #[serde(default)]
    source_epoch: Option<String>,
    #[serde(default)]
    source_cursor: Option<u64>,
    #[serde(default)]
    source_record_id: Option<String>,
}

#[derive(Clone, Default, Deserialize)]
struct MatcherFill {
    quantity: f64,
    price: f64,
    #[serde(default)]
    source_execution_id: Option<String>,
    #[serde(default)]
    source_cursor: Option<u64>,
    #[serde(default)]
    source_cursor_kind: Option<String>,
    #[serde(default)]
    recorded_at_utc_ms: Option<u64>,
    #[serde(default)]
    fee: Option<f64>,
    #[serde(default)]
    fee_currency: Option<String>,
    #[serde(default)]
    fee_schedule_id: Option<String>,
    #[serde(default)]
    fee_schedule_version: Option<String>,
    #[serde(default)]
    fee_order_class: Option<String>,
    #[serde(default)]
    fee_schedule_evidence: Option<serde_json::Value>,
    #[serde(skip)]
    source_price: Option<f64>,
    /// Original quantity returned by Paper Exchange before PM applies a cap.
    #[serde(skip)]
    source_quantity: Option<f64>,
}
#[derive(Clone, Deserialize)]
struct MatcherResponse {
    #[serde(default)]
    schema_version: Option<String>,
    #[serde(default)]
    capture_complete: Option<bool>,
    #[serde(default)]
    order_id: Option<String>,
    #[serde(default)]
    parent_order_id: Option<String>,
    #[serde(default)]
    request_fingerprint: Option<String>,
    #[serde(default)]
    recorded_at_utc_ms: Option<u64>,
    #[serde(default)]
    source_cursor: Option<u64>,
    #[serde(default)]
    source_cursor_kind: Option<String>,
    #[serde(default)]
    source_epoch: Option<String>,
    reference_price: Option<f64>,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    worst_ask: Option<f64>,
    book_updated_at_ms: u64,
    fills: Vec<MatcherFill>,
    remaining_quantity: f64,
    filled_quantity: f64,
    average_execution_price: Option<f64>,
    /// Kept separately because PM may clip a match for reduce-only safety.
    #[serde(skip)]
    source_reported_filled_quantity: Option<f64>,
}

#[derive(Clone, Default)]
struct PaperExchangeFillEvidence {
    schema_version: Option<String>,
    source_epoch: Option<String>,
    source_cursor: Option<u64>,
    request_fingerprint: Option<String>,
    execution_id: Option<String>,
    source_quantity: Option<f64>,
    source_price: Option<f64>,
    complete: Option<bool>,
    fee: Option<f64>,
    fee_currency: Option<String>,
    fee_schedule_id: Option<String>,
    fee_schedule_version: Option<String>,
    fee_order_class: Option<String>,
    fee_schedule_evidence: Option<serde_json::Value>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct AccountTopup {
    amount: f64,
    timestamp: u64,
    #[serde(default)]
    contribution_id: Option<String>,
    #[serde(default)]
    financial_evidence_epoch: Option<String>,
    #[serde(default)]
    financial_evidence_cursor: Option<u64>,
}
#[derive(Deserialize)]
struct TopupRequest {
    amount: f64,
    contribution_id: String,
}
#[derive(Deserialize)]
struct LeverageRequest {
    max_leverage: f64,
}
#[derive(Serialize)]
struct AccountResponse {
    currency: &'static str,
    cash: f64,
    equity: f64,
    realized_pnl: f64,
    unrealized_pnl: f64,
    fees_paid: f64,
    fees_complete: bool,
    unknown_fee_fills: usize,
    estimated_exit_fees: f64,
    equity_available: bool,
    initial_cash: f64,
    topups: Vec<AccountTopup>,
    max_leverage: f64,
}
#[derive(Serialize)]
struct FeeRatesResponse {
    maker: f64,
    taker: f64,
}
struct FeeRates {
    maker: f64,
    taker: f64,
}
fn configured_fee_rates() -> FeeRates {
    FeeRates {
        maker: -parse_fee_rate(std::env::var("PAPER_OKX_SPOT_MAKER_FEE_RATE").ok(), 0.0008),
        taker: -parse_fee_rate(std::env::var("PAPER_OKX_SPOT_TAKER_FEE_RATE").ok(), 0.001),
    }
}

fn parse_fee_rate(value: Option<String>, default: f64) -> f64 {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1)
        .unwrap_or(default)
}

fn default_account_cash() -> f64 {
    parse_positive_cash(std::env::var("PAPER_ACCOUNT_INITIAL_CASH").ok())
}

fn parse_positive_cash(value: Option<String>) -> f64 {
    value
        .and_then(|value| value.parse().ok())
        .filter(|value: &f64| value.is_finite() && *value > 0.0)
        .unwrap_or(10_000.0)
}

fn paper_max_leverage() -> f64 {
    parse_max_leverage(std::env::var("PAPER_ACCOUNT_MAX_LEVERAGE").ok())
}

fn parse_max_leverage(value: Option<String>) -> f64 {
    value
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 1.0 && *value <= 10.0)
        .unwrap_or(2.0)
}
fn configured_account_leverage(ledger: &Ledger) -> f64 {
    ledger.max_leverage.unwrap_or_else(paper_max_leverage)
}

#[derive(Clone, Default, Serialize, Deserialize)]
struct Ledger {
    trades: Vec<PositionTrade>,
    /// Allocator state is committed in the same serialized ledger snapshot as
    /// every newly appended fill. `None` is retained for legacy ledgers until
    /// the first new fill is written.
    #[serde(default)]
    fill_id_epoch: Option<String>,
    #[serde(default)]
    fill_id_sequence: u64,
    #[serde(default)]
    portfolio_baseline_source_system: Option<String>,
    #[serde(default)]
    portfolio_baseline_source_record_id: Option<String>,
    #[serde(default)]
    portfolio_baseline_source_epoch: Option<String>,
    #[serde(default)]
    portfolio_baseline_source_cursor: Option<u64>,
    #[serde(default)]
    dated_carry_positions: Vec<DatedCarryLedgerPosition>,
    #[serde(default = "default_account_cash")]
    initial_cash: f64,
    #[serde(default)]
    topups: Vec<AccountTopup>,
    #[serde(default)]
    max_leverage: Option<f64>,
    #[serde(default)]
    orders: Vec<RestingOrder>,
    #[serde(default)]
    barrier_orders: Vec<BarrierOrder>,
    #[serde(default)]
    scheduled_runs: Vec<VwapRunStatus>,
    /// Successful no-fill client-order outcomes, retained for the latest 4096
    /// distinct requests. This bounded FIFO horizon prevents a retry from
    /// rematching after a timeout or restart while limiting ledger growth.
    #[serde(default)]
    no_fill_order_responses: Vec<NoFillOrderResponse>,
    /// Bounded terminal passive-order records retain the exact observations
    /// used by the matcher after active reservations leave `orders`.
    #[serde(default)]
    execution_path_history: Vec<ExecutionPathTerminalRecord>,
    #[serde(default)]
    execution_path_history_truncated: bool,
}

fn allocate_fill_id(ledger: &mut Ledger, timestamp_ms: u64) -> String {
    let epoch = ledger
        .fill_id_epoch
        .get_or_insert_with(|| create_fill_id_epoch(timestamp_ms));
    ledger.fill_id_sequence = ledger
        .fill_id_sequence
        .checked_add(1)
        .expect("canonical fill ID sequence exhausted");
    format!("pm-fill:v1:{epoch}:{}", ledger.fill_id_sequence)
}

fn create_fill_id_epoch(timestamp_ms: u64) -> String {
    static EPOCH_NONCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nonce = EPOCH_NONCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{timestamp_ms}-{}-{nanos}-{nonce}", std::process::id())
}

const NO_FILL_IDEMPOTENCY_CAPACITY: usize = 4_096;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct NoFillOrderResponse {
    client_order_id: String,
    estimated_fee: f64,
    fee_complete: bool,
    unknown_fee_fills: usize,
    filled_quantity: f64,
    remaining_quantity: f64,
    average_execution_price: Option<f64>,
}

fn remember_no_fill_order_response(ledger: &mut Ledger, response: NoFillOrderResponse) {
    ledger.no_fill_order_responses.push(response);
    if ledger.no_fill_order_responses.len() > NO_FILL_IDEMPOTENCY_CAPACITY {
        let excess = ledger.no_fill_order_responses.len() - NO_FILL_IDEMPOTENCY_CAPACITY;
        ledger.no_fill_order_responses.drain(..excess);
    }
}
const POSITION_UI_EVENT_QUEUE_CAPACITY: usize = 32;

struct PositionStore {
    path: PathBuf,
    ledger: Arc<Mutex<Ledger>>,
    persistence_notify: Arc<Notify>,
    /// Serializes background and awaited ledger writes. Writers take this lock
    /// before snapshotting so an older queued snapshot cannot overwrite a
    /// newer durable no-fill commit.
    persistence_write_lock: Arc<Mutex<()>>,
    /// IDs inserted in memory whose explicit durable commit has not completed.
    /// Loaded ledger IDs are already committed and need no duplicate rewrite.
    pending_client_order_commits: StdMutex<HashSet<String>>,
    state_updates: broadcast::Sender<()>,
    persistence_timings_ms: Arc<StdMutex<BTreeMap<String, Vec<f64>>>>,
    trade_index: StdMutex<LedgerTradeIndex>,
    realized_pnl_cache: StdMutex<Option<RealizedPnlLedgerSnapshot>>,
    market_marks: StdRwLock<HashMap<String, MarketMark>>,
    market_mark_pollers: StdMutex<HashSet<String>>,
    vwap_runs: Mutex<HashMap<String, VwapRunEntry>>,
}

#[cfg(test)]
mod book_matching_tests {
    use super::*;

    #[test]
    fn legacy_ledger_decode_defaults_no_fill_registry_and_registry_is_bounded() {
        let mut legacy = serde_json::to_value(Ledger::default()).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("no_fill_order_responses");
        legacy.as_object_mut().unwrap().remove("fill_id_epoch");
        legacy.as_object_mut().unwrap().remove("fill_id_sequence");
        let decoded: Ledger = serde_json::from_value(legacy).unwrap();
        assert!(decoded.no_fill_order_responses.is_empty());
        assert_eq!(decoded.fill_id_epoch, None);
        assert_eq!(decoded.fill_id_sequence, 0);

        let mut ledger = Ledger::default();
        for index in 0..=NO_FILL_IDEMPOTENCY_CAPACITY {
            remember_no_fill_order_response(
                &mut ledger,
                NoFillOrderResponse {
                    client_order_id: format!("id-{index}"),
                    estimated_fee: 0.0,
                    fee_complete: true,
                    unknown_fee_fills: 0,
                    filled_quantity: 0.0,
                    remaining_quantity: 1.0,
                    average_execution_price: None,
                },
            );
        }
        assert_eq!(
            ledger.no_fill_order_responses.len(),
            NO_FILL_IDEMPOTENCY_CAPACITY
        );
        assert_eq!(ledger.no_fill_order_responses[0].client_order_id, "id-1");
        assert_eq!(
            ledger
                .no_fill_order_responses
                .last()
                .unwrap()
                .client_order_id,
            format!("id-{NO_FILL_IDEMPOTENCY_CAPACITY}")
        );
    }

    #[test]
    fn fill_id_epoch_and_sequence_survive_ledger_restart_and_keep_partial_fills_distinct() {
        let mut ledger = Ledger::default();
        let first = allocate_fill_id(&mut ledger, 1234);
        let second = allocate_fill_id(&mut ledger, 1234);
        assert_ne!(first, second);
        assert!(first.starts_with("pm-fill:v1:"));

        let persisted = serde_json::to_vec(&ledger).unwrap();
        let mut restarted: Ledger = serde_json::from_slice(&persisted).unwrap();
        let third = allocate_fill_id(&mut restarted, 1235);
        assert_ne!(first, third);
        assert_ne!(second, third);
        assert!(third.starts_with(&format!(
            "pm-fill:v1:{}:",
            restarted.fill_id_epoch.as_ref().unwrap()
        )));
        assert_eq!(restarted.fill_id_sequence, 3);
    }

    #[test]
    fn account_environment_parsers_accept_only_finite_values_within_bounds() {
        assert_eq!(parse_fee_rate(Some("0.02".into()), 0.0008), 0.02);
        assert_eq!(parse_fee_rate(Some("0.11".into()), 0.0008), 0.0008);
        assert_eq!(parse_fee_rate(Some("NaN".into()), 0.001), 0.001);
        assert_eq!(parse_positive_cash(Some("1250.5".into())), 1250.5);
        assert_eq!(parse_positive_cash(Some("0".into())), 10_000.0);
        assert_eq!(parse_positive_cash(Some("inf".into())), 10_000.0);
        assert_eq!(parse_max_leverage(Some("4".into())), 4.0);
        assert_eq!(parse_max_leverage(Some("0.5".into())), 2.0);
        assert_eq!(parse_max_leverage(Some("11".into())), 2.0);
    }
}
