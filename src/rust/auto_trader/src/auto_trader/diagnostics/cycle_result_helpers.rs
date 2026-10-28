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
