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
    #[serde(default)]
    positions: BTreeMap<String, TraderPosition>,
    #[serde(default)]
    strategy_positions: BTreeMap<String, BTreeMap<String, TraderPosition>>,
    #[serde(default)]
    learning: BTreeMap<String, LearningStats>,
    #[serde(default)]
    context_learning_imported: bool,
    #[serde(default)]
    feedback_outbox: Vec<strategies::FeedbackEvent>,
    #[serde(default)]
    post_exit_observations: Vec<serde_json::Value>,
    #[serde(default)]
    exit_timing_feedback_outbox: Vec<serde_json::Value>,
    #[serde(default)]
    profile_registration_outbox: Vec<strategies::StrategyProfileRegistration>,
    #[serde(default)]
    registered_profile_ids: BTreeSet<String>,
    #[serde(default)]
    learned_profiles: BTreeMap<String, strategies::StrategyProfile>,
    #[serde(default)]
    learned_scout_parameter_variants: Vec<strategies::ScoutParameterVariant>,
    #[serde(default)]
    learned_strategy_size_multipliers: BTreeMap<String, f64>,
    #[serde(default)]
    learned_context_stats: BTreeMap<String, strategies::ContextLearningStat>,
    #[serde(default)]
    learned_execution_stats: BTreeMap<String, strategies::ExecutionLearningStat>,
    #[serde(default)]
    execution_learning: BTreeMap<String, ExecutionLearningStats>,
    #[serde(default)]
    execution_learning_imported: bool,
    #[serde(default)]
    execution_learning_order_ids: BTreeSet<String>,
    #[serde(default)]
    execution_feedback_outbox: Vec<strategies::ExecutionOutcomeEvent>,
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
    reentry_cooldown_until_ms: BTreeMap<String, u64>,
    // Persisted so a service restart cannot replay the same closed-candle signal.
    #[serde(default)]
    daily_strategy_signal_cursors: BTreeMap<String, u64>,
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
fn default_max_open_positions() -> usize { 20 }
fn default_max_intraday_open_positions() -> usize { DEFAULT_MAX_INTRADAY_OPEN_POSITIONS }
fn default_max_daily_open_positions() -> usize { DEFAULT_MAX_DAILY_OPEN_POSITIONS }
fn default_intraday_notional_limit() -> f64 { DEFAULT_INTRADAY_NOTIONAL_LIMIT }
fn default_daily_strategy_notional_limit() -> f64 { DEFAULT_DAILY_STRATEGY_NOTIONAL_LIMIT }
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
fn default_live_scout_parameter_samples() -> usize { 3 }

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
            closed_trade_metrics_version: 3,
            daily_realized_pnl: 0.0,
            pnl_utc_day: utc_day(),
            utc_day: utc_day(),
            orders: Vec::new(),
            positions: BTreeMap::new(),
            strategy_positions: BTreeMap::new(),
            learning: BTreeMap::new(),
            context_learning_imported: false,
            feedback_outbox: Vec::new(),
            post_exit_observations: Vec::new(),
            exit_timing_feedback_outbox: Vec::new(),
            profile_registration_outbox: Vec::new(),
            registered_profile_ids: BTreeSet::new(),
            learned_profiles: BTreeMap::new(),
            learned_scout_parameter_variants: Vec::new(),
            learned_strategy_size_multipliers: BTreeMap::new(),
            learned_context_stats: BTreeMap::new(),
            learned_execution_stats: BTreeMap::new(),
            execution_learning: BTreeMap::new(),
            execution_learning_imported: false,
            execution_learning_order_ids: BTreeSet::new(),
            execution_feedback_outbox: Vec::new(),
            strategy_parameter_overrides: BTreeMap::new(),
            last_hedge_at: BTreeMap::new(),
            last_decision: String::new(),
            last_decision_at: 0,
            last_risk_note: String::new(),
            risk_limited_sizing: false,
            last_signal_at: BTreeMap::new(),
            last_signal_direction: BTreeMap::new(),
            last_signal_strength: BTreeMap::new(),
            reentry_cooldown_until_ms: BTreeMap::new(),
            daily_strategy_signal_cursors: BTreeMap::new(),
        }
    }
}

#[cfg(test)]
mod strategy_settings_migration_tests {
    use super::*;

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
            ]
        );
    }
}
