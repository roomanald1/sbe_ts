impl Default for AutoOrderType {
    fn default() -> Self {
        Self::Ioc
    }
}
#[derive(Clone, Serialize, Deserialize)]
struct Settings {
    #[serde(default)]
    enabled: bool,
    #[serde(
        default = "default_strategies",
        alias = "research_strategies",
        deserialize_with = "deserialize_strategies"
    )]
    strategies: Vec<Strategy>,
    #[serde(
        default = "default_signal_strategies_enabled",
        alias = "generic_signals_enabled"
    )]
    signal_strategies_enabled: bool,
    #[serde(default = "default_order_limit")]
    max_order_notional: f64,
    #[serde(default = "default_daily_limit")]
    daily_notional_limit: f64,
    #[serde(default)]
    used_notional_today: f64,
    #[serde(default = "default_usd_rates")]
    usd_rates: BTreeMap<String, f64>,
    #[serde(default)]
    realized_pnl: f64,
    #[serde(default = "default_daily_loss_limit")]
    daily_realized_loss_limit: f64,
    #[serde(default = "default_exploration_percent")]
    exploration_percent: f64,
    #[serde(default = "default_risk_appetite")]
    risk_appetite: f64,
    #[serde(default = "default_max_open_positions")]
    max_open_positions: usize,
    #[serde(default = "default_max_intraday_open_positions")]
    max_intraday_open_positions: usize,
    #[serde(default = "default_max_daily_open_positions")]
    max_daily_open_positions: usize,
    #[serde(default = "default_intraday_notional_limit")]
    intraday_notional_limit: f64,
    #[serde(default = "default_daily_strategy_notional_limit")]
    daily_strategy_notional_limit: f64,
    #[serde(default)]
    daily_strategy_budgets: BTreeMap<String, f64>,
    #[serde(default)]
    intraday_strategy_budgets: BTreeMap<String, f64>,
    #[serde(default = "default_max_entries_per_auto_position")]
    max_entries_per_auto_position: u32,
    #[serde(default = "default_stop_loss_percent")]
    stop_loss_percent: f64,
    #[serde(default = "default_take_profit_percent")]
    take_profit_percent: f64,
    #[serde(default)]
    daily_loss_limit_reduced: bool,
    #[serde(default = "default_vwap_duration")]
    vwap_duration_minutes: u64,
    #[serde(default = "default_vwap_participation")]
    vwap_participation_percent: f64,
    #[serde(default = "default_limit_offset")]
    limit_offset_bps: f64,
    #[serde(default = "default_trigger_offset")]
    trigger_offset_bps: f64,
    #[serde(default = "default_trailing_distance")]
    trailing_distance_percent: f64,
    #[serde(default = "default_closed_trade_window")]
    closed_trade_window: usize,
    #[serde(default = "default_live_scout_parameter_samples")]
    minimum_live_scout_parameter_samples: usize,
    #[serde(default)]
    closed_trade_returns_percent: Vec<f64>,
    #[serde(default)]
    closed_trade_timestamps_ms: Vec<u64>,
    #[serde(default)]
    closed_trade_realized_usd: Vec<f64>,
    #[serde(default)]
    closed_trade_fees_usd: Vec<f64>,
    /// Audit eligibility aligned with the legacy closed-trade metric arrays.
    /// Missing entries from older settings are treated as eligible history.
    #[serde(default)]
    closed_trade_audit: Vec<ClosedTradeAuditRecord>,
    #[serde(default)]
    closed_trade_metrics_version: u32,
    #[serde(default)]
    daily_realized_pnl: f64,
    #[serde(default)]
    pnl_utc_day: String,
    #[serde(default)]
    utc_day: String,
    #[serde(default)]
    orders: Vec<TraderOrder>,
    /// Durable canonical links keyed by the existing decision correlation key.
    /// The key is not promoted to a canonical lifecycle identity.
    #[serde(default)]
    canonical_lifecycle_links: BTreeMap<String, CanonicalLifecycleIdentity>,
    #[serde(default)]
    canonical_lifecycle_sequence: u64,
    #[serde(default)]
    canonical_lifecycle_epoch: String,
    /// Stable worker event reservations for bounded exploratory deduplication.
    /// Missing entries from older settings and signals without source IDs stay unknown.
    #[serde(default)]
    exploratory_event_reservations: BTreeMap<String, u64>,
    /// Persisted stop-fill re-entry gates keyed by symbol and strategy.
    #[serde(default)]
    post_stop_reentry_cooldowns: BTreeMap<String, PostStopReentryCooldown>,
    #[serde(default)]
    positions: BTreeMap<String, TraderPosition>,
    #[serde(default)]
    strategy_positions: BTreeMap<String, BTreeMap<String, TraderPosition>>,
    #[serde(default)]
    learning: BTreeMap<String, LearningStats>,
    #[serde(default)]
    feedback_outbox: Vec<strategy::FeedbackEvent>,
    #[serde(default)]
    missed_opportunity_last_enqueued_at_ms: BTreeMap<String, u64>,
    #[serde(default)]
    post_exit_observations: Vec<serde_json::Value>,
    #[serde(default)]
    post_exit_adaptation_history: Vec<serde_json::Value>,
    #[serde(default)]
    exit_timing_feedback_outbox: Vec<serde_json::Value>,
    #[serde(default)]
    exit_timing_history_backfill_version: u32,
    #[serde(default)]
    profile_registration_outbox: Vec<strategy::StrategyProfileRegistration>,
    /// Enqueue times for profile registrations; absent entries are legacy/unknown.
    #[serde(default)]
    profile_registration_enqueued_at_ms: BTreeMap<String, u64>,
    #[serde(default)]
    registered_profile_ids: BTreeSet<String>,
    #[serde(default)]
    learned_profiles: BTreeMap<String, strategy::StrategyProfile>,
    #[serde(default)]
    learned_exit_protection_policies: BTreeMap<String, strategy::LearnedExitProtectionPolicy>,
    #[serde(default)]
    learned_walk_forward_evidence: BTreeMap<String, strategy::StrategyWalkForwardEvidence>,
    #[serde(default)]
    learned_context_thresholds: BTreeMap<String, strategy::ContextThresholds>,
    /// Explicitly selected concurrent live parameter profiles, keyed by strategy.
    #[serde(default)]
    active_live_strategy_profiles: BTreeMap<String, Vec<strategy::LiveStrategyProfile>>,
    #[serde(default)]
    learned_scout_parameter_variants: Vec<strategy::ScoutParameterVariant>,
    #[serde(default)]
    learned_strategy_size_multipliers: BTreeMap<String, f64>,
    /// Temporary bounded paper exploration window started by Reset Learning.
    #[serde(default)]
    paper_learning_exploration_until_ms: u64,
    #[serde(default)]
    recent_profile_context_returns: BTreeMap<String, Vec<(u64, f64)>>,
    #[serde(default)]
    contextual_stop_loss_percents: BTreeMap<String, f64>,
    #[serde(default)]
    learned_context_stats: BTreeMap<String, strategy::ContextLearningStat>,
    #[serde(default)]
    learned_execution_stats: BTreeMap<String, strategy::ExecutionLearningStat>,
    #[serde(default)]
    execution_learning: BTreeMap<String, ExecutionLearningStats>,
    #[serde(default)]
    execution_learning_imported: bool,
    #[serde(default)]
    execution_learning_order_ids: BTreeSet<String>,
    #[serde(default)]
    execution_feedback_outbox: Vec<strategy::ExecutionOutcomeEvent>,
    /// Durable delivery lifecycle counters keyed by feedback event class.
    #[serde(default)]
    learning_feedback_stats: BTreeMap<String, LearningFeedbackQueueStats>,
    // Retained only long enough to import in-flight state created before
    // Scouts became the owner of paired scout outcomes.
    #[serde(default)]
    strategy_parameter_overrides: BTreeMap<String, f64>,
    #[serde(default)]
    last_hedge_at: BTreeMap<String, u64>,
    #[serde(default)]
    last_decision: String,
    #[serde(default)]
    last_decision_at: u64,
    #[serde(default)]
    last_risk_note: String,
    #[serde(default)]
    risk_limited_sizing: bool,
    #[serde(default)]
    last_signal_at: BTreeMap<String, u64>,
    #[serde(default)]
    last_signal_direction: BTreeMap<String, String>,
    #[serde(default)]
    last_signal_strength: BTreeMap<String, f64>,
    #[serde(default)]
    entry_floor_lift_until_ms: BTreeMap<String, u64>,
    // Persisted so a service restart cannot replay the same closed-candle signal.
    #[serde(default)]
    daily_strategy_signal_cursors: BTreeMap<String, u64>,
}

