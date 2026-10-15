use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use bytes::Bytes;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::{Arc, Mutex as StdMutex},
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
        .route("/positions/trades", get(get_position_trades))
        .route("/positions/orders", get(get_orders).post(submit_order))
        .route(
            "/positions/orders/{id}",
            axum::routing::delete(cancel_order),
        )
        .with_state(store))
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
    book_changed: Notify,
    subscriptions: StdMutex<HashMap<String, mpsc::Sender<anyhow::Result<Bytes>>>>,
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
            book_changed: Notify::new(),
            subscriptions: StdMutex::new(HashMap::new()),
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
                            store.books.lock().await.insert(symbol.clone(), book);
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

async fn get_positions(State(store): State<Arc<PositionStore>>) -> Json<PositionsResponse> {
    let ledger = store.ledger.lock().await;
    Json(summarize_positions(&ledger.trades))
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
        remaining_quantity: if order.is_some() { remaining } else { 0.0 },
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
