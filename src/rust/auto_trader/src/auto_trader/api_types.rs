#[derive(Serialize)]
struct StatusResponse {
    enabled: bool,
    audit_healthy: bool,
    audit_failure_latched: bool,
    decision_audit_queue_depth: usize,
    execution_audit_queue_depth: usize,
    decision_audit_queue_estimated_bytes: usize,
    execution_audit_queue_estimated_bytes: usize,
    audit_coalesced_decisions: u64,
    audit_dropped_events: u64,
    paper_learning_exploration_until_ms: u64,
    strategy: String,
    strategies: Vec<String>,
    signal_strategies_enabled: bool,
    max_order_notional: f64,
    daily_notional_limit: f64,
    daily_realized_loss_limit: f64,
    exploration_percent: f64,
    risk_appetite: f64,
    max_open_positions: usize,
    max_intraday_open_positions: usize,
    max_daily_open_positions: usize,
    intraday_notional_limit: f64,
    daily_strategy_notional_limit: f64,
    daily_strategy_budgets: Vec<StrategyBudgetStatus>,
    intraday_strategy_budgets: Vec<StrategyBudgetStatus>,
    intraday_used_notional: f64,
    daily_strategy_used_notional: f64,
    max_entries_per_auto_position: u32,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    vwap_duration_minutes: u64,
    vwap_participation_percent: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
    closed_trade_window: usize,
    minimum_scout_parameter_samples: usize,
    used_notional_today: f64,
    available_notional: f64,
    last_decision: String,
    last_decision_at: u64,
    decisions_since_trade: u64,
    gap_reasons: Vec<GapReason>,
    gate_hits: Vec<GateHit>,
    stale_signal_candidates_total: u64,
    stale_signal_cycles_total: u64,
    symbol_gate_hits: Vec<SymbolGateHit>,
    latest_signal_diagnostic: String,
    latest_entry_diagnostic: Option<LatestEntryDiagnostic>,
    entry_diagnostic_history: Vec<LatestEntryDiagnostic>,
    latest_symbol_entry_results: Vec<SymbolEntryResult>,
    strategy_diagnostics: Vec<StrategyDiagnostic>,
    latest_strategy_blockers: Vec<StrategyBlocker>,
}
#[derive(Deserialize)]
struct DailyStrategyDiagnosticsUpdate {
    as_of_ms: u64,
    diagnostics: Vec<StrategyDiagnostic>,
}
#[derive(Clone, Serialize)]
struct StrategyBlocker {
    strategy: String,
    symbol: String,
    reason: String,
    timestamp_ms: u64,
}
#[derive(Clone, Serialize)]
struct SymbolEntryResult {
    strategy: String,
    symbol: String,
    profile_id: Option<String>,
    event_id: Option<String>,
    decision: Option<strategy::LiveStrategyDecision>,
    reason: String,
    timestamp_ms: Option<u64>,
}
#[derive(Serialize)]
struct StrategyBudgetStatus {
    strategy: String,
    base_budget: f64,
    effective_budget: f64,
    used_notional: f64,
    learning_multiplier: f64,
    learning_feedback_multiplier: f64,
    learning_samples: usize,
    positive_rate: Option<f64>,
    realized_pnl: f64,
}
#[derive(Serialize)]
struct GapReason {
    reason: String,
    count: u64,
}
#[derive(Clone, Serialize)]
struct GateHit {
    gate: String,
    count: u64,
}
#[derive(Serialize)]
struct SymbolGateHit {
    symbol: String,
    direction: String,
    gate: String,
    count: u64,
    last_detail: String,
    last_seen_ms: u64,
    distance_to_pass: Option<f64>,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum EntryCycleOutcome {
    NoSignals,
    NoEligibleCandidates,
    NoFreshCandidates,
    CandidatesRejected,
    ExecutionBlocked,
    OrderQueued,
    OrderSubmitted,
    WaitingForStrategySignals,
    GlobalRiskBlock,
    CycleError,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct LatestEntryDiagnostic {
    cycle_id: String,
    timestamp_ms: u64,
    worker_event_ids: Vec<String>,
    candidate_decision_ids: Vec<String>,
    gate_counts: Vec<GateHit>,
    candidate_gate_counts: Vec<EntryGateCandidateCount>,
    candidate_details: Vec<EntryCandidateDiagnostic>,
    outcome: EntryCycleOutcome,
    raw_signal_count: usize,
    allowed_signal_count: usize,
    strategy_candidate_count: usize,
    fresh_directional_candidate_count: usize,
    stale_candidate_count: usize,
    non_directional_candidate_count: usize,
    position_blocked_candidate_count: usize,
    strategy_rejected_candidate_count: usize,
    evidence_present_candidate_count: usize,
    eligible_candidate_count: usize,
    rejected_candidate_count: usize,
    selected_candidate_count: usize,
    order_attempt_count: usize,
    submitted_order_count: usize,
    fill_confirmed: bool,
    primary_gate_id: Option<String>,
    primary_blocker: Option<String>,
    detail: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryCandidateDiagnostic {
    decision_id: String,
    worker_event_id: Option<String>,
    profile_id: Option<String>,
    symbol: String,
    strategy: Option<String>,
    stage: String,
    observed: serde_json::Value,
    required: serde_json::Value,
    checks: serde_json::Value,
    evidence_contract: serde_json::Value,
    measurements: Vec<EntryGateMeasurement>,
    reasons: Vec<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryGateMeasurement {
    gate_id: String,
    observed: Option<f64>,
    operator: Option<String>,
    required: Option<f64>,
    unit: Option<String>,
    status: String,
    detail: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct EntryGateCandidateCount {
    gate_id: String,
    symbol: String,
    strategy: Option<String>,
    count: u64,
}
#[derive(Default)]
struct SymbolGateCount {
    count: u64,
    last_detail: String,
    last_seen_ms: u64,
    distance_to_pass: Option<f64>,
}
#[derive(Clone, Deserialize)]
struct SettingsRequest {
    max_order_notional: f64,
    #[serde(default)]
    strategy: Option<Strategy>,
    #[serde(default, alias = "research_strategies")]
    strategies: Option<Vec<Strategy>>,
    #[serde(default, alias = "generic_signals_enabled")]
    signal_strategies_enabled: Option<bool>,
    daily_notional_limit: f64,
    daily_realized_loss_limit: f64,
    exploration_percent: f64,
    #[serde(default)]
    risk_appetite: Option<f64>,
    #[serde(default)]
    take_profit_percent: Option<f64>,
    max_open_positions: usize,
    #[serde(default)]
    max_intraday_open_positions: Option<usize>,
    #[serde(default)]
    max_daily_open_positions: Option<usize>,
    #[serde(default)]
    intraday_notional_limit: Option<f64>,
    #[serde(default)]
    daily_strategy_notional_limit: Option<f64>,
    #[serde(default)]
    daily_strategy_budgets: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    intraday_strategy_budgets: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    max_entries_per_auto_position: Option<u32>,
    #[serde(default)]
    vwap_duration_minutes: Option<u64>,
    #[serde(default)]
    vwap_participation_percent: Option<f64>,
    #[serde(default)]
    limit_offset_bps: Option<f64>,
    #[serde(default)]
    trigger_offset_bps: Option<f64>,
    #[serde(default)]
    trailing_distance_percent: Option<f64>,
    #[serde(default)]
    closed_trade_window: Option<usize>,
    #[serde(default, alias = "minimum_live_scout_parameter_samples")]
    minimum_scout_parameter_samples: Option<usize>,
    #[serde(default)]
    reset_learning: bool,
    #[serde(default)]
    strategy_parameter_overrides: Option<BTreeMap<String, f64>>,
    #[serde(default)]
    active_live_strategy_profiles: Option<BTreeMap<String, Vec<strategy::LiveStrategyProfile>>>,
    #[serde(default)]
    reset_strategy_profile: Option<StrategyProfileReset>,
}
#[derive(Clone, Deserialize)]
struct StrategyProfileReset {
    symbol: String,
    strategy: String,
}
#[derive(Clone, Deserialize)]
struct EnabledRequest {
    enabled: bool,
}
#[derive(Clone, Serialize)]
struct DecisionParameter {
    section: String,
    name: String,
    value: f64,
    unit: String,
}
#[derive(Clone, Serialize)]
struct ContextDecisionParameters {
    context: String,
    learning_samples: usize,
    positive_rate: Option<f64>,
    scout_learning_samples: usize,
    scout_positive_rate: Option<f64>,
    strength_floor: f64,
    exploration_score_floor: f64,
    exploration_allowed: bool,
}
#[derive(Clone, Serialize)]
struct LatestExecutionParameters {
    timestamp: u64,
    symbol: String,
    context: String,
    baseline_order_type: String,
    selected_order_type: String,
    profile: usize,
    vwap_duration_minutes: u64,
    vwap_participation_percent: f64,
    schedule_depth_threshold: f64,
    proposed_depth_ratio: f64,
    schedule_threshold_samples: usize,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}
#[derive(Serialize)]
struct DecisionParametersResponse {
    refreshed_at: u64,
    thresholds: Vec<DecisionParameter>,
    contexts: Vec<ContextDecisionParameters>,
    latest_execution: Option<LatestExecutionParameters>,
    strategy_profiles: Vec<StrategyParameterProfile>,
}

#[derive(Default)]
struct GapSummary {
    utc_day: String,
    decisions: u64,
    reasons: BTreeMap<String, u64>,
    gate_hits: BTreeMap<String, u64>,
    symbol_gate_hits: BTreeMap<(String, String, String), SymbolGateCount>,
    latest_detail: String,
    latest_entry_diagnostic: Option<LatestEntryDiagnostic>,
    entry_diagnostic_history: Vec<LatestEntryDiagnostic>,
}
