use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use bytes::Bytes;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    time::{Duration, timeout},
};

const SYMBOL_PRICE_MESSAGE_LENGTH: usize = 64;
const BOOK_DEPTH: usize = 7;
const NUM_VWAP_BINS: usize = 78;
const EPSILON: f64 = 1e-12;
static HTTP_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

pub async fn router() -> anyhow::Result<Router> {
    let spot_symbols = Arc::new(crate::feed_client::spot_symbols().await?);
    let store = Arc::new(PositionStore::load(Arc::clone(&spot_symbols)).await?);
    let mut symbols = store
        .ledger
        .lock()
        .await
        .orders
        .iter()
        .map(|order| order.symbol.clone())
        .collect::<std::collections::HashSet<_>>();
    symbols.extend(store.ledger.lock().await.barrier_orders.iter().filter(|order| order.status == "armed").map(|order| order.symbol.clone()));
    symbols.extend(summarize_positions(&store.ledger.lock().await.trades).positions.into_iter()
        .filter(|position| position.quantity.abs() > EPSILON)
        .map(|position| position.symbol));
    for symbol in symbols {
        store.ensure_subscription(&symbol);
    }
    let barrier_store = Arc::clone(&store);
    tokio::spawn(async move { barrier_order_loop(barrier_store).await; });
    let expiry_store = Arc::clone(&store);
    tokio::spawn(async move { expire_orders_loop(expiry_store).await; });
    let sample_store = Arc::clone(&store);
    tokio::spawn(async move {
        loop {
            match crate::feed_client::stream("tickers:*".to_owned()).await {
                Ok(mut stream) => while let Some(Ok(bytes)) = stream.next().await {
                    for message in bytes.chunks_exact(SYMBOL_PRICE_MESSAGE_LENGTH) {
                        let instrument = String::from_utf8_lossy(&message[8..40]).trim_end_matches([' ', '\0']).to_owned();
                        let price = f64::from_le_bytes(message[40..48].try_into().unwrap_or_default());
                        if price.is_finite() && price > 0.0 {
                            if let Ok(timestamp) = now_ms() { sample_store.record_ticker_sample(instrument, price, timestamp).await; }
                        }
                    }
                },
                Err(error) => eprintln!("FeedHandlers ticker API disconnected: {error:#}"),
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    });
    Ok(Router::new()
        .route("/positions", get(get_positions))
        .route("/positions/account", get(get_account))
        .route("/account", get(get_account))
        .route("/positions/account/topups", post(top_up_account))
        .route("/positions/account/leverage", post(set_account_leverage))
        .route("/account/topups", post(top_up_account))
        .route("/positions/usd-rates", get(get_usd_rates))
        .route("/positions/fee-rates", get(get_fee_rates))
        .route("/positions/hedge-candidates", get(get_hedge_candidates))
        .route("/internal/insights-inputs", get(get_insights_inputs))
        .route("/positions/trades", get(get_position_trades))
        .route("/positions/vwap", get(get_vwap_runs).post(start_vwap))
        .route("/positions/twap", post(start_twap))
        .route("/positions/vwap/{id}", axum::routing::delete(cancel_vwap))
        .route("/positions/orders", get(get_orders).post(submit_order))
        .route("/positions/barriers", get(get_barrier_orders))
        .route("/positions/barriers/profit-target", post(create_profit_barrier))
        .route("/positions/close-profitable", post(close_profitable))
        .route(
            "/positions/orders/{id}",
            axum::routing::delete(cancel_order),
        )
        .with_state(store))
}


async fn expire_orders_loop(store: Arc<PositionStore>) {
    loop {
        tokio::time::sleep(Duration::from_secs(1)).await;
        let Ok(now) = now_ms() else { continue; };
        let mut ledger = store.ledger.lock().await;
        let old_len = ledger.orders.len();
        ledger.orders.retain(|order| order.expires_at.is_none_or(|expires| expires > now));
        if ledger.orders.len() != old_len {
            if let Err(error) = store.persist(&ledger).await { eprintln!("Could not persist GTD expiry: {error}"); }
        }
    }
}

async fn barrier_order_loop(store: Arc<PositionStore>) {
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let barriers = store.ledger.lock().await.barrier_orders.iter()
            .filter(|order| order.status == "armed").cloned().collect::<Vec<_>>();
        if barriers.is_empty() { continue; }
        let books = store.books.lock().await.clone();
        let Ok(now) = now_ms() else { continue; };
        for barrier in barriers {
            let Some(book) = books.get(&barrier.symbol) else { continue; };
            if now.saturating_sub(book.updated_at_ms) > 5_000 { continue; }
            let current = match barrier.side {
                TradeSide::Buy => book.asks.first().map(|level| level.price),
                TradeSide::Sell => book.bids.first().map(|level| level.price),
            };
            let Some(current) = current.filter(|price| price.is_finite() && *price > 0.0) else { continue; };
            let mut effective_barrier = barrier.clone();
            if barrier.trailing_amount.is_some() || barrier.trailing_percent.is_some() {
                let prior = barrier.extreme_price.unwrap_or(current);
                let favorable = if matches!(barrier.side, TradeSide::Sell) { current > prior } else { current < prior };
                if favorable {
                    let distance = barrier.trailing_amount.unwrap_or(current * barrier.trailing_percent.unwrap_or_default());
                    effective_barrier.extreme_price = Some(current);
                    effective_barrier.trigger_price = if matches!(barrier.side, TradeSide::Sell) { current - distance } else { current + distance };
                    let mut ledger = store.ledger.lock().await;
                    if let Some(order) = ledger.barrier_orders.iter_mut().find(|order| order.id == barrier.id && order.status == "armed") {
                        order.extreme_price = effective_barrier.extreme_price;
                        order.trigger_price = effective_barrier.trigger_price;
                        if let Err(error) = store.persist(&ledger).await { eprintln!("Could not persist trailing stop {}: {error}", barrier.id); }
                    }
                }
            }
            let triggered = match effective_barrier.trigger_direction {
                BarrierDirection::Above => current >= effective_barrier.trigger_price,
                BarrierDirection::Below => current <= effective_barrier.trigger_price,
            };
            if !triggered { continue; }
            let claimed = {
                let mut ledger = store.ledger.lock().await;
                let Some(index) = ledger.barrier_orders.iter().position(|order| order.id == barrier.id && order.status == "armed") else { continue; };
                ledger.barrier_orders[index].status = "triggering".to_owned();
                ledger.barrier_orders[index].triggered_at = Some(now);
                if let Err(error) = store.persist(&ledger).await {
                    eprintln!("Could not persist triggered barrier {}: {error}", barrier.id);
                    ledger.barrier_orders[index].status = "armed".to_owned();
                    ledger.barrier_orders[index].triggered_at = None;
                    continue;
                }
                ledger.barrier_orders[index].clone()
            };
            let result = submit_order(State(Arc::clone(&store)), Json(NewOrder {
                symbol: claimed.symbol.clone(), side: claimed.side.clone(), order_type: if claimed.execution_type == "limit" { OrderType::Limit } else { OrderType::Market },
                quantity: claimed.remaining_quantity, limit_price: claimed.limit_price, trigger_price: None,
                trigger_direction: None, trailing_amount: None, trailing_percent: None, expires_at: None, time_in_force: None, measure_execution_quality: false,
            })).await;
            let mut ledger = store.ledger.lock().await;
            if let Some(order) = ledger.barrier_orders.iter_mut().find(|order| order.id == claimed.id) {
                match result {
                    Ok(response) => {
                        let response = response.0;
                        order.filled_quantity += response.filled_quantity;
                        order.remaining_quantity = (order.quantity - order.filled_quantity).max(0.0);
                        order.average_execution_price = response.average_execution_price;
                        order.fee_paid += response.estimated_fee;
                        order.status = if claimed.execution_type == "limit" && response.order.is_some() { "activated".to_owned() }
                            else if order.remaining_quantity <= EPSILON { "filled".to_owned() }
                            else if order.filled_quantity > EPSILON { "partially_filled".to_owned() }
                            else { "unfilled".to_owned() };
                        order.error = None;
                    }
                    Err((_, error)) => { order.status = "rejected".to_owned(); order.error = Some(error); }
                }
            }
            if let Err(error) = store.persist(&ledger).await {
                eprintln!("Could not persist barrier execution {}: {error}", claimed.id);
            }
        }
    }
}

#[derive(Serialize)]
struct UsdRatesResponse {
    rates: BTreeMap<String, f64>,
    updated_at: u64,
}

async fn get_usd_rates() -> Result<Json<UsdRatesResponse>, (StatusCode, String)> {
    let response = HTTP_CLIENT.get_or_init(reqwest::Client::new)
        .get("https://www.okx.com/api/v5/market/index-tickers?quoteCcy=USD")
        .send()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("OKX USD index request failed: {error}")))?
        .error_for_status()
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("OKX USD index request failed: {error}")))?
        .text()
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Could not read OKX USD index response: {error}")))?;
    let response: serde_json::Value = serde_json::from_str(&response)
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid OKX USD index response: {error}")))?;
    let data = response.get("data").and_then(serde_json::Value::as_array)
        .ok_or_else(|| (StatusCode::BAD_GATEWAY, "OKX USD index response has no data".to_owned()))?;
    let mut rates = BTreeMap::from([("USD".to_owned(), 1.0)]);
    let mut updated_at = 0;
    for item in data {
        let Some(instrument) = item.get("instId").and_then(serde_json::Value::as_str) else { continue; };
        let Some((currency, quote)) = instrument.split_once('-') else { continue; };
        if quote != "USD" { continue; }
        let Some(rate) = item.get("idxPx").and_then(serde_json::Value::as_str).and_then(|value| value.parse::<f64>().ok()) else { continue; };
        if !rate.is_finite() || rate <= 0.0 { continue; }
        rates.insert(currency.to_owned(), rate);
        if let Some(timestamp) = item.get("ts").and_then(serde_json::Value::as_str).and_then(|value| value.parse::<u64>().ok()) {
            updated_at = updated_at.max(timestamp);
        }
    }
    if rates.len() == 1 {
        return Err((StatusCode::BAD_GATEWAY, "OKX returned no usable USD index rates".to_owned()));
    }
    Ok(Json(UsdRatesResponse { rates, updated_at }))
}

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
}
fn default_max_participation() -> f64 { 1.0 }
#[derive(Clone, Copy, Default, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum ExecutionStrategy { #[default] Vwap, Twap }
#[derive(Clone, Serialize)]
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

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TradeSide {
    Buy,
    Sell,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PositionTrade {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    execution_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    reference_price: Option<f64>,
    timestamp: u64,
    #[serde(default)]
    fee: f64,
    #[serde(default)]
    fee_currency: String,
    #[serde(default)]
    fee_rate: f64,
    #[serde(default)]
    fee_liquidity: String,
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
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum BarrierDirection { Above, Below }

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
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
enum OrderType {
    Ioc, Market, Limit, Barrier, StopMarket, StopLimit, TrailingStop, PostOnlyLimit, Fok,
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
}

#[derive(Clone, Debug, Serialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    realized_pnl: f64,
    entry_fees: f64,
    fees_paid: f64,
    estimated_exit_fee_rate: f64,
    last_trade_timestamp: u64,
}
#[derive(Serialize)]
struct PositionsResponse {
    positions: Vec<Position>,
    trade_count: usize,
}
#[derive(Serialize)]
struct TradesResponse {
    trades: Vec<PositionTrade>,
}
#[derive(Serialize)]
struct HedgeCandidate {
    symbol: String,
    correlation: Option<f64>,
    paired_samples: usize,
    latest_price: Option<f64>,
}
#[derive(Serialize)]
struct HedgePosition {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    latest_price: Option<f64>,
    candidates: Vec<HedgeCandidate>,
}
#[derive(Serialize)]
struct HedgePairCorrelation {
    symbol_a: String,
    symbol_b: String,
    correlation: Option<f64>,
}
#[derive(Serialize)]
struct HedgeCandidatesResponse {
    positions: Vec<HedgePosition>,
    pair_correlations: Vec<HedgePairCorrelation>,
}
#[derive(Serialize)]
struct OrdersResponse {
    orders: Vec<RestingOrder>,
    barrier_orders: Vec<BarrierOrder>,
}
#[derive(Serialize)]
struct OrderResponse {
    estimated_fee: f64,
    filled_quantity: f64,
    remaining_quantity: f64,
    average_execution_price: Option<f64>,
    order: Option<RestingOrder>,
    barrier_order: Option<BarrierOrder>,
}
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
struct MatcherFill { quantity: f64, price: f64 }
#[derive(Deserialize)]
struct MatcherResponse { reference_price: Option<f64>, fills: Vec<MatcherFill>, remaining_quantity: f64, filled_quantity: f64, average_execution_price: Option<f64> }

#[derive(Clone, Serialize, Deserialize)]
struct AccountTopup { amount: f64, timestamp: u64 }
#[derive(Deserialize)]
struct TopupRequest { amount: f64 }
#[derive(Deserialize)]
struct LeverageRequest { max_leverage: f64 }
#[derive(Serialize)]
struct AccountResponse { currency: &'static str, cash: f64, equity: f64, realized_pnl: f64, unrealized_pnl: f64, fees_paid: f64, equity_available: bool, initial_cash: f64, topups: Vec<AccountTopup>, max_leverage: f64 }
#[derive(Serialize)]
struct FeeRatesResponse { maker: f64, taker: f64 }
struct FeeRates { maker: f64, taker: f64 }
fn configured_fee_rates() -> FeeRates {
    let parse = |name: &str, default: f64| std::env::var(name).ok().and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1).unwrap_or(default);
    FeeRates { maker: -parse("PAPER_OKX_SPOT_MAKER_FEE_RATE", 0.0008), taker: -parse("PAPER_OKX_SPOT_TAKER_FEE_RATE", 0.001) }
}
pub(crate) fn configured_taker_fee_rate() -> f64 { configured_fee_rates().taker.abs() }
fn default_account_cash() -> f64 { std::env::var("PAPER_ACCOUNT_INITIAL_CASH").ok().and_then(|value| value.parse().ok()).filter(|value: &f64| value.is_finite() && *value > 0.0).unwrap_or(10_000.0) }
fn paper_max_leverage() -> f64 { std::env::var("PAPER_ACCOUNT_MAX_LEVERAGE").ok().and_then(|value| value.parse::<f64>().ok()).filter(|value| value.is_finite() && *value >= 1.0 && *value <= 10.0).unwrap_or(2.0) }
fn configured_account_leverage(ledger: &Ledger) -> f64 { ledger.max_leverage.unwrap_or_else(paper_max_leverage) }

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
}
struct PositionStore {
    path: PathBuf,
    ledger: Mutex<Ledger>,
    books: Mutex<HashMap<String, BookSnapshot>>,
    price_samples: Mutex<HashMap<String, Vec<(u64, f64)>>>,
    hedge_samples: Mutex<HashMap<String, Vec<(u64, f64)>>>,
    book_changed: Notify,
    subscriptions: StdMutex<HashMap<String, mpsc::Sender<anyhow::Result<Bytes>>>>,
    spot_symbols: Arc<Vec<String>>,
    vwap_runs: Mutex<HashMap<String, VwapRunEntry>>,
}

