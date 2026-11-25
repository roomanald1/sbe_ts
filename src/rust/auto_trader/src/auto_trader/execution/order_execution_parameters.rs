fn configured_fee_rates_maker_bps() -> f64 {
    std::env::var("PAPER_OKX_SPOT_MAKER_FEE_RATE")
        .ok()
        .and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1)
        .unwrap_or(0.0008)
        * 10_000.0
}

#[derive(Clone, Copy)]
struct ExecutionParameters {
    post_only_timeout_minutes: u64,
    vwap_duration_minutes: u64,
    vwap_participation_rate: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}
impl Default for ExecutionParameters {
    fn default() -> Self {
        Self {
            post_only_timeout_minutes: 5,
            vwap_duration_minutes: default_vwap_duration(),
            vwap_participation_rate: default_vwap_participation() / 100.0,
            limit_offset_bps: default_limit_offset(),
            trigger_offset_bps: default_trigger_offset(),
            trailing_distance_percent: default_trailing_distance(),
        }
    }
}
fn parameter_profile(mut params: ExecutionParameters, profile: usize) -> ExecutionParameters {
    match profile {
        1 => {
            params.vwap_duration_minutes = (params.vwap_duration_minutes * 3 / 4).clamp(5, 390);
            params.vwap_participation_rate = (params.vwap_participation_rate * 1.25).min(0.50);
            params.limit_offset_bps = (params.limit_offset_bps - 2.0).max(-10.0);
            params.trigger_offset_bps = (params.trigger_offset_bps * 0.75).max(5.0);
            params.trailing_distance_percent = (params.trailing_distance_percent * 0.75).max(0.05);
        }
        2 => {
            params.vwap_duration_minutes = (params.vwap_duration_minutes * 3 / 2).clamp(5, 390);
            params.vwap_participation_rate = (params.vwap_participation_rate * 0.75).max(0.01);
            params.limit_offset_bps = (params.limit_offset_bps + 2.0).min(10.0);
            params.trigger_offset_bps = (params.trigger_offset_bps * 1.25).min(100.0);
            params.trailing_distance_percent = (params.trailing_distance_percent * 1.25).min(1.0);
        }
        _ => {}
    }
    params
}

fn offset_limit(side: &str, bid: f64, ask: f64, offset_bps: f64) -> f64 {
    let reference = if side == "buy" { bid } else { ask };
    reference
        * (1.0
            + (if side == "buy" {
                -offset_bps
            } else {
                offset_bps
            }) / 10_000.0)
}

