//! Predeclared settings for the Scout strategy/profile rolling-origin replay.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutReplayTrainingWindow {
    Expanding,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutReplaySelectionObjective {
    MaximizeNetPortfolioExpectancyVersusBaseline,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoutWalkForwardReplayConfig {
    pub config_version: u32,
    pub training_window: ScoutReplayTrainingWindow,
    pub minimum_history_day_clusters: usize,
    pub minimum_effective_sample_size: f64,
    pub origin_cadence_days: u64,
    pub evaluation_window_days: u64,
    pub final_holdout_days: u64,
    pub origin_time_zone: String,
    pub selection_objective: ScoutReplaySelectionObjective,
    pub period_assignment: String,
}

impl Default for ScoutWalkForwardReplayConfig {
    fn default() -> Self {
        Self {
            config_version: 1,
            training_window: ScoutReplayTrainingWindow::Expanding,
            minimum_history_day_clusters: 30,
            minimum_effective_sample_size: 10.0,
            origin_cadence_days: 7,
            evaluation_window_days: 7,
            final_holdout_days: 28,
            origin_time_zone: "UTC".into(),
            selection_objective:
                ScoutReplaySelectionObjective::MaximizeNetPortfolioExpectancyVersusBaseline,
            period_assignment: "assign candidates by signal timestamp; train only on outcomes with evaluated timestamp strictly before the origin; allow evaluation outcomes to mature after the evaluation window".into(),
        }
    }
}

impl ScoutWalkForwardReplayConfig {
    /// Stable hash over the serialized, ordered config fields. Bump
    /// `config_version` whenever the interpretation of a field changes.
    pub fn config_hash(&self) -> String {
        let bytes = serde_json::to_vec(self).expect("replay config is serializable");
        let hash = bytes
            .into_iter()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        format!("fnv1a64:{hash:016x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_replay_config_is_predeclared_and_hash_is_deterministic() {
        let first = ScoutWalkForwardReplayConfig::default();
        let second = ScoutWalkForwardReplayConfig::default();
        assert_eq!(first.training_window, ScoutReplayTrainingWindow::Expanding);
        assert_eq!(first.minimum_history_day_clusters, 30);
        assert_eq!(first.minimum_effective_sample_size, 10.0);
        assert_eq!(first.origin_cadence_days, 7);
        assert_eq!(first.evaluation_window_days, 7);
        assert_eq!(first.final_holdout_days, 28);
        assert_eq!(first.config_hash(), second.config_hash());

        let mut changed = first.clone();
        changed.final_holdout_days += 7;
        assert_ne!(first.config_hash(), changed.config_hash());
    }
}
