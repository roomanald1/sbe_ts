use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ExecutionLearningStat {
    pub samples: usize,
    pub filled_samples: usize,
    pub fill_rate_total: f64,
    pub cost_notional: f64,
    pub all_in_cost_notional_bps: f64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionOutcomeEvent {
    pub event_id: String,
    pub observed_at_ms: u64,
    pub context: String,
    pub order_type: String,
    pub filled: bool,
    pub fill_rate: f64,
    pub cost_notional: f64,
    pub all_in_cost_notional_bps: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionLearningRequest {
    pub context: String,
    pub baseline_order_type: String,
    pub order_types: Vec<String>,
    pub profile_count: usize,
    pub risk_appetite: f64,
    pub allow_order_type_adaptation: bool,
    pub explore_profile_order_types: Vec<String>,
    pub learning: BTreeMap<String, ExecutionLearningStat>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ExecutionLearningResponse {
    pub learned_order_type: Option<String>,
    pub schedule_depth_threshold: f64,
    pub schedule_threshold_samples: usize,
    pub parameter_profiles: BTreeMap<String, usize>,
}