fn build_order_request(
    symbol: &str,
    side: &str,
    quantity: f64,
    bid: f64,
    ask: f64,
    kind: AutoOrderType,
    params: ExecutionParameters,
    client_order_id: &str,
    now_ms: u64,
    reduce_only: bool,
    lifecycle_identity: Option<&CanonicalLifecycleIdentity>,
) -> (&'static str, serde_json::Value) {
    let mut body = serde_json::json!({
        "symbol": symbol,
        "side": side,
        "quantity": quantity,
        "measure_execution_quality": true,
        "source": "auto",
        "client_order_id": client_order_id,
        "reduce_only": reduce_only,
    });
    if let Some(identity) = lifecycle_identity {
        body["lifecycle"] = serde_json::json!({
            "schema_version": identity.schema_version,
            "opportunity_id": identity.opportunity_id,
            "episode_id": identity.episode_id,
            "trade_id": identity.trade_id,
            "profile_id": identity.profile_id,
            "profile_hash": identity.profile_hash,
            "profile_version": identity.profile_version,
            "execution_mode": identity.execution_mode,
            "entry_policy": identity.entry_policy,
            "source": identity.source,
        });
    }
    let endpoint = match kind {
        AutoOrderType::Vwap | AutoOrderType::Twap => {
            body["duration_minutes"] = serde_json::json!(params.vwap_duration_minutes);
            if matches!(kind, AutoOrderType::Vwap) {
                body["max_participation_rate"] = serde_json::json!(params.vwap_participation_rate);
            } else {
                body["strategy"] = serde_json::json!("twap");
            }
            "/positions/vwap"
        }
        _ => {
            body["order_type"] = serde_json::json!(match kind {
                AutoOrderType::Market => "market",
                AutoOrderType::Ioc => "ioc",
                AutoOrderType::Fok => "fok",
                AutoOrderType::Limit | AutoOrderType::Gtd => "limit",
                AutoOrderType::PostOnlyLimit => "post-only-limit",
                AutoOrderType::Barrier => "barrier",
                AutoOrderType::StopMarket => "stop-market",
                AutoOrderType::StopLimit => "stop-limit",
                AutoOrderType::TrailingStop => "trailing-stop",
                _ => unreachable!(),
            });
            match kind {
                AutoOrderType::Limit => {
                    body["limit_price"] = serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps));
                }
                AutoOrderType::Gtd => {
                    body["limit_price"] = serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps));
                    body["time_in_force"] = serde_json::json!("gtd");
                    body["expires_at"] = serde_json::json!(now_ms + 300_000);
                }
                AutoOrderType::PostOnlyLimit => {
                    body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask });
                    body["expires_at"] = serde_json::json!(
                        now_ms + params.post_only_timeout_minutes * 60_000
                    );
                    if reduce_only {
                        body["time_in_force"] = serde_json::json!("gtd");
                    }
                }
                AutoOrderType::Barrier | AutoOrderType::StopMarket => {
                    body["trigger_price"] = serde_json::json!(if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    });
                    body["trigger_direction"] = serde_json::json!(if side == "buy" { "above" } else { "below" });
                }
                AutoOrderType::StopLimit => {
                    let trigger = if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    };
                    body["trigger_price"] = serde_json::json!(trigger);
                    body["trigger_direction"] = serde_json::json!(if side == "buy" { "above" } else { "below" });
                    body["limit_price"] = serde_json::json!(if side == "buy" {
                        trigger * (1.0 + params.limit_offset_bps / 10_000.0)
                    } else {
                        trigger * (1.0 - params.limit_offset_bps / 10_000.0)
                    });
                }
                AutoOrderType::TrailingStop => {
                    body["trailing_percent"] = serde_json::json!(params.trailing_distance_percent / 100.0);
                }
                _ => {}
            }
            "/positions/orders"
        }
    };
    (endpoint, body)
}

const SUBMIT_READY_TIMING_BOUNDARY: &str = "order_payload_ready_after_local_construction";
const RSOCKET_TIMING_BOUNDARY: &str = "position_management_rsocket_request_response";
const BENCHMARK_ACCEPTANCE_STATE: &str = "unaccepted_release_execution_required";

/// Attach separately named local and service-call durations to a production
/// order response. `request_prepare_local_ms` covers time to the constructed
/// payload boundary only; it does not include preceding signal/exit decisions
/// or their risk validation. The RSocket duration includes the request/response
/// round trip and is not an exchange acknowledgement or fill measurement.
fn add_order_submission_timing_metadata(
    payload: &mut serde_json::Value,
    request_prepare_local_ms: f64,
    rsocket_request_response_ms: f64,
) {
    payload["performance_timing_boundaries"] = serde_json::json!({
        "submit_ready_boundary": SUBMIT_READY_TIMING_BOUNDARY,
        "request_prepare_local_ms": request_prepare_local_ms,
        "rsocket_boundary": RSOCKET_TIMING_BOUNDARY,
        "rsocket_request_response_ms": rsocket_request_response_ms,
        "acceptance_state": BENCHMARK_ACCEPTANCE_STATE,
    });
}

