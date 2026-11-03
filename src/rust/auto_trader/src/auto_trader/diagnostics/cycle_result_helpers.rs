fn skipped_for_signal(signal: &Signal, reason: &str) -> CycleResult {
    let mut cycle = skipped(reason);
    cycle.symbol = Some(signal.symbol.clone());
    cycle.side = Some(signal.direction.clone());
    cycle
}
fn skipped(reason: &str) -> CycleResult {
    CycleResult {
        status: "skipped",
        reason: Some(reason.into()),
        symbol: None,
        side: None,
        quantity: None,
        order_type: None,
        order_response: None,
    }
}
fn upstream_error(error: reqwest::Error) -> (StatusCode, String) {
    (
        StatusCode::BAD_GATEWAY,
        format!("Auto trader upstream request failed: {error}"),
    )
}

#[cfg(test)]
mod cycle_result_helper_tests {
    use super::*;

    #[test]
    fn skipped_results_preserve_signal_identity_and_reason() {
        let signal = Signal {
            symbol: "ETH-USDT".into(), direction: "bearish".into(), strength: 0.8,
            timestamp: 1, stale: false, score: 0.8, ranging_probability: None,
            components: SignalComponents::default(), strategy_candidate: false,
            strategy: None, strategy_decision: None, market_regime: None,
            news_attribution: None, entry_gate: None,
        };
        let result = skipped_for_signal(&signal, "test reason");
        assert_eq!(result.status, "skipped");
        assert_eq!(result.reason.as_deref(), Some("test reason"));
        assert_eq!(result.symbol.as_deref(), Some("ETH-USDT"));
        assert_eq!(result.side.as_deref(), Some("bearish"));
        assert!(result.quantity.is_none());
        let generic = skipped("generic");
        assert_eq!(generic.reason.as_deref(), Some("generic"));
        assert!(generic.symbol.is_none());
    }
}
