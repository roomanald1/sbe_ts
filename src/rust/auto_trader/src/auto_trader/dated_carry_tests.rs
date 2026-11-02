use super::strategy_dated_futures_carry::evaluate::evaluate;
use super::*;
use std::collections::BTreeMap;

fn candidate(
    evaluated_at_ms: u64,
    basis_percent: f64,
    expiry_days: f64,
) -> DatedCarryCandidate {
    let spot_ask = 100.0;
    let future_bid = spot_ask * (1.0 + basis_percent / 100.0);
    let expiry_ms = evaluated_at_ms + (expiry_days * 86_400_000.0) as u64;
    let actual_basis = (future_bid / spot_ask - 1.0) * 100.0;
    DatedCarryCandidate {
        spot_symbol: "BTCUSDT".into(),
        future_symbol: format!("BTCUSDT_{}", expiry_ms),
        expiry_ms,
        days_to_expiry: expiry_days,
        quantity: 0.01,
        spot_ask,
        future_bid,
        spot_bid: 99.9,
        future_ask: future_bid + 0.1,
        basis_percent: actual_basis,
        net_after_budget_percent: actual_basis - CARRY_COST_GATE_PERCENT,
        eligible: true,
        reason: "test candidate".into(),
    }
}

#[test]
fn entry_requires_executable_spot_future_edge_above_cost_and_seven_days_to_expiry() {
    let parameters = BTreeMap::new();
    let now = 1_000 * 86_400_000;
    let eligible = candidate(now, 0.60, 8.0);
    let at_cost_floor = candidate(now, 0.50, 8.0);
    let too_close = candidate(now, 0.80, 6.99);
    let entry = |candidate: &DatedCarryCandidate| {
        let input = StrategySignal::dated_carry_entry(candidate, now, now, &parameters);
        evaluate(&input).unwrap()
    };
    assert_eq!(entry(&eligible), StrategyDecision::Enter);
    assert_eq!(entry(&at_cost_floor), StrategyDecision::Reject);
    assert_eq!(entry(&too_close), StrategyDecision::Reject);
}

#[test]
fn entry_rejects_prices_that_do_not_match_the_published_basis() {
    let parameters = BTreeMap::new();
    let now = 1_000 * 86_400_000;
    let mut invalid = candidate(now, 0.70, 9.0);
    invalid.future_bid = invalid.spot_ask * 1.001;
    let input = StrategySignal::dated_carry_entry(&invalid, now, now, &parameters);
    assert_eq!(evaluate(&input).unwrap(), StrategyDecision::Reject);
}

#[test]
fn exit_requires_a_valid_settlement_signal_for_an_expired_contract() {
    let parameters = BTreeMap::new();
    let now = 1_010 * 86_400_000;
    let expiry = now - 1;
    let settlement = |future: &'static str, price: f64, source| {
        StrategySignal::dated_carry_settlement(
            now,
            now,
            "BTCUSDT",
            future,
            expiry,
            101.0,
            price,
            source,
            &parameters,
        )
    };
    let contract = "BTCUSDT_Q1";
    assert_eq!(
        evaluate(&settlement(
            contract,
            100.0,
            Some(DatedCarrySettlementSource::Official)
        ))
        .unwrap(),
        StrategyDecision::Exit,
    );
    assert_eq!(
        evaluate(&settlement(
            contract,
            0.0,
            Some(DatedCarrySettlementSource::Official)
        ))
        .unwrap(),
        StrategyDecision::Hold,
    );
    assert_eq!(
        evaluate(&settlement(contract, 100.0, None)).unwrap(),
        StrategyDecision::Hold,
    );
    assert_eq!(
        evaluate(&settlement(
            "ETHUSDT_1",
            100.0,
            Some(DatedCarrySettlementSource::Official)
        ))
        .unwrap(),
        StrategyDecision::Hold,
    );
}
