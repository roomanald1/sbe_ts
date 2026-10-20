mod logging;
mod daily_summary;
mod feed_client;
use axum::{Json, Router, http::StatusCode, routing::get};
use serde::{Deserialize, Serialize};
use std::{collections::{BTreeMap, HashMap}, sync::{Arc, OnceLock}, time::{SystemTime, UNIX_EPOCH}};
use tokio::sync::{Mutex, Notify};
use futures_util::StreamExt;
use insights::signals::{ExternalMarketFeatures, SignalEngine, SignalSnapshot, SignalStore, StoredSignalRecord};


#[derive(Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SignalFeedEvent {
    Price { timestamp: u64, price: f64, volatility_1m: f64 },
    Trade { timestamp: u64, price: f64, size: f64, is_buy: bool },
    Book { timestamp: u64, best_bid: f64, best_ask: f64, bid_sizes: Vec<f64>, ask_sizes: Vec<f64>, #[serde(default)] bid_levels: Vec<(f64, f64)>, #[serde(default)] ask_levels: Vec<(f64, f64)> },
}
#[derive(Deserialize)]
struct SignalEventRequest { symbol: String, #[serde(flatten)] event: SignalFeedEvent }
struct SignalRuntime { store: SignalStore, engines: Mutex<HashMap<String, SignalEngine>>, signal_push_cache: Arc<Mutex<BTreeMap<String, serde_json::Value>>>, signal_push_notify: Arc<Notify>, ui_snapshot_cache: Arc<Mutex<BTreeMap<String, serde_json::Value>>>, ui_snapshot_notify: Arc<Notify> }

static POSITION_MANAGEMENT_CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

fn json_number(value: &serde_json::Value, field: &str) -> Option<f64> {
    value.get(field)?.as_str()?.parse::<f64>().ok().filter(|number| number.is_finite())
}

async fn okx_first(client: &reqwest::Client, path: &str, query: &[(&str, &str)]) -> Option<serde_json::Value> {
    let query = query.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("&");
    let response = client.get(format!("https://www.okx.com{path}?{query}")).send().await.ok()?.error_for_status().ok()?.json::<serde_json::Value>().await.ok()?;
    if response.get("code")?.as_str()? != "0" { return None; }
    response.get("data")?.as_array()?.first().cloned()
}

async fn okx_data(client: &reqwest::Client, path: &str, query: &[(&str, &str)]) -> Option<Vec<serde_json::Value>> {
    let query = query.iter().map(|(key, value)| format!("{key}={value}")).collect::<Vec<_>>().join("&");
    let response = client.get(format!("https://www.okx.com{path}?{query}")).send().await.ok()?.error_for_status().ok()?.json::<serde_json::Value>().await.ok()?;
    if response.get("code")?.as_str()? != "0" { return None; }
    response.get("data")?.as_array().cloned()
}

fn option_volatility_summary(rows: &[serde_json::Value], now: u64) -> (Option<f64>, Option<f64>, Option<u64>) {
    let days_now = 86_400_000.0;
    let options = rows.iter().filter_map(|row| {
        let expiry = row.get("expTime")?.as_str()?.parse::<u64>().ok()?;
        let days = expiry.saturating_sub(now) as f64 / days_now;
        if !(7.0..=60.0).contains(&days) { return None; }
        Some((row.get("optType")?.as_str()?.to_owned(), json_number(row, "delta")?, json_number(row, "markVol")?, expiry, days))
    }).collect::<Vec<_>>();
    let select = |kind: &str, target_delta: f64| options.iter().filter(|(option_type, _, _, _, _)| option_type == kind)
        .min_by(|a, b| {
            let score_a = (a.4 - 30.0).abs() + (a.1.abs() - target_delta).abs() * 30.0;
            let score_b = (b.4 - 30.0).abs() + (b.1.abs() - target_delta).abs() * 30.0;
            score_a.total_cmp(&score_b)
        });
    let call_atm = select("C", 0.5);
    let put_atm = select("P", 0.5);
    let iv = call_atm.zip(put_atm).map(|(call, put)| (call.2 + put.2) / 2.0).or_else(|| call_atm.map(|row| row.2)).or_else(|| put_atm.map(|row| row.2));
    let call_25d = select("C", 0.25);
    let put_25d = select("P", 0.25);
    let skew = put_25d.zip(call_25d).map(|(put, call)| put.2 - call.2);
    let expiry = call_atm.map(|row| row.3).or_else(|| put_atm.map(|row| row.3));
    (iv, skew, expiry)
}

async fn poll_external_market_features(store: SignalStore) {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(8)).build().unwrap_or_else(|_| reqwest::Client::new());
    let mut previous_open_interest = HashMap::<String, f64>::new();
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        for underlying in ["BTC", "ETH"] {
            let swap = format!("{underlying}-USDT-SWAP");
            let spot_index = format!("{underlying}-USDT");
            let option_underlying = format!("{underlying}-USD");
            let funding_q = [("instId", swap.as_str())];
            let oi_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let mark_q = [("instType", "SWAP"), ("instId", swap.as_str())];
            let index_q = [("instId", spot_index.as_str())];
            let options_q = [("uly", option_underlying.as_str())];
            let (funding, oi, mark, index, options) = tokio::join!(
                okx_first(&client, "/api/v5/public/funding-rate", &funding_q),
                okx_first(&client, "/api/v5/public/open-interest", &oi_q),
                okx_first(&client, "/api/v5/public/mark-price", &mark_q),
                okx_first(&client, "/api/v5/market/index-tickers", &index_q),
                okx_data(&client, "/api/v5/public/opt-summary", &options_q),
            );
            let timestamp = now_ms();
            let open_interest_contracts = oi.as_ref().and_then(|row| json_number(row, "oi"));
            let open_interest_change_percent = open_interest_contracts.and_then(|current| {
                let change = previous_open_interest.insert(underlying.to_owned(), current).map(|previous| (current / previous - 1.0) * 100.0);
                change.filter(|value| value.is_finite())
            });
            let mark_price = mark.as_ref().and_then(|row| json_number(row, "markPx"));
            let index_price = index.as_ref().and_then(|row| json_number(row, "idxPx"));
            let option_summary = options.as_deref().map(|rows| option_volatility_summary(rows, timestamp)).unwrap_or((None, None, None));
            let features = ExternalMarketFeatures {
                underlying: underlying.to_owned(), timestamp, swap_instrument: swap,
                funding_rate: funding.as_ref().and_then(|row| json_number(row, "fundingRate")),
                next_funding_rate: funding.as_ref().and_then(|row| json_number(row, "nextFundingRate")),
                open_interest_contracts,
                open_interest_currency: oi.as_ref().and_then(|row| json_number(row, "oiCcy")),
                open_interest_change_percent,
                mark_price, index_price,
                perp_basis_bps: mark_price.zip(index_price).filter(|(_, index)| *index > 0.0).map(|(mark, index)| (mark / index - 1.0) * 10_000.0),
                options_atm_iv: option_summary.0, options_25d_skew: option_summary.1, options_expiry_timestamp: option_summary.2,
            };
            let has_data = features.funding_rate.is_some() || features.open_interest_contracts.is_some() || features.mark_price.is_some() || features.options_atm_iv.is_some() || features.options_25d_skew.is_some();
            if has_data { if let Err(error) = store.append_external_market_features(&features).await { crate::service_log!("ERROR", "Failed to store {underlying} derivatives/options context: {error}"); } }
        }
    }
}


