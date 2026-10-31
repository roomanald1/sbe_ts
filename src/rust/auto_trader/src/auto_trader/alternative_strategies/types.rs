use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const DAY: u64 = 86_400_000;
pub const IDS: [&str; 5] = [
    "residual-momentum",
    "filtered-covered-call",
    "staking-redemption-arbitrage",
    "liquidation-keeper",
    "adaptive-stablecoin-liquidity",
];

#[derive(Clone, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub max_trade_usd: f64,
    pub daily_loss_limit_usd: f64,
    pub spot_fee_bps: f64,
    pub short_borrow_apr: f64,
    pub min_edge_bps: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            enabled: false,
            max_trade_usd: 100.0,
            daily_loss_limit_usd: 25.0,
            spot_fee_bps: 15.0,
            short_borrow_apr: 0.10,
            min_edge_bps: 10.0,
        }
    }
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct FeedInsight {
    pub strategy: String,
    pub ready: bool,
    pub signal: String,
    pub source_timestamp_ms: u64,
    pub computed_at_ms: u64,
    pub summary: String,
    pub features: serde_json::Value,
}
impl FeedInsight {
    pub fn current(&self, now: u64) -> bool {
        self.ready
            && self.computed_at_ms <= now
            && now - self.computed_at_ms <= 120_000
            && self.source_timestamp_ms <= now
            && now - self.source_timestamp_ms <= 120_000
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Sleeve {
    pub id: String,
    pub config: Config,
    #[serde(default)]
    pub feed: FeedInsight,
    pub initial_capital_usd: f64,
    pub cash_usd: f64,
    pub equity_usd: f64,
    pub peak_equity_usd: f64,
    pub max_drawdown_percent: f64,
    pub realized_pnl_usd: f64,
    pub fees_usd: f64,
    pub daily_pnl_usd: f64,
    pub utc_day: u64,
    pub last_observation_ms: u64,
    pub last_rebalance_day: Option<u64>,
    pub status: String,
    pub positions: BTreeMap<String, Position>,
    pub processed_ids: BTreeSet<String>,
    pub events: Vec<Event>,
}
impl Sleeve {
    pub fn new(id: &str) -> Self {
        Self {
            id: id.into(),
            config: Config {
                max_trade_usd: if id == IDS[1] { 1000.0 } else { 100.0 },
                ..Config::default()
            },
            feed: FeedInsight::default(),
            initial_capital_usd: 1000.0,
            cash_usd: 1000.0,
            equity_usd: 1000.0,
            peak_equity_usd: 1000.0,
            max_drawdown_percent: 0.0,
            realized_pnl_usd: 0.0,
            fees_usd: 0.0,
            daily_pnl_usd: 0.0,
            utc_day: 0,
            last_observation_ms: 0,
            last_rebalance_day: None,
            status: "Disabled; awaiting observations".into(),
            positions: BTreeMap::new(),
            processed_ids: BTreeSet::new(),
            events: Vec::new(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Position {
    Spot {
        quantity: f64,
        entry_price: f64,
        mark_price: f64,
    },
    CoveredCall {
        quantity: f64,
        spot_entry: f64,
        premium_entry: f64,
        strike: f64,
        expiry_ms: u64,
        spot_bid: f64,
        call_ask: f64,
    },
    Redemption {
        token: String,
        quantity: f64,
        cost_usd: f64,
        expected_eth: f64,
        redeem_after_ms: u64,
        mark_usd: f64,
    },
    Liquidity {
        liquidity: f64,
        #[serde(default)]
        tick_lower: Option<i32>,
        #[serde(default)]
        tick_upper: Option<i32>,
        lower_price: f64,
        upper_price: f64,
        cost_usd: f64,
        last_fee_growth: f64,
        token0_usd: f64,
        token1_usd: f64,
        price: f64,
        accrued_fees_usd: f64,
        exit_cost_usd: f64,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    pub timestamp_ms: u64,
    pub observation_id: String,
    pub action: String,
    pub detail: String,
    pub cash_usd: f64,
    pub equity_usd: f64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Book {
    pub sleeves: BTreeMap<String, Sleeve>,
}
impl Book {
    pub fn initialized() -> Self {
        Self {
            sleeves: IDS
                .into_iter()
                .map(|id| (id.into(), Sleeve::new(id)))
                .collect(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Observation {
    pub id: String,
    pub observed_at_ms: u64,
    pub source: String,
    #[serde(flatten)]
    pub data: Input,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "strategy", content = "data", rename_all = "kebab-case")]
pub enum Input {
    ResidualMomentum(ResidualInput),
    FilteredCoveredCall(CallInput),
    StakingRedemptionArbitrage(StakingInput),
    LiquidationKeeper(LiquidationInput),
    AdaptiveStablecoinLiquidity(LiquidityInput),
}
impl Input {
    pub fn id(&self) -> &'static str {
        match self {
            Self::ResidualMomentum(_) => IDS[0],
            Self::FilteredCoveredCall(_) => IDS[1],
            Self::StakingRedemptionArbitrage(_) => IDS[2],
            Self::LiquidationKeeper(_) => IDS[3],
            Self::AdaptiveStablecoinLiquidity(_) => IDS[4],
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Quote {
    pub bid: f64,
    pub ask: f64,
    pub bid_quantity: f64,
    pub ask_quantity: f64,
    pub timestamp_ms: u64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Close {
    pub timestamp_ms: u64,
    pub close: f64,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct ResidualAsset {
    pub symbol: String,
    pub closes: Vec<Close>,
    pub quote: Quote,
    pub short_available: bool,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct ResidualInput {
    #[serde(default)]
    pub scores: Option<BTreeMap<String, f64>>,
    pub assets: Vec<ResidualAsset>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct CallInput {
    pub instrument: String,
    #[serde(default)]
    pub minimum_quantity: f64,
    #[serde(default)]
    pub quantity_step: f64,
    pub spot: Quote,
    // Premiums, strike and all fees are USD per one BTC of option notional.
    pub call: Quote,
    pub strike_usd: f64,
    pub expiry_ms: u64,
    pub delta: f64,
    pub implied_volatility: f64,
    pub prior_30d_mean_iv: f64,
    pub prior_close_usd: f64,
    pub prior_sma50_usd: f64,
    pub indicators_at_ms: u64,
    pub option_fee_usd_per_btc: f64,
    pub settlement: Option<Settlement>,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Settlement {
    pub price_usd: f64,
    pub timestamp_ms: u64,
    pub source: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum StakingInput {
    // Executable route amounts for exactly input_eth, not a midpoint premium.
    Open {
        opportunity_id: String,
        token: String,
        input_eth: f64,
        token_quantity: f64,
        output_eth: f64,
        eth_usd: f64,
        total_cost_usd: f64,
        atomic: bool,
        redeem_after_ms: u64,
        chain_id: u64,
        block_number: u64,
    },
    Settle {
        opportunity_id: String,
        received_eth: f64,
        eth_usd: f64,
        total_cost_usd: f64,
        chain_id: u64,
        transaction_hash: String,
    },
}
#[derive(Clone, Deserialize, Serialize)]
pub struct LiquidationInput {
    pub opportunity_id: String,
    pub chain_id: u64,
    pub block_number: u64,
    pub protocol: String,
    pub health_factor: f64,
    pub debt_repayment_usd: f64,
    pub executable_collateral_sale_usd: f64,
    // Include gas, builder payment, flash-loan fee and swap fees.
    pub total_cost_usd: f64,
    pub simulation_succeeded: bool,
    pub atomic: bool,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct LiquidityInput {
    pub chain_id: u64,
    pub pool: String,
    pub block_number: u64,
    // Human-normalized token1 per token0, not raw sqrtPriceX96.
    pub price: f64,
    pub token0_usd: f64,
    pub token1_usd: f64,
    // Cumulative USD fees per normalized liquidity unit *inside our range*.
    pub fee_growth_inside_usd: f64,
    pub fee_lower_price: Option<f64>,
    pub fee_upper_price: Option<f64>,
    pub prior_daily_volatility: f64,
    #[serde(default)]
    pub range_lower_price: Option<f64>,
    #[serde(default)]
    pub range_upper_price: Option<f64>,
    #[serde(default)]
    pub tick_lower: Option<i32>,
    #[serde(default)]
    pub tick_upper: Option<i32>,
    #[serde(default)]
    pub tick_spacing: Option<i32>,
    // Token-USD observations are taken before the current block.
    pub indicators_at_ms: u64,
    pub enter_cost_usd: f64,
    pub exit_cost_usd: f64,
}
