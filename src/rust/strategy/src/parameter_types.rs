use serde::Serialize;

/// One parameter value in an evaluated strategy profile.
#[derive(Clone, Serialize)]
pub struct StrategyParameterValue {
    pub key: String,
    pub label: String,
    pub value: f64,
    pub learned_value: f64,
    pub configured_value: Option<f64>,
    pub default_value: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: String,
    pub overridden: bool,
    pub scout_variants: Vec<StrategyParameterScoutVariant>,
}

/// Current evidence for a bounded Scout parameter variant.
#[derive(Clone, Serialize)]
pub struct StrategyParameterScoutVariant {
    pub candidate_value: f64,
    pub sample_count: usize,
    pub minimum_samples: usize,
    pub average_net_improvement_percent: f64,
    pub lower_90_one_sided_improvement_percent: Option<f64>,
    pub upper_90_one_sided_improvement_percent: Option<f64>,
    pub paired_baseline_increase_samples: usize,
    pub paired_baseline_decrease_samples: usize,
    pub paired_baseline_same_samples: usize,
    pub paired_baseline_unknown_samples: usize,
    pub paired_baseline_delta_mean: Option<f64>,
    pub paired_baseline_delta_samples: usize,
    pub promoted: bool,
    pub rejected: bool,
}

/// API view of the parameter profile selected for a symbol and strategy.
#[derive(Clone, Serialize)]
pub struct StrategyParameterProfile {
    pub symbol: String,
    pub strategy: String,
    pub learning_samples: usize,
    pub positive_rate: Option<f64>,
    pub realized_pnl: f64,
    pub parameters: Vec<StrategyParameterValue>,
}
