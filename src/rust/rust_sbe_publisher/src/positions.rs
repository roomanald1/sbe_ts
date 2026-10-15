use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::{Mutex, Notify, mpsc},
    time::{Duration, timeout},
};

const BOOK_DEPTH: usize = 7;
const EPSILON: f64 = 1e-12;

pub async fn router() -> anyhow::Result<Router> {
    let store = Arc::new(PositionStore::load().await?);
    let symbols = store
        .ledger
        .lock()
        .await
        .orders
        .iter()
        .map(|order| order.symbol.clone())
        .collect::<std::collections::HashSet<_>>();
    for symbol in symbols {
        store.ensure_subscription(&symbol);
    }
    Ok(Router::new()
        .route("/positions", get(get_positions))
        .route("/positions/insights", get(get_close_insights))
        .route("/positions/trades", get(get_position_trades))
        .route("/positions/vwap", get(get_vwap_runs).post(start_vwap))
        .route("/positions/vwap/{id}", axum::routing::delete(cancel_vwap))
        .route("/positions/orders", get(get_orders).post(submit_order))
        .route("/positions/close-profitable", post(close_profitable))
        .route(
            "/positions/orders/{id}",
            axum::routing::delete(cancel_order),
        )
        .with_state(store))
}

#[derive(Deserialize)]
struct NewVwapRun {
    symbol: String,
    side: TradeSide,
    quantity: f64,
    duration_minutes: u64,
    max_participation_rate: f64,
}
#[derive(Clone, Serialize)]
struct VwapRunStatus {
    id: String,
    symbol: String,
    side: TradeSide,
    quantity: f64,
    executed_quantity: f64,
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
    timestamp: u64,
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
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum OrderType {
    Ioc,
    Limit,
}

#[derive(Deserialize)]
struct NewOrder {
    symbol: String,
    side: TradeSide,
    order_type: OrderType,
    quantity: f64,
    limit_price: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    realized_pnl: f64,
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
struct OrdersResponse {
    orders: Vec<RestingOrder>,
}
#[derive(Serialize)]
struct OrderResponse {
    filled_quantity: f64,
    remaining_quantity: f64,
    average_execution_price: Option<f64>,
    order: Option<RestingOrder>,
}
#[derive(Clone, Serialize)]
struct BookLevel {
    price: f64,
    size: f64,
}
#[derive(Clone)]
struct BookSnapshot {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
}

#[derive(Default, Serialize, Deserialize)]
struct Ledger {
    trades: Vec<PositionTrade>,
    #[serde(default)]
    orders: Vec<RestingOrder>,
}
struct PositionStore {
    path: PathBuf,
    ledger: Mutex<Ledger>,
    books: Mutex<HashMap<String, BookSnapshot>>,
    price_samples: Mutex<HashMap<String, Vec<(u64, f64)>>>,
    book_changed: Notify,
    subscriptions: StdMutex<HashMap<String, mpsc::Sender<anyhow::Result<Bytes>>>>,
    vwap_runs: Mutex<HashMap<String, VwapRunEntry>>,
}

impl PositionStore {
    async fn load() -> anyhow::Result<Self> {
        let path = std::env::var_os("POSITIONS_FILE")
            .map(PathBuf::from)
            .unwrap_or(std::env::current_dir()?.join("positions.json"));
        let ledger = match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let value: serde_json::Value = serde_json::from_slice(&bytes)?;
                if value.is_array() {
                    Ledger {
                        trades: serde_json::from_value(value)?,
                        orders: Vec::new(),
                    }
                } else {
                    serde_json::from_value(value)?
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ledger::default(),
            Err(error) => return Err(error.into()),
        };
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
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
            vwap_runs: Mutex::new(HashMap::new()),
        })
    }
    async fn persist(&self, ledger: &Ledger) -> anyhow::Result<()> {
        let bytes = serde_json::to_vec_pretty(ledger)?;
        if let Some(parent) = self.path.parent().filter(|p| !p.as_os_str().is_empty()) {
            tokio::fs::create_dir_all(parent).await?;
        }
        let temp = self.path.with_extension("positions.tmp");
        tokio::fs::write(&temp, bytes).await?;
        tokio::fs::rename(temp, &self.path).await?;
        Ok(())
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
                if let Err(error) = crate::okx::connectivity::subscribe_book_once(
                    &subscription_symbol,
                    sender.clone(),
                )
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
        let book = self.books.lock().await.get(symbol).cloned();
        let Some(book) = book else {
            return Ok(());
        };
        let mut ledger = self.ledger.lock().await;
        let mut fills = Vec::new();
        for order in &mut ledger.orders {
            if order.symbol != symbol {
                continue;
            }
            let levels = match order.side {
                TradeSide::Buy => &book.asks,
                TradeSide::Sell => &book.bids,
            };
            let mut remaining = order.remaining_quantity;
            for level in levels {
                let crosses = match order.side {
                    TradeSide::Buy => level.price <= order.limit_price,
                    TradeSide::Sell => level.price >= order.limit_price,
                };
                if !crosses || remaining <= EPSILON {
                    continue;
                }
                let quantity = remaining.min(level.size);
                if quantity > EPSILON {
                    fills.push((order.side.clone(), quantity, level.price));
                    remaining -= quantity;
                }
            }
            order.remaining_quantity = remaining.max(0.0);
        }
        ledger
            .orders
            .retain(|order| order.remaining_quantity > EPSILON);
        if !fills.is_empty() {
            let timestamp = now_ms()?;
            let trade_offset = ledger.trades.len();
            for (index, (side, quantity, execution_price)) in fills.into_iter().enumerate() {
                ledger.trades.push(PositionTrade {
                    id: format!("{timestamp}-{index}-{}", trade_offset),
                    symbol: symbol.to_owned(),
                    side,
                    quantity,
                    execution_price,
                    timestamp,
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

async fn start_vwap(
    State(store): State<Arc<PositionStore>>,
    Json(request): Json<NewVwapRun>,
) -> Result<Json<VwapRunStatus>, (StatusCode, String)> {
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
    if !request.max_participation_rate.is_finite()
        || !(0.01..=1.0).contains(&request.max_participation_rate)
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Participation rate must be between 1% and 100%".into(),
        ));
    }
    let timestamp = now_ms().map_err(internal_error)?;
    let id = format!("vwap-{timestamp}-{}", symbol.to_ascii_lowercase());
    let status = Arc::new(Mutex::new(VwapRunStatus {
        id: id.clone(),
        symbol: symbol.clone(),
        side: request.side.clone(),
        quantity: request.quantity,
        executed_quantity: 0.0,
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
    let initial_status = status.lock().await.clone();
    tokio::spawn(run_vwap(
        Arc::clone(&store),
        request,
        symbol,
        Arc::clone(&status),
        cancelled,
    ));
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

async fn run_vwap(
    store: Arc<PositionStore>,
    request: NewVwapRun,
    symbol: String,
    status: Arc<Mutex<VwapRunStatus>>,
    cancelled: Arc<AtomicBool>,
) {
    use crate::vwap::{NUM_BINS, OrderAction, OrderManager, VwapConfig};
    let mut curve = [0.0; NUM_BINS];
    let mut total_weight = 0.0;
    for (index, weight) in curve.iter_mut().enumerate() {
        let x = (index as f64 / (NUM_BINS - 1) as f64) * 2.0 - 1.0;
        *weight = 1.0 + 3.0 * x * x;
        total_weight += *weight;
    }
    for weight in &mut curve {
        *weight /= total_weight;
    }
    let expected = std::array::from_fn(|index| {
        request.quantity * curve[index] / request.max_participation_rate
    });
    let mut manager = match OrderManager::new(
        VwapConfig {
            parent_qty: request.quantity,
            historical_vwap_curve: curve,
            max_participation_rate: request.max_participation_rate,
        },
        expected,
        (request.quantity * 0.0001).max(1e-8),
        request.duration_minutes as f64 * 60.0 / NUM_BINS as f64,
    ) {
        Ok(manager) => manager,
        Err(error) => {
            let mut state = status.lock().await;
            state.status = "failed".into();
            state.error = Some(error.into());
            return;
        }
    };
    let (sender, mut receiver) = mpsc::channel(64);
    let trade_symbol = symbol.clone();
    let feed_handle = tokio::spawn(async move {
        loop {
            if let Err(error) =
                crate::okx::connectivity::subscribe_trades_once(&trade_symbol, sender.clone()).await
            {
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
        let bin_duration = duration.as_secs_f64() / NUM_BINS as f64;
        let target_bin = (elapsed / bin_duration).floor() as usize;
        while manager.state().current_bin < target_bin.min(NUM_BINS) {
            manager.on_bin_transition();
        }
        if target_bin >= NUM_BINS {
            continue;
        }
        let elapsed_in_bin = elapsed - target_bin as f64 * bin_duration;
        if let Some(action) = manager.on_market_trade(trade_qty, trade_price, elapsed_in_bin) {
            let child_qty = match action {
                OrderAction::NewOrder { qty } => qty,
                OrderAction::ModifyOrder { new_qty } => new_qty,
                OrderAction::CancelOrder => 0.0,
            };
            if child_qty > EPSILON {
                match execute_vwap_child(&store, &symbol, &request.side, child_qty).await {
                    Ok((filled, average_price)) => {
                        manager.on_execution_fill(average_price.unwrap_or(trade_price), filled)
                    }
                    Err(error) => {
                        let mut state = status.lock().await;
                        state.error = Some(error);
                    }
                }
                manager.on_order_complete();
                let benchmark = manager.benchmark();
                let mut state = status.lock().await;
                state.executed_quantity = manager.state().executed_qty;
                state.market_vwap = benchmark.market_vwap();
                state.execution_vwap = benchmark.execution_vwap();
                state.slippage = benchmark.slippage();
                if manager.state().remaining_qty <= EPSILON {
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
    let levels = match side {
        TradeSide::Buy => &book.asks,
        TradeSide::Sell => &book.bids,
    };
    let mut remaining = quantity;
    let mut fills = Vec::new();
    for level in levels {
        if remaining <= EPSILON {
            break;
        }
        let fill = remaining.min(level.size);
        if fill > EPSILON {
            fills.push((fill, level.price));
            remaining -= fill;
        }
    }
    let filled = quantity - remaining;
    if filled <= EPSILON {
        return Ok((0.0, None));
    }
    let average = fills.iter().map(|(qty, price)| qty * price).sum::<f64>() / filled;
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
            timestamp,
        });
    }
    store
        .persist(&ledger)
        .await
        .map_err(|error| format!("Unable to persist VWAP fills: {error}"))?;
    Ok((filled, Some(average)))
}

async fn get_positions(State(store): State<Arc<PositionStore>>) -> Json<PositionsResponse> {
    let ledger = store.ledger.lock().await;
    Json(summarize_positions(&ledger.trades))
}
#[derive(Serialize)]
struct CloseInsight {
    symbol: String,
    close_side: TradeSide,
    close_quantity: f64,
    close_price: Option<f64>,
    close_pnl: Option<f64>,
    depth_covers_position: bool,
    profitable_close_probability_5m: Option<f64>,
    probability_samples: usize,
}
#[derive(Serialize)]
struct CloseInsightsResponse { insights: Vec<CloseInsight> }

async fn get_close_insights(State(store): State<Arc<PositionStore>>) -> Json<CloseInsightsResponse> {
    let positions = summarize_positions(&store.ledger.lock().await.trades).positions;
    for position in positions.iter().filter(|p| p.quantity.abs() > EPSILON) {
        store.ensure_subscription(&position.symbol);
    }
    let books = store.books.lock().await;
    let histories = store.price_samples.lock().await;
    let insights = positions.into_iter().filter(|p| p.quantity.abs() > EPSILON).map(|position| {
        let closing_long = position.quantity > 0.0;
        let close_side = if closing_long { TradeSide::Sell } else { TradeSide::Buy };
        let mut remaining = position.quantity.abs();
        let mut notional = 0.0;
        if let Some(book) = books.get(&position.symbol) {
            let levels = if closing_long { &book.bids } else { &book.asks };
            for level in levels {
                let qty = remaining.min(level.size);
                notional += qty * level.price;
                remaining -= qty;
                if remaining <= EPSILON { break; }
            }
        }
        let depth_covers_position = remaining <= EPSILON;
        let close_price = if depth_covers_position && position.quantity.abs() > EPSILON { Some(notional / position.quantity.abs()) } else { None };
        let close_pnl = close_price.map(|price| (price - position.average_entry_price) * position.quantity);
        let samples = histories.get(&position.symbol).map(Vec::as_slice).unwrap_or(&[]);
        let probability = if samples.len() >= 31 && close_price.is_some() {
            let returns: Vec<f64> = samples.windows(2).filter_map(|pair| {
                let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
                (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0).then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
            }).collect();
            if returns.len() >= 30 {
                let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                let variance = returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
                let projected_mean = close_price.unwrap().ln() + mean * 30.0;
                let projected_sd = (variance * 30.0).sqrt();
                if projected_sd > 0.0 {
                    let z = (position.average_entry_price.ln() - projected_mean) / projected_sd;
                    let below = normal_cdf(z);
                    Some(if closing_long { 1.0 - below } else { below })
                } else { None }
            } else { None }
        } else { None };
        CloseInsight { symbol: position.symbol, close_side, close_quantity: position.quantity.abs(), close_price, close_pnl, depth_covers_position, profitable_close_probability_5m: probability, probability_samples: samples.len() }
    }).collect();
    Json(CloseInsightsResponse { insights })
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0 - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t) * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}

async fn get_position_trades(State(store): State<Arc<PositionStore>>) -> Json<TradesResponse> {
    Json(TradesResponse {
        trades: store.ledger.lock().await.trades.clone(),
    })
}
async fn get_orders(State(store): State<Arc<PositionStore>>) -> Json<OrdersResponse> {
    Json(OrdersResponse {
        orders: store.ledger.lock().await.orders.clone(),
    })
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
    let book = store.books.lock().await.get(&symbol).cloned()
        .ok_or((StatusCode::SERVICE_UNAVAILABLE, "Live order book unavailable".into()))?;
    let levels = if position.quantity > 0.0 { &book.bids } else { &book.asks };
    let mut remaining = position.quantity.abs();
    let mut value = 0.0;
    for level in levels {
        let size = remaining.min(level.size);
        value += size * level.price;
        remaining -= size;
        if remaining <= EPSILON { break; }
    }
    if remaining > EPSILON {
        return Err((StatusCode::CONFLICT, "Visible book depth cannot cover the full position".into()));
    }
    let estimated_pnl = (value / position.quantity.abs() - position.average_entry_price) * position.quantity;
    if estimated_pnl <= 0.0 {
        return Err((StatusCode::CONFLICT, "Position is no longer estimated profitable at the live book".into()));
    }
    submit_order(
        State(store),
        Json(NewOrder {
            symbol,
            side: if position.quantity > 0.0 { TradeSide::Sell } else { TradeSide::Buy },
            order_type: OrderType::Ioc,
            quantity: position.quantity.abs(),
            limit_price: None,
        }),
    ).await
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
    let is_limit = matches!(request.order_type, OrderType::Limit);
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
    let limit = limit_price.unwrap_or(match request.side {
        TradeSide::Buy => f64::INFINITY,
        TradeSide::Sell => 0.0,
    });
    let levels = match request.side {
        TradeSide::Buy => &book.asks,
        TradeSide::Sell => &book.bids,
    };
    let mut remaining = request.quantity;
    let mut fills = Vec::new();
    for level in levels {
        let crosses = match request.side {
            TradeSide::Buy => level.price <= limit,
            TradeSide::Sell => level.price >= limit,
        };
        if !crosses || remaining <= EPSILON {
            continue;
        }
        let quantity = remaining.min(level.size);
        if quantity > EPSILON {
            fills.push((quantity, level.price));
            remaining -= quantity;
        }
    }
    let filled_quantity = request.quantity - remaining;
    let average_execution_price = if filled_quantity > EPSILON {
        Some(fills.iter().map(|(q, p)| q * p).sum::<f64>() / filled_quantity)
    } else {
        None
    };
    let timestamp = now_ms().map_err(internal_error)?;
    let mut ledger = store.ledger.lock().await;
    let start = ledger.trades.len();
    for (index, (quantity, execution_price)) in fills.into_iter().enumerate() {
        ledger.trades.push(PositionTrade {
            id: format!("{timestamp}-{index}-{}", start),
            symbol: symbol.clone(),
            side: request.side.clone(),
            quantity,
            execution_price,
            timestamp,
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
        remaining_quantity: remaining,
        average_execution_price,
        order,
    }))
}

async fn cancel_order(
    State(store): State<Arc<PositionStore>>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, String)> {
    let mut ledger = store.ledger.lock().await;
    let original = ledger.orders.len();
    ledger.orders.retain(|order| order.id != id);
    if ledger.orders.len() == original {
        return Err((StatusCode::NOT_FOUND, "Resting order not found".into()));
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
    Some(BookSnapshot { bids, asks })
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
    let incoming = match trade.side {
        TradeSide::Buy => trade.quantity,
        TradeSide::Sell => -trade.quantity,
    };
    if old == 0.0 || old.signum() == incoming.signum() {
        let total = old.abs() + incoming.abs();
        position.average_entry_price = (old.abs() * position.average_entry_price
            + incoming.abs() * trade.execution_price)
            / total;
        position.quantity = old + incoming;
        return;
    }
    let closed = old.abs().min(incoming.abs());
    position.realized_pnl += closed
        * if old > 0.0 {
            trade.execution_price - position.average_entry_price
        } else {
            position.average_entry_price - trade.execution_price
        };
    let net = old + incoming;
    if net.abs() < EPSILON {
        position.quantity = 0.0;
        position.average_entry_price = 0.0;
    } else {
        if net.signum() != old.signum() {
            position.average_entry_price = trade.execution_price;
        }
        position.quantity = net;
    }
}
