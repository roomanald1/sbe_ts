use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ContextLearningStat {
    pub samples: usize,
    pub positive: usize,
    pub realized_pnl: f64,
    #[serde(default)]
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextMissedOpportunityBand {
    pub gate: String,
    pub threshold: f64,
    pub horizon_seconds: u64,
    pub sample_count: usize,
    pub average_net_return_percent: Option<f64>,
    pub positive_net_rate: Option<f64>,
    pub time_to_profit_sample_count: usize,
    pub time_to_profit_rate: Option<f64>,
    pub average_time_to_profit_seconds: Option<f64>,
    pub stop_target_sample_count: usize,
    pub stop_target_average_net_return_percent: Option<f64>,
    pub stop_target_positive_net_rate: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextThresholdInput {
    pub context: String,
    pub missed_opportunity_bands: Vec<ContextMissedOpportunityBand>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextThresholdRequest {
    pub learning: BTreeMap<String, ContextLearningStat>,
    pub contexts: Vec<ContextThresholdInput>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextThresholds {
    pub strength_floor: f64,
    pub exploration_score_floor: f64,
    pub learning_samples: usize,
    pub positive_rate: Option<f64>,
    pub scout_outcome_samples: usize,
    pub scout_outcome_positive_rate: Option<f64>,
    pub scout_outcome_realized_pnl: f64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextThresholdResponse {
    pub thresholds: BTreeMap<String, ContextThresholds>,
}
