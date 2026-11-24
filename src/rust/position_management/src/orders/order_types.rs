#[derive(Deserialize)]

struct NewVwapRun {
    symbol: String,
    side: TradeSide,
    quantity: f64,
    duration_minutes: u64,
    #[serde(default = "default_max_participation")]
    max_participation_rate: f64,
    #[serde(default)]
    strategy: ExecutionStrategy,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    client_order_id: Option<String>,
}
fn default_max_participation() -> f64 {
    1.0
}
fn default_trade_source() -> String {
    "manual".to_owned()
}
#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ExecutionStrategy {
    #[default]
    Vwap,
    Twap,
}
#[derive(Clone, Serialize, Deserialize)]
struct VwapRunStatus {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    executed_quantity: f64,
    strategy: String,
    duration_minutes: u64,
    started_at: u64,
    status: String,
    market_vwap: f64,
    execution_vwap: f64,
    slippage: f64,
    error: Option<String>,
}
struct VwapRunEntry {
    status: Arc<Mutex<VwapRunStatus>>,
    cancelled: Arc<AtomicBool>,
}
#[derive(Serialize)]
struct VwapRunsResponse {
    runs: Vec<VwapRunStatus>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TradeSide {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Serialize)]
struct PositionTrade {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    execution_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference_price: Option<f64>,
    timestamp: u64,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    parent_order_id: Option<String>,
    #[serde(default)]
    fee: f64,
    /// False when a legacy row omitted fee metadata; fee is then only a
    /// compatibility placeholder, not evidence of a zero-fee fill.
    #[serde(default)]
    fee_known: bool,
    #[serde(default)]
    fee_currency: String,
    #[serde(default)]
    fee_rate: f64,
    #[serde(default)]
    fee_liquidity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resting_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_timestamp: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    decision_mid_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slippage: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    slippage_bps: Option<f64>,
}

#[derive(Deserialize)]
struct PositionTradeWire {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    execution_price: f64,
    #[serde(default)]
    reference_price: Option<f64>,
    timestamp: u64,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    parent_order_id: Option<String>,
    fee: Option<f64>,
    fee_known: Option<bool>,
    fee_currency: Option<String>,
    #[serde(default)]
    fee_rate: f64,
    #[serde(default)]
    fee_liquidity: String,
    #[serde(default)]
    resting_order_id: Option<String>,
    #[serde(default)]
    decision_timestamp: Option<u64>,
    #[serde(default)]
    decision_price: Option<f64>,
    #[serde(default)]
    decision_mid_price: Option<f64>,
    #[serde(default)]
    slippage: Option<f64>,
    #[serde(default)]
    slippage_bps: Option<f64>,
}

impl<'de> Deserialize<'de> for PositionTrade {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let wire = PositionTradeWire::deserialize(deserializer)?;
        let fee = wire.fee.unwrap_or_default();
        let fee_currency = wire.fee_currency.unwrap_or_default();
        // Legacy rows are known only when the serialized record contained both
        // the fee amount (including an explicit zero) and its currency. Missing
        // or defaulted placeholders remain unknown; explicit status is retained.
        let fee_metadata_present = wire.fee.is_some()
            && !fee_currency.trim().is_empty()
            && fee.is_finite();
        let fee_known = wire.fee_known.unwrap_or(fee_metadata_present) && fee_metadata_present;
        Ok(Self {
            id: wire.id,
            symbol: wire.symbol,
            side: wire.side,
            quantity: wire.quantity,
            execution_price: wire.execution_price,
            reference_price: wire.reference_price,
            timestamp: wire.timestamp,
            source: wire.source,
            parent_order_id: wire.parent_order_id,
            fee,
            fee_known,
            fee_currency,
            fee_rate: wire.fee_rate,
            fee_liquidity: wire.fee_liquidity,
            resting_order_id: wire.resting_order_id,
            decision_timestamp: wire.decision_timestamp,
            decision_price: wire.decision_price,
            decision_mid_price: wire.decision_mid_price,
            slippage: wire.slippage,
            slippage_bps: wire.slippage_bps,
        })
    }
}

