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
    #[serde(default)]
    lifecycle: Option<CanonicalLifecycleLink>,
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
    #[serde(default)]
    lifecycle: Option<CanonicalLifecycleLink>,
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct CanonicalLifecycleLink {
    schema_version: u16,
    opportunity_id: String,
    episode_id: String,
    trade_id: String,
    #[serde(default)]
    profile_id: Option<String>,
    #[serde(default)]
    profile_hash: Option<String>,
    #[serde(default)]
    profile_version: Option<u64>,
    execution_mode: String,
    entry_policy: String,
    source: String,
}

#[derive(Clone, Debug, Serialize)]
struct PositionTrade {
    id: String,
    /// Versioned identity for this individual execution fill. Missing on
    /// historical rows; never inferred from the legacy row/order IDs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fill_id: Option<String>,
    /// Additive match evidence from Paper Exchange; this is not a settled-fill claim.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_schema_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_source_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_source_cursor: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_request_fingerprint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_execution_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_source_quantity: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_source_price: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    paper_exchange_source_complete: Option<bool>,
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
    /// Explicit client identity; legacy rows leave this unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_order_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle: Option<CanonicalLifecycleLink>,
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
    #[serde(default)]
    fill_id: Option<String>,
    #[serde(default)]
    paper_exchange_schema_version: Option<String>,
    #[serde(default)]
    paper_exchange_source_epoch: Option<String>,
    #[serde(default)]
    paper_exchange_source_cursor: Option<u64>,
    #[serde(default)]
    paper_exchange_request_fingerprint: Option<String>,
    #[serde(default)]
    paper_exchange_execution_id: Option<String>,
    #[serde(default)]
    paper_exchange_source_quantity: Option<f64>,
    #[serde(default)]
    paper_exchange_source_price: Option<f64>,
    #[serde(default)]
    paper_exchange_source_complete: Option<bool>,
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
    #[serde(default)]
    client_order_id: Option<String>,
    #[serde(default)]
    lifecycle: Option<CanonicalLifecycleLink>,
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
        let fee_metadata_present =
            wire.fee.is_some() && !fee_currency.trim().is_empty() && fee.is_finite();
        let fee_known = wire.fee_known.unwrap_or(fee_metadata_present) && fee_metadata_present;
        Ok(Self {
            id: wire.id,
            fill_id: wire.fill_id,
            paper_exchange_schema_version: wire.paper_exchange_schema_version,
            paper_exchange_source_epoch: wire.paper_exchange_source_epoch,
            paper_exchange_source_cursor: wire.paper_exchange_source_cursor,
            paper_exchange_request_fingerprint: wire.paper_exchange_request_fingerprint,
            paper_exchange_execution_id: wire.paper_exchange_execution_id,
            paper_exchange_source_quantity: wire.paper_exchange_source_quantity,
            paper_exchange_source_price: wire.paper_exchange_source_price,
            paper_exchange_source_complete: wire.paper_exchange_source_complete,
            symbol: wire.symbol,
            side: wire.side,
            quantity: wire.quantity,
            execution_price: wire.execution_price,
            reference_price: wire.reference_price,
            timestamp: wire.timestamp,
            source: wire.source,
            parent_order_id: wire.parent_order_id,
            client_order_id: wire.client_order_id,
            lifecycle: wire.lifecycle,
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
        assert_eq!(trade.fill_id, None);
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
        }))
        .unwrap();
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

    #[test]
    fn canonical_lifecycle_link_round_trips_and_legacy_rows_remain_unlinked() {
        let link = CanonicalLifecycleLink {
            schema_version: 1,
            opportunity_id: "autotrader:v1:opportunity:17".into(),
            episode_id: "autotrader:v1:episode:17".into(),
            trade_id: "autotrader:v1:trade:17".into(),
            profile_id: Some("profile-v1".into()),
            profile_hash: None,
            profile_version: Some(4),
            execution_mode: "paper".into(),
            entry_policy: "strict".into(),
            source: "auto_trader_entry_reservation".into(),
        };
        let mut value = serde_json::json!({
            "id":"fill-1", "symbol":"BTC-USDT", "side":"buy", "quantity":1.0,
            "execution_price":100.0, "timestamp":7, "fee":0.0, "fee_currency":"USDT",
            "fee_known":true, "parent_order_id":"order-1", "lifecycle":link
        });
        let linked: PositionTrade = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(linked.lifecycle, Some(link.clone()));
        assert_eq!(linked.fill_id, None);
        value.as_object_mut().unwrap().remove("lifecycle");
        let legacy: PositionTrade = serde_json::from_value(value).unwrap();
        assert_eq!(legacy.lifecycle, None);
        assert_eq!(legacy.fill_id, None);
    }

    #[test]
    fn canonical_fill_id_is_additive_and_legacy_rows_do_not_gain_one() {
        let legacy: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"row-1", "symbol":"BTC-USDT", "side":"buy", "quantity":0.5,
            "execution_price":100.0, "timestamp":7
        }))
        .unwrap();
        assert_eq!(legacy.fill_id, None);
        assert!(
            serde_json::to_value(&legacy)
                .unwrap()
                .get("fill_id")
                .is_none()
        );

        let value = serde_json::json!({
            "id":"row-2", "fill_id":"pm-fill:v1:epoch:2", "symbol":"BTC-USDT",
            "side":"buy", "quantity":0.5, "execution_price":100.0, "timestamp":8
        });
        let actual: PositionTrade = serde_json::from_value(value).unwrap();
        assert_eq!(actual.fill_id.as_deref(), Some("pm-fill:v1:epoch:2"));
        assert_eq!(
            serde_json::to_value(actual).unwrap()["fill_id"],
            "pm-fill:v1:epoch:2"
        );
    }

    #[test]
    fn paper_exchange_match_evidence_round_trips_and_legacy_rows_stay_absent() {
        let legacy: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"legacy", "symbol":"BTC-USDT", "side":"buy", "quantity":0.5,
            "execution_price":100.0, "timestamp":7
        }))
        .unwrap();
        assert_eq!(legacy.paper_exchange_schema_version, None);
        assert_eq!(legacy.paper_exchange_source_complete, None);
        let legacy_json = serde_json::to_value(legacy).unwrap();
        assert!(legacy_json.get("paper_exchange_execution_id").is_none());

        let trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"fill-2", "symbol":"BTC-USDT", "side":"sell", "quantity":0.25,
            "execution_price":110.0, "timestamp":8,
            "paper_exchange_schema_version":"paper_exchange_match:v1",
            "paper_exchange_source_epoch":"epoch-4",
            "paper_exchange_source_cursor":18,
            "paper_exchange_request_fingerprint":"sha256:req-2",
            "paper_exchange_execution_id":"pe-exec-42",
            "paper_exchange_source_quantity":0.5,
            "paper_exchange_source_price":111.0,
            "paper_exchange_source_complete":false
        }))
        .unwrap();
        let serialized = serde_json::to_value(&trade).unwrap();
        assert_eq!(
            serialized["paper_exchange_schema_version"],
            "paper_exchange_match:v1"
        );
        assert_eq!(serialized["paper_exchange_source_epoch"], "epoch-4");
        assert_eq!(serialized["paper_exchange_source_cursor"], 18);
        assert_eq!(
            serialized["paper_exchange_request_fingerprint"],
            "sha256:req-2"
        );
        assert_eq!(serialized["paper_exchange_execution_id"], "pe-exec-42");
        assert_eq!(serialized["paper_exchange_source_quantity"], 0.5);
        assert_eq!(serialized["paper_exchange_source_price"], 111.0);
        assert_eq!(serialized["paper_exchange_source_complete"], false);
    }

    #[test]
    fn client_order_identity_is_explicit_and_legacy_rows_stay_unknown() {
        let legacy_trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"legacy-fill", "symbol":"BTC-USDT", "side":"buy", "quantity":0.5,
            "execution_price":100.0, "timestamp":7, "parent_order_id":"legacy-parent"
        }))
        .unwrap();
        assert_eq!(
            legacy_trade.parent_order_id.as_deref(),
            Some("legacy-parent")
        );
        assert_eq!(legacy_trade.client_order_id, None);
        let legacy_trade_json = serde_json::to_value(&legacy_trade).unwrap();
        assert!(legacy_trade_json.get("client_order_id").is_none());

        let trade: PositionTrade = serde_json::from_value(serde_json::json!({
            "id":"fill", "symbol":"BTC-USDT", "side":"buy", "quantity":0.5,
            "execution_price":100.0, "timestamp":8, "parent_order_id":"legacy-parent",
            "client_order_id":"client-123"
        }))
        .unwrap();
        assert_eq!(trade.parent_order_id.as_deref(), Some("legacy-parent"));
        assert_eq!(trade.client_order_id.as_deref(), Some("client-123"));
        assert_eq!(
            serde_json::to_value(&trade).unwrap()["client_order_id"],
            "client-123"
        );

        let legacy_order: RestingOrder = serde_json::from_value(serde_json::json!({
            "id":"legacy-order", "symbol":"BTC-USDT", "side":"buy", "quantity":1.0,
            "remaining_quantity":1.0, "limit_price":99.0, "timestamp":9,
            "parent_order_id":"legacy-order-parent"
        }))
        .unwrap();
        assert_eq!(legacy_order.client_order_id, None);
        assert!(
            serde_json::to_value(&legacy_order)
                .unwrap()
                .get("client_order_id")
                .is_none()
        );
        let order: RestingOrder = serde_json::from_value(serde_json::json!({
            "id":"order", "symbol":"BTC-USDT", "side":"buy", "quantity":1.0,
            "remaining_quantity":1.0, "limit_price":99.0, "timestamp":10,
            "parent_order_id":"legacy-order-parent", "client_order_id":"client-456"
        }))
        .unwrap();
        assert_eq!(
            order.parent_order_id.as_deref(),
            Some("legacy-order-parent")
        );
        assert_eq!(order.client_order_id.as_deref(), Some("client-456"));
        assert_eq!(
            serde_json::to_value(&order).unwrap()["client_order_id"],
            "client-456"
        );

        let legacy_barrier: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id":"legacy-barrier", "symbol":"BTC-USDT", "side":"sell", "quantity":1.0,
            "remaining_quantity":1.0, "filled_quantity":0.0, "trigger_price":110.0,
            "trigger_direction":"above", "created_at":11, "status":"armed",
            "parent_order_id":"legacy-barrier-parent"
        }))
        .unwrap();
        assert_eq!(legacy_barrier.client_order_id, None);
        assert!(
            serde_json::to_value(&legacy_barrier)
                .unwrap()
                .get("client_order_id")
                .is_none()
        );
        let barrier: BarrierOrder = serde_json::from_value(serde_json::json!({
            "id":"barrier", "symbol":"BTC-USDT", "side":"sell", "quantity":1.0,
            "remaining_quantity":1.0, "filled_quantity":0.0, "trigger_price":110.0,
            "trigger_direction":"above", "created_at":12, "status":"armed",
            "parent_order_id":"legacy-barrier-parent", "client_order_id":"client-789"
        }))
        .unwrap();
        assert_eq!(
            barrier.parent_order_id.as_deref(),
            Some("legacy-barrier-parent")
        );
        assert_eq!(barrier.client_order_id.as_deref(), Some("client-789"));
        assert_eq!(
            serde_json::to_value(&barrier).unwrap()["client_order_id"],
            "client-789"
        );
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_order_id: Option<String>,
    #[serde(default)]
    lifecycle: Option<CanonicalLifecycleLink>,
    #[serde(default)]
    reduce_only: bool,
    /// Actual Paper Exchange match-response observations. They describe the
    /// source book seen by the matcher, not queue priority or fillability.
    #[serde(default)]
    market_path: Vec<ObservedMarketPathSample>,
    /// `observed_only` means retained match-response samples are available,
    /// not a continuous book history. `source_incomplete` preserves a source
    /// completeness failure, while `truncated` marks local retention loss.
    #[serde(default = "default_market_path_completeness")]
    market_path_completeness: String,
    #[serde(default)]
    market_path_dropped_samples: u64,
}

