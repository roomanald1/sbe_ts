use super::{ContextLearningStat, ExecutionLearningStat, ParameterSpec, StrategyProfile};
use crate::StrategyWalkForwardEvidence;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct LearnedProfileSnapshot {
    pub profiles: Vec<StrategyProfile>,
    pub strategy_size_multipliers: BTreeMap<String, f64>,
    #[serde(default)]
    pub recent_profile_context_returns: BTreeMap<String, Vec<(u64, f64)>>,
    /// Stop distances selected from executable post-exit replays. Keys are the
    /// exact entering profile id and the signal's market context.
    #[serde(default)]
    pub contextual_stop_loss_percents: BTreeMap<String, f64>,
    #[serde(default)]
    pub context_learning: BTreeMap<String, ContextLearningStat>,
    #[serde(default)]
    pub execution_learning: BTreeMap<String, ExecutionLearningStat>,
    #[serde(default)]
    pub scout_parameter_variants: Vec<ScoutParameterVariant>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct ScoutParameterVariant {
    pub symbol: String,
    pub strategy: String,
    pub parameter: String,
    pub candidate_value: f64,
    pub sample_count: usize,
    pub minimum_samples: usize,
    pub average_net_improvement_percent: f64,
    pub lower_90_one_sided_improvement_percent: Option<f64>,
    #[serde(default)]
    pub upper_90_one_sided_improvement_percent: Option<f64>,
    #[serde(default)]
    pub paired_baseline_increase_samples: usize,
    #[serde(default)]
    pub paired_baseline_decrease_samples: usize,
    #[serde(default)]
    pub paired_baseline_same_samples: usize,
    #[serde(default)]
    pub paired_baseline_unknown_samples: usize,
    #[serde(default)]
    pub paired_baseline_delta_mean: Option<f64>,
    #[serde(default)]
    pub paired_baseline_delta_samples: usize,
    pub promoted: bool,
    #[serde(default)]
    pub rejected: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct StrategyThresholdObservation {
    pub parameter_key: String,
    pub threshold: f64,
    pub feature_value: f64,
}

/// Explicit attribution for a Scout outcome that is one arm of a comparison.
/// The group identifies the shared opportunity; horizon is present only for
/// horizon-based markout comparisons.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ScoutComparisonMetadata {
    Independent,
    Paired {
        group_id: String,
        horizon_seconds: Option<u64>,
    },
}

/// Feedback event envelope; event IDs make retries safe to deduplicate.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FeedbackEvent {
    MissedOpportunityCandidate {
        event_id: String,
        observed_at_ms: u64,
        symbol: String,
        strategy: String,
        profile_id: String,
        parameters: BTreeMap<String, f64>,
        direction: String,
        strength: f64,
        score: f64,
        learning_context: String,
    },
    ScoutOutcome {
        event_id: String,
        observed_at_ms: u64,
        #[serde(default)]
        learning_context: Option<String>,
        symbol: String,
        strategy: String,
        profile_id: String,
        parameters: BTreeMap<String, f64>,
        #[serde(default)]
        baseline_parameters: BTreeMap<String, f64>,
        comparison: ScoutComparisonMetadata,
        specs: Vec<ParameterSpec>,
        net_return_percent: f64,
    },
    TradeOutcome {
        event_id: String,
        observed_at_ms: u64,
        #[serde(default)]
        learning_context: Option<String>,
        symbol: String,
        strategy: String,
        profile_id: String,
        parameters: BTreeMap<String, f64>,
        specs: Vec<ParameterSpec>,
        scout: bool,
        realized_pnl_usd: f64,
        #[serde(default)]
        net_return_percent: f64,
        #[serde(default)]
        threshold_observations: Vec<StrategyThresholdObservation>,
        /// Exact close and entry facts captured by AutoTrader. Learning must
        /// report replay as unsupported if the historical strategy inputs or
        /// execution-cost evidence needed for a comparison are unavailable.
        #[serde(default)]
        replay_context: Option<TradeReplayContext>,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct TradeReplayContext {
    pub market_symbol: String,
    pub entry_timestamp_ms: u64,
    pub exit_timestamp_ms: u64,
    pub side: String,
    pub entry_price: f64,
    pub exit_price: f64,
    pub quantity: f64,
    pub entry_fees: f64,
    pub exit_fees: f64,
    #[serde(default)]
    pub entry_walk_forward_evidence: Option<StrategyWalkForwardEvidence>,
    /// Whether both sides have measured execution costs, rather than estimates.
    #[serde(default)]
    pub execution_costs_complete: bool,
}