#[cfg(test)]
mod position_trade_fee_migration_tests {
    use super::*;

    #[test]
    fn legacy_trade_without_fee_metadata_remains_unknown_on_round_trip() {
        let legacy = serde_json::json!({
            "id": "legacy-1", "symbol": "BTC-USDT", "side": "buy",
            "quantity": 1.0, "execution_price": 100.0, "timestamp": 7
        });
        let trade: PositionTrade = serde_json::from_value(legacy).unwrap();
        assert_eq!(trade.fee, 0.0); // compatibility placeholder only
        assert!(!trade.fee_known);

        let migrated = serde_json::to_value(trade).unwrap();
        assert_eq!(migrated["fee"], 0.0);
        assert_eq!(migrated["fee_known"], false);
        let reloaded: PositionTrade = serde_json::from_value(migrated).unwrap();
        assert!(!reloaded.fee_known);
    }

    #[test]
    fn known_zero_fee_is_distinct_from_a_legacy_missing_fee() {
        let mut actual_zero: PositionTrade = serde_json::from_value(serde_json::json!({
            "id": "zero-1", "symbol": "BTC-USDT", "side": "buy",
            "quantity": 1.0, "execution_price": 100.0, "timestamp": 8,
            "fee": 0.0, "fee_known": true, "fee_currency": "USDT"
        })).unwrap();
        assert_eq!(actual_zero.fee, 0.0);
        assert!(actual_zero.fee_known);

        let serialized = serde_json::to_value(&actual_zero).unwrap();
        assert_eq!(serialized["fee"], 0.0);
        assert_eq!(serialized["fee_known"], true);
        actual_zero.fee = -0.25;
        assert!(actual_zero.fee_known);
    }

