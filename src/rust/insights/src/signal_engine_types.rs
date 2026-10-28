#[derive(Clone, Debug)]
struct BookHistorySample {
    timestamp: u64,
    bid: f64,
    ask: f64,
    bid_levels: [(f64, f64); 7],
    ask_levels: [(f64, f64); 7],
}

struct PendingOutcome {
    symbol: String,
    timestamp: u64,
    base_price: f64,
    direction: String,
    strength: f64,
    horizon_seconds: u64,
    entry_bid: Option<f64>,
    entry_ask: Option<f64>,
    trend_5m_volatility_units: Option<f64>,
    features: Option<[f64; 4]>,
    score: f64,
    ranging_probability: Option<f64>,
    entry_quote_timestamp: Option<u64>,
    taker_fee_rate: f64,
    first_long_positive_net_timestamp: Option<u64>,
    first_short_positive_net_timestamp: Option<u64>,
}
#[derive(Default)]
struct PerformanceTotals {
    sample_count: usize,
    directional_count: usize,
    neutral_count: usize,
    bullish_count: usize,
    bullish_matches: usize,
    bearish_count: usize,
    bearish_matches: usize,
    directional_matches: usize,
    raw_return_total: f64,
    net_count: usize,
    net_return_total: f64,
}

#[derive(Clone, Copy)]
struct AbsorptionTrade {
    timestamp: u64,
    price: f64,
    size: f64,
    is_buy: bool,
    bid: f64,
    ask: f64,
    bid_depth: f64,
    ask_depth: f64,
}

pub struct SignalEngine {
    prices: VecDeque<(u64, f64)>,
    pending: VecDeque<PendingOutcome>,
    trades: VecDeque<(u64, f64, f64, bool, Option<f64>)>,
    absorption_trades: VecDeque<AbsorptionTrade>,
    book_imbalance: Option<f64>,
    best_quote: Option<(u64, f64, f64)>,
    book_bid_levels: Vec<(f64, f64)>,
    book_ask_levels: Vec<(f64, f64)>,
    book_history: VecDeque<BookHistorySample>,
    volatility_1m: Option<f64>,
    last_research_sample_at: Option<u64>,
    last_live_snapshot_at: Option<u64>,
}