impl PositionStore {
    async fn load(spot_symbols: Arc<Vec<String>>) -> anyhow::Result<Self> {
        let path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("positions.json"));
        let mut ledger = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value.is_array() {
                    Ledger {
                        trades: serde_json::from_value(value)?,
                        initial_cash: default_account_cash(),
                        topups: Vec::new(),
                        max_leverage: None,
                        orders: Vec::new(),
                        barrier_orders: Vec::new(),
                    }
                } else {
                    serde_json::from_value(value)?
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ledger { initial_cash: default_account_cash(), ..Ledger::default() },
            Err(error) => return Err(error.into()),
        };
        for order in &mut ledger.orders {
            if order.maker_fee_rate == 0.0 { order.maker_fee_rate = configured_fee_rates().maker; }
            order.estimated_fee = order.remaining_quantity * order.limit_price * -order.maker_fee_rate;
        }
        for order in &mut ledger.barrier_orders {
            if order.status == "triggering" {
                order.status = "interrupted".to_owned();
                order.error = Some("Backend restarted during barrier execution; review fills before replacing this order".to_owned());
            }
        }
        println!(
            "Loaded {} paper fills and {} resting orders from {}",
            ledger.trades.len(),
            ledger.orders.len(),
            path.display()
        );
        Ok(Self {
            path,
            ledger: Mutex::new(ledger),
            books: Mutex::new(HashMap::new()),
            price_samples: Mutex::new(HashMap::new()),
            hedge_samples: Mutex::new(HashMap::new()),
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            spot_symbols,
            vwap_runs: Mutex::new(HashMap::new()),
        })
    }
    async fn persist(&self, ledger: &Ledger) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec(ledger)?;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temp = self.path.with_extension("positions.tmp");
        tokio::fs::write(&temp, bytes).await?;
        tokio::fs::rename(temp, &self.path).await?;
        Ok(())
    }
    async fn record_ticker_sample(&self, instrument: String, price: f64, timestamp: u64) {
        let mut histories = self.hedge_samples.lock().await;
        let samples = histories.entry(instrument).or_default();
        if samples.last().map_or(true, |(last, _)| timestamp.saturating_sub(*last) >= 10_000) {
            samples.push((timestamp, price));
            samples.retain(|(time, _)| timestamp.saturating_sub(*time) <= 30 * 60_000);
        }
    }

    fn ensure_subscription(self: &Arc<Self>, symbol: &str) {
        let mut subscriptions = self
            .subscriptions
            .lock()
            .expect("subscription lock poisoned");
        if subscriptions.contains_key(symbol) {
            return;
        }
        let (sender, mut receiver) = mpsc::channel(64);
        subscriptions.insert(symbol.to_owned(), sender.clone());
        let store = Arc::clone(self);
        let symbol = symbol.to_owned();
        let subscription_symbol = symbol.clone();
        let log_symbol = symbol.clone();
        tokio::spawn(async move {
            println!("Paper order manager subscribing to OKX books for {symbol}");
            while let Some(result) = receiver.recv().await {
                match result {
                    Ok(bytes) => {
                        if let Some(book) = decode_book(&bytes) {
                            let mid = match (book.bids.first(), book.asks.first()) {
                                (Some(bid), Some(ask)) => Some((bid.price + ask.price) / 2.0),
                                _ => None,
                            };
                            store.books.lock().await.insert(symbol.clone(), book);
                            if let (Some(mid), Ok(now)) = (mid, now_ms()) {
                                let mut histories = store.price_samples.lock().await;
                                let samples = histories.entry(symbol.clone()).or_default();
                                if samples.last().map_or(true, |(time, _)| now.saturating_sub(*time) >= 10_000) {
                                    samples.push((now, mid));
                                    samples.retain(|(time, _)| now.saturating_sub(*time) <= 30 * 60_000);
                                }
                            }
                            store.book_changed.notify_waiters();
                            if let Err(error) = store.match_resting(&symbol).await {
                                eprintln!("Unable to match resting orders for {symbol}: {error:#}");
                            }
                        }
                    }
                    Err(error) => {
                        eprintln!("Paper order book stream for {symbol} failed: {error:#}")
                    }
                }
            }
        });
        tokio::spawn(async move {
            loop {
                if let Err(error) = async {
                    let mut stream = crate::feed_client::stream(format!("book:{subscription_symbol}")).await?;
                    while let Some(item) = stream.next().await { sender.send(item).await.map_err(|_| anyhow::anyhow!("book consumer stopped"))?; }
                    Ok::<(), anyhow::Error>(())
                }
                .await
                {
                    eprintln!(
                        "Paper OKX book subscription for {log_symbol} disconnected: {error:#}; retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        });
    }
    async fn match_resting(&self, symbol: &str) -> anyhow::Result<()> {
        let now = now_ms()?;
        {
            let mut ledger = self.ledger.lock().await;
            let old_len = ledger.orders.len();
            ledger.orders.retain(|order| order.expires_at.is_none_or(|expires| expires > now));
            if ledger.orders.len() != old_len { self.persist(&ledger).await?; }
        }
        let Some(book) = self.books.lock().await.get(symbol).cloned() else { return Ok(()); };
        {
            let mut ledger = self.ledger.lock().await;
            let old_len = ledger.orders.len();
            ledger.orders.retain(|order| {
                if !order.post_only || order.symbol != symbol { return true; }
                match order.side {
                    TradeSide::Buy => !book.asks.first().is_some_and(|ask| order.limit_price >= ask.price),
                    TradeSide::Sell => !book.bids.first().is_some_and(|bid| order.limit_price <= bid.price),
                }
            });
            if ledger.orders.len() != old_len { self.persist(&ledger).await?; }
        }
        let orders = self.ledger.lock().await.orders.iter().filter(|order| order.symbol == symbol).cloned().collect::<Vec<_>>();
        if orders.is_empty() { return Ok(()); }
        let base = std::env::var("EXECUTION_URL").unwrap_or_else(|_| "http://127.0.0.1:3003".to_owned());
        let client = HTTP_CLIENT.get_or_init(reqwest::Client::new);
        let mut matches = Vec::new();
        for order in orders {
            let response: MatcherResponse = client.post(format!("{base}/internal/orders/match"))
                .json(&serde_json::json!({"side":order.side,"quantity":order.remaining_quantity,"limit_price":order.limit_price,"book":book}))
                .send().await?.error_for_status()?.json().await?;
            matches.push((order.id, response));
        }
        let mut ledger = self.ledger.lock().await;
        let mut fills = Vec::new();
        for (id, result) in matches {
            if let Some(order) = ledger.orders.iter_mut().find(|order| order.id == id) {
                order.remaining_quantity = result.remaining_quantity.max(0.0);
                order.estimated_fee = order.remaining_quantity * order.limit_price * -order.maker_fee_rate;
                let fee_rate = if order.maker_fee_rate == 0.0 { configured_fee_rates().maker } else { order.maker_fee_rate };
                fills.extend(result.fills.into_iter().map(|fill| (order.side.clone(), fill.quantity, fill.price, fee_rate)));
            }
        }
        ledger.orders.retain(|order| order.remaining_quantity > EPSILON);
        if !fills.is_empty() {
            let timestamp = now_ms()?;
            let offset = ledger.trades.len();
            for (index, (side, quantity, execution_price, fee_rate)) in fills.into_iter().enumerate() {
                ledger.trades.push(PositionTrade {
                    id: format!("{timestamp}-{index}-{offset}"), symbol: symbol.to_owned(), side, quantity, execution_price,
                    reference_price: None, timestamp, fee: quantity * execution_price * -fee_rate,
                    fee_currency: symbol.split_once('-').map(|(_, quote)| quote.to_owned()).unwrap_or_default(),
                    fee_rate, fee_liquidity: "maker".to_owned(),
                    decision_timestamp: None, decision_price: None, decision_mid_price: None,
                    slippage: None, slippage_bps: None,
                });
            }
            self.persist(&ledger).await?;
        }
        Ok(())
    }
}

async fn get_vwap_runs(State(store): State<Arc<PositionStore>>) -> Json<VwapRunsResponse> {
    let entries = store.vwap_runs.lock().await;
    let mut runs = Vec::with_capacity(entries.len());
    for entry in entries.values() {
        runs.push(entry.status.lock().await.clone());
    }
    Json(VwapRunsResponse { runs })
}

async fn start_vwap(State(store): State<Arc<PositionStore>>, Json(request): Json<NewVwapRun>) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    start_scheduled_run(store, request).await
}
async fn start_twap(State(store): State<Arc<PositionStore>>, Json(mut request): Json<NewVwapRun>) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    request.strategy = ExecutionStrategy::Twap;
    start_scheduled_run(store, request).await
}
async fn start_scheduled_run(store: Arc<PositionStore>, request: NewVwapRun) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !request.quantity.is_finite() || request.quantity <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Quantity must be finite and greater than zero".into(),
        ));
    }
    if !(5..=390).contains(&request.duration_minutes) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Duration must be between 5 and 390 minutes".into(),
        ));
    }
    if request.strategy == ExecutionStrategy::Vwap && (!request.max_participation_rate.is_finite()
        || !(0.01..=1.0).contains(&request.max_participation_rate))
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Participation rate must be between 1% and 100%".into(),
        ));
    }
    store.ensure_subscription(&symbol);
    if store.books.lock().await.get(&symbol).is_none() {
        let wait = async {
            loop {
                let notified = store.book_changed.notified();
                if store.books.lock().await.contains_key(&symbol) { break; }
                notified.await;
            }
        };
        timeout(Duration::from_secs(8), wait).await.map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "Waiting for live OKX book timed out".to_owned()))?;
    }
    let side_book = store.books.lock().await.get(&symbol).cloned().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Live book is required to reserve VWAP buying power".to_owned()))?;
    let price = match request.side { TradeSide::Buy => side_book.asks.iter().map(|level| level.price).fold(0.0_f64, f64::max), TradeSide::Sell => side_book.bids.first().map(|level| level.price).unwrap_or(0.0) };
    if price <= 0.0 { return Err((StatusCode::SERVICE_UNAVAILABLE, "Live reference price unavailable for VWAP".into())); }
    let account_ledger = store.ledger.lock().await;
    ensure_paper_capacity(&store, &account_ledger, &symbol, &request.side, request.quantity, price, configured_fee_rates().taker).await?;
    let timestamp = now_ms().map_err(internal_error)?;
    let id = format!("vwap-{timestamp}-{}", symbol.to_ascii_lowercase());
    let status = Arc::new(Mutex::new(VwapRunStatus {
        id: id.clone(),
        symbol: symbol.clone(),
        side: request.side.clone(),
        quantity: request.quantity,
        executed_quantity: 0.0,
        strategy: if request.strategy == ExecutionStrategy::Twap { "twap".to_owned() } else { "vwap".to_owned() },
        duration_minutes: request.duration_minutes,
        started_at: timestamp,
        status: "running".into(),
        market_vwap: 0.0,
        execution_vwap: 0.0,
        slippage: 0.0,
        error: None,
    }));
    let cancelled = Arc::new(AtomicBool::new(false));
    store.vwap_runs.lock().await.insert(
        id,
        VwapRunEntry {
            status: status.clone(),
            cancelled: cancelled.clone(),
        },
    );
    store.ensure_subscription(&symbol);
    drop(account_ledger);
    let initial_status = status.lock().await.clone();
    if request.strategy == ExecutionStrategy::Twap {
        tokio::spawn(run_twap(Arc::clone(&store), request, symbol, Arc::clone(&status), cancelled));
    } else {
        tokio::spawn(run_vwap(Arc::clone(&store), request, symbol, Arc::clone(&status), cancelled));
    }
    Ok(Json(initial_status))
}