const UNATTRIBUTED_POSITION_MANAGER_CONTEXT: &str = "unattributed:position-manager-reconciliation";

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PostStopReentryCooldown {
    stopped_at_ms: u64,
    cooldown_until_ms: u64,
    profile_id: Option<String>,
    event_identity_version: u16,
    stopped_event_id: Option<String>,
    stopped_episode_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ClosedTradeAuditStatus {
    #[default]
    Eligible,
    NeedsAudit,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct ClosedTradeAuditRecord {
    #[serde(default)]
    status: ClosedTradeAuditStatus,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
    #[serde(default)]
    trade_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct LearningFeedbackQueueStats {
    #[serde(default)]
    attempts: u64,
    #[serde(default)]
    acknowledged: u64,
    #[serde(default)]
    rejected: u64,
    #[serde(default)]
    transient_errors: u64,
    #[serde(default)]
    terminal_drops: u64,
    #[serde(default)]
    evicted_oldest: u64,
    #[serde(default)]
    outcome_capacity_rejections: u64,
    #[serde(default)]
    permanent_rejections: u64,
    #[serde(default)]
    last_attempt_at_ms: Option<u64>,
    #[serde(default)]
    latest_reason: Option<String>,
    #[serde(default)]
    latest_reason_at_ms: Option<u64>,
}

fn deserialize_strategies<'de, D>(deserializer: D) -> Result<Vec<Strategy>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;

    let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
    let mut strategies = Vec::new();
    for value in values {
        if value.as_str() == Some("live-insights") {
            strategies.extend([
                Strategy::LiveMomentum,
                Strategy::LiveTradeFlow,
                Strategy::LiveOrderBook,
                Strategy::LiveVwapDislocation,
                Strategy::LiveAbsorptionPressure,
                Strategy::LiveRangeMeanReversion,
            ]);
        } else {
            strategies.push(serde_json::from_value(value).map_err(D::Error::custom)?);
        }
    }
    Ok(strategies)
}
fn default_order_limit() -> f64 {
    DEFAULT_MAX_ORDER_NOTIONAL
}
fn default_daily_loss_limit() -> f64 {
    DEFAULT_DAILY_REALIZED_LOSS_LIMIT
}
fn default_exploration_percent() -> f64 {
    EXPLORATION_NOTIONAL_FRACTION * 100.0
}
fn default_risk_appetite() -> f64 {
    50.0
}
fn default_max_open_positions() -> usize {
    20
}
fn default_max_intraday_open_positions() -> usize {
    DEFAULT_MAX_INTRADAY_OPEN_POSITIONS
}
fn default_max_daily_open_positions() -> usize {
    DEFAULT_MAX_DAILY_OPEN_POSITIONS
}
fn default_intraday_notional_limit() -> f64 {
    DEFAULT_INTRADAY_NOTIONAL_LIMIT
}
fn default_daily_strategy_notional_limit() -> f64 {
    DEFAULT_DAILY_STRATEGY_NOTIONAL_LIMIT
}
fn default_max_entries_per_auto_position() -> u32 {
    DEFAULT_MAX_ENTRIES_PER_AUTO_POSITION
}
fn default_stop_loss_percent() -> f64 {
    DEFAULT_STOP_LOSS_PERCENT
}
fn default_take_profit_percent() -> f64 {
    DEFAULT_TAKE_PROFIT_PERCENT
}
fn default_vwap_duration() -> u64 {
    5
}
fn default_vwap_participation() -> f64 {
    20.0
}
fn default_limit_offset() -> f64 {
    0.0
}
fn default_trigger_offset() -> f64 {
    20.0
}
fn default_trailing_distance() -> f64 {
    0.10
}
fn default_closed_trade_window() -> usize {
    200
}
fn default_daily_limit() -> f64 {
    DEFAULT_DAILY_NOTIONAL
}
fn default_usd_rates() -> BTreeMap<String, f64> {
    BTreeMap::from([
        ("USD".to_owned(), 1.0),
        ("USDT".to_owned(), 1.0),
        ("USDC".to_owned(), 1.0),
    ])
}
fn default_strategies() -> Vec<Strategy> {
    std::iter::once(Strategy::default())
        .chain(Strategy::live_strategies().filter(|strategy| !strategy.is_legacy_composite()))
        .collect()
}
fn default_signal_strategies_enabled() -> bool {
    true
}
fn default_live_scout_parameter_samples() -> usize {
    3
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            strategies: default_strategies(),
            signal_strategies_enabled: default_signal_strategies_enabled(),
            max_order_notional: default_order_limit(),
            daily_notional_limit: default_daily_limit(),
            used_notional_today: 0.0,
            usd_rates: default_usd_rates(),
            realized_pnl: 0.0,
            daily_realized_loss_limit: default_daily_loss_limit(),
            exploration_percent: default_exploration_percent(),
            risk_appetite: default_risk_appetite(),
            max_open_positions: default_max_open_positions(),
            max_intraday_open_positions: default_max_intraday_open_positions(),
            max_daily_open_positions: default_max_daily_open_positions(),
            intraday_notional_limit: default_intraday_notional_limit(),
            daily_strategy_notional_limit: default_daily_strategy_notional_limit(),
            daily_strategy_budgets: BTreeMap::new(),
            intraday_strategy_budgets: BTreeMap::new(),
            max_entries_per_auto_position: default_max_entries_per_auto_position(),
            stop_loss_percent: default_stop_loss_percent(),
            take_profit_percent: default_take_profit_percent(),
            daily_loss_limit_reduced: false,
            vwap_duration_minutes: default_vwap_duration(),
            vwap_participation_percent: default_vwap_participation(),
            limit_offset_bps: default_limit_offset(),
            trigger_offset_bps: default_trigger_offset(),
            trailing_distance_percent: default_trailing_distance(),
            closed_trade_window: default_closed_trade_window(),
            minimum_live_scout_parameter_samples: default_live_scout_parameter_samples(),
            closed_trade_returns_percent: Vec::new(),
            closed_trade_timestamps_ms: Vec::new(),
            closed_trade_realized_usd: Vec::new(),
            closed_trade_fees_usd: Vec::new(),
            closed_trade_audit: Vec::new(),
            closed_trade_metrics_version: 4,
            daily_realized_pnl: 0.0,
            pnl_utc_day: utc_day(),
            utc_day: utc_day(),
            orders: Vec::new(),
            canonical_lifecycle_links: BTreeMap::new(),
            canonical_lifecycle_sequence: 0,
            canonical_lifecycle_epoch: new_canonical_lifecycle_epoch(),
            exploratory_event_reservations: BTreeMap::new(),
            post_stop_reentry_cooldowns: BTreeMap::new(),
            positions: BTreeMap::new(),
            strategy_positions: BTreeMap::new(),
            learning: BTreeMap::new(),
            feedback_outbox: Vec::new(),
            missed_opportunity_last_enqueued_at_ms: BTreeMap::new(),
            post_exit_observations: Vec::new(),
            post_exit_adaptation_history: Vec::new(),
            exit_timing_feedback_outbox: Vec::new(),
            exit_timing_history_backfill_version: 0,
            profile_registration_outbox: Vec::new(),
            profile_registration_enqueued_at_ms: BTreeMap::new(),
            registered_profile_ids: BTreeSet::new(),
            learned_profiles: BTreeMap::new(),
            learned_exit_protection_policies: BTreeMap::new(),
            learned_walk_forward_evidence: BTreeMap::new(),
            learned_context_thresholds: BTreeMap::new(),
            active_live_strategy_profiles: BTreeMap::new(),
            learned_scout_parameter_variants: Vec::new(),
            learned_strategy_size_multipliers: BTreeMap::new(),
            paper_learning_exploration_until_ms: 0,
            recent_profile_context_returns: BTreeMap::new(),
            contextual_stop_loss_percents: BTreeMap::new(),
            learned_context_stats: BTreeMap::new(),
            learned_execution_stats: BTreeMap::new(),
            execution_learning: BTreeMap::new(),
            execution_learning_imported: false,
            execution_learning_order_ids: BTreeSet::new(),
            execution_feedback_outbox: Vec::new(),
            learning_feedback_stats: BTreeMap::new(),
            strategy_parameter_overrides: BTreeMap::new(),
            last_hedge_at: BTreeMap::new(),
            last_decision: String::new(),
            last_decision_at: 0,
            last_risk_note: String::new(),
            risk_limited_sizing: false,
            last_signal_at: BTreeMap::new(),
            last_signal_direction: BTreeMap::new(),
            last_signal_strength: BTreeMap::new(),
            entry_floor_lift_until_ms: BTreeMap::new(),
            daily_strategy_signal_cursors: BTreeMap::new(),
        }
    }
}