fn default_market_path_completeness() -> String {
    "unavailable".to_owned()
}

const MAX_MARKET_PATH_SAMPLES_PER_ORDER: usize = 32;
const MAX_MARKET_PATH_SAMPLES_GLOBAL: usize = 4096;
const MAX_TERMINAL_EXECUTION_PATH_ORDERS: usize = 64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
struct ObservedMarketPathSample {
    source_system: String,
    source_epoch: Option<String>,
    source_cursor: Option<u64>,
    source_cursor_kind: Option<String>,
    source_capture_complete: Option<bool>,
    request_fingerprint: Option<String>,
    attempt_order_id: String,
    parent_order_id: Option<String>,
    recorded_at_utc_ms: Option<u64>,
    book_updated_at_ms: u64,
    best_bid: Option<f64>,
    best_ask: Option<f64>,
    reference_price: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ExecutionPathTerminalRecord {
    order: RestingOrder,
    terminal_reason: String,
    terminal_at_ms: u64,
}

fn record_execution_path_sample(
    ledger: &mut Ledger,
    order_id: &str,
    sample: ObservedMarketPathSample,
) {
    let Some(order) = ledger.orders.iter_mut().find(|order| order.id == order_id) else {
        return;
    };
    if order.market_path.iter().any(|previous| {
        previous.source_epoch == sample.source_epoch
            && previous.source_cursor == sample.source_cursor
            && previous.book_updated_at_ms == sample.book_updated_at_ms
            && previous.attempt_order_id == sample.attempt_order_id
    }) {
        return;
    }
    order.market_path.push(sample);
    let source_incomplete = order
        .market_path
        .iter()
        .any(|sample| sample.source_capture_complete == Some(false));
    order.market_path_completeness = if source_incomplete {
        "source_incomplete".to_owned()
    } else {
        "observed_only".to_owned()
    };
    if order.market_path.len() > MAX_MARKET_PATH_SAMPLES_PER_ORDER {
        order.market_path.remove(0);
        order.market_path_dropped_samples = order.market_path_dropped_samples.saturating_add(1);
        order.market_path_completeness = "truncated".to_owned();
    }
    trim_global_execution_path_samples(ledger);
}

fn terminalize_resting_order(
    ledger: &mut Ledger,
    order: RestingOrder,
    terminal_reason: &str,
    terminal_at_ms: u64,
) {
    ledger
        .execution_path_history
        .push(ExecutionPathTerminalRecord {
            order,
            terminal_reason: terminal_reason.to_owned(),
            terminal_at_ms,
        });
    while ledger.execution_path_history.len() > MAX_TERMINAL_EXECUTION_PATH_ORDERS {
        ledger.execution_path_history.remove(0);
        ledger.execution_path_history_truncated = true;
    }
    trim_global_execution_path_samples(ledger);
}

fn trim_global_execution_path_samples(ledger: &mut Ledger) {
    let mut total = ledger
        .orders
        .iter()
        .map(|order| order.market_path.len())
        .sum::<usize>()
        + ledger
            .execution_path_history
            .iter()
            .map(|record| record.order.market_path.len())
            .sum::<usize>();
    while total > MAX_MARKET_PATH_SAMPLES_GLOBAL {
        let oldest = ledger
            .orders
            .iter()
            .enumerate()
            .flat_map(|(order_index, order)| {
                order
                    .market_path
                    .iter()
                    .enumerate()
                    .map(move |(sample_index, sample)| {
                        (sample.book_updated_at_ms, false, order_index, sample_index)
                    })
            })
            .chain(ledger.execution_path_history.iter().enumerate().flat_map(
                |(order_index, record)| {
                    record.order.market_path.iter().enumerate().map(
                        move |(sample_index, sample)| {
                            (sample.book_updated_at_ms, true, order_index, sample_index)
                        },
                    )
                },
            ))
            .min();
        let Some((_, terminal, order_index, sample_index)) = oldest else {
            break;
        };
        let order = if terminal {
            &mut ledger.execution_path_history[order_index].order
        } else {
            &mut ledger.orders[order_index]
        };
        order.market_path.remove(sample_index);
        order.market_path_dropped_samples = order.market_path_dropped_samples.saturating_add(1);
        order.market_path_completeness = "truncated".to_owned();
        total -= 1;
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    client_order_id: Option<String>,
    #[serde(default)]
    lifecycle: Option<CanonicalLifecycleLink>,
    /// Durable request identity for monotonic trigger adjustments. Kept with
    /// the barrier row so retries remain idempotent across service restarts.
    #[serde(default)]
    trigger_adjustments: Vec<TriggerAdjustmentRecord>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct TriggerAdjustmentRecord {
    request_id: String,
    #[serde(default)]
    adjusted_at_ms: u64,
    barrier_id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    parent_order_id: Option<String>,
    lifecycle_trade_id: Option<String>,
    profile_id: String,
    profile_version: u64,
    policy_source: String,
    reason: TriggerAdjustmentReason,
    target_net_return_percent: f64,
    expected_trigger_price: f64,
    new_trigger_price: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum TriggerAdjustmentReason {
    TrailingStop,
    BreakevenProtection,
}

const MAX_TRIGGER_ADJUSTMENTS_PER_BARRIER: usize = 64;

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
    lifecycle: Option<CanonicalLifecycleLink>,
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
    /// Authoritative fill rows from the durable ledger, including versioned
    /// fill ID and fee-currency provenance. Empty for legacy/unattributed calls.
    #[serde(default)]
    fill_evidence: Vec<PositionTrade>,
    #[serde(default)]
    processing_timings_ms: BTreeMap<String, f64>,
}

#[cfg(test)]
mod execution_path_bounds_tests {
    use super::*;

    fn order() -> RestingOrder {
        serde_json::from_value(serde_json::json!({
            "id":"order-1", "symbol":"BTC-USDT", "side":"buy", "quantity":1.0,
            "remaining_quantity":1.0, "limit_price":99.0, "timestamp":10,
            "parent_order_id":"parent-1", "client_order_id":"client-1"
        }))
        .unwrap()
    }

    fn sample(index: u64) -> ObservedMarketPathSample {
        ObservedMarketPathSample {
            source_system: "paper_exchange.match_response".to_owned(),
            source_epoch: Some("epoch-1".to_owned()),
            source_cursor: Some(index),
            source_cursor_kind: Some("global".to_owned()),
            source_capture_complete: Some(true),
            request_fingerprint: Some(format!("request-{index}")),
            attempt_order_id: format!("order-1:resting:{index}"),
            parent_order_id: Some("parent-1".to_owned()),
            recorded_at_utc_ms: Some(1_000 + index),
            book_updated_at_ms: 1_000 + index,
            best_bid: Some(98.0),
            best_ask: Some(100.0),
            reference_price: Some(99.0),
        }
    }

    #[test]
    fn order_path_is_bounded_and_terminal_evidence_keeps_identity_and_completeness() {
        let mut ledger = Ledger::default();
        ledger.orders.push(order());
        for index in 0..=(MAX_MARKET_PATH_SAMPLES_PER_ORDER as u64) {
            record_execution_path_sample(&mut ledger, "order-1", sample(index));
        }
        assert_eq!(
            ledger.orders[0].market_path.len(),
            MAX_MARKET_PATH_SAMPLES_PER_ORDER
        );
        assert_eq!(ledger.orders[0].market_path_dropped_samples, 1);
        assert_eq!(ledger.orders[0].market_path_completeness, "truncated");
        assert_eq!(ledger.orders[0].market_path[0].source_cursor, Some(1));

        let order = ledger.orders.remove(0);
        terminalize_resting_order(&mut ledger, order, "expired", 2_000);
        assert!(ledger.orders.is_empty());
        assert_eq!(ledger.execution_path_history.len(), 1);
        let restored: Ledger =
            serde_json::from_value(serde_json::to_value(&ledger).unwrap()).unwrap();
        let terminal = &restored.execution_path_history[0];
        assert_eq!(terminal.terminal_reason, "expired");
        assert_eq!(terminal.terminal_at_ms, 2_000);
        assert_eq!(terminal.order.parent_order_id.as_deref(), Some("parent-1"));
        assert_eq!(terminal.order.client_order_id.as_deref(), Some("client-1"));
        assert_eq!(terminal.order.market_path_completeness, "truncated");
        assert_eq!(
            terminal.order.market_path[0].source_epoch.as_deref(),
            Some("epoch-1")
        );
        assert_eq!(
            terminal.order.market_path[0].source_cursor_kind.as_deref(),
            Some("global")
        );
    }

    #[test]
    fn legacy_orders_explicitly_report_unavailable_market_path() {
        let legacy = order();
        assert!(legacy.market_path.is_empty());
        assert_eq!(legacy.market_path_completeness, "unavailable");
        assert_eq!(legacy.market_path_dropped_samples, 0);
    }

    #[test]
    fn global_path_sample_cap_truncates_oldest_observations_explicitly() {
        let mut ledger = Ledger::default();
        let orders_needed = MAX_MARKET_PATH_SAMPLES_GLOBAL / MAX_MARKET_PATH_SAMPLES_PER_ORDER + 1;
        for order_index in 0..orders_needed {
            let mut current = order();
            current.id = format!("order-{order_index}");
            for sample_index in 0..MAX_MARKET_PATH_SAMPLES_PER_ORDER {
                current.market_path.push(sample(sample_index as u64));
            }
            current.market_path_completeness = "observed_only".to_owned();
            ledger.orders.push(current);
        }
        trim_global_execution_path_samples(&mut ledger);

        let retained = ledger
            .orders
            .iter()
            .map(|order| order.market_path.len())
            .sum::<usize>();
        assert_eq!(retained, MAX_MARKET_PATH_SAMPLES_GLOBAL);
        assert!(
            ledger
                .orders
                .iter()
                .any(|order| order.market_path_completeness == "truncated")
        );
        assert!(
            ledger
                .orders
                .iter()
                .any(|order| order.market_path_dropped_samples > 0)
        );
    }
}
