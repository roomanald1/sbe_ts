use serde::{Deserialize, Serialize};

pub const DATED_CARRY_COST_BUDGET_PERCENT: f64 = 0.5;
const DATED_CARRY_METRIC_TOLERANCE: f64 = 1e-6;
pub const DATED_CARRY_FUTURE_SIGNAL_TOLERANCE_MS: u64 = 5_000;

/// Market snapshot required to evaluate a dated-carry entry consistently
/// across services. `eligible` and the precomputed metrics are validated
/// against the executable prices before they are trusted.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct DatedCarryEntryCandidate {
    pub spot_symbol: String,
    pub future_symbol: String,
    pub expiry_ms: u64,
    pub days_to_expiry: f64,
    pub quantity: f64,
    pub spot_ask: f64,
    pub future_bid: f64,
    pub spot_bid: f64,
    pub future_ask: f64,
    pub basis_percent: f64,
    pub net_after_budget_percent: f64,
    pub eligible: bool,
}

/// Apply the shared dated-carry entry guards and profile gates.
pub fn dated_carry_entry_is_eligible(
    candidate: &DatedCarryEntryCandidate,
    emitted_at_ms: u64,
    evaluated_at_ms: u64,
    minimum_days: f64,
    cost_gate_percent: f64,
) -> bool {
    if !minimum_days.is_finite()
        || minimum_days < 1.0
        || !cost_gate_percent.is_finite()
        || cost_gate_percent < 0.0
        || emitted_at_ms > evaluated_at_ms.saturating_add(DATED_CARRY_FUTURE_SIGNAL_TOLERANCE_MS)
        || candidate.expiry_ms <= evaluated_at_ms
        || !candidate
            .future_symbol
            .starts_with(&format!("{}_", candidate.spot_symbol))
        || !matches!(candidate.spot_symbol.as_str(), "BTCUSDT" | "ETHUSDT")
        || ![
            candidate.days_to_expiry,
            candidate.quantity,
            candidate.spot_ask,
            candidate.future_bid,
            candidate.spot_bid,
            candidate.future_ask,
            candidate.basis_percent,
            candidate.net_after_budget_percent,
        ]
        .iter()
        .all(|value| value.is_finite())
        || candidate.quantity <= 0.0
        || candidate.spot_ask <= 0.0
        || candidate.future_bid <= 0.0
        || candidate.spot_bid <= 0.0
        || candidate.future_ask <= 0.0
    {
        return false;
    }

    let days_to_expiry = (candidate.expiry_ms - evaluated_at_ms) as f64 / 86_400_000.0;
    if days_to_expiry < minimum_days || (candidate.days_to_expiry - days_to_expiry).abs() > 0.02 {
        return false;
    }

    let basis_percent = (candidate.future_bid / candidate.spot_ask - 1.0) * 100.0;
    let expected_net_after_budget_percent = basis_percent - DATED_CARRY_COST_BUDGET_PERCENT;
    candidate.eligible
        && basis_percent.is_finite()
        && candidate.net_after_budget_percent >= cost_gate_percent
        && (candidate.basis_percent - basis_percent).abs() <= DATED_CARRY_METRIC_TOLERANCE
        && (candidate.net_after_budget_percent - expected_net_after_budget_percent).abs()
            <= DATED_CARRY_METRIC_TOLERANCE
}

#[cfg(test)]
mod dated_carry_entry_tests {
    use super::*;

    const NOW_MS: u64 = 1_700_000_000_000;
    const DAY_MS: u64 = 86_400_000;

    fn candidate() -> DatedCarryEntryCandidate {
        DatedCarryEntryCandidate {
            spot_symbol: "BTCUSDT".into(),
            future_symbol: "BTCUSDT_20240101".into(),
            expiry_ms: NOW_MS + 10 * DAY_MS,
            days_to_expiry: 10.0,
            quantity: 0.01,
            spot_ask: 100.0,
            future_bid: 101.0,
            spot_bid: 99.0,
            future_ask: 102.0,
            basis_percent: 1.0,
            net_after_budget_percent: 1.0 - DATED_CARRY_COST_BUDGET_PERCENT,
            eligible: true,
        }
    }

    #[test]
    fn accepts_consistent_candidate_at_learned_threshold_boundaries() {
        let candidate = candidate();
        assert!(dated_carry_entry_is_eligible(
            &candidate,
            NOW_MS,
            NOW_MS,
            10.0,
            candidate.net_after_budget_percent,
        ));
        assert!(dated_carry_entry_is_eligible(
            &candidate,
            NOW_MS + 5_000,
            NOW_MS,
            1.0,
            0.0,
        ));
    }

    #[test]
    fn rejects_candidate_when_profile_thresholds_or_market_guards_fail() {
        let candidate = candidate();
        assert!(!dated_carry_entry_is_eligible(
            &candidate, NOW_MS, NOW_MS, 10.01, 0.0
        ));
        assert!(!dated_carry_entry_is_eligible(
            &candidate, NOW_MS, NOW_MS, 0.99, 0.501
        ));
        assert!(!dated_carry_entry_is_eligible(
            &candidate,
            NOW_MS + 5_001,
            NOW_MS,
            1.0,
            0.0
        ));

        let mut invalid = candidate.clone();
        invalid.eligible = false;
        assert!(!dated_carry_entry_is_eligible(
            &invalid, NOW_MS, NOW_MS, 1.0, 0.0
        ));

        invalid = candidate.clone();
        invalid.future_symbol = "ETHUSDT_20240101".into();
        assert!(!dated_carry_entry_is_eligible(
            &invalid, NOW_MS, NOW_MS, 1.0, 0.0
        ));

        invalid = candidate;
        invalid.future_bid = f64::NAN;
        assert!(!dated_carry_entry_is_eligible(
            &invalid, NOW_MS, NOW_MS, 1.0, 0.0
        ));
    }
}