fn new_canonical_lifecycle_epoch() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nanos}", std::process::id())
}

/// Hash the complete declared strategy parameter set using a versioned,
/// sorted JSON representation. Partial or non-finite profiles remain unknown.
fn canonical_profile_hash_v1(
    symbol: &str,
    strategy: &str,
    profile_id: &str,
    parameters: &BTreeMap<String, f64>,
) -> Option<String> {
    let strategy_kind = Strategy::from_name(strategy)?;
    let specs = strategy_kind.parameter_specs();
    if specs.is_empty() {
        return None;
    }
    let canonical_parameters = specs
        .iter()
        .map(|spec| {
            parameters
                .get(spec.key.as_str())
                .filter(|value| value.is_finite())
                .map(|value| (spec.key.as_str(), *value))
        })
        .collect::<Option<BTreeMap<_, _>>>()?;
    let encoded = serde_json::to_vec(&serde_json::json!({
        "canonical_hash_version": 1,
        "symbol": symbol,
        "strategy": strategy,
        "profile_id": profile_id,
        "parameters": canonical_parameters,
    }))
    .ok()?;
    use sha2::Digest;
    Some(format!("sha256:v1:{:x}", sha2::Sha256::digest(encoded)))
}

fn canonical_lifecycle_identity_for_reservation(
    settings: &mut Settings,
    decision_key: &str,
    profile_id: Option<String>,
    profile_hash: Option<String>,
    profile_version: Option<u64>,
    exploratory: bool,
    existing_trade_id: Option<String>,
) -> Option<CanonicalLifecycleIdentity> {
    if let Some(existing) = settings.canonical_lifecycle_links.get(decision_key) {
        return Some(existing.clone());
    }
    if settings.canonical_lifecycle_epoch.is_empty() {
        settings.canonical_lifecycle_epoch = new_canonical_lifecycle_epoch();
    }
    settings.canonical_lifecycle_sequence = settings.canonical_lifecycle_sequence.checked_add(1)?;
    let sequence = settings.canonical_lifecycle_sequence;
    let epoch = settings.canonical_lifecycle_epoch.as_str();
    let identity = CanonicalLifecycleIdentity {
        schema_version: 1,
        opportunity_id: format!("autotrader:v1:opportunity:{epoch}:{sequence}"),
        // V1 starts one episode for each accepted decision thesis. Continuation
        // and reset grouping can extend this only after signal semantics define
        // those boundaries explicitly.
        episode_id: format!("autotrader:v1:episode:{epoch}:{sequence}"),
        trade_id: existing_trade_id
            .unwrap_or_else(|| format!("autotrader:v1:trade:{epoch}:{sequence}")),
        profile_id,
        profile_hash,
        profile_version,
        execution_mode: "paper".to_owned(),
        entry_policy: if exploratory { "exploratory" } else { "strict" }.to_owned(),
        source: "auto_trader_entry_reservation".to_owned(),
    };
    settings
        .canonical_lifecycle_links
        .insert(decision_key.to_owned(), identity.clone());
    Some(identity)
}