async fn cancel_vwap(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let runs = store.vwap_runs.lock().await;
    let Some(entry) = runs.get(&id) else {
        return Err((StatusCode::NOT_FOUND, "VWAP execution not found".into()));
    };
    entry.cancelled.store(true, Ordering::Relaxed);
    let mut status = entry.status.lock().await;
    if status.status == "running" {
        status.status = "cancelled".into();
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn run_twap(
    store: Arc<PositionStore>, request: NewVwapRun, symbol: String,
    status: Arc<Mutex<VwapRunStatus>>, cancelled: Arc<AtomicBool>,
) {
    const SLICES: usize = 78;
    let interval = Duration::from_secs_f64(request.duration_minutes as f64 * 60.0 / SLICES as f64);
    let mut executed = 0.0;
    let mut execution_notional = 0.0;
    let mut market_notional = 0.0;
    let mut market_samples = 0usize;
    for slice in 0..SLICES {
        if cancelled.load(Ordering::Relaxed) { break; }
        if slice > 0 { tokio::time::sleep(interval).await; }
        if cancelled.load(Ordering::Relaxed) { break; }
        let remaining = (request.quantity - executed).max(0.0);
        if remaining <= EPSILON { break; }
        let target_cumulative = if slice + 1 == SLICES { request.quantity } else { request.quantity * (slice + 1) as f64 / SLICES as f64 };
        let qty = (target_cumulative - executed).max(0.0).min(remaining);
        let book = store.books.lock().await.get(&symbol).cloned();
        if let Some(book) = book {
            if let (Some(bid), Some(ask)) = (book.bids.first(), book.asks.first()) { market_notional += (bid.price + ask.price) * 0.5; market_samples += 1; }
        }
        match execute_vwap_child(&store, &symbol, &request.side, qty).await {
            Ok((filled, average)) => {
                executed += filled;
                if let Some(price) = average { execution_notional += filled * price; }
                let mut state = status.lock().await;
                state.executed_quantity = executed;
                state.execution_vwap = if executed > 0.0 { execution_notional / executed } else { 0.0 };
                state.market_vwap = if market_samples > 0 { market_notional / market_samples as f64 } else { 0.0 };
                state.slippage = match request.side { TradeSide::Buy => state.execution_vwap - state.market_vwap, TradeSide::Sell => state.market_vwap - state.execution_vwap };
                if executed + EPSILON >= request.quantity { state.status = "completed".to_owned(); break; }
            }
            Err(error) => { status.lock().await.error = Some(error); }
        }
    }
    let mut state = status.lock().await;
    if state.status == "running" { state.status = if cancelled.load(Ordering::Relaxed) { "cancelled" } else { "completed" }.to_owned(); }
}

async fn run_vwap(
    store: Arc<PositionStore>,
    request: NewVwapRun,
    symbol: String,
    status: Arc<Mutex<VwapRunStatus>>,
    cancelled: Arc<AtomicBool>,
) {
    let mut curve = [0.0; NUM_VWAP_BINS];
    let mut total_weight = 0.0;
    for (index, weight) in curve.iter_mut().enumerate() {
        let x = (index as f64 / (NUM_VWAP_BINS - 1) as f64) * 2.0 - 1.0;
        *weight = 1.0 + 3.0 * x * x;
        total_weight += *weight;
    }
    for weight in &mut curve {
        *weight /= total_weight;
    }
    let expected = (0..NUM_VWAP_BINS).map(|index| request.quantity * curve[index] / request.max_participation_rate).collect::<Vec<_>>();
    let execution_base = std::env::var("EXECUTION_URL").unwrap_or_else(|_| "http://127.0.0.1:3003".to_owned());
    let client = HTTP_CLIENT.get_or_init(reqwest::Client::new);
    let manager_id: String = match client.post(format!("{execution_base}/internal/vwap/managers"))
        .json(&serde_json::json!({"parent_qty": request.quantity, "curve": curve.to_vec(), "expected": expected,
            "max_participation_rate": request.max_participation_rate,
            "min_trade_threshold": (request.quantity * 0.0001).max(1e-8),
            "bin_duration_secs": request.duration_minutes as f64 * 60.0 / NUM_VWAP_BINS as f64}))
        .send().await.and_then(|response| response.error_for_status().map_err(Into::into)) {
            Ok(response) => match response.json().await {
                Ok(id) => id,
                Err(error) => { let mut state=status.lock().await; state.status="failed".into(); state.error=Some(error.to_string()); return; }
            },
            Err(error) => { let mut state=status.lock().await; state.status="failed".into(); state.error=Some(error.to_string()); return; }
        };
    let (sender, mut receiver) = mpsc::channel(64);
    let trade_symbol = symbol.clone();
    let feed_handle = tokio::spawn(async move {
        loop {
            if let Err(error) = async {
                    let mut stream = crate::feed_client::stream(format!("trades:{trade_symbol}")).await?;
                    while let Some(item) = stream.next().await { sender.send(item).await.map_err(|_| anyhow::anyhow!("trade consumer stopped"))?; }
                    Ok::<(), anyhow::Error>(())
                }.await {
                eprintln!("VWAP trade feed for {trade_symbol} disconnected: {error:#}; retrying");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    });
    let started = tokio::time::Instant::now();
    let duration = Duration::from_secs(request.duration_minutes * 60);
    loop {
        if cancelled.load(Ordering::Relaxed) {
            break;
        }
        if started.elapsed() >= duration {
            let mut state = status.lock().await;
            if state.status == "running" {
                state.status = "completed".into();
            }
            break;
        }
        let Ok(Some(Ok(bytes))) = timeout(Duration::from_secs(1), receiver.recv()).await else {
            continue;
        };
        if bytes.len() < 97
            || u16::from_le_bytes([bytes[2], bytes[3]]) != 3
            || std::str::from_utf8(&bytes[8..40]).ok().map(str::trim) != Some(symbol.as_str())
        {
            continue;
        }
        let trade_price = f64::from_le_bytes(bytes[80..88].try_into().unwrap());
        let trade_qty = f64::from_le_bytes(bytes[88..96].try_into().unwrap());
        let elapsed = started.elapsed().as_secs_f64();
        let bin_duration = duration.as_secs_f64() / NUM_VWAP_BINS as f64;
        let target_bin = (elapsed / bin_duration).floor() as usize;
        if target_bin >= NUM_VWAP_BINS {
            continue;
        }
        let elapsed_in_bin = elapsed - target_bin as f64 * bin_duration;
        let market_result = match client.post(format!("{execution_base}/internal/vwap/managers/{manager_id}/market"))
            .json(&serde_json::json!({"trade_qty":trade_qty,"trade_price":trade_price,"elapsed_bin_secs":elapsed_in_bin,"target_bin":target_bin}))
            .send().await.and_then(|response| response.error_for_status().map_err(Into::into)) {
                Ok(response) => match response.json::<serde_json::Value>().await { Ok(value) => value, Err(error) => { eprintln!("Execution scheduler response failed: {error}"); continue; } },
                Err(error) => { eprintln!("Execution scheduler request failed: {error}"); continue; }
            };
        let scheduler_state = market_result.clone();
        if let Some(action) = market_result.get("action").filter(|action| !action.is_null()) {
            let child_qty = match action.get("action").and_then(|value| value.as_str()) {
                Some("new_order") => action.get("qty").and_then(|v|v.as_f64()).unwrap_or_default(),
                Some("modify_order") => action.get("new_qty").and_then(|v|v.as_f64()).unwrap_or_default(),
                _ => 0.0,
            };
            if child_qty > EPSILON {
                match execute_vwap_child(&store, &symbol, &request.side, child_qty).await {
                    Ok((filled, average_price)) => {
                        if let Err(error) = client.post(format!("{execution_base}/internal/vwap/managers/{manager_id}/fill"))
                            .json(&serde_json::json!({"fill_price":average_price.unwrap_or(trade_price),"filled_qty":filled})).send().await {
                            eprintln!("Execution fill report failed: {error}");
                        }
                    }
                    Err(error) => {
                        let mut state = status.lock().await;
                        state.error = Some(error);
                    }
                }
                let _ = client.post(format!("{execution_base}/internal/vwap/managers/{manager_id}/complete")).send().await;
                let scheduler_state: serde_json::Value = match client.get(format!("{execution_base}/internal/vwap/managers/{manager_id}")).send().await {
                    Ok(response) => response.json().await.unwrap_or(scheduler_state),
                    Err(_) => scheduler_state,
                };
                let mut state = status.lock().await;
                state.executed_quantity = scheduler_state.get("executed_qty").and_then(|v|v.as_f64()).unwrap_or_default();
                state.market_vwap = scheduler_state.get("market_vwap").and_then(|v|v.as_f64()).unwrap_or_default();
                state.execution_vwap = scheduler_state.get("execution_vwap").and_then(|v|v.as_f64()).unwrap_or_default();
                state.slippage = scheduler_state.get("slippage").and_then(|v|v.as_f64()).unwrap_or_default();
                if scheduler_state.get("remaining_qty").and_then(|v|v.as_f64()).unwrap_or_default() <= EPSILON {
                    state.status = "completed".into();
                }
                let finished = state.status == "completed";
                drop(state);
                if finished {
                    break;
                }
            }
        }
    }
    feed_handle.abort();
    let _ = client.delete(format!("{execution_base}/internal/vwap/managers/{manager_id}")).send().await;
}

async fn execute_vwap_child(
    store: &PositionStore,
    symbol: &str,
    side: &TradeSide,
    quantity: f64,
) -> Result<(f64, Option<f64>), String> {
    let book = store
        .books
        .lock()
        .await
        .get(symbol)
        .cloned()
        .ok_or_else(|| "Live order book unavailable for VWAP child order".to_string())?;
    let execution_base = std::env::var("EXECUTION_URL").unwrap_or_else(|_| "http://127.0.0.1:3003".to_owned());
    let matched: MatcherResponse = HTTP_CLIENT.get_or_init(reqwest::Client::new)
        .post(format!("{execution_base}/internal/orders/match"))
        .json(&serde_json::json!({"side": if matches!(side, TradeSide::Buy) { "buy" } else { "sell" }, "quantity": quantity, "limit_price": null, "book": book}))
        .send().await.map_err(|error| format!("Execution match request failed: {error}"))?
        .error_for_status().map_err(|error| format!("Execution match rejected VWAP child: {error}"))?
        .json().await.map_err(|error| format!("Invalid Execution match response: {error}"))?;
    let filled = matched.filled_quantity;
    if filled <= EPSILON { return Ok((0.0, None)); }
    let average = matched.average_execution_price.unwrap_or_default();
    let reference_price = matched.reference_price;
    let fills = matched.fills.into_iter().map(|fill| (fill.quantity, fill.price)).collect::<Vec<_>>();
    let fee_rate = configured_fee_rates().taker;
    let timestamp = now_ms().map_err(|error| error.to_string())?;
    let mut ledger = store.ledger.lock().await;
    let offset = ledger.trades.len();
    for (index, (qty, execution_price)) in fills.into_iter().enumerate() {
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{offset}"),
            symbol: symbol.to_owned(),
            side: side.clone(),
            quantity: qty,
            execution_price,
            reference_price,
            timestamp,
            fee: qty * execution_price * -fee_rate,
            fee_currency: symbol.split_once('-').map(|(_, quote)| quote.to_owned()).unwrap_or_default(),
            fee_rate,
            fee_liquidity: "taker".to_owned(),
            decision_timestamp: None, decision_price: None, decision_mid_price: None,
            slippage: None, slippage_bps: None,
        });
    }
    store
        .persist(&ledger)
        .await
        .map_err(|error| format!("Unable to persist VWAP fills: {error}"))?;
    Ok((filled, Some(average)))
}

async fn get_fee_rates() -> Json<FeeRatesResponse> {
    let rates = configured_fee_rates();
    Json(FeeRatesResponse { maker: rates.maker.abs(), taker: rates.taker.abs() })
}
async fn account_snapshot(store: &PositionStore, ledger: &Ledger) -> Result<AccountResponse, (StatusCode, String)> {
    let rates = get_usd_rates().await?.0.rates;
    let mut cash = ledger.initial_cash + ledger.topups.iter().map(|topup| topup.amount).sum::<f64>();
    let mut fees_paid = 0.0;
    let mut equity_available = true;
    for trade in &ledger.trades {
        let quote = trade.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
        if let Some(rate) = rates.get(quote) {
            let notional = trade.quantity * trade.execution_price * rate;
            cash += match trade.side { TradeSide::Buy => -notional, TradeSide::Sell => notional };
            let fee = trade.fee * rate;
            cash -= fee;
            fees_paid += fee;
        } else if trade.quantity * trade.execution_price > 0.0 {
            equity_available = false;
        }
    }
    let positions = summarize_positions(&ledger.trades).positions;
    let books = store.books.lock().await;
    let mut equity = cash;
    let mut realized_pnl = 0.0;
    let mut unrealized_pnl = 0.0;
    for position in &positions {
        let quote = position.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
        let Some(rate) = rates.get(quote) else {
            if position.quantity.abs() > EPSILON || position.realized_pnl.abs() > EPSILON {
                equity_available = false;
            }
            continue;
        };
        // Keep realized P&L from fully closed symbols; they remain in the
        // ledger summary with zero quantity, but still contribute to account P&L.
        realized_pnl += position.realized_pnl * rate;
        if position.quantity.abs() <= EPSILON { continue; }
        let Some(book) = books.get(&position.symbol) else { equity_available = false; continue; };
        let mark = if position.quantity > 0.0 { book.bids.first() } else { book.asks.first() };
        let Some(mark) = mark else { equity_available = false; continue; };
        let marked_value = position.quantity * mark.price * rate;
        equity += marked_value;
        let gross_unrealized = (mark.price - position.average_entry_price) * position.quantity;
        let close_fee = position.quantity.abs() * mark.price * position.estimated_exit_fee_rate;
        unrealized_pnl += (gross_unrealized - position.entry_fees - close_fee) * rate;
    }
    Ok(AccountResponse { currency: "USD", cash, equity, realized_pnl, unrealized_pnl, fees_paid, equity_available, initial_cash: ledger.initial_cash, topups: ledger.topups.clone(), max_leverage: configured_account_leverage(ledger) })
}
async fn ensure_paper_capacity(store: &PositionStore, ledger: &Ledger, symbol: &str, side: &TradeSide, quantity: f64, price: f64, fee_rate: f64) -> Result<(), (StatusCode, String)> {
    let rates = get_usd_rates().await?.0.rates;
    let quote = symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
    let quote_rate = *rates.get(quote).ok_or((StatusCode::SERVICE_UNAVAILABLE, format!("No OKX USD conversion rate for {quote}")))?;
    let account = account_snapshot(store, ledger).await?;
    if !account.equity_available { return Err((StatusCode::SERVICE_UNAVAILABLE, "Account equity cannot be valued until every open position has a live mark and USD conversion rate".into())); }
    let mut reserved_buying_power = 0.0;
    for barrier in ledger.barrier_orders.iter().filter(|order| order.status == "armed" && matches!(order.side, TradeSide::Buy)) {
        let barrier_quote = barrier.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
        let rate = *rates.get(barrier_quote).ok_or((StatusCode::SERVICE_UNAVAILABLE, format!("No OKX USD conversion rate for {barrier_quote}")))?;
        reserved_buying_power += barrier.remaining_quantity * barrier.trigger_price * rate * (1.0 - configured_fee_rates().taker);
    }
    for order in &ledger.orders {
        if matches!(order.side, TradeSide::Buy) {
            let order_quote = order.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            let rate = *rates.get(order_quote).ok_or((StatusCode::SERVICE_UNAVAILABLE, format!("No OKX USD conversion rate for {order_quote}")))?;
            reserved_buying_power += (order.remaining_quantity * order.limit_price + order.estimated_fee) * rate;
        }
    }
    let runs = store.vwap_runs.lock().await;
    let books = store.books.lock().await;
    let mut reserved_sell = ledger.barrier_orders.iter()
        .filter(|order| order.status == "armed" && order.symbol == symbol && matches!(order.side, TradeSide::Sell))
        .map(|order| order.remaining_quantity).sum::<f64>();
    for entry in runs.values() {
        let status = entry.status.lock().await;
        if status.status != "running" { continue; }
        let remaining = (status.quantity - status.executed_quantity).max(0.0);
        if matches!(status.side, TradeSide::Buy) {
            let run_quote = status.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            let rate = *rates.get(run_quote).ok_or((StatusCode::SERVICE_UNAVAILABLE, format!("No OKX USD conversion rate for {run_quote}")))?;
            let run_price = books.get(&status.symbol).and_then(|book| book.asks.first()).map(|level| level.price).ok_or((StatusCode::SERVICE_UNAVAILABLE, "Cannot reserve VWAP without a live ask price".into()))?;
            reserved_buying_power += remaining * run_price * rate * (1.0 - configured_fee_rates().taker);
        } else if status.symbol == symbol {
            reserved_sell += remaining;
        }
    }
    if matches!(side, TradeSide::Buy) {
        let required = quantity * price * quote_rate * (1.0 - fee_rate);
        let available = account.cash - reserved_buying_power;
        if required > available + EPSILON { return Err((StatusCode::CONFLICT, format!("Insufficient paper account buying power: need {:.2} USD, have {:.2} USD", required, available.max(0.0)))); }
    } else {
        let positions = summarize_positions(&ledger.trades).positions;
        let held = positions.iter().find(|position| position.symbol == symbol).map(|position| position.quantity.max(0.0)).unwrap_or(0.0);
        let reserved = ledger.orders.iter().filter(|order| order.symbol == symbol && matches!(order.side, TradeSide::Sell)).map(|order| order.remaining_quantity).sum::<f64>() + reserved_sell;
        let new_borrow = (quantity - (held - reserved).max(0.0)).max(0.0);
        let short_exposure = positions.iter().filter(|position| position.quantity < -EPSILON).map(|position| {
            let mark = books.get(&position.symbol).and_then(|book| book.asks.first()).map(|level| level.price).unwrap_or(position.average_entry_price);
            let quote = position.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            position.quantity.abs() * mark * rates.get(quote).copied().unwrap_or(0.0)
        }).sum::<f64>();
        let pending_short = ledger.orders.iter().enumerate().filter(|(index, order)| matches!(order.side, TradeSide::Sell) && !ledger.orders[..*index].iter().any(|prior| prior.symbol == order.symbol && matches!(prior.side, TradeSide::Sell))).map(|(_, order)| {
            let long = positions.iter().find(|position| position.symbol == order.symbol).map(|position| position.quantity.max(0.0)).unwrap_or(0.0);
            let pending = ledger.orders.iter().filter(|other| other.symbol == order.symbol && matches!(other.side, TradeSide::Sell)).map(|other| other.remaining_quantity).sum::<f64>();
            let quote = order.symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
            (pending - long).max(0.0) * order.limit_price * rates.get(quote).copied().unwrap_or(0.0)
        }).sum::<f64>();
        let exposure = short_exposure + pending_short + new_borrow * price * quote_rate;
        let limit = account.equity.max(0.0) * account.max_leverage;
        if exposure > limit + EPSILON { return Err((StatusCode::CONFLICT, format!("Paper spot margin limit exceeded: short exposure {:.2} USD exceeds {:.2} USD ({:.1}x equity)", exposure, limit, account.max_leverage))); }
    }
    Ok(())
}
async fn get_account(State(store): State<Arc<PositionStore>>) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    let ledger = store.ledger.lock().await.clone();
    Ok(Json(account_snapshot(&store, &ledger).await?))
}
async fn set_account_leverage(State(store): State<Arc<PositionStore>>, Json(request): Json<LeverageRequest>) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    if !request.max_leverage.is_finite() || !(1.0..=10.0).contains(&request.max_leverage) { return Err((StatusCode::BAD_REQUEST, "Maximum leverage must be between 1x and 10x".into())); }
    let mut ledger = store.ledger.lock().await;
    ledger.max_leverage = Some(request.max_leverage);
    store.persist(&ledger).await.map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to persist leverage setting: {error}")))?;
    Ok(Json(account_snapshot(&store, &ledger).await?))
}
async fn top_up_account(State(store): State<Arc<PositionStore>>, Json(request): Json<TopupRequest>) -> Result<Json<AccountResponse>, (StatusCode, String)> {
    if !request.amount.is_finite() || request.amount <= 0.0 { return Err((StatusCode::BAD_REQUEST, "Top-up amount must be positive".into())); }
    let mut ledger = store.ledger.lock().await;
    ledger.topups.push(AccountTopup { amount: request.amount, timestamp: now_ms().map_err(internal_error)? });
    store.persist(&ledger).await.map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to persist account top-up: {error}")))?;
    Ok(Json(account_snapshot(&store, &ledger).await?))
}async fn get_positions(State(store): State<Arc<PositionStore>>) -> Json<PositionsResponse> {
    let ledger = store.ledger.lock().await;
    Json(summarize_positions(&ledger.trades))
}