    #[test]
    fn legacy_explicit_zero_with_currency_migrates_as_known_but_partial_metadata_does_not() {
        let explicit_zero = serde_json::json!({
            "id":"legacy-zero", "symbol":"BTC-USDT", "side":"buy",
            "quantity":1.0, "execution_price":100.0, "timestamp":9,
            "fee":0.0, "fee_currency":"USDT"
        });
        let trade: PositionTrade = serde_json::from_value(explicit_zero).unwrap();
        assert_eq!(trade.fee, 0.0);
        assert!(trade.fee_known);

        let missing_amount = serde_json::json!({
            "id":"legacy-partial", "symbol":"BTC-USDT", "side":"buy",
            "quantity":1.0, "execution_price":100.0, "timestamp":10,
            "fee_currency":"USDT"
        });
        let trade: PositionTrade = serde_json::from_value(missing_amount).unwrap();
        assert_eq!(trade.fee, 0.0);
        assert!(!trade.fee_known);

        let explicit_unknown = serde_json::json!({
            "id":"unknown", "symbol":"BTC-USDT", "side":"buy",
            "quantity":1.0, "execution_price":100.0, "timestamp":11,
            "fee":0.0, "fee_currency":"USDT", "fee_known":false
        });
        let trade: PositionTrade = serde_json::from_value(explicit_unknown).unwrap();
        assert!(!trade.fee_known);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DatedCarryLedgerPosition {
    spot_symbol: String,
    future_symbol: String,
    expiry_ms: u64,
    entry_ms: u64,
    quantity: f64,
    spot_entry_price: f64,
    future_entry_price: f64,
    entry_fees: f64,
    status: String,
    #[serde(default)]
    spot_exit_twap: Option<f64>,
    #[serde(default)]
    settlement_price: Option<f64>,
    #[serde(default)]
    net_pnl_usdt: Option<f64>,
    #[serde(default)]
    settlement_source: Option<String>,
    #[serde(default)]
    spot_mark_bid: Option<f64>,
    #[serde(default)]
    future_mark_ask: Option<f64>,
    #[serde(default)]
    marked_net_pnl_usdt: Option<f64>,
    #[serde(default)]
    closed_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RestingOrder {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    remaining_quantity: f64,
    limit_price: f64,
    timestamp: u64,
    #[serde(default)]
    maker_fee_rate: f64,
    #[serde(default)]
    estimated_fee: f64,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    time_in_force: String,
    #[serde(default)]
    post_only: bool,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    parent_order_id: Option<String>,
    #[serde(default)]
    reduce_only: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum BarrierDirection {
    Above,
    Below,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct BarrierOrder {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    remaining_quantity: f64,
    filled_quantity: f64,
    trigger_price: f64,
    trigger_direction: BarrierDirection,
    created_at: u64,
    #[serde(default)]
    triggered_at: Option<u64>,
    #[serde(default)]
    retry_after: Option<u64>,
    status: String,
    #[serde(default)]
    average_execution_price: Option<f64>,
    #[serde(default)]
    fee_paid: f64,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    order_kind: String,
    #[serde(default)]
    execution_type: String,
    #[serde(default)]
    limit_price: Option<f64>,
    #[serde(default)]
    trailing_amount: Option<f64>,
    #[serde(default)]
    trailing_percent: Option<f64>,
    #[serde(default)]
    extreme_price: Option<f64>,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    parent_order_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OrderType {
    Ioc,
    Market,
    Limit,
    Barrier,
    StopMarket,
    StopLimit,
    TrailingStop,
    PostOnlyLimit,
    Fok,
}

#[derive(Deserialize)]
struct NewOrder {
    symbol: String,
    side: TradeSide,
    order_type: OrderType,
    quantity: f64,
    limit_price: Option<f64>,
    #[serde(default)]
    measure_execution_quality: bool,
    #[serde(default)]
    trigger_price: Option<f64>,
    #[serde(default)]
    trigger_direction: Option<BarrierDirection>,
    #[serde(default)]
    trailing_amount: Option<f64>,
    #[serde(default)]
    trailing_percent: Option<f64>,
    #[serde(default)]
    expires_at: Option<u64>,
    #[serde(default)]
    time_in_force: Option<String>,
    #[serde(default = "default_trade_source")]
    source: String,
    #[serde(default)]
    client_order_id: Option<String>,
    #[serde(default)]
    reduce_only: bool,
}

#[derive(Clone, Debug, Serialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    realized_pnl: f64,
    entry_fees: f64,
    fees_paid: f64,
    known_fee_fills: usize,
    unknown_fee_fills: usize,
    estimated_exit_fee_rate: f64,
    last_trade_timestamp: u64,
}
#[derive(Clone, Serialize)]
struct PositionsResponse {
    positions: Vec<Position>,
    trade_count: usize,
}
#[derive(Serialize)]
struct RealizedPnlResponse {
    realized_pnl_by_quote: BTreeMap<String, f64>,
    realized_pnl_usd: Option<f64>,
    realized_pnl_by_parent_order: BTreeMap<String, f64>,
    realized_pnl_by_barrier_created_at: BTreeMap<String, f64>,
    realized_pnl_all_time_by_parent_order: BTreeMap<String, f64>,
    realized_pnl_all_time_by_quote: BTreeMap<String, f64>,
    realized_pnl_all_time_usd: Option<f64>,
}
#[derive(Deserialize)]
struct RealizedPnlQuery {
    source: Option<String>,
    since_ms: Option<u64>,
}
#[derive(Serialize)]
struct TradesResponse {
    trades: Vec<PositionTrade>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    next_offset: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    has_older: Option<bool>,
}
#[derive(Serialize)]
struct OrdersResponse {
    orders: Vec<RestingOrder>,
    barrier_orders: Vec<BarrierOrder>,
}
#[derive(Serialize)]
struct OrderResponse {
    estimated_fee: f64,
    fee_complete: bool,
    unknown_fee_fills: usize,
    filled_quantity: f64,
    remaining_quantity: f64,
    average_execution_price: Option<f64>,
    order: Option<RestingOrder>,
    barrier_order: Option<BarrierOrder>,
    #[serde(default)]
    processing_timings_ms: BTreeMap<String, f64>,
}

