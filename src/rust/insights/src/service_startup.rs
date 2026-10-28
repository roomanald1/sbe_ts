#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let signal_store = SignalStore::from_environment()?;
    daily_summary::initialize().await?;
    daily_summary::spawn_nightly();
    tokio::spawn(poll_external_market_features(signal_store.clone()));
    tokio::spawn(run_dated_carry_signal_worker(signal_store.clone()));
    tokio::spawn(poll_daily_strategy_signals(signal_store.clone()));
    let signal_push_cache = Arc::new(DashMap::new());
    let signal_push_notify = Arc::new(Notify::new());
    tokio::spawn(run_signal_push_worker(
        Arc::clone(&signal_push_cache),
        Arc::clone(&signal_push_notify),
    ));
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
        engines: Mutex::new(HashMap::new()),
        signal_push_cache,
        signal_push_notify,
        ui_snapshot_cache,
        ui_snapshot_notify,
        news: news_rx,
        news_learning: Mutex::new(NewsLearning { weight: 0.20, ..NewsLearning::default() }),
        snapshot_diagnostic_at: DashMap::new(),
    });
    tokio::spawn(run_feed_signal_consumer(Arc::clone(&signal_runtime)));
    let app = Router::new()
        .merge(daily_summary::router())
        .route("/health", get(|| async { "ok" }))
        .route("/process-cpu-seconds", get(process_cpu_seconds))
        .route("/positions/insights", get(get_position_insights))
        .route(
            "/internal/signals/event",
            axum::routing::post(ingest_signal_event).with_state(signal_runtime),
        )
        .merge(signal_store.router());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 3004)).await?;
    crate::service_log!("INFO", "Insights service listening on http://0.0.0.0:3004");
    axum::serve(listener, app).await?;
    Ok(())
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