async fn get_hedge_candidates(
    State(store): State<Arc<PositionStore>>,
) -> Json<HedgeCandidatesResponse> {
    let positions = summarize_positions(&store.ledger.lock().await.trades)
        .positions
        .into_iter()
        .filter(|position| position.quantity.abs() > EPSILON)
        .collect::<Vec<_>>();
    for position in &positions {
        store.ensure_subscription(&position.symbol);
    }
    let histories = store.hedge_samples.lock().await.clone();
    let mut pair_correlations = Vec::new();
    for left_index in 0..positions.len() {
        for right_index in (left_index + 1)..positions.len() {
            let left = &positions[left_index];
            let right = &positions[right_index];
            let left_key = hedge_history_key(&left.symbol);
            let right_key = hedge_history_key(&right.symbol);
            let left_samples = histories.get(&left_key).map(Vec::as_slice).unwrap_or(&[]);
            let right_samples = histories.get(&right_key).map(Vec::as_slice).unwrap_or(&[]);
            let (correlation, _) = return_correlation(left_samples, right_samples);
            pair_correlations.push(HedgePairCorrelation {
                symbol_a: left.symbol.clone(),
                symbol_b: right.symbol.clone(),
                correlation,
            });
        }
    }
    let response = positions
        .into_iter()
        .map(|position| {
            let position_key = hedge_history_key(&position.symbol);
            let quote = position_key.split_once('-').map(|(_, quote)| quote);
            let position_samples = histories.get(&position_key).map(Vec::as_slice).unwrap_or(&[]);
            let mut candidates: Vec<HedgeCandidate> = store.spot_symbols
                .iter()
                .filter(|symbol| {
                    symbol.as_str() != position.symbol
                        && quote.is_some_and(|quote| symbol.split_once('-').is_some_and(|(_, candidate_quote)| candidate_quote == quote))
                })
                .map(|symbol| {
                    let samples = histories.get(symbol).map(Vec::as_slice).unwrap_or(&[]);
                    let (correlation, paired_samples) = return_correlation(position_samples, samples);
                    HedgeCandidate {
                        symbol: symbol.clone(),
                        correlation,
                        paired_samples,
                        latest_price: samples.last().map(|(_, price)| *price),
                    }
                })
                .collect();
            candidates.sort_by(|left: &HedgeCandidate, right: &HedgeCandidate| {
                match (left.correlation, right.correlation) {
                    (Some(left), Some(right)) => left.total_cmp(&right),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => right.paired_samples.cmp(&left.paired_samples),
                }
                .then_with(|| left.symbol.cmp(&right.symbol))
            });
            candidates.truncate(10);
            HedgePosition {
                symbol: position.symbol,
                quantity: position.quantity,
                average_entry_price: position.average_entry_price,
                latest_price: position_samples.last().map(|(_, price)| *price),
                candidates,
            }
        })
        .collect();
    Json(HedgeCandidatesResponse { positions: response, pair_correlations })
}

