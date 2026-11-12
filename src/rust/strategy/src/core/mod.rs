//! Shared strategy contracts and runtime learning DTOs.
//!
//! This module preserves the public API formerly provided by the strategy_core crate.

pub mod context;
pub mod contracts;
pub mod dated_carry;
pub mod execution;
pub mod feedback;
pub mod profiles;

pub use context::{
    ContextLearningStat, ContextMissedOpportunityBand, ContextThresholdInput,
    ContextThresholdRequest, ContextThresholdResponse, ContextThresholds,
};
pub use contracts::{StrategyContract, StrategyDecision, StrategyId, StrategyParameterKey};
pub use dated_carry::{
    DATED_CARRY_COST_BUDGET_PERCENT, DATED_CARRY_FUTURE_SIGNAL_TOLERANCE_MS,
    DatedCarryEntryCandidate, dated_carry_entry_is_eligible,
};
pub use execution::{
    ExecutionLearningRequest, ExecutionLearningResponse, ExecutionLearningStat,
    ExecutionOutcomeEvent,
};
pub use feedback::{
    FeedbackEvent, LearnedProfileSnapshot, ScoutComparisonMetadata, ScoutParameterVariant,
    StrategyThresholdObservation, TradeReplayContext,
};
pub use profiles::{ParameterSpec, StrategyProfile, StrategyProfileRegistration, profile_id};

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn strategy_wire_names_round_trip() {
        let profile = StrategyProfile {
            profile_id: "p1".into(),
            version: 1,
            symbol: "BTC-USDT".into(),
            strategy: "live-momentum".into(),
            parameters: BTreeMap::new(),
            learning_samples: 0,
            positive_samples: 0,
            realized_pnl: 0.0,
        };
        let encoded = serde_json::to_string(&profile).unwrap();
        let decoded: StrategyProfile = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn parameter_keys_keep_the_persisted_snake_case_name() {
        assert_eq!(
            serde_json::to_string(&StrategyParameterKey::MinimumMomentum).unwrap(),
            "\"minimum_momentum\""
        );
        assert_eq!(
            StrategyParameterKey::MinimumMomentum.as_str(),
            "minimum_momentum"
        );
    }
}
