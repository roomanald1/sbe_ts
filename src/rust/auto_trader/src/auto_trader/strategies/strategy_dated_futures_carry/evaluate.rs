use crate::auto_trader::{
    strategy_parameter_value, DatedCarryCandidate, DatedCarrySettlementSource, Strategy,
    StrategyDecision, StrategyParameterKey, StrategySignal, StrategySpecificSignal,
    CARRY_COST_GATE_PERCENT,
};

const CARRY_METRIC_TOLERANCE: f64 = 1e-6;
const FUTURE_SIGNAL_TOLERANCE_MS: u64 = 5_000;

pub(crate) fn evaluate(input: &StrategySignal<'_>) -> Result<StrategyDecision, String> {
    if input
        .strategy
        .is_some_and(|strategy| strategy != Strategy::DatedCarry)
    {
        return Ok(StrategyDecision::Reject);
    }
    if input.position_open && input.specific_signal.is_none() {
        return Ok(StrategyDecision::Hold);
    }
    if input.strategy != Some(Strategy::DatedCarry) {
        return Ok(StrategyDecision::Reject);
    }

    match input.specific_signal {
        Some(StrategySpecificSignal::DatedCarryEntry {
            emitted_at_ms,
            evaluated_at_ms,
            candidate,
        }) if !input.position_open => {
            let (Some(minimum_days), Some(cost_gate_percent)) = (
                strategy_parameter_value(
                    input.parameters,
                    StrategyParameterKey::CarryMinimumDaysToExpiry,
                ),
                strategy_parameter_value(
                    input.parameters,
                    StrategyParameterKey::CarryCostGatePercent,
                ),
            ) else {
                return Ok(StrategyDecision::Reject);
            };
            if dated_carry_entry_is_eligible(
                candidate, emitted_at_ms, evaluated_at_ms,
                minimum_days, cost_gate_percent,
            ) {
                Ok(StrategyDecision::Enter)
            } else {
                Ok(StrategyDecision::Reject)
            }
        }
        Some(StrategySpecificSignal::DatedCarrySettlement {
            emitted_at_ms,
            evaluated_at_ms,
            spot_symbol,
            future_symbol,
            expiry_ms,
            spot_exit_twap,
            settlement_price,
            settlement_source,
        }) if input.position_open => {
            if dated_carry_settlement_is_valid(
                emitted_at_ms,
                evaluated_at_ms,
                spot_symbol,
                future_symbol,
                expiry_ms,
                spot_exit_twap,
                settlement_price,
                settlement_source,
            ) {
                Ok(StrategyDecision::Exit)
            } else {
                Ok(StrategyDecision::Hold)
            }
        }
        _ if input.position_open => Ok(StrategyDecision::Hold),
        _ => Ok(StrategyDecision::Reject),
    }
}

fn dated_carry_entry_is_eligible(
    candidate: &DatedCarryCandidate,
    emitted_at_ms: u64,
    evaluated_at_ms: u64,
    minimum_days: f64,
    cost_gate_percent: f64,
) -> bool {
    if !minimum_days.is_finite() || minimum_days < 1.0
        || !cost_gate_percent.is_finite() || cost_gate_percent < 0.0
        || emitted_at_ms > evaluated_at_ms.saturating_add(FUTURE_SIGNAL_TOLERANCE_MS)
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
    if days_to_expiry < minimum_days
        || (candidate.days_to_expiry - days_to_expiry).abs() > 0.02
    {
        return false;
    }

    // The executable carry edge is futures bid minus the spot ask, expressed
    // relative to spot cost. Recompute it instead of trusting the prefilled text fields.
    let basis_percent = (candidate.future_bid / candidate.spot_ask - 1.0) * 100.0;
    let net_after_cost_percent = candidate.net_after_budget_percent;
    let expected_net_after_budget_percent = basis_percent - CARRY_COST_GATE_PERCENT;
    candidate.eligible
        && basis_percent.is_finite()
        && net_after_cost_percent.is_finite()
        && net_after_cost_percent >= cost_gate_percent
        && (candidate.basis_percent - basis_percent).abs() <= CARRY_METRIC_TOLERANCE
        && (candidate.net_after_budget_percent - expected_net_after_budget_percent).abs()
            <= CARRY_METRIC_TOLERANCE
}