fn prune_canonical_lifecycle_links(settings: &mut Settings) {
    let mut retained_keys = settings
        .orders
        .iter()
        .filter_map(|order| order.decision_id.clone())
        .collect::<BTreeSet<_>>();
    let mut retain_position = |position: &TraderPosition| {
        if position.quantity.abs() > 1e-12 {
            retained_keys.extend(position.entry_decision_id.iter().cloned());
            retained_keys.extend(position.trade_id.iter().cloned());
        }
    };
    for position in settings.positions.values() {
        retain_position(position);
    }
    for position in settings
        .strategy_positions
        .values()
        .flat_map(|lots| lots.values())
    {
        retain_position(position);
    }
    settings
        .canonical_lifecycle_links
        .retain(|key, _| retained_keys.contains(key));
}

#[cfg(test)]
mod canonical_lifecycle_identity_tests {
    use super::*;

    #[test]
    fn reservation_identity_is_idempotent_and_survives_settings_restart() {
        let mut settings = Settings::default();
        let first = canonical_lifecycle_identity_for_reservation(
            &mut settings,
            "decision-a",
            Some("profile-a".into()),
            None,
            None,
            false,
            None,
        )
        .unwrap();
        let persisted = serde_json::to_value(&settings).unwrap();
        let mut restarted: Settings = serde_json::from_value(persisted).unwrap();
        let retried = canonical_lifecycle_identity_for_reservation(
            &mut restarted,
            "decision-a",
            Some("profile-a".into()),
            None,
            None,
            false,
            None,
        )
        .unwrap();
        let next = canonical_lifecycle_identity_for_reservation(
            &mut restarted,
            "decision-b",
            None,
            None,
            None,
            true,
            None,
        )
        .unwrap();
        assert_eq!(first, retried);
        assert_ne!(first.opportunity_id, next.opportunity_id);
        assert_ne!(first.episode_id, next.episode_id);
        assert_eq!(restarted.canonical_lifecycle_sequence, 2);
    }

