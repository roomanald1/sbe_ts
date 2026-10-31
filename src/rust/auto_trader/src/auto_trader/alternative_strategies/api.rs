use super::{engine, types::*};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio::sync::Mutex;

#[derive(Clone)]
struct PaperState {
    book: Arc<Mutex<Book>>,
    path: PathBuf,
    enabled: Arc<AtomicBool>,
    load_error: Option<String>,
}
#[derive(Serialize)]
struct Snapshot {
    mode: &'static str,
    auto_trader_enabled: bool,
    load_error: Option<String>,
    book: Book,
}
type Error = (StatusCode, String);
fn bad(e: String) -> Error {
    (StatusCode::BAD_REQUEST, e)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
async fn persist(s: &PaperState, b: &Book) -> Result<(), Error> {
    if let Some(error) = &s.load_error {
        return Err((StatusCode::SERVICE_UNAVAILABLE, error.clone()));
    }
    let bytes =
        serde_json::to_vec(b).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let tmp = s.path.with_extension("alternatives.tmp");
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    // Linux atomic rename; failed writes leave the in-memory book untouched.
    tokio::fs::rename(&tmp, &s.path)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

fn valid_book(book: &Book) -> bool {
    book.sleeves.len() == IDS.len()
        && IDS.iter().all(|id| {
            book.sleeves.get(*id).is_some_and(|s| {
                s.id == *id
                    && engine::validate_config(&s.config).is_ok()
                    && [
                        s.initial_capital_usd,
                        s.cash_usd,
                        s.equity_usd,
                        s.peak_equity_usd,
                        s.realized_pnl_usd,
                        s.fees_usd,
                        s.daily_pnl_usd,
                    ]
                    .into_iter()
                    .all(f64::is_finite)
                    && s.initial_capital_usd == 1000.0
                    && s.peak_equity_usd > 0.0
            })
        })
}

pub fn router(parent: &super::super::AutoTraderState) -> Router {
    let path = parent.settings_path.with_extension("alternatives.json");
    let (book, load_error) = match std::fs::read(&path) {
        Ok(bytes) => match serde_json::from_slice::<Book>(&bytes) {
            Ok(book) if valid_book(&book) => (book, None),
            Ok(_) => (
                Book::initialized(),
                Some("Invalid alternative paper account state; refusing to overwrite".into()),
            ),
            Err(e) => (
                Book::initialized(),
                Some(format!(
                    "Paper account cannot load {}; refusing to overwrite: {e}",
                    path.display()
                )),
            ),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Book::initialized(), None),
        Err(e) => (
            Book::initialized(),
            Some(format!("Paper account read failed: {e}")),
        ),
    };
    let state = PaperState {
        book: Arc::new(Mutex::new(book)),
        path,
        enabled: parent.enabled.clone(),
        load_error,
    };
    Router::new()
        .route("/positions/auto-trader/alternatives", get(snapshot))
        .route(
            "/positions/auto-trader/alternatives/{strategy}",
            axum::routing::put(configure),
        )
        .route(
            "/internal/auto-trader/alternative-observation",
            post(observe),
        )
        .route("/internal/auto-trader/alternative-insights", post(insights))
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024))
        .with_state(state)
}
async fn snapshot(State(s): State<PaperState>) -> Json<Snapshot> {
    Json(Snapshot {
        mode: "independent-paper-accounts",
        auto_trader_enabled: s.enabled.load(Ordering::Relaxed),
        load_error: s.load_error.clone(),
        book: s.book.lock().await.clone(),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Update {
    enabled: bool,
    max_trade_usd: f64,
    daily_loss_limit_usd: f64,
    spot_fee_bps: f64,
    short_borrow_apr: f64,
    min_edge_bps: f64,
}
async fn configure(
    State(s): State<PaperState>,
    Path(id): Path<String>,
    Json(x): Json<Update>,
) -> Result<Json<Book>, Error> {
    let c = Config {
        enabled: x.enabled,
        max_trade_usd: x.max_trade_usd,
        daily_loss_limit_usd: x.daily_loss_limit_usd,
        spot_fee_bps: x.spot_fee_bps,
        short_borrow_apr: x.short_borrow_apr,
        min_edge_bps: x.min_edge_bps,
    };
    engine::validate_config(&c).map_err(bad)?;
    let mut guard = s.book.lock().await;
    let mut book = guard.clone();
    let sleeve = book
        .sleeves
        .get_mut(&id)
        .ok_or((StatusCode::NOT_FOUND, "Unknown strategy".into()))?;
    if c.enabled && !sleeve.config.enabled && !sleeve.feed.current(now()) {
        return Err(bad(
            "Current native Insights feed required before enabling".into()
        ));
    }
    sleeve.config = c;
    sleeve.status = if sleeve.config.enabled {
        "Enabled; waiting for current observations"
    } else {
        "Disabled; held positions close on the next active observation"
    }
    .into();
    persist(&s, &book).await?;
    *guard = book.clone();
    Ok(Json(book))
}
async fn observe(
    State(s): State<PaperState>,
    Json(o): Json<Observation>,
) -> Result<Json<Book>, Error> {
    let current = now();
    if o.observed_at_ms > current || current - o.observed_at_ms > 60_000 {
        return Err(bad("Observation must be no more than 60 seconds old".into()));
    }
    let mut guard = s.book.lock().await;
    let mut book = guard.clone();
    let sleeve = book.sleeves.get_mut(o.data.id()).ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "Strategy account missing".into(),
    ))?;
    engine::apply(sleeve, &o, s.enabled.load(Ordering::Relaxed)).map_err(bad)?;
    persist(&s, &book).await?;
    *guard = book.clone();
    Ok(Json(book))
}

#[derive(Deserialize)]
struct InsightBatch {
    as_of_ms: u64,
    insights: Vec<FeedInsight>,
    observations: Vec<Observation>,
}
async fn insights(
    State(s): State<PaperState>,
    Json(batch): Json<InsightBatch>,
) -> Result<Json<Book>, Error> {
    let current = now();
    if batch.as_of_ms > current
        || current - batch.as_of_ms > 60_000
        || batch.insights.len() != IDS.len()
        || batch.observations.len() > 32
    {
        return Err(bad(
            "Expected five current strategy insights and at most 32 observations".into(),
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for feed in &batch.insights {
        if !IDS.contains(&feed.strategy.as_str())
            || !seen.insert(feed.strategy.clone())
            || feed.computed_at_ms > current
            || current - feed.computed_at_ms > 120_000
            || (feed.ready && !feed.current(current))
        {
            return Err(bad("Invalid, duplicate or stale strategy insight".into()));
        }
    }
    let mut guard = s.book.lock().await;
    let mut book = guard.clone();
    for feed in batch.insights {
        let id = feed.strategy.clone();
        book.sleeves
            .get_mut(&id)
            .ok_or_else(|| bad("Missing account".into()))?
            .feed = feed;
    }
    for observation in batch.observations {
        let sleeve = book
            .sleeves
            .get_mut(observation.data.id())
            .ok_or_else(|| bad("Missing account".into()))?;
        if !sleeve.feed.current(current) {
            continue;
        }
        let mut candidate = sleeve.clone();
        let result = if observation.observed_at_ms > current
            || current - observation.observed_at_ms > 60_000
        {
            Err("Observation must be current".into())
        } else {
            engine::apply(
                &mut candidate,
                &observation,
                s.enabled.load(Ordering::Relaxed),
            )
        };
        match result {
            Ok(()) => *sleeve = candidate,
            Err(error) => {
                sleeve.feed.ready = false;
                sleeve.feed.signal = "rejected".into();
                sleeve.feed.summary = error;
            }
        }
    }
    persist(&s, &book).await?;
    *guard = book.clone();
    Ok(Json(book))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn state(path: PathBuf) -> PaperState {
        PaperState {
            book: Arc::new(Mutex::new(Book::initialized())),
            path,
            enabled: Arc::new(AtomicBool::new(true)),
            load_error: None,
        }
    }
    #[tokio::test]
    async fn failed_persistence_does_not_change_in_memory_settings() {
        let s = state(
            std::env::temp_dir().join(format!("missing-alternative-parent-{}/book.json", now())),
        );
        let time = now();
        s.book.lock().await.sleeves.get_mut(IDS[0]).unwrap().feed = FeedInsight {
            ready: true,
            source_timestamp_ms: time,
            computed_at_ms: time,
            ..FeedInsight::default()
        };
        let x = Update {
            enabled: true,
            max_trade_usd: 100.0,
            daily_loss_limit_usd: 25.0,
            spot_fee_bps: 15.0,
            short_borrow_apr: 0.1,
            min_edge_bps: 10.0,
        };
        assert!(
            configure(State(s.clone()), Path(IDS[0].into()), Json(x))
                .await
                .is_err()
        );
        assert!(!s.book.lock().await.sleeves[IDS[0]].config.enabled);
    }
    #[tokio::test]
    async fn fresh_observation_is_idempotent_and_persists_across_restart() {
        let path = std::env::temp_dir().join(format!(
            "alternative-test-{}-{}.json",
            std::process::id(),
            now()
        ));
        let s = state(path.clone());
        s.book
            .lock()
            .await
            .sleeves
            .get_mut(IDS[3])
            .unwrap()
            .config
            .enabled = true;
        let time = now();
        let o = Observation {
            id: "deduplicate".into(),
            source: "test".into(),
            observed_at_ms: time,
            data: Input::LiquidationKeeper(LiquidationInput {
                opportunity_id: "loan".into(),
                chain_id: 1,
                block_number: 1,
                protocol: "aave".into(),
                health_factor: 0.9,
                debt_repayment_usd: 90.0,
                executable_collateral_sale_usd: 99.0,
                total_cost_usd: 2.0,
                simulation_succeeded: true,
                atomic: true,
            }),
        };
        let _ = observe(State(s.clone()), Json(o.clone())).await.unwrap();
        let _ = observe(State(s.clone()), Json(o.clone())).await.unwrap();
        let stored: Book = serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(stored.sleeves[IDS[3]].cash_usd, 1007.0);
        *s.book.lock().await = stored;
        let _ = observe(State(s.clone()), Json(o)).await.unwrap();
        assert_eq!(s.book.lock().await.sleeves[IDS[3]].cash_usd, 1007.0);
        tokio::fs::remove_file(path).await.unwrap();
    }
    #[tokio::test]
    async fn api_rejects_historical_observations() {
        let s = state(std::env::temp_dir().join("unused-alternative-test.json"));
        let o = Observation {
            id: "old".into(),
            source: "test".into(),
            observed_at_ms: 1,
            data: Input::ResidualMomentum(ResidualInput {
                scores: None,
                assets: Vec::new(),
            }),
        };
        assert_eq!(
            observe(State(s), Json(o)).await.err().unwrap().0,
            StatusCode::BAD_REQUEST
        );
    }
    #[tokio::test]
    async fn invalid_observation_cannot_partially_reset_daily_ledger() {
        let s = state(std::env::temp_dir().join("unused-alternative-test.json"));
        let o = Observation {
            id: "bad".into(),
            source: "test".into(),
            observed_at_ms: now(),
            data: Input::ResidualMomentum(ResidualInput {
                scores: None,
                assets: Vec::new(),
            }),
        };
        assert!(observe(State(s.clone()), Json(o)).await.is_err());
        assert_eq!(s.book.lock().await.sleeves[IDS[0]].utc_day, 0);
    }
    #[tokio::test]
    async fn load_error_prevents_overwrite() {
        let mut s = state(std::env::temp_dir().join("unused-alternative-test.json"));
        s.load_error = Some("Corrupt account".into());
        assert_eq!(
            persist(&s, &Book::initialized()).await.err().unwrap().0,
            StatusCode::SERVICE_UNAVAILABLE
        );
    }

    fn native_batch(time: u64) -> InsightBatch {
        InsightBatch {
            as_of_ms: time,
            insights: IDS
                .iter()
                .map(|id| FeedInsight {
                    strategy: (*id).into(),
                    ready: true,
                    signal: "waiting".into(),
                    source_timestamp_ms: time,
                    computed_at_ms: time,
                    summary: "Native data healthy".into(),
                    features: serde_json::json!({"test":true}),
                })
                .collect(),
            observations: Vec::new(),
        }
    }
    #[tokio::test]
    async fn native_readiness_is_required_to_enable_and_disable_remains_available() {
        let path = std::env::temp_dir().join(format!(
            "alternative-readiness-{}-{}.json",
            std::process::id(),
            now()
        ));
        let s = state(path.clone());
        let update = || Update {
            enabled: true,
            max_trade_usd: 100.0,
            daily_loss_limit_usd: 25.0,
            spot_fee_bps: 15.0,
            short_borrow_apr: 0.1,
            min_edge_bps: 10.0,
        };
        assert!(
            configure(State(s.clone()), Path(IDS[0].into()), Json(update()))
                .await
                .is_err()
        );
        let _ = insights(State(s.clone()), Json(native_batch(now())))
            .await
            .unwrap();
        let _ = configure(State(s.clone()), Path(IDS[0].into()), Json(update()))
            .await
            .unwrap();
        {
            let mut book = s.book.lock().await;
            book.sleeves
                .get_mut(IDS[0])
                .unwrap()
                .feed
                .source_timestamp_ms = 1;
        }
        let mut disabled = update();
        disabled.enabled = false;
        let _ = configure(State(s.clone()), Path(IDS[0].into()), Json(disabled))
            .await
            .unwrap();
        assert!(!s.book.lock().await.sleeves[IDS[0]].config.enabled);
        tokio::fs::remove_file(path).await.unwrap();
    }
    #[tokio::test]
    async fn native_batch_isolates_bad_strategy_and_persists_healthy_opportunity() {
        let path = std::env::temp_dir().join(format!(
            "alternative-batch-{}-{}.json",
            std::process::id(),
            now()
        ));
        let s = state(path.clone());
        let time = now();
        s.book
            .lock()
            .await
            .sleeves
            .get_mut(IDS[3])
            .unwrap()
            .config
            .enabled = true;
        let mut batch = native_batch(time);
        batch.observations = vec![
            Observation {
                id: "invalid-residual".into(),
                source: "Insights".into(),
                observed_at_ms: time,
                data: Input::ResidualMomentum(ResidualInput {
                    scores: None,
                    assets: Vec::new(),
                }),
            },
            Observation {
                id: "native-liquidation".into(),
                source: "Insights".into(),
                observed_at_ms: time,
                data: Input::LiquidationKeeper(LiquidationInput {
                    opportunity_id: "native-loan".into(),
                    chain_id: 1,
                    block_number: 123,
                    protocol: "Aave".into(),
                    health_factor: 0.9,
                    debt_repayment_usd: 90.0,
                    executable_collateral_sale_usd: 99.0,
                    total_cost_usd: 2.0,
                    simulation_succeeded: true,
                    atomic: true,
                }),
            },
        ];
        let _ = insights(State(s.clone()), Json(batch)).await.unwrap();
        let book: Book = serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert!(!book.sleeves[IDS[0]].feed.ready);
        assert_eq!(book.sleeves[IDS[0]].utc_day, 0);
        assert!(book.sleeves[IDS[3]].feed.ready);
        assert_eq!(book.sleeves[IDS[3]].cash_usd, 1007.0);
        tokio::fs::remove_file(path).await.unwrap();
    }
    #[tokio::test]
    async fn stale_or_duplicate_native_status_cannot_replace_current_book() {
        let s = state(std::env::temp_dir().join("unused-native-batch.json"));
        let mut batch = native_batch(now());
        batch.insights[0].source_timestamp_ms = 1;
        assert!(insights(State(s.clone()), Json(batch)).await.is_err());
        let mut batch = native_batch(now());
        batch.insights[1] = batch.insights[0].clone();
        assert!(insights(State(s.clone()), Json(batch)).await.is_err());
        assert_eq!(s.book.lock().await.sleeves[IDS[0]].feed.computed_at_ms, 0);
    }
}
