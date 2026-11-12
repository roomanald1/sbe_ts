fn is_live_scout_only_signal(candidate: &Signal) -> bool {
    candidate
        .strategy_decision
        .as_ref()
        .and_then(|value| value.get("scout_only"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
        && candidate
            .strategy
            .as_deref()
            .and_then(Strategy::from_name)
            .is_some_and(|strategy| strategy.is_live_insight() && !strategy.is_legacy_composite())
}
