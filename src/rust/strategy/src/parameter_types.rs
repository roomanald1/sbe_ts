use serde::Serialize;

/// One parameter value in an evaluated strategy profile.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
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
    /// Exact Learning-owned parameter-set identity when one was published.
    /// Catalog/bootstrap rows leave this absent rather than synthesizing a
    /// learned profile identity from their defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_version: Option<u64>,
    pub learning_samples: usize,
    pub positive_rate: Option<f64>,
    pub realized_pnl: f64,
    pub walk_forward_evidence: Option<crate::StrategyWalkForwardEvidence>,
    pub walk_forward_evidence_by_regime:
        std::collections::BTreeMap<String, crate::StrategyWalkForwardEvidence>,
    pub parameters: Vec<StrategyParameterValue>,
}
