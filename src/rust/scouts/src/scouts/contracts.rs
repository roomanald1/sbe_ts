/// Scout scout planning, markout feedback, and lifecycle coordination, separate from trade execution.

use axum::{
    Json, Router,
    extract::{OriginalUri, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;
use strategies::FeedbackEvent;
pub use strategies::ParameterSpec;
use tokio::sync::Mutex;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ParameterVariantsRequest {
    pub parameters: BTreeMap<String, f64>,
    pub specs: Vec<ParameterSpec>,
    pub day: u64,
}

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
}

impl Default for ScoutConfiguration {
    fn default() -> Self {
        Self { strategies: Vec::new(), max_active_samples: 5, max_active_positions: 100 }
    }
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

pub async fn request_scout_entry_selection(
    base_url: &str,
    request: &ScoutEntrySelectionRequest,
) -> Result<Option<usize>, String> {
    reqwest::Client::new()
        .post(format!("{}/select-entry", base_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_millis(250))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<ScoutEntrySelectionResponse>()
        .await
        .map(|response| response.candidate_index)
        .map_err(|error| error.to_string())
}

pub async fn request_live_candidates(
    base_url: &str,
    request: &LiveScoutCandidatesRequest,
) -> Result<Vec<serde_json::Value>, String> {
    reqwest::Client::new()
        .post(format!(
            "{}/live-candidates",
            base_url.trim_end_matches('/')
        ))
        .timeout(std::time::Duration::from_secs(3))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json()
        .await
        .map_err(|error| error.to_string())
}

#[derive(Debug, Deserialize, Serialize)]
struct ParameterVariantsResponse {
    variants: Option<Vec<ParameterVariant>>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ParameterVariantsBatchResponse {
    responses: Vec<ParameterVariantsResponse>,
}

/// Generate parameter variants in the Scout process, keeping bounded sweep
/// work out of AutoTrader's signal and order cycle.
pub async fn request_parameter_variants(
    base_url: &str,
    request: &ParameterVariantsRequest,
) -> Result<Option<Vec<ParameterVariant>>, String> {
    reqwest::Client::new()
        .post(format!("{}/variants", base_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(2))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<ParameterVariantsResponse>()
        .await
        .map(|response| response.variants)
        .map_err(|error| error.to_string())
}

pub async fn request_parameter_variants_batch(
    base_url: &str,
    request: &ParameterVariantsBatchRequest,
) -> Result<Vec<Option<Vec<ParameterVariant>>>, String> {
    #[derive(Deserialize)]
    struct Response {
        responses: Vec<ParameterVariantsResponse>,
    }
    reqwest::Client::new()
        .post(format!("{}/variants/batch", base_url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(5))
        .json(request)
        .send()
        .await
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?
        .json::<Response>()
        .await
        .map(|response| {
            response
                .responses
                .into_iter()
                .map(|item| item.variants)
                .collect()
        })
        .map_err(|error| error.to_string())
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

