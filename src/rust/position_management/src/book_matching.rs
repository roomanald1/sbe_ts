#[derive(Clone, Serialize, Deserialize)]

struct BookLevel {
    price: f64,
    size: f64,
}
#[derive(Clone, Serialize, Deserialize)]
struct BookSnapshot {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
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
    fills: Vec<MatcherFill>,
    remaining_quantity: f64,
    filled_quantity: f64,
    average_execution_price: Option<f64>,
}

fn match_book(
    side: &TradeSide,
    quantity: f64,
    limit_price: Option<f64>,
    book: &BookSnapshot,
) -> MatcherResponse {
    let buy = matches!(side, TradeSide::Buy);
    let levels = if buy { &book.asks } else { &book.bids };
    let mut remaining = quantity;
    let mut fills = Vec::with_capacity(levels.len());
    let mut notional = 0.0;
    for level in levels {
        let crosses = match limit_price {
            Some(limit) if buy => level.price <= limit,
            Some(limit) => level.price >= limit,
            None => true,
        };
        if !crosses || remaining <= EPSILON {
            continue;
        }
        let filled = remaining.min(level.size);
        if filled > EPSILON {
            fills.push(MatcherFill {
                quantity: filled,
                price: level.price,
            });
            remaining -= filled;
            notional += filled * level.price;
        }
    }
    let filled_quantity = quantity - remaining;
    MatcherResponse {
        reference_price: levels.first().map(|level| level.price),
        fills,
        remaining_quantity: remaining,
        filled_quantity,
        average_execution_price: (filled_quantity > EPSILON).then_some(notional / filled_quantity),
    }
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
    let parse = |name: &str, default: f64| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1)
            .unwrap_or(default)
    };
    FeeRates {
        maker: -parse("PAPER_OKX_SPOT_MAKER_FEE_RATE", 0.0008),
        taker: -parse("PAPER_OKX_SPOT_TAKER_FEE_RATE", 0.001),
    }
}
fn default_account_cash() -> f64 {
    std::env::var("PAPER_ACCOUNT_INITIAL_CASH")
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value: &f64| value.is_finite() && *value > 0.0)
        .unwrap_or(10_000.0)
}
fn paper_max_leverage() -> f64 {
    std::env::var("PAPER_ACCOUNT_MAX_LEVERAGE")
        .ok()
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
struct PositionStore {
    path: PathBuf,
    ledger: Arc<Mutex<Ledger>>,
    persistence_notify: Arc<Notify>,
    state_updates: broadcast::Sender<()>,
    persistence_timings_ms: Arc<StdMutex<BTreeMap<String, Vec<f64>>>>,
    trade_index: StdMutex<LedgerTradeIndex>,
    realized_pnl_cache: StdMutex<Option<RealizedPnlLedgerSnapshot>>,
    books: StdRwLock<HashMap<String, Arc<ArcSwapOption<BookSnapshot>>>>,
    price_samples: Mutex<HashMap<String, Vec<(u64, f64)>>>,
    hedge_samples: Mutex<HashMap<String, Vec<(u64, f64)>>>,
    book_changed: Notify,
    subscriptions: StdMutex<HashMap<String, mpsc::Sender<anyhow::Result<Bytes>>>>,
    spot_symbols: Arc<Vec<String>>,
    vwap_runs: Mutex<HashMap<String, VwapRunEntry>>,
}

#[cfg(test)]
mod book_matching_tests {
    use super::*;

    fn book() -> BookSnapshot {
        BookSnapshot {
            bids: vec![BookLevel { price: 99.0, size: 2.0 }, BookLevel { price: 98.0, size: 3.0 }],
            asks: vec![BookLevel { price: 101.0, size: 1.0 }, BookLevel { price: 102.0, size: 4.0 }],
            updated_at_ms: 1,
        }
    }

    #[test]
    fn matches_buy_across_eligible_ask_levels_and_reports_vwap() {
        let result = match_book(&TradeSide::Buy, 3.0, Some(102.0), &book());
        assert_eq!(result.reference_price, Some(101.0));
        assert_eq!(result.filled_quantity, 3.0);
        assert_eq!(result.remaining_quantity, 0.0);
        assert_eq!(result.average_execution_price, Some((101.0 + 2.0 * 102.0) / 3.0));
        assert_eq!(result.fills.len(), 2);
    }

    #[test]
    fn respects_limit_and_leaves_unfilled_quantity() {
        let result = match_book(&TradeSide::Sell, 4.0, Some(99.0), &book());
        assert_eq!(result.filled_quantity, 2.0);
        assert_eq!(result.remaining_quantity, 2.0);
        assert_eq!(result.average_execution_price, Some(99.0));
    }

    #[test]
    fn empty_book_returns_full_remainder_without_average() {
        let empty = BookSnapshot { bids: vec![], asks: vec![], updated_at_ms: 0 };
        let result = match_book(&TradeSide::Buy, 1.0, None, &empty);
        assert_eq!(result.filled_quantity, 0.0);
        assert_eq!(result.remaining_quantity, 1.0);
        assert_eq!(result.average_execution_price, None);
    }
}