async fn submit_configured_order(
    symbol: &str,
    side: &str,
    quantity: f64,
    notional: f64,
    bid: f64,
    ask: f64,
    kind: AutoOrderType,
    params: ExecutionParameters,
    client_order_id: String,
    reduce_only: bool,
    lifecycle_identity: Option<&CanonicalLifecycleIdentity>,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let request_prepare_started = Instant::now();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let (endpoint, body) = build_order_request(
        symbol, side, quantity, bid, ask, kind, params, &client_order_id, now_ms,
        reduce_only, lifecycle_identity,
    );
    let request_prepare_ms = request_prepare_started.elapsed().as_secs_f64() * 1_000.0;
    let request_started = Instant::now();
    let response = crate::service_rpc::position_request("POST", endpoint, Some(body))
        .await
        .map_err(|error| (StatusCode::BAD_GATEWAY, error.to_string()))?;
    let rsocket_request_response_ms = request_started.elapsed().as_secs_f64() * 1_000.0;
    let status = StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY);
    let position_manager_ms = response.headers
        .get("x-position-manager-handler-ms")
        .map(String::as_str)
        .and_then(|value| value.parse::<f64>().ok());
    let body_started = Instant::now();
    let mut payload = response.body;
    let body_read_ms = body_started.elapsed().as_secs_f64() * 1_000.0;
    let decode_started = Instant::now();
    if !payload.is_object() {
        payload = serde_json::json!({"error": payload.to_string()});
    }
    let response_decode_ms = decode_started.elapsed().as_secs_f64() * 1_000.0;
    payload["client_request_prepare_local_ms"] = serde_json::json!(request_prepare_ms);
    // Preserve the established response field while exposing its full
    // request/response semantics under an unambiguous boundary name below.
    payload["client_response_headers_ms"] = serde_json::json!(rsocket_request_response_ms);
    payload["client_body_read_ms"] = serde_json::json!(body_read_ms);
    payload["client_response_decode_local_ms"] = serde_json::json!(response_decode_ms);
    add_order_submission_timing_metadata(&mut payload, request_prepare_ms, rsocket_request_response_ms);
    if let Some(handler_ms) = position_manager_ms.or(Some(rsocket_request_response_ms)) {
        payload["position_manager_handler_ms"] = serde_json::json!(handler_ms);
    }
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!(
                "Configured paper order rejected ({status}): {}",
                payload.get("error").and_then(serde_json::Value::as_str)
                    .map(str::to_owned).unwrap_or_else(|| payload.to_string())
            ),
        ));
    }
    if !payload.is_object() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Position Manager returned an invalid order response: {payload}"),
        ));
    }
    let _ = notional;
    Ok(payload)
}



#[cfg(test)]
mod order_request_tests {
    use super::*;

    fn params() -> ExecutionParameters {
        ExecutionParameters {
            post_only_timeout_minutes: 5,
            vwap_duration_minutes: 30,
            vwap_participation_rate: 0.2,
            limit_offset_bps: 5.0,
            trigger_offset_bps: 10.0,
            trailing_distance_percent: 0.5,
        }
    }

    #[test]
    fn timing_metadata_separates_local_submit_ready_and_rsocket_boundaries() {
        let mut response = serde_json::json!({});
        add_order_submission_timing_metadata(&mut response, 0.125, 1.75);

        let boundaries = &response["performance_timing_boundaries"];
        assert_eq!(boundaries["submit_ready_boundary"], SUBMIT_READY_TIMING_BOUNDARY);
        assert_eq!(boundaries["request_prepare_local_ms"], 0.125);
        assert_eq!(boundaries["rsocket_boundary"], RSOCKET_TIMING_BOUNDARY);
        assert_eq!(boundaries["rsocket_request_response_ms"], 1.75);
        assert_eq!(boundaries["acceptance_state"], BENCHMARK_ACCEPTANCE_STATE);
        assert_ne!(
            boundaries["submit_ready_boundary"],
            boundaries["rsocket_boundary"]
        );
    }

    fn request(kind: AutoOrderType, side: &str) -> (&'static str, serde_json::Value) {
        build_order_request("BTC-USDT", side, 2.0, 99.0, 101.0, kind, params(), "client-1", 1_000, false, None)
    }

    #[test]
    fn builds_scheduled_order_variants() {
        let (endpoint, vwap) = request(AutoOrderType::Vwap, "buy");
        assert_eq!(endpoint, "/positions/vwap");
        assert_eq!(vwap["duration_minutes"], 30);
        assert_eq!(vwap["max_participation_rate"], 0.2);
        let (endpoint, twap) = request(AutoOrderType::Twap, "sell");
        assert_eq!(endpoint, "/positions/vwap");
        assert_eq!(twap["strategy"], "twap");
        assert!(twap.get("max_participation_rate").is_none());
    }