fn hedge_history_key(symbol: &str) -> String {
    let mut parts = symbol.split('-');
    match (parts.next(), parts.next()) {
        (Some(base), Some(quote)) => format!("{base}-{quote}"),
        _ => symbol.to_owned(),
    }
}
fn return_correlation(a: &[(u64, f64)], b: &[(u64, f64)]) -> (Option<f64>, usize) {
    const BUCKET_MS: u64 = 10_000;
    let bucket_prices = |samples: &[(u64, f64)]| {
        samples
            .iter()
            .filter(|(_, price)| price.is_finite() && *price > 0.0)
            .map(|(timestamp, price)| (timestamp / BUCKET_MS, *price))
            .collect::<HashMap<_, _>>()
    };
    let left = bucket_prices(a);
    let right = bucket_prices(b);
    let mut shared_buckets = left
        .keys()
        .copied()
        .filter(|bucket| right.contains_key(bucket))
        .collect::<Vec<_>>();
    shared_buckets.sort_unstable();
    let returns = shared_buckets
        .windows(2)
        .map(|buckets| {
            let previous = buckets[0];
            let current = buckets[1];
            (
                (left[&current] / left[&previous]).ln(),
                (right[&current] / right[&previous]).ln(),
            )
        })
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect::<Vec<_>>();
    let count = returns.len();
    if count < 10 {
        return (None, count);
    }
    let mean_a = returns.iter().map(|(x, _)| x).sum::<f64>() / count as f64;
    let mean_b = returns.iter().map(|(_, y)| y).sum::<f64>() / count as f64;
    let covariance = returns.iter().map(|(x, y)| (x - mean_a) * (y - mean_b)).sum::<f64>();
    let variance_a = returns.iter().map(|(x, _)| (x - mean_a).powi(2)).sum::<f64>();
    let variance_b = returns.iter().map(|(_, y)| (y - mean_b).powi(2)).sum::<f64>();
    let correlation = if variance_a > 0.0 && variance_b > 0.0 {
        Some((covariance / (variance_a * variance_b).sqrt()).clamp(-1.0, 1.0))
    } else {
        None
    };
    (correlation.filter(|value| value.is_finite()), count)
}
#[derive(Serialize)]
struct PositionInsightsInput {
    positions: Vec<Position>,
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Serialize)]
struct PositionMarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}
#[derive(Deserialize, Default)]
struct InsightsInputQuery { symbol: Option<String> }
async fn get_insights_inputs(State(store): State<Arc<PositionStore>>, Query(query): Query<InsightsInputQuery>) -> Json<PositionInsightsInput> {
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions
        .into_iter().filter(|position| position.quantity.abs() > EPSILON).collect::<Vec<_>>();
    let mut symbols = positions.iter().map(|position| position.symbol.clone()).collect::<std::collections::BTreeSet<_>>();
    if let Some(symbol) = query.symbol.map(|symbol| symbol.to_ascii_uppercase()).filter(|symbol| valid_position_symbol(symbol) && store.spot_symbols.iter().any(|spot| spot == symbol)) {
        symbols.insert(symbol);
    }
    for symbol in &symbols { store.ensure_subscription(symbol); }
    let books = store.books.lock().await;
    let histories = store.price_samples.lock().await;
    let markets = symbols.into_iter().map(|symbol| {
        let book = books.get(&symbol);
        (symbol.clone(), PositionMarketInput {
            bids: book.map(|snapshot| snapshot.bids.clone()).unwrap_or_default(),
            asks: book.map(|snapshot| snapshot.asks.clone()).unwrap_or_default(),
            price_samples: histories.get(&symbol).cloned().unwrap_or_default(),
        })
    }).collect();
    Json(PositionInsightsInput { positions, markets })
}

