#[derive(Clone, Deserialize)]
struct MarketMark {
    bid: Option<f64>,
    ask: Option<f64>,
    mid: Option<f64>,
    worst_ask: Option<f64>,
    updated_at_ms: u64,
}

#[derive(Deserialize)]
struct MatcherFill {
    quantity: f64,
    price: f64,
}
#[derive(Deserialize)]
struct MatcherResponse {
    reference_price: Option<f64>,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    worst_ask: Option<f64>,
    book_updated_at_ms: u64,
    fills: Vec<MatcherFill>,
    remaining_quantity: f64,
    filled_quantity: f64,
    average_execution_price: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize)]
struct AccountTopup {
    amount: f64,
    timestamp: u64,
}
#[derive(Deserialize)]
struct TopupRequest {
    amount: f64,
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
}
const POSITION_UI_EVENT_QUEUE_CAPACITY: usize = 32;

struct PositionStore {
    path: PathBuf,
    ledger: Arc<Mutex<Ledger>>,
    persistence_notify: Arc<Notify>,
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