    #[test]
    fn selected_entry_request_carries_versioned_lifecycle_identity() {
        let identity = CanonicalLifecycleIdentity {
            schema_version: 1,
            opportunity_id: "autotrader:v1:opportunity:4".into(),
            episode_id: "autotrader:v1:episode:4".into(),
            trade_id: "autotrader:v1:trade:4".into(),
            profile_id: Some("profile-4".into()),
            profile_hash: None,
            profile_version: Some(4),
            execution_mode: "paper".into(),
            entry_policy: "strict".into(),
            source: "auto_trader_entry_reservation".into(),
        };
        let (_, body) = build_order_request(
            "BTC-USDT", "buy", 1.0, 99.0, 101.0, AutoOrderType::Ioc,
            params(), &identity.opportunity_id, 1_000, false, Some(&identity),
        );
        assert_eq!(body["client_order_id"], identity.opportunity_id);
        assert_eq!(body["lifecycle"]["schema_version"], 1);
        assert_eq!(body["lifecycle"]["opportunity_id"], identity.opportunity_id);
        assert_eq!(body["lifecycle"]["episode_id"], identity.episode_id);
        assert_eq!(body["lifecycle"]["trade_id"], identity.trade_id);
        assert_eq!(body["lifecycle"]["execution_mode"], "paper");
        assert_eq!(body["lifecycle"]["entry_policy"], "strict");
        assert_eq!(body["lifecycle"]["profile_hash"], serde_json::Value::Null);
        assert_eq!(body["lifecycle"]["profile_version"], 4);
    }

    #[test]
    fn builds_basic_and_remaining_order_variants() {
        for (kind, expected) in [
            (AutoOrderType::Market, "market"),
            (AutoOrderType::Ioc, "ioc"),
            (AutoOrderType::Fok, "fok"),
            (AutoOrderType::Barrier, "barrier"),
            (AutoOrderType::StopMarket, "stop-market"),
        ] {
            let (endpoint, body) = request(kind, "sell");
            assert_eq!(endpoint, "/positions/orders");
            assert_eq!(body["order_type"], expected);
        }
        let (_, post_only) = request(AutoOrderType::PostOnlyLimit, "sell");
        assert_eq!(post_only["limit_price"], 101.0);
        assert_eq!(post_only["expires_at"], 301_000);
        let (_, sell_stop) = request(AutoOrderType::StopMarket, "sell");
        assert_eq!(sell_stop["trigger_direction"], "below");
        assert!((sell_stop["trigger_price"].as_f64().unwrap() - 98.901).abs() < 1e-10);
    }

    #[test]
    fn post_only_expiry_uses_configured_timeout() {
        let mut configured = params();
        configured.post_only_timeout_minutes = 9;
        let (_, body) = build_order_request(
            "BTC-USDT", "buy", 2.0, 99.0, 101.0,
            AutoOrderType::PostOnlyLimit, configured, "client-2", 1_000, false,
            None,
        );
        assert_eq!(body["expires_at"], 541_000);
    }

    #[test]
    fn reduce_only_post_only_exit_has_a_bounded_gtd_lifetime() {
        let (endpoint, body) = build_order_request(
            "BTC-USDT",
            "sell",
            2.0,
            99.0,
            101.0,
            AutoOrderType::PostOnlyLimit,
            params(),
            "exit-client",
            1_000,
            true,
            None,
        );
        assert_eq!(endpoint, "/positions/orders");
        assert_eq!(body["order_type"], "post-only-limit");
        assert_eq!(body["reduce_only"], true);
        assert_eq!(body["time_in_force"], "gtd");
        assert_eq!(body["expires_at"], 301_000);
    }

    #[test]
    fn builds_price_and_trigger_order_variants() {
        let (endpoint, gtd) = request(AutoOrderType::Gtd, "buy");
        assert_eq!(endpoint, "/positions/orders");
        assert_eq!(gtd["order_type"], "limit");
        assert_eq!(gtd["time_in_force"], "gtd");
        assert_eq!(gtd["expires_at"], 301_000);
        let (_, stop) = request(AutoOrderType::StopLimit, "buy");
        assert_eq!(stop["trigger_direction"], "above");
        assert!((stop["trigger_price"].as_f64().unwrap() - 101.101).abs() < 1e-10);
        assert!(stop["limit_price"].as_f64().unwrap() > stop["trigger_price"].as_f64().unwrap());
        let (_, trailing) = request(AutoOrderType::TrailingStop, "sell");
        assert_eq!(trailing["trailing_percent"], 0.005);
    }
}