    #[test]
    fn canonical_profile_hash_is_versioned_deterministic_and_requires_full_profile() {
        let kind = Strategy::LiveMomentum;
        let parameters = kind
            .parameter_specs()
            .into_iter()
            .map(|spec| (spec.key.as_str().to_owned(), spec.default))
            .collect::<BTreeMap<_, _>>();
        let first =
            canonical_profile_hash_v1("BTC-USDT", kind.as_str(), "profile-1", &parameters).unwrap();
        let second =
            canonical_profile_hash_v1("BTC-USDT", kind.as_str(), "profile-1", &parameters).unwrap();
        assert_eq!(first, second);
        assert!(first.starts_with("sha256:v1:"));
        let mut incomplete = parameters.clone();
        incomplete.pop_first();
        assert_eq!(
            canonical_profile_hash_v1("BTC-USDT", kind.as_str(), "profile-1", &incomplete),
            None
        );
        let mut invalid = parameters;
        let key = invalid.keys().next().unwrap().clone();
        invalid.insert(key, f64::NAN);
        assert_eq!(
            canonical_profile_hash_v1("BTC-USDT", kind.as_str(), "profile-1", &invalid),
            None
        );
    }

    #[test]
    fn legacy_settings_keep_absent_lifecycle_links_unattributed() {
        let settings: Settings = serde_json::from_value(serde_json::json!({
            "enabled": true, "orders": []
        }))
        .unwrap();
        assert!(settings.canonical_lifecycle_links.is_empty());
        assert_eq!(settings.canonical_lifecycle_sequence, 0);
    }
}