fn dated_carry_settlement_is_valid(
    emitted_at_ms: u64,
    evaluated_at_ms: u64,
    spot_symbol: &str,
    future_symbol: &str,
    expiry_ms: u64,
    spot_exit_twap: f64,
    settlement_price: f64,
    settlement_source: Option<DatedCarrySettlementSource>,
) -> bool {
    emitted_at_ms <= evaluated_at_ms.saturating_add(FUTURE_SIGNAL_TOLERANCE_MS)
        && expiry_ms <= evaluated_at_ms
        && matches!(spot_symbol, "BTCUSDT" | "ETHUSDT")
        && future_symbol.starts_with(&format!("{spot_symbol}_"))
        && spot_exit_twap.is_finite()
        && settlement_price.is_finite()
        && spot_exit_twap > 0.0
        && settlement_price > 0.0
        && settlement_source.is_some()
}

pub(super) fn should_exit_when_disabled(_enabled: bool) -> bool {
    false
}

pub(super) fn description() -> &'static str {
    "spot TWAP over the 30-minute settlement window and futures cash settlement"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auto_trader::{DatedCarryCandidate, StrategySpecificSignal};
    use std::collections::BTreeMap;
    use std::sync::OnceLock;

    const NOW_MS: u64 = 1_700_000_000_000;
    const DAY_MS: u64 = 86_400_000;

    fn candidate() -> DatedCarryCandidate {
        DatedCarryCandidate {
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
            net_after_budget_percent: 1.0 - CARRY_COST_GATE_PERCENT,
            eligible: true,
            reason: "eligible".into(),
        }
    }

    fn parameters() -> &'static BTreeMap<String, f64> {
        static PARAMETERS: OnceLock<BTreeMap<String, f64>> = OnceLock::new();
        PARAMETERS.get_or_init(|| {
            BTreeMap::from([
                ("carry_minimum_days_to_expiry".to_owned(), 7.0),
                ("carry_cost_gate_percent".to_owned(), 0.5),
            ])
        })
    }

    fn input<'a>(
        specific_signal: StrategySpecificSignal<'a>,
        position_open: bool,
    ) -> StrategySignal<'a> {
        StrategySignal {
            signal: None,
            specific_signal: Some(specific_signal),
            daily_features: None,
            direction: None,
            strategy: Some(Strategy::DatedCarry),
            parameters: parameters(),
            position_open,
            strategy_enabled: true,
            signal_active: None,
            rebalance_required: false,
        }
    }

    fn entry(candidate: &DatedCarryCandidate) -> StrategySpecificSignal<'_> {
        StrategySpecificSignal::DatedCarryEntry {
            emitted_at_ms: NOW_MS,
            evaluated_at_ms: NOW_MS,
            candidate,
        }
    }

    #[test]
    fn entry_requires_a_consistent_eligible_candidate() {
        let good = candidate();
        assert_eq!(
            evaluate(&input(entry(&good), false)).unwrap(),
            StrategyDecision::Enter
        );

        let mut ineligible = candidate();
        ineligible.eligible = false;
        assert_eq!(
            evaluate(&input(entry(&ineligible), false)).unwrap(),
            StrategyDecision::Reject
        );

        let mut below_cost = candidate();
        below_cost.future_bid = 100.4;
        below_cost.basis_percent = 0.4;
        below_cost.net_after_budget_percent = -0.1;
        assert_eq!(
            evaluate(&input(entry(&below_cost), false)).unwrap(),
            StrategyDecision::Reject
        );
    }

    #[test]
    fn entry_uses_profile_edge_gate_and_rejects_missing_parameters() {
        let mut candidate = candidate();
        candidate.future_bid = 100.95;
        candidate.basis_percent = 0.95;
        candidate.net_after_budget_percent = 0.45;
        let lower_gate = BTreeMap::from([
            ("carry_minimum_days_to_expiry".to_owned(), 7.0),
            ("carry_cost_gate_percent".to_owned(), 0.4),
        ]);
        let mut lower_gate_input = input(entry(&candidate), false);
        lower_gate_input.parameters = &lower_gate;
        assert_eq!(
            evaluate(&lower_gate_input).unwrap(),
            StrategyDecision::Enter
        );

        let higher_gate = BTreeMap::from([
            ("carry_minimum_days_to_expiry".to_owned(), 7.0),
            ("carry_cost_gate_percent".to_owned(), 0.5),
        ]);
        let mut higher_gate_input = input(entry(&candidate), false);
        higher_gate_input.parameters = &higher_gate;
        assert_eq!(
            evaluate(&higher_gate_input).unwrap(),
            StrategyDecision::Reject
        );

        let missing = BTreeMap::from([("carry_minimum_days_to_expiry".to_owned(), 7.0)]);
        let mut missing_input = input(entry(&candidate), false);
        missing_input.parameters = &missing;
        assert_eq!(
            evaluate(&missing_input).unwrap(),
            StrategyDecision::Reject
        );
    }

    #[test]
    fn entry_rejects_invalid_expiry_market_and_stale_metrics() {
        let mut too_soon = candidate();
        too_soon.expiry_ms = NOW_MS + 6 * DAY_MS;
        too_soon.days_to_expiry = 6.0;
        assert_eq!(
            evaluate(&input(entry(&too_soon), false)).unwrap(),
            StrategyDecision::Reject
        );

        let mut mismatched = candidate();
        mismatched.future_symbol = "ETHUSDT_20240101".into();
        assert_eq!(
            evaluate(&input(entry(&mismatched), false)).unwrap(),
            StrategyDecision::Reject
        );

        let mut stale_metric = candidate();
        stale_metric.basis_percent += 0.1;
        assert_eq!(
            evaluate(&input(entry(&stale_metric), false)).unwrap(),
            StrategyDecision::Reject
        );
    }

    #[test]
    fn settlement_exits_only_with_valid_expired_position_data() {
        let valid = StrategySpecificSignal::DatedCarrySettlement {
            emitted_at_ms: NOW_MS,
            evaluated_at_ms: NOW_MS,
            spot_symbol: "BTCUSDT",
            future_symbol: "BTCUSDT_20240101",
            expiry_ms: NOW_MS,
            spot_exit_twap: 100.0,
            settlement_price: 101.0,
            settlement_source: Some(DatedCarrySettlementSource::Official),
        };
        assert_eq!(
            evaluate(&input(valid, true)).unwrap(),
            StrategyDecision::Exit
        );

        let not_expired = StrategySpecificSignal::DatedCarrySettlement {
            emitted_at_ms: NOW_MS,
            evaluated_at_ms: NOW_MS,
            spot_symbol: "BTCUSDT",
            future_symbol: "BTCUSDT_20240101",
            expiry_ms: NOW_MS + 1,
            spot_exit_twap: 100.0,
            settlement_price: 101.0,
            settlement_source: Some(DatedCarrySettlementSource::Official),
        };
        assert_eq!(
            evaluate(&input(not_expired, true)).unwrap(),
            StrategyDecision::Hold
        );

        let missing_source = StrategySpecificSignal::DatedCarrySettlement {
            emitted_at_ms: NOW_MS,
            evaluated_at_ms: NOW_MS,
            spot_symbol: "BTCUSDT",
            future_symbol: "BTCUSDT_20240101",
            expiry_ms: NOW_MS,
            spot_exit_twap: 100.0,
            settlement_price: 101.0,
            settlement_source: None,
        };
        assert_eq!(
            evaluate(&input(missing_source, true)).unwrap(),
            StrategyDecision::Hold
        );
    }

    #[test]
    fn lifecycle_context_holds_or_rejects_mismatched_signals() {
        let candidate = candidate();
        assert_eq!(
            evaluate(&input(entry(&candidate), true)).unwrap(),
            StrategyDecision::Hold
        );

        let mut wrong_strategy = input(entry(&candidate), false);
        wrong_strategy.strategy = Some(Strategy::LiveMomentum);
        assert_eq!(evaluate(&wrong_strategy).unwrap(), StrategyDecision::Reject);
    }
}