async fn get_position_trades(State(store): State<Arc<PositionStore>>) -> Json<TradesResponse> {
    Json(TradesResponse {
        trades: store.ledger.lock().await.trades.clone(),
    })
}
async fn get_orders(State(store): State<Arc<PositionStore>>) -> Json<OrdersResponse> {
    let ledger = store.ledger.lock().await;
    Json(OrdersResponse { orders: ledger.orders.clone(), barrier_orders: ledger.barrier_orders.clone() })
}
async fn get_barrier_orders(State(store): State<Arc<PositionStore>>) -> Json<Vec<BarrierOrder>> {
    Json(store.ledger.lock().await.barrier_orders.clone())
}

#[derive(Deserialize)]
struct ProfitBarrierRequest { symbol: String, target_profit_usd: f64 }

async fn create_profit_barrier(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<ProfitBarrierRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !request.target_profit_usd.is_finite() || request.target_profit_usd <= 0.0 {
        return Err((StatusCode::BAD_REQUEST, "Target profit must be a finite USD amount greater than zero".into()));
    }
    let rates = get_usd_rates().await?.0.rates;
    let quote = symbol.split_once('-').map(|(_, quote)| quote).unwrap_or_default();
    let rate = *rates.get(quote).ok_or((StatusCode::SERVICE_UNAVAILABLE, format!("No OKX USD conversion rate for {quote}")))?;
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions;
    let position = positions.into_iter().find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
    let quantity = position.quantity.abs();
    let fee_rate = position.estimated_exit_fee_rate.abs();
    let denominator = position.quantity - quantity * fee_rate;
    if !denominator.is_finite() || denominator.abs() <= EPSILON {
        return Err((StatusCode::BAD_REQUEST, "Cannot calculate a profit trigger for this position and fee rate".into()));
    }
    let trigger_price = (request.target_profit_usd / rate + position.average_entry_price * position.quantity + position.entry_fees) / denominator;
    if !trigger_price.is_finite() || trigger_price <= 0.0 {
        return Err((StatusCode::BAD_REQUEST, "Target profit produces an invalid trigger price".into()));
    }
    let side = if position.quantity > 0.0 { TradeSide::Sell } else { TradeSide::Buy };
    let trigger_direction = if position.quantity > 0.0 { BarrierDirection::Above } else { BarrierDirection::Below };
    store.ensure_subscription(&symbol);
    let timestamp = now_ms().map_err(internal_error)?;
    let mut ledger = store.ledger.lock().await;
    ensure_paper_capacity(&store, &ledger, &symbol, &side, quantity, trigger_price, configured_fee_rates().taker).await?;
    let barrier_order = BarrierOrder {
        id: format!("barrier-{timestamp}-{}", ledger.barrier_orders.len()), symbol, side,
        quantity, remaining_quantity: quantity, filled_quantity: 0.0, trigger_price, trigger_direction,
        created_at: timestamp, triggered_at: None, status: "armed".to_owned(),
        average_execution_price: None, fee_paid: 0.0, error: None, order_kind: "profit-target".to_owned(), execution_type: "market".to_owned(), limit_price: None,
        trailing_amount: None, trailing_percent: None, extreme_price: None,
    };
    ledger.barrier_orders.push(barrier_order.clone());
    if let Err(error) = store.persist(&ledger).await {
        ledger.barrier_orders.pop();
        return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to persist profit barrier: {error}")));
    }
    Ok(Json(OrderResponse { estimated_fee: 0.0, filled_quantity: 0.0, remaining_quantity: quantity, average_execution_price: None, order: None, barrier_order: Some(barrier_order) }))
}