#[cfg(test)]
mod strategy_settings_migration_tests {
    use super::*;

    #[test]
    fn missing_learning_queue_telemetry_fields_migrate_to_defaults() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("learning_feedback_stats");
        value
            .as_object_mut()
            .unwrap()
            .remove("profile_registration_enqueued_at_ms");
        value.as_object_mut().unwrap().remove("closed_trade_audit");
        value["closed_trade_returns_percent"] = serde_json::json!([-0.5]);
        value["closed_trade_timestamps_ms"] = serde_json::json!([123]);
        let settings: Settings = serde_json::from_value(value).unwrap();
        assert!(settings.learning_feedback_stats.is_empty());
        assert!(settings.profile_registration_enqueued_at_ms.is_empty());
        assert!(settings.closed_trade_audit.is_empty());
        assert_eq!(settings.closed_trade_returns_percent, vec![-0.5]);
        assert_eq!(settings.closed_trade_timestamps_ms, vec![123]);
    }

    #[test]
    fn expands_removed_live_insights_setting_into_component_strategies() {
        let mut value = serde_json::to_value(Settings::default()).unwrap();
        value["strategies"] = serde_json::json!(["live-insights"]);
        let settings: Settings = serde_json::from_value(value).unwrap();
        assert_eq!(
            settings.strategies,
            vec![
                Strategy::LiveMomentum,
                Strategy::LiveTradeFlow,
                Strategy::LiveOrderBook,
                Strategy::LiveVwapDislocation,
                Strategy::LiveAbsorptionPressure,
                Strategy::LiveRangeMeanReversion,
            ]
        );
    }
}
