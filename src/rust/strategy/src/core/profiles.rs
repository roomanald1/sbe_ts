use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub fn profile_id(symbol: &str, strategy: &str, parameters: &BTreeMap<String, f64>) -> String {
    let mut hash = 14_695_981_039_346_656_037_u64;
    for byte in symbol.bytes().chain([b':']).chain(strategy.bytes()) {
        hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
    }
    for (key, value) in parameters {
        for byte in format!("{key}={value:.8};").bytes() {
            hash = (hash ^ u64::from(byte)).wrapping_mul(1_099_511_628_211);
        }
    }
    format!("{hash:016x}")
}

/// Immutable, versioned parameter set consumed by the decision hot path.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrategyProfile {
    pub profile_id: String,
    pub version: u64,
    pub symbol: String,
    pub strategy: String,
    pub parameters: BTreeMap<String, f64>,
    #[serde(default)]
    pub learning_samples: usize,
    #[serde(default)]
    pub positive_samples: usize,
    #[serde(default)]
    pub realized_pnl: f64,
}

/// Bounded tuning metadata exported with a strategy definition.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct ParameterSpec {
    pub key: String,
    pub label: String,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub unit: String,
    /// +1 raises the parameter after a positive outcome; -1 lowers it.
    pub learning_direction: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct StrategyProfileRegistration {
    pub profile: StrategyProfile,
    pub specs: Vec<ParameterSpec>,
}