#[derive(Deserialize)]
struct CloseProfitableRequest { symbol: String }

async fn close_profitable(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<CloseProfitableRequest>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions;
    let position = positions.into_iter().find(|position| position.symbol == symbol && position.quantity.abs() > EPSILON)
        .ok_or((StatusCode::NOT_FOUND, "Open position not found".into()))?;
    store.ensure_subscription(&symbol);
    let side = if position.quantity > 0.0 { TradeSide::Sell } else { TradeSide::Buy };
    let quantity = position.quantity.abs();
    submit_order(
        State(store),
        Json(NewOrder {
            symbol: symbol.clone(),
            side: side.clone(),
            order_type: OrderType::Ioc,
            quantity,
            limit_price: None,
            measure_execution_quality: true,
            trigger_price: None,
            trigger_direction: None,
            trailing_amount: None,
            trailing_percent: None,
            expires_at: None,
            time_in_force: None,
        }),
    ).await.map_err(|(status, error)| (
        status,
        format!("Close IOC {symbol} {} {quantity} failed: {error}", match side { TradeSide::Buy => "buy", TradeSide::Sell => "sell" }),
    ))
}

async fn submit_order(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<NewOrder>,
) -> Result<Json<OrderResponse>, (StatusCode, String)> {
    let symbol = request.symbol.to_ascii_uppercase();
    if !valid_position_symbol(&symbol) {
        return Err((StatusCode::BAD_REQUEST, "Invalid instrument symbol".into()));
    }
    if !request.quantity.is_finite() || request.quantity <= 0.0 {
        return Err((
            StatusCode::BAD_REQUEST,
            "Quantity must be finite and greater than zero".into(),
        ));
    }
    if matches!(&request.order_type, OrderType::Barrier | OrderType::StopMarket | OrderType::StopLimit | OrderType::TrailingStop) {
        store.ensure_subscription(&symbol);
        let trailing = matches!(&request.order_type, OrderType::TrailingStop);
        if trailing && store.books.lock().await.get(&symbol).is_none() {
            let wait = async { loop { let notified = store.book_changed.notified(); if store.books.lock().await.contains_key(&symbol) { break; } notified.await; } };
            timeout(Duration::from_secs(8), wait).await.map_err(|_| (StatusCode::SERVICE_UNAVAILABLE, "Waiting for live book to arm trailing stop timed out".into()))?;
        }
        let (trigger_price, trigger_direction, trailing_amount, trailing_percent, extreme_price) = if trailing {
            let amount = request.trailing_amount.filter(|value| value.is_finite() && *value > 0.0);
            let percent = request.trailing_percent.filter(|value| value.is_finite() && *value > 0.0 && *value < 1.0);
            if amount.is_some() == percent.is_some() { return Err((StatusCode::BAD_REQUEST, "Trailing stop needs exactly one positive amount or percentage below 100%".into())); }
            let book = store.books.lock().await.get(&symbol).cloned().ok_or((StatusCode::SERVICE_UNAVAILABLE, "Live book required to arm trailing stop".into()))?;
            let current = match request.side { TradeSide::Sell => book.bids.first().map(|l| l.price), TradeSide::Buy => book.asks.first().map(|l| l.price) }.ok_or((StatusCode::SERVICE_UNAVAILABLE, "Live trigger reference price unavailable".into()))?;
            let distance = amount.unwrap_or(current * percent.unwrap());
            let initial = if matches!(request.side, TradeSide::Sell) { current - distance } else { current + distance };
            if !initial.is_finite() || initial <= 0.0 { return Err((StatusCode::BAD_REQUEST, "Trailing distance produces an invalid trigger price".into())); }
            (initial, if matches!(request.side, TradeSide::Sell) { BarrierDirection::Below } else { BarrierDirection::Above }, amount, percent, Some(current))
        } else {
            (request.trigger_price.filter(|price| price.is_finite() && *price > 0.0).ok_or((StatusCode::BAD_REQUEST, "Trigger price must be finite and greater than zero".to_owned()))?, request.trigger_direction.clone().ok_or((StatusCode::BAD_REQUEST, "Trigger direction must be above or below".to_owned()))?, None, None, None)
        };
        let execution_type = if matches!(&request.order_type, OrderType::StopLimit) { "limit" } else { "market" };
        let stop_limit_price = if execution_type == "limit" { Some(request.limit_price.filter(|p| p.is_finite() && *p > 0.0).ok_or((StatusCode::BAD_REQUEST, "Stop-limit requires a positive limit price".into()))?) } else { None };
        let timestamp = now_ms().map_err(internal_error)?;
        let mut ledger = store.ledger.lock().await;
        ensure_paper_capacity(&store, &ledger, &symbol, &request.side, request.quantity, stop_limit_price.unwrap_or(trigger_price), configured_fee_rates().taker).await?;
        let barrier_order = BarrierOrder {
            id: format!("barrier-{timestamp}-{}", ledger.barrier_orders.len()),
            symbol,
            side: request.side,
            quantity: request.quantity,
            remaining_quantity: request.quantity,
            filled_quantity: 0.0,
            trigger_price,
            trigger_direction,
            created_at: timestamp,
            triggered_at: None,
            status: "armed".to_owned(),
            average_execution_price: None,
            fee_paid: 0.0,
            error: None,
            order_kind: if trailing { "trailing-stop" } else if execution_type == "limit" { "stop-limit" } else if matches!(&request.order_type, OrderType::StopMarket) { "stop-market" } else { "barrier" }.to_owned(),
            execution_type: execution_type.to_owned(), limit_price: stop_limit_price,
            trailing_amount, trailing_percent, extreme_price,
        };
        ledger.barrier_orders.push(barrier_order.clone());
        if let Err(error) = store.persist(&ledger).await {
            ledger.barrier_orders.pop();
            return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("Unable to persist barrier order: {error}")));
        }
        return Ok(Json(OrderResponse {
            estimated_fee: 0.0,
            filled_quantity: 0.0,
            remaining_quantity: request.quantity,
            average_execution_price: None,
            order: None,
            barrier_order: Some(barrier_order),
        }));
    }
    let fee_rates = configured_fee_rates();
    let maker_fee_rate = fee_rates.maker;
    let taker_fee_rate = fee_rates.taker;
    let is_post_only = matches!(request.order_type, OrderType::PostOnlyLimit);
    let is_limit = matches!(request.order_type, OrderType::Limit | OrderType::PostOnlyLimit);
    let is_fok = matches!(request.order_type, OrderType::Fok);
    let limit_price = if is_limit {
        let p = request.limit_price.unwrap_or(f64::NAN);
        if !p.is_finite() || p <= 0.0 {
            return Err((
                StatusCode::BAD_REQUEST,
                "Limit price must be finite and greater than zero".into(),
            ));
        }
        Some(p)
    } else {
        None
    };
    if request.expires_at.is_some_and(|expires| expires <= now_ms().unwrap_or_default()) {
        return Err((StatusCode::BAD_REQUEST, "Good-till-date must be in the future".into()));
    }
    if request.expires_at.is_some() && !is_limit { return Err((StatusCode::BAD_REQUEST, "Good-till-date is supported only for limit orders".into())); }
    if request.time_in_force.as_deref().is_some_and(|tif| tif != "gtd" && tif != "gtc") { return Err((StatusCode::BAD_REQUEST, "Supported time-in-force values are gtc and gtd".into())); }
    if request.time_in_force.as_deref() == Some("gtd") && request.expires_at.is_none() { return Err((StatusCode::BAD_REQUEST, "GTD limit order requires expires_at".into())); }
    if request.time_in_force.as_deref() == Some("gtc") && request.expires_at.is_some() { return Err((StatusCode::BAD_REQUEST, "GTC order cannot include expires_at".into())); }
    store.ensure_subscription(&symbol);
    if store.books.lock().await.get(&symbol).is_none() {
        let wait = async {
            loop {
                let notified = store.book_changed.notified();
                if store.books.lock().await.contains_key(&symbol) {
                    break;
                }
                notified.await;
            }
        };
        timeout(Duration::from_secs(8), wait).await.map_err(|_| {
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "Waiting for live OKX book timed out".into(),
            )
        })?;
    }
    let book = store.books.lock().await.get(&symbol).cloned().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Live OKX book unavailable".into(),
    ))?;
    let levels = match request.side { TradeSide::Buy => &book.asks, TradeSide::Sell => &book.bids };
    if is_post_only {
        let crosses = match (request.side.clone(), limit_price, book.bids.first(), book.asks.first()) {
            (TradeSide::Buy, Some(price), _, Some(ask)) => price >= ask.price,
            (TradeSide::Sell, Some(price), Some(bid), _) => price <= bid.price,
            _ => false,
        };
        if crosses { return Err((StatusCode::CONFLICT, "Post-only limit would take liquidity; order was not placed".into())); }
    }
    let decision_timestamp = if request.measure_execution_quality { Some(now_ms().map_err(internal_error)?) } else { None };
    let decision_price = if request.measure_execution_quality {
        levels.first().map(|level| level.price).filter(|price| price.is_finite() && *price > 0.0)
    } else { None };
    let decision_mid_price = if request.measure_execution_quality {
        book.bids.first().zip(book.asks.first()).map(|(bid, ask)| (bid.price + ask.price) / 2.0)
            .filter(|price| price.is_finite() && *price > 0.0)
    } else { None };
    let execution_base = std::env::var("EXECUTION_URL").unwrap_or_else(|_| "http://127.0.0.1:3003".to_owned());
    let matcher_response = HTTP_CLIENT.get_or_init(reqwest::Client::new)
        .post(format!("{execution_base}/internal/orders/match"))
        .json(&serde_json::json!({"side": if matches!(request.side, TradeSide::Buy) { "buy" } else { "sell" },
            "quantity": request.quantity, "limit_price": limit_price, "book": book}))
        .send().await.map_err(|error| (StatusCode::BAD_GATEWAY, format!("Execution service request failed: {error}")))?;
    if !matcher_response.status().is_success() {
        let status = matcher_response.status();
        let detail = matcher_response.text().await.unwrap_or_else(|error| format!("could not read response body: {error}"));
        return Err((StatusCode::BAD_GATEWAY, format!("Execution service rejected order match (HTTP {status}): {}", detail.trim())));
    }
    let matched: MatcherResponse = matcher_response.json().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid Execution match response: {error}")))?;
    if is_fok && matched.remaining_quantity > EPSILON {
        return Err((StatusCode::CONFLICT, format!("FOK order cancelled: only {} of {} was available", matched.filled_quantity, request.quantity)));
    }
    let reference_price = matched.reference_price;
    let remaining = matched.remaining_quantity;
    let filled_quantity = matched.filled_quantity;
    let average_execution_price = matched.average_execution_price;
    let fills = matched.fills.into_iter().map(|fill| (fill.quantity, fill.price)).collect::<Vec<_>>();
    let timestamp = now_ms().map_err(internal_error)?;
    let mut ledger = store.ledger.lock().await;
    let reserve_price = if matches!(request.side, TradeSide::Buy) { levels.iter().map(|level| level.price).fold(0.0_f64, f64::max) } else { reference_price.unwrap_or(0.0) };
    ensure_paper_capacity(&store, &ledger, &symbol, &request.side, request.quantity, reserve_price, taker_fee_rate).await?;
    let start = ledger.trades.len();
    for (index, (quantity, execution_price)) in fills.into_iter().enumerate() {
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{}", start),
            symbol: symbol.clone(),
            side: request.side.clone(),
            quantity,
            execution_price,
            reference_price,
            timestamp,
            fee: quantity * execution_price * -taker_fee_rate,
            fee_currency: symbol.split_once('-').map(|(_, quote)| quote.to_owned()).unwrap_or_default(),
            fee_rate: taker_fee_rate,
            fee_liquidity: "taker".to_owned(),
            decision_timestamp,
            decision_price,
            decision_mid_price,
            slippage: decision_price.map(|price| if matches!(request.side, TradeSide::Sell) { execution_price - price } else { price - execution_price }),
            slippage_bps: decision_price.filter(|price| *price > 0.0).map(|price| {
                let favorable_difference = if matches!(request.side, TradeSide::Sell) { execution_price - price } else { price - execution_price };
                favorable_difference / price * 10_000.0
            }),
        });
    }
    let order = if is_limit && remaining > EPSILON {
        let order = RestingOrder {
            id: format!("order-{timestamp}-{}", ledger.orders.len()),
            symbol,
            side: request.side,
            quantity: request.quantity,
            remaining_quantity: remaining,
            limit_price: limit_price.unwrap(),
            timestamp,
            maker_fee_rate,
            estimated_fee: remaining * limit_price.unwrap() * -maker_fee_rate,
            expires_at: request.expires_at,
            time_in_force: if request.expires_at.is_some() { "gtd".to_owned() } else { "gtc".to_owned() },
            post_only: is_post_only,
        };
        ledger.orders.push(order.clone());
        Some(order)
    } else {
        None
    };
    if let Err(error) = store.persist(&ledger).await {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist paper order: {error}"),
        ));
    }
    Ok(Json(OrderResponse {
        filled_quantity,
        estimated_fee: ledger.trades[start..].iter().map(|trade| trade.fee).sum::<f64>(),
        remaining_quantity: remaining,
        average_execution_price,
        order,
        barrier_order: None,
    }))
}

