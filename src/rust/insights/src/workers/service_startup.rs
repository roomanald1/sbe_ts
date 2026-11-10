#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let signal_store = SignalStore::from_environment()?;
    daily_summary::initialize().await?;
    tokio::spawn(poll_external_market_features(signal_store.clone()));
    tokio::spawn(run_dated_carry_signal_worker(signal_store.clone()));
    tokio::spawn(poll_daily_strategy_signals(signal_store.clone()));
    let (research_record_sender, research_record_receiver) = tokio::sync::mpsc::channel(4_096);
    tokio::spawn(run_research_record_writer(
        signal_store.clone(),
        research_record_receiver,
    ));
    let signal_push_cache = Arc::new(DashMap::new());
    let signal_push_notify = Arc::new(Notify::new());
    let scout_push_cache = Arc::new(DashMap::new());
    let scout_push_notify = Arc::new(Notify::new());
    if !research_only() {
        tokio::spawn(run_signal_push_worker(
            Arc::clone(&signal_push_cache),
            Arc::clone(&signal_push_notify),
            Arc::clone(&scout_push_cache),
            Arc::clone(&scout_push_notify),
        ));
        tokio::spawn(run_scout_input_push_worker(
            Arc::clone(&scout_push_cache),
            Arc::clone(&scout_push_notify),
        ));
    }
    let market_cache = Arc::new(DashMap::new());
    let ui_snapshot_cache = Arc::new(DashMap::new());
    let ui_snapshot_notify = Arc::new(Notify::new());
    tokio::spawn(run_ui_snapshot_push_worker(
        Arc::clone(&ui_snapshot_cache),
        Arc::clone(&ui_snapshot_notify),
    ));
    let initial_news = Arc::new(NewsFeedSnapshot {
        source: "CoinDesk RSS".to_owned(),
        stale: true,
        ..NewsFeedSnapshot::default()
    });
    let (news_tx, news_rx) = watch::channel(initial_news);
    tokio::spawn(poll_news_feed(news_tx));
    let signal_runtime = Arc::new(SignalRuntime {
        store: signal_store.clone(),
        engines: DashMap::new(),
        research_record_sender,
        signal_push_cache,
        signal_push_notify,
        market_cache,
        market_push_at: DashMap::new(),
        ui_snapshot_cache,
        ui_snapshot_notify,
        news: news_rx,
        news_learning: Mutex::new(NewsLearning {
            weight: 0.20,
            ..NewsLearning::default()
        }),
        snapshot_diagnostic_at: DashMap::new(),
        latest_prices: DashMap::new(),
    });
    let (signal_event_sender, signal_event_receiver) = tokio::sync::mpsc::channel(8_192);
    tokio::spawn(run_signal_event_workers(
        Arc::clone(&signal_runtime),
        signal_event_receiver,
    ));
    tokio::spawn(run_feed_signal_consumer(signal_event_sender));
    let service_name = if research_only() { "Insights Research" } else { "Insights" };
    let app = Router::new()
        .merge(service_docs::routes(service_name, &[("/health", &["get"]), ("/signals/stream", &["get"]), ("/positions/insights", &["get"]), ("/signals/daily-summary", &["get"]), ("/signals/dashboard", &["get"]), ("/signals/history", &["get"])]))
        .merge(daily_summary::router())
        .route("/health", get(|| async { "ok" }))
        .route(
            "/internal/latency",
            get(|| async { axum::Json(crate::latency::snapshot()) }),
        )
        .route(
            "/internal/signal-feed-status",
            get(|| async { axum::Json(crate::latency::signal_feed_status()) }),
        )
        .route(
            "/internal/live-context",
            get(live_context).with_state(Arc::clone(&signal_runtime)),
        )
        .route("/process-cpu-seconds", get(process_cpu_seconds)).route("/process-memory-bytes", get(process_memory_bytes))
        .route(
            "/internal/signals/event",
            axum::routing::post(ingest_signal_event).with_state(Arc::clone(&signal_runtime)),
        )
        .route(
            "/internal/scouts/register",
            axum::routing::post(register_scout).with_state(Arc::clone(&signal_runtime)),
        )
        .route(
            "/internal/signals/counterfactual",
            axum::routing::post(register_counterfactual).with_state(Arc::clone(&signal_runtime)),
        )
        .route(
            "/internal/signals/scout-strategy-status",
            axum::routing::post(update_scout_strategy_status).with_state(Arc::clone(&signal_runtime)),
        )
        .merge(signal_store.router());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3004)).await?;
    crate::service_log!("INFO", "{service_name} service listening on http://0.0.0.0:3004");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn live_context(
    axum::extract::State(runtime): axum::extract::State<Arc<SignalRuntime>>,
) -> Json<serde_json::Value> {
    // Trigger cached research refreshes from the background poller. These methods
    // schedule expensive scans asynchronously and return the last completed values.
    let _ = runtime.store.walk_forward().await;
    let _ = runtime.store.missed_opportunity_bands().await;
    let news = Arc::clone(&runtime.news.borrow());
    let news_weight = runtime
        .news_learning
        .try_lock()
        .map(|learning| learning.weight)
        .unwrap_or(0.0);
    Json(serde_json::json!({
        "news": news.as_ref(),
        "weight": news_weight,
        "walkForward": runtime.store.cached_walk_forward(),
        "missedOpportunityBands": runtime.store.cached_missed_opportunity_bands(),
    }))
}

async fn process_memory_bytes() -> String {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|status| status.lines().find_map(|line| line.strip_prefix("VmRSS:")?.split_whitespace().next()?.parse::<u64>().ok()))
        .map(|kilobytes| (kilobytes * 1024).to_string())
        .unwrap_or_else(|| "0".to_owned())
}

async fn process_cpu_seconds() -> String {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    let result = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    if result != 0 {
        return "0".to_owned();
    }
    let usage = unsafe { usage.assume_init() };
    let user = usage.ru_utime.tv_sec as f64 + usage.ru_utime.tv_usec as f64 / 1_000_000.0;
    let system = usage.ru_stime.tv_sec as f64 + usage.ru_stime.tv_usec as f64 / 1_000_000.0;
    (user + system).to_string()
}
#[path = "../../../service_docs.rs"]
mod service_docs;
