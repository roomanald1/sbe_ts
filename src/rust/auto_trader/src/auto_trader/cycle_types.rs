#[derive(Serialize)]
struct PnlBreakdown {
    symbol: String,
    direction: String,
    purpose: String,
    strategy: String,
    realized_pnl: f64,
    outcomes: usize,
}
#[derive(Serialize)]
struct PositionExitPolicy {
    symbol: String,
    stop_loss_percent: f64,
    take_profit_percent: f64,
}
#[derive(Serialize)]
struct ActivityResponse {
    orders: Vec<TraderOrder>,
    position_exit_policies: Vec<PositionExitPolicy>,
    open_position_symbols: Vec<String>,
    pnl_breakdown: Vec<PnlBreakdown>,
    realized_pnl: Option<f64>,
    realized_pnl_by_quote: Option<BTreeMap<String, f64>>,
    unrealized_pnl: Option<f64>,
    unrealized_pnl_by_quote: Option<BTreeMap<String, f64>>,
    exit_learning_samples: usize,
    closed_trade_average_return_percent: Option<f64>,
    closed_trade_wins: usize,
    closed_trade_losses: usize,
    closed_trade_flat: usize,
    closed_trade_realized_usd: Option<f64>,
    closed_trade_fees_usd: Option<f64>,
    learning_samples: usize,
    learning_win_rate: Option<f64>,
    preferred_order_type: Option<String>,
    parameter_profile_samples: [usize; 3],
    scout_open_positions: usize,
    scout_max_active_positions: usize,
    scout_learning_samples: usize,
    scout_learning_win_rate: Option<f64>,
    scout_learning_realized_pnl: f64,
    insight_processing_latest_ms: Option<f64>,
    insight_processing_average_ms: Option<f64>,
    insight_processing_p95_ms: Option<f64>,
    insight_processing_samples: usize,
    cycle_lock_wait_ms: BTreeMap<String, CycleLockWaitSummary>,
    cycle_lock_hold_ms: BTreeMap<String, CycleLockWaitSummary>,
    settings_persistence_ms: BTreeMap<String, CycleLockWaitSummary>,
}
#[derive(Clone, Serialize)]
struct CycleLockWaitSummary {
    latest_ms: f64,
    average_ms: f64,
    p95_ms: f64,
    samples: usize,
}
struct CycleLockGuard {
    _guard: tokio::sync::OwnedMutexGuard<()>,
    hold_samples: Arc<std::sync::Mutex<BTreeMap<String, Vec<f64>>>>,
    worker: String,
    acquired_at: Instant,
}

impl Drop for CycleLockGuard {
    fn drop(&mut self) {
        if let Ok(mut samples) = self.hold_samples.lock() {
            let values = samples.entry(self.worker.clone()).or_default();
            values.push(self.acquired_at.elapsed().as_secs_f64() * 1000.0);
            if values.len() > 500 {
                values.remove(0);
            }
        }
    }
}

#[derive(Serialize)]
struct CycleResult {
    status: &'static str,
    reason: Option<String>,
    symbol: Option<String>,
    side: Option<String>,
    quantity: Option<f64>,
    order_type: Option<String>,
    order_response: Option<serde_json::Value>,
}
