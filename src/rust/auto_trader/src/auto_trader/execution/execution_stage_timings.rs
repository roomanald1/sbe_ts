fn record_execution_stage_timings(
    response: &serde_json::Value,
    submit_response_ms: f64,
    stage_timings_ms: &mut BTreeMap<String, f64>,
) {
    stage_timings_ms.insert("order_submit_response".to_owned(), submit_response_ms);
    for (response_key, stage) in [
        ("client_response_headers_ms", "client_request_to_headers"),
        ("client_request_prepare_local_ms", "order_request_prepare_local"),
        ("client_body_read_ms", "client_body_read"),
        ("client_response_decode_local_ms", "client_response_decode_local"),
    ] {
        if let Some(milliseconds) = response
            .get(response_key)
            .and_then(serde_json::Value::as_f64)
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            stage_timings_ms.insert(stage.to_owned(), milliseconds);
        }
    }
    let Some(handler_ms) = response
        .get("position_manager_handler_ms")
        .and_then(serde_json::Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
    else {
        return;
    };
    stage_timings_ms.insert("position_manager_handler".to_owned(), handler_ms);
    stage_timings_ms.insert(
        "client_transport_and_body_read".to_owned(),
        (submit_response_ms - handler_ms).max(0.0),
    );
    let Some(phases) = response
        .get("processing_timings_ms")
        .and_then(serde_json::Value::as_object)
    else {
        return;
    };
    for (phase, value) in phases {
        if let Some(milliseconds) = value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
        {
            stage_timings_ms.insert(format!("position_manager_{phase}"), milliseconds);
        }
    }
}