async fn cancel_order(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut ledger = store.ledger.lock().await;
    let original_orders = ledger.orders.len();
    let original_barriers = ledger.barrier_orders.len();
    ledger.orders.retain(|order| order.id != id);
    if ledger.orders.len() == original_orders {
        if let Some(order) = ledger.barrier_orders.iter().find(|order| order.id == id) {
            if order.status != "armed" { return Err((StatusCode::CONFLICT, "Barrier order has already triggered and cannot be cancelled".into())); }
        }
        ledger.barrier_orders.retain(|order| order.id != id);
        if ledger.barrier_orders.len() == original_barriers {
            return Err((StatusCode::NOT_FOUND, "Order not found".into()));
        }
    }
    store.persist(&ledger).await.map_err(|error| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Unable to persist order cancellation: {error}"),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

fn decode_book(bytes: &Bytes) -> Option<BookSnapshot> {
    if bytes.len() < 272 {
        return None;
    }
    let symbol = std::str::from_utf8(&bytes[8..40]).ok()?.trim();
    if !valid_position_symbol(symbol) {
        return None;
    }
    let mut bids = Vec::new();
    let mut asks = Vec::new();
    for index in 0..BOOK_DEPTH {
        let bid_offset = 48 + index * 16;
        let ask_offset = 160 + index * 16;
        let bid = read_level(bytes, bid_offset)?;
        let ask = read_level(bytes, ask_offset)?;
        if bid.size > 0.0 {
            bids.push(bid);
        }
        if ask.size > 0.0 {
            asks.push(ask);
        }
    }
    Some(BookSnapshot { bids, asks, updated_at_ms: now_ms().ok()? })
}
fn read_level(bytes: &[u8], offset: usize) -> Option<BookLevel> {
    let price = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    let size = f64::from_le_bytes(bytes.get(offset + 8..offset + 16)?.try_into().ok()?);
    (price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0)
        .then_some(BookLevel { price, size })
}
fn now_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64)
}
fn internal_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}
fn valid_position_symbol(symbol: &str) -> bool {
    symbol.len() <= 32
        && symbol.is_ascii()
        && symbol.split_once('-').is_some_and(|(base, quote)| {
            !base.is_empty()
                && !quote.is_empty()
                && base.bytes().all(|b| b.is_ascii_alphanumeric())
                && quote.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn summarize_positions(trades: &[PositionTrade]) -> PositionsResponse {
    let mut positions = BTreeMap::<String, Position>::new();
    for trade in trades {
        let position = positions.entry(trade.symbol.clone()).or_insert(Position {
            symbol: trade.symbol.clone(),
            quantity: 0.0,
            average_entry_price: 0.0,
            realized_pnl: 0.0,
            entry_fees: 0.0,
            fees_paid: 0.0,
            estimated_exit_fee_rate: 0.0,
            last_trade_timestamp: trade.timestamp,
        });
        apply_trade(position, trade);
        position.last_trade_timestamp = trade.timestamp;
    }
    PositionsResponse {
        positions: positions.into_values().collect(),
        trade_count: trades.len(),
    }
}
fn apply_trade(position: &mut Position, trade: &PositionTrade) {
    let old = position.quantity;
    position.fees_paid += trade.fee;
    let incoming = match trade.side { TradeSide::Buy => trade.quantity, TradeSide::Sell => -trade.quantity };
    position.estimated_exit_fee_rate = if trade.fee_rate == 0.0 { configured_fee_rates().taker.abs() } else { trade.fee_rate.abs() };
    if old == 0.0 || old.signum() == incoming.signum() {
        let total = old.abs() + incoming.abs();
        position.average_entry_price = (old.abs() * position.average_entry_price + incoming.abs() * trade.execution_price) / total;
        position.quantity = old + incoming;
        position.entry_fees += trade.fee;
        return;
    }
    let closed = old.abs().min(incoming.abs());
    let entry_fee_closed = position.entry_fees * (closed / old.abs());
    let closing_fee = trade.fee * (closed / incoming.abs());
    let gross = closed * if old > 0.0 { trade.execution_price - position.average_entry_price } else { position.average_entry_price - trade.execution_price };
    position.realized_pnl += gross - entry_fee_closed - closing_fee;
    let net = old + incoming;
    if net.abs() < EPSILON {
        position.quantity = 0.0;
        position.average_entry_price = 0.0;
        position.entry_fees = 0.0;
    } else if net.signum() != old.signum() {
        position.average_entry_price = trade.execution_price;
        position.entry_fees = (trade.fee - closing_fee).max(0.0);
        position.quantity = net;
    } else {
        position.entry_fees -= entry_fee_closed;
        position.quantity = net;
    }
}