async fn run_feed_signal_consumer(runtime: Arc<SignalRuntime>) {
    loop {
        match feed_client::signal_events().await {
            Ok(mut events) => {
                crate::service_log!("INFO", "Connected to FeedHandlers RSocket signal-event stream");
                while let Some(event) = events.next().await {
                    match event.and_then(|bytes| serde_json::from_slice::<SignalEventRequest>(&bytes).map_err(anyhow::Error::from)) {
                        Ok(request) => {
                            if let Err((status, error)) = ingest_signal_event(axum::extract::State(Arc::clone(&runtime)), Json(request)).await {
                                crate::service_log!("WARN", "Signal event rejected with {status}: {error}");
                            }
                        }
                        Err(error) => crate::service_log!("WARN", "Invalid FeedHandlers signal event: {error}"),
                    }
                }
                crate::service_log!("WARN", "FeedHandlers RSocket signal-event stream disconnected");
            }
            Err(error) => crate::service_log!("WARN", "Could not connect to FeedHandlers signal-event stream: {error:#}"),
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

async fn run_ui_snapshot_push_worker(
    pending_snapshots: Arc<Mutex<BTreeMap<String, serde_json::Value>>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base = std::env::var("FEED_HANDLERS_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3001".to_owned());
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let snapshots = {
            let mut pending = pending_snapshots.lock().await;
            if pending.is_empty() { None } else { Some(std::mem::take(&mut *pending)) }
        };
        let Some(snapshots) = snapshots else {
            notified.await;
            continue;
        };
        for (_, snapshot) in snapshots {
            if let Err(error) = client.post(format!("{base}/internal/signals/snapshot"))
                .timeout(std::time::Duration::from_millis(250))
                .json(&snapshot).send().await.and_then(reqwest::Response::error_for_status) {
                crate::service_log!("WARN", "Signal snapshot fanout to FeedHandlers failed: {error}");
            }
        }
    }
}

async fn run_signal_push_worker(
    pending_signals: Arc<Mutex<BTreeMap<String, serde_json::Value>>>,
    notify: Arc<Notify>,
) {
    let client = POSITION_MANAGEMENT_CLIENT.get_or_init(reqwest::Client::new);
    let base = std::env::var("POSITION_MANAGEMENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3002".to_owned());
    let mut retry_delay = std::time::Duration::from_millis(250);
    let mut endpoint_available = true;
    loop {
        let notified = notify.notified();
        tokio::pin!(notified);
        let dashboards = {
            let mut pending = pending_signals.lock().await;
            if pending.is_empty() { None } else { Some(std::mem::take(&mut *pending)) }
        };
        let Some(dashboards) = dashboards else {
            notified.await;
            continue;
        };
        let mut failed = BTreeMap::new();
        for (symbol, dashboard) in dashboards {
            let result = client.post(format!("{base}/internal/auto-trader/signal"))
                .timeout(std::time::Duration::from_secs(2))
                .json(&dashboard).send().await.and_then(reqwest::Response::error_for_status);
            if let Err(error) = result {
                if endpoint_available {
                    crate::service_log!("WARN", "Auto-trader signal delivery unavailable; retaining latest signal per symbol and retrying: {error}");
                    endpoint_available = false;
                }
                failed.insert(symbol, dashboard);
            }
        }
        if !failed.is_empty() {
            let mut pending = pending_signals.lock().await;
            for (symbol, dashboard) in failed {
                pending.entry(symbol).or_insert(dashboard);
            }
            drop(pending);
            tokio::time::sleep(retry_delay).await;
            retry_delay = (retry_delay * 2).min(std::time::Duration::from_secs(5));
        } else {
            if !endpoint_available {
                crate::service_log!("INFO", "Auto-trader signal delivery recovered");
            }
            endpoint_available = true;
            retry_delay = std::time::Duration::from_millis(250);
        }
    }
}

async fn ingest_signal_event(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
    Json(request): Json<SignalEventRequest>,
) -> Result<Json<Option<SignalSnapshot>>, (StatusCode, String)> {
    if request.symbol.is_empty() || !request.symbol.is_ascii() || request.symbol.len() > 32 {
        return Err((StatusCode::BAD_REQUEST, "Invalid signal symbol".to_owned()));
    }
    let (snapshot, records, market) = {
        let mut engines = runtime.engines.lock().await;
        let event_timestamp = match &request.event {
            SignalFeedEvent::Price { timestamp, .. }
            | SignalFeedEvent::Trade { timestamp, .. }
            | SignalFeedEvent::Book { timestamp, .. } => *timestamp,
        };
        let (btc_context, eth_context) = (
            engines.get("BTC-USDT").map_or((None, None), |reference| reference.return_context(event_timestamp)),
            engines.get("ETH-USDT").map_or((None, None), |reference| reference.return_context(event_timestamp)),
        );
        let engine = engines.entry(request.symbol.clone()).or_default();
        let mut records = Vec::new();
        let mut market = None;
        let snapshot = match request.event {
            SignalFeedEvent::Price { timestamp, price, volatility_1m } => {
                for outcome in engine.on_price(timestamp, price, volatility_1m) {
                    records.push(StoredSignalRecord::Outcome(outcome.legacy));
                    records.push(StoredSignalRecord::Evaluation(outcome.evaluation));
                }
                let snapshot = engine.snapshot(&request.symbol, timestamp);
                if let Some(snapshot) = &snapshot {
                    // The pushed decision needs the latest available market cache on
                    // every price event, regardless of the research sampling cadence.
                    if let Some(book_snapshot) = engine.book_snapshot(&request.symbol, snapshot.timestamp) {
                        let levels = |levels: Vec<(f64, f64)>| levels.into_iter()
                            .map(|(price, size)| serde_json::json!({"price": price, "size": size}))
                            .collect::<Vec<_>>();
                        market = Some(serde_json::json!({
                            request.symbol.clone(): {
                                "bids": levels(book_snapshot.bid_levels.clone()),
                                "asks": levels(book_snapshot.ask_levels.clone()),
                                "updated_at_ms": book_snapshot.timestamp
                            }
                        }));
                        if engine.research_sample_due(snapshot.timestamp) {
                            let features = engine.research_features(&request.symbol, snapshot.timestamp, btc_context, eth_context);
                            records.push(StoredSignalRecord::BookSnapshot(book_snapshot));
                            records.push(StoredSignalRecord::ResearchFeatures(features));
                            engine.track(snapshot);
                            records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
                        }
                    } else if engine.research_sample_due(snapshot.timestamp) {
                        // Preserve feature and snapshot samples when a fresh book is
                        // unavailable; depth data is optional in the research schema.
                        let features = engine.research_features(&request.symbol, snapshot.timestamp, btc_context, eth_context);
                        records.push(StoredSignalRecord::ResearchFeatures(features));
                        engine.track(snapshot);
                        records.push(StoredSignalRecord::Snapshot(snapshot.clone()));
                    }
                }
                snapshot
            }
            SignalFeedEvent::Trade { timestamp, price, size, is_buy } => {
                engine.on_trade(timestamp, price, size, is_buy);
                None
            }
            SignalFeedEvent::Book { timestamp, best_bid, best_ask, bid_sizes, ask_sizes, bid_levels, ask_levels } => {
                engine.on_book(timestamp, best_bid, best_ask, &bid_sizes, &ask_sizes, &bid_levels, &ask_levels);
                None
            }
        };
        (snapshot, records, market)
    };
    if !records.is_empty() {
        let store = runtime.store.clone();
        tokio::spawn(async move {
            if let Err(error) = store.append_records(records).await {
                crate::service_log!("ERROR", "Could not persist signal event off hot path: {error}");
            }
        });
    }
    if let Some(snapshot) = snapshot.as_ref() {
        let markets = market.unwrap_or_else(|| serde_json::json!({}));
        let dashboard = serde_json::json!({
            "best": [snapshot],
            "walkForward": runtime.store.cached_walk_forward(),
            "markets": markets
        });
        runtime.signal_push_cache.lock().await.insert(snapshot.symbol.clone(), dashboard);
        runtime.signal_push_notify.notify_one();
        let snapshot_json = serde_json::to_value(snapshot).unwrap_or(serde_json::Value::Null);
        runtime.ui_snapshot_cache.lock().await.insert(snapshot.symbol.clone(), snapshot_json);
        runtime.ui_snapshot_notify.notify_one();
    }
    Ok(Json(snapshot))
}

#[derive(Clone, Deserialize)]
struct Position {
    symbol: String,
    quantity: f64,
    average_entry_price: f64,
    entry_fees: f64,
    estimated_exit_fee_rate: f64,
}
#[derive(Clone, Deserialize)]
struct BookLevel { price: f64, size: f64 }
#[derive(Deserialize)]
struct PositionMarketInput {
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
    price_samples: Vec<(u64, f64)>,
}
#[derive(Deserialize)]
struct PositionInsightsInput {
    positions: Vec<Position>,
    markets: HashMap<String, PositionMarketInput>,
}
#[derive(Serialize)]
#[serde(rename_all = "lowercase")]
enum TradeSide { Buy, Sell }
#[derive(Serialize)]
struct CloseInsight {
    symbol: String,
    close_side: TradeSide,
    close_quantity: f64,
    close_price: Option<f64>,
    current_price: Option<f64>,
    breakeven_price: Option<f64>,
    close_pnl: Option<f64>,
    close_fee: Option<f64>,
    depth_covers_position: bool,
    close_depth_quantity: f64,
    profitable_close_probability_5m: Option<f64>,
    probability_samples: usize,
}
#[derive(Serialize)]
struct CloseInsightsResponse { insights: Vec<CloseInsight> }

async fn get_position_insights() -> Result<Json<CloseInsightsResponse>, (StatusCode, String)> {
    let base = std::env::var("POSITION_MANAGEMENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3002".to_owned());
    let source: PositionInsightsInput = POSITION_MANAGEMENT_CLIENT
        .get_or_init(reqwest::Client::new)
        .get(format!("{base}/internal/insights-inputs"))
        .send().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Position Management request failed: {error}")))?
        .error_for_status()
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Position Management returned an error: {error}")))?
        .json().await
        .map_err(|error| (StatusCode::BAD_GATEWAY, format!("Invalid position insight input: {error}")))?;
    let insights = source.positions.into_iter().filter(|p| p.quantity.abs() > 1e-12).map(|position| {
        let market = source.markets.get(&position.symbol);
        let closing_long = position.quantity > 0.0;
        let close_side = if closing_long { TradeSide::Sell } else { TradeSide::Buy };
        let levels = market.map(|m| if closing_long { &m.bids } else { &m.asks }).cloned().unwrap_or_default();
        let mut remaining = position.quantity.abs();
        let mut notional: f64 = 0.0;
        for level in &levels {
            let qty = remaining.min(level.size);
            notional += qty * level.price;
            remaining -= qty;
            if remaining <= 1e-12 { break; }
        }
        let close_depth_quantity = (position.quantity.abs() - remaining).max(0.0);
        let depth_covers_position = remaining <= 1e-12;
        let current_price = market.and_then(|m| {
            let bid = m.bids.first()?.price;
            let ask = m.asks.first()?.price;
            let mid = (bid + ask) / 2.0;
            (bid.is_finite() && ask.is_finite() && bid > 0.0 && ask >= bid && mid.is_finite()).then_some(mid)
        });
        let close_price = if depth_covers_position && position.quantity.abs() > 1e-12 {
            let average = notional / position.quantity.abs();
            (notional.is_finite() && notional > 0.0 && average.is_finite() && average > 0.0).then_some(average)
        } else { None };
        let breakeven_price = {
            let quantity = position.quantity.abs();
            let fee_rate = position.estimated_exit_fee_rate;
            let (numerator, denominator) = if closing_long {
                (position.average_entry_price * quantity + position.entry_fees, quantity * (1.0 - fee_rate))
            } else {
                (position.average_entry_price * quantity - position.entry_fees, quantity * (1.0 + fee_rate))
            };
            (numerator.is_finite() && denominator.is_finite() && numerator > 0.0 && denominator > 0.0)
                .then_some(numerator / denominator).filter(|price| price.is_finite() && *price > 0.0)
        };
        let close_fee = close_price.map(|price| position.quantity.abs() * price * position.estimated_exit_fee_rate);
        let close_pnl = close_price.map(|price| (price - position.average_entry_price) * position.quantity
            - position.entry_fees - position.quantity.abs() * price * position.estimated_exit_fee_rate)
            .filter(|pnl| pnl.is_finite());
        let samples = market.map(|m| m.price_samples.as_slice()).unwrap_or(&[]);
        let probability = if samples.len() >= 31 && close_price.is_some() {
            let returns: Vec<f64> = samples.windows(2).filter_map(|pair| {
                let dt = pair[1].0.saturating_sub(pair[0].0) as f64;
                (dt > 0.0 && pair[0].1 > 0.0 && pair[1].1 > 0.0)
                    .then_some((pair[1].1 / pair[0].1).ln() / (dt / 10_000.0))
            }).collect();
            if returns.len() >= 30 {
                let mean = returns.iter().sum::<f64>() / returns.len() as f64;
                let variance = returns.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / (returns.len() - 1) as f64;
                let projected_mean = close_price.unwrap().ln() + mean * 30.0;
                let projected_sd = (variance * 30.0).sqrt();
                if projected_sd > 0.0 {
                    let below = normal_cdf((position.average_entry_price.ln() - projected_mean) / projected_sd);
                    Some(if closing_long { 1.0 - below } else { below })
                } else { None }
            } else { None }
        } else { None };
        CloseInsight { symbol: position.symbol, close_side, close_quantity: position.quantity.abs(), close_price,
            current_price, breakeven_price, close_pnl, close_fee, depth_covers_position, close_depth_quantity,
            profitable_close_probability_5m: probability, probability_samples: samples.len() }
    }).collect();
    Ok(Json(CloseInsightsResponse { insights }))
}

fn normal_cdf(value: f64) -> f64 {
    let sign = if value < 0.0 { -1.0 } else { 1.0 };
    let x = value.abs() / std::f64::consts::SQRT_2;
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let erf = 1.0 - (((((1.061405429 * t - 1.453152027) * t + 1.421413741) * t - 0.284496736) * t + 0.254829592) * t) * (-x * x).exp();
    0.5 * (1.0 + sign * erf)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let signal_store = SignalStore::from_environment()?;
    signal_store.migrate_to_binary().await?;
    daily_summary::spawn_nightly();
    tokio::spawn(poll_external_market_features(signal_store.clone()));
    let warm_store = signal_store.clone();
    tokio::spawn(async move {
        if let Err(error) = warm_store.walk_forward().await {
            crate::service_log!("WARN", "Initial walk-forward cache warm failed: {error}");
        }
    });
    let signal_push_cache = Arc::new(Mutex::new(BTreeMap::new()));
    let signal_push_notify = Arc::new(Notify::new());
    tokio::spawn(run_signal_push_worker(Arc::clone(&signal_push_cache), Arc::clone(&signal_push_notify)));
    let ui_snapshot_cache = Arc::new(Mutex::new(BTreeMap::new()));
    let ui_snapshot_notify = Arc::new(Notify::new());
    tokio::spawn(run_ui_snapshot_push_worker(Arc::clone(&ui_snapshot_cache), Arc::clone(&ui_snapshot_notify)));
    let signal_runtime = Arc::new(SignalRuntime { store: signal_store.clone(), engines: Mutex::new(HashMap::new()), signal_push_cache, signal_push_notify, ui_snapshot_cache, ui_snapshot_notify });
    tokio::spawn(run_feed_signal_consumer(Arc::clone(&signal_runtime)));
    let app = Router::new()
        .merge(daily_summary::router())
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/positions/insights", get(get_position_insights))
        .route("/internal/signals/event", axum::routing::post(ingest_signal_event).with_state(signal_runtime))
        .merge(signal_store.router());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3004)).await?;
    crate::service_log!("INFO", "Insights service listening on http://0.0.0.0:3004");
    axum::serve(listener, app).await?;
    Ok(())
}
async fn process_cpu_seconds() -> String {
    let stat = std::fs::read_to_string("/proc/self/stat").unwrap_or_default();
    let fields = stat.rsplit_once(')').map(|(_, fields)| fields).unwrap_or("");
    let values: Vec<u64> = fields.split_whitespace().filter_map(|value| value.parse().ok()).collect();
    values.get(11).zip(values.get(12)).map(|(user, system)| (*user + *system) as f64 / 100.0).unwrap_or(0.0).to_string()
}
