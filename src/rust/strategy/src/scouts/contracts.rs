/// Scout scout planning, markout feedback, and lifecycle coordination, separate from trade execution.

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use rusqlite::OptionalExtension;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use strategies::FeedbackEvent;
pub use strategies::ParameterSpec;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterVariant {
    pub key: String,
    pub value: f64,
    pub changes: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterVariantsBatchRequest {
    pub requests: Vec<ParameterVariantsRequest>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterVariantsResponse {
    pub variants: Option<Vec<ParameterVariant>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterVariantsBatchResponse {
    pub responses: Vec<ParameterVariantsResponse>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveScoutProfile {
    pub symbol: String,
    pub strategy: String,
    pub parameters: BTreeMap<String, f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveScoutCandidatesRequest {
    pub signals: Vec<serde_json::Value>,
    pub profiles: Vec<LiveScoutProfile>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct ScoutConfiguration {
    pub strategies: Vec<String>,
    pub max_active_samples: usize,
    pub max_active_positions: usize,
    pub max_sample_notional: f64,
    pub stop_loss_percent: f64,
    pub take_profit_percent: f64,
}

impl Default for ScoutConfiguration {
    fn default() -> Self {
        Self {
            strategies: vec![
                "live-momentum".into(),
                "live-trade-flow".into(),
                "live-order-book".into(),
                "live-vwap-dislocation".into(),
                "live-absorption-pressure".into(),
                "donchian-55-btc200".into(),
                "donchian-20-sma200".into(),
                "donchian-20-btc200".into(),
                "donchian-10-btc200".into(),
                "donchian-20-btc200-hold3".into(),
                "risk-adjusted-trend-portfolio".into(),
                "trend-sleeves-btc200".into(),
                "trend-50-200".into(),
                "relative-strength-90-sma200".into(),
                "risk-adjusted-monthly-momentum".into(),
                "rsi2-oversold-rebound".into(),
                "dated-futures-carry".into(),
            ],
            max_active_samples: 5_000,
            max_active_positions: 5_000,
            max_sample_notional: 100.0,
            stop_loss_percent: 0.5,
            take_profit_percent: 0.8,
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoutStatus {
    pub active_samples: usize,
    pub learning_samples: usize,
    pub positive_samples: usize,
    pub win_rate: Option<f64>,
    pub realized_pnl_usd: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DailyScoutSignalsRequest {
    pub request_id: String,
    pub now_ms: u64,
    pub signals: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveScoutSignalsRequest {
    pub signals: Vec<serde_json::Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutEntrySelectionRequest {
    pub candidates: Vec<serde_json::Value>,
    pub walk_forward: Vec<serde_json::Value>,
    pub strategy_parameter_specs: BTreeMap<String, Vec<ParameterSpec>>,
    pub active_scout_count: usize,
    pub scout_max_active_positions: usize,
    pub risk_appetite: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ScoutEntrySelectionResponse {
    candidate_index: Option<usize>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StrategyEvidenceRequest {
    pub candidates: Vec<StrategyEvidenceCandidate>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StrategyWalkForwardEvidence {
    pub symbol: String,
    pub strategy: String,
    pub sample_count: usize,
    pub positive_net_rate: f64,
    pub average_net_return_percent: f64,
    pub last_sample_timestamp_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutSample {
    pub event_id: String,
    pub opened_at_ms: u64,
    pub symbol: String,
    pub strategy: String,
    pub profile_id: String,
    pub parameters: BTreeMap<String, f64>,
    pub specs: Vec<ParameterSpec>,
    pub entry_price: f64,
    pub side: i8,
}

/// Fee-aware markout produced by Insights. This intentionally remains wire
/// compatible with AutoTrader's scout callback, with specs added optionally.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutComparisonZeroSide {
    pub comparison_id: String,
    pub parameter_key: String,
    pub parameter_value: f64,
    pub profile_symbol: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutOutcomeRequest {
    pub decision_id: String,
    pub learning_context: String,
    pub simulated_net_pnl_usd: f64,
    pub net_return_percent: f64,
    pub signal_timestamp_ms: u64,
    pub evaluated_timestamp_ms: u64,
    pub taker_fee_rate: f64,
    pub symbol: String,
    pub strategy: String,
    #[serde(default)]
    pub strategy_parameters: BTreeMap<String, f64>,
    pub exit_scenario: String,
    #[serde(default)]
    pub sample_group_id: String,
    #[serde(default)]
    pub scout_parameter_key: Option<String>,
    #[serde(default)]
    pub scout_parameter_value: Option<f64>,
    #[serde(default)]
    pub scout_parameter_profile_symbol: Option<String>,
    #[serde(default)]
    pub scout_comparison_id: Option<String>,
    #[serde(default)]
    pub scout_comparison_ids: Vec<String>,
    #[serde(default)]
    pub scout_comparison_zero_sides: Vec<ScoutComparisonZeroSide>,
    #[serde(default)]
    pub scout_comparison_role: Option<String>,
    #[serde(default)]
    pub scout_comparison_zero_role: Option<String>,
    #[serde(default)]
    pub scout_comparison_parameter_key: Option<String>,
    #[serde(default)]
    pub scout_comparison_parameter_value: Option<f64>,
    #[serde(default)]
    pub scout_comparison_parameter_profile_symbol: Option<String>,
    #[serde(default)]
    pub strategy_parameter_specs: Vec<ParameterSpec>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoutMarkoutRequest {
    pub decision_id: String,
    pub symbol: String,
    pub signal_timestamp_ms: u64,
    pub evaluated_timestamp_ms: u64,
    pub taker_fee_rate: f64,
    #[serde(default)]
    pub observation_only: bool,
    #[serde(default)]
    pub horizon_seconds: u64,
    pub long_net_percent: Option<f64>,
    pub short_net_percent: Option<f64>,
    #[serde(default)]
    pub reference_entry_price: Option<f64>,
    #[serde(default)]
    pub reference_exit_price: Option<f64>,
    #[serde(default)]
    pub entry_bid: Option<f64>,
    #[serde(default)]
    pub entry_ask: Option<f64>,
    #[serde(default)]
    pub exit_bid: Option<f64>,
    #[serde(default)]
    pub exit_ask: Option<f64>,
    #[serde(default)]
    pub long_gross_percent: Option<f64>,
    #[serde(default)]
    pub short_gross_percent: Option<f64>,
    #[serde(default)]
    pub path_high: Option<f64>,
    #[serde(default)]
    pub path_low: Option<f64>,
    #[serde(default)]
    pub long_time_to_positive_net_seconds: Option<u64>,
    #[serde(default)]
    pub short_time_to_positive_net_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct ScoutPair {
    baseline: Option<ScoutOutcomeRequest>,
    variant: Option<ScoutOutcomeRequest>,
    updated_at_ms: u64,
    #[serde(default)]
    legacy_baseline_return_percent: Option<f64>,
    #[serde(default)]
    legacy_variant_return_percent: Option<f64>,
    #[serde(default)]
    legacy_learning_key: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct PersistedState {
    #[serde(default)]
    scout_configuration: ScoutConfiguration,
    #[serde(default)]
    seen: BTreeSet<String>,
    #[serde(default)]
    outcomes: BTreeMap<String, ScoutOutcomeRequest>,
    #[serde(skip)]
    strategy_outcomes_by_profile:
        Arc<std::sync::RwLock<BTreeMap<String, Vec<StrategyOutcomeProfileIndexEntry>>>>,
    #[serde(skip)]
    portfolio_instances_by_group:
        Arc<std::sync::RwLock<BTreeMap<String, Vec<PortfolioInstanceEvidence>>>>,
    #[serde(default)]
    scout_instances: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pairs: BTreeMap<String, ScoutPair>,
    #[serde(default)]
    outbox: Vec<ScoutOutboxItem>,
    #[serde(default, alias = "shadow_sample_at_ms")]
    scout_sample_at_ms: BTreeMap<String, u64>,
    #[serde(default, alias = "shadow_sample_active_scenarios")]
    scout_sample_active_scenarios: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    scout_variant_rotation_cursor: usize,
    #[serde(default, alias = "shadow_lifecycle_updated_at_ms")]
    scout_lifecycle_updated_at_ms: u64,
    #[serde(default, alias = "shadow_plan_profiles")]
    scout_plan_profiles: BTreeMap<String, String>,
    #[serde(default, alias = "shadow_plan_results")]
    scout_plan_results: BTreeMap<String, ScoutPlanResponse>,
    #[serde(default, alias = "shadow_pending_samples")]
    scout_pending_samples: BTreeMap<String, ScoutSamplePayload>,
    #[serde(default, alias = "shadow_observation_results")]
    scout_observation_results: BTreeMap<String, ScoutLifecycleObserveResponse>,
    #[serde(default, alias = "shadow_active_audits")]
    scout_active_audits: BTreeMap<String, Vec<serde_json::Value>>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
enum ScoutOutboxItem {
    Learning(FeedbackEvent),
    Audit(serde_json::Value),
    Registration(serde_json::Value),
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ScoutLifecycleSnapshot {
    #[serde(default)]
    pub updated_at_ms: u64,
    #[serde(default, alias = "shadow_sample_at_ms")]
    pub scout_sample_at_ms: BTreeMap<String, u64>,
    #[serde(default, alias = "shadow_sample_active_scenarios")]
    pub scout_sample_active_scenarios: BTreeMap<String, BTreeSet<String>>,
    #[serde(default)]
    pub scout_variant_rotation_cursor: usize,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ScoutPlanCandidate {
    #[serde(default)]
    pub generate_group_id: bool,
    pub sample_group_id: String,
    pub cooldown_key: String,
    pub profile_key: String,
    #[serde(default)]
    pub eligible: bool,
    #[serde(default)]
    pub parameter_variant: bool,
    pub cooldown_ms: u64,
    pub audit_plans: Vec<serde_json::Value>,
    #[serde(default)]
    pub audit_templates: Vec<ScoutAuditTemplate>,
    #[serde(default)]
    pub sample_payload: Option<ScoutSamplePayload>,
    #[serde(default)]
    pub daily_policy: Option<DailyScoutPolicy>,
    #[serde(default)]
    pub live_policy: Option<LiveScoutPolicy>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DailyScoutPolicy {
    pub evaluator_decision: String,
    pub selected_scout: bool,
    pub strategy_candidate: bool,
    pub daily_features_present: bool,
    pub independent_non_donchian_strategy: bool,
    #[serde(default)]
    pub parameter_key: Option<String>,
    #[serde(default)]
    pub parameter_value: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LiveScoutPolicy {
    pub now_ms: u64,
    #[serde(default)]
    pub strategy_enabled: bool,
    #[serde(default)]
    pub book_updated_at_ms: Option<u64>,
    #[serde(default)]
    pub best_bid: Option<f64>,
    #[serde(default)]
    pub best_ask: Option<f64>,
}
