fn configured_fee_rates_maker_bps() -> f64 {
    std::env::var("PAPER_OKX_SPOT_MAKER_FEE_RATE")
        .ok().and_then(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0 && *value <= 0.1)
        .unwrap_or(0.0008) * 10_000.0
}

#[derive(Clone, Copy)]
struct ExecutionParameters {
    vwap_duration_minutes: u64,
    vwap_participation_rate: f64,
    limit_offset_bps: f64,
    trigger_offset_bps: f64,
    trailing_distance_percent: f64,
}
impl Default for ExecutionParameters {
    fn default() -> Self {
        Self {
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

async fn submit_configured_order(
    client: &reqwest::Client,
    base: &str,
    symbol: &str,
    side: &str,
    quantity: f64,
    notional: f64,
    bid: f64,
    ask: f64,
    kind: AutoOrderType,
    params: ExecutionParameters,
    client_order_id: String,
) -> Result<serde_json::Value, (StatusCode, String)> {
    let mut body = serde_json::json!({ "symbol": symbol, "side": side, "quantity": quantity, "measure_execution_quality": true, "source": "auto", "client_order_id": client_order_id });
    let endpoint = match kind {
        AutoOrderType::Vwap | AutoOrderType::Twap => {
            body["duration_minutes"] = serde_json::json!(params.vwap_duration_minutes);
            if matches!(kind, AutoOrderType::Vwap) {
                body["max_participation_rate"] = serde_json::json!(params.vwap_participation_rate);
            }
            if matches!(kind, AutoOrderType::Twap) {
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
                    body["limit_price"] =
                        serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps))
                }
                AutoOrderType::Gtd => {
                    body["limit_price"] =
                        serde_json::json!(offset_limit(side, bid, ask, params.limit_offset_bps));
                    body["time_in_force"] = serde_json::json!("gtd");
                    body["expires_at"] = serde_json::json!(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                            + 300_000
                    );
                }
                AutoOrderType::PostOnlyLimit => {
                    // Rest at the touch so a maker order has a realistic chance
                    // of being hit. The normal limit offset moves buys below
                    // the bid and sells above the ask.
                    body["limit_price"] = serde_json::json!(if side == "buy" { bid } else { ask });
                    body["expires_at"] = serde_json::json!(
                        SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64
                            + 300_000
                    );
                }
                AutoOrderType::Barrier | AutoOrderType::StopMarket => {
                    body["trigger_price"] = serde_json::json!(if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    });
                    body["trigger_direction"] =
                        serde_json::json!(if side == "buy" { "above" } else { "below" });
                }
                AutoOrderType::StopLimit => {
                    let trigger = if side == "buy" {
                        ask * (1.0 + params.trigger_offset_bps / 10_000.0)
                    } else {
                        bid * (1.0 - params.trigger_offset_bps / 10_000.0)
                    };
                    body["trigger_price"] = serde_json::json!(trigger);
                    body["trigger_direction"] =
                        serde_json::json!(if side == "buy" { "above" } else { "below" });
                    body["limit_price"] = serde_json::json!(if side == "buy" {
                        trigger * (1.0 + params.limit_offset_bps / 10_000.0)
                    } else {
                        trigger * (1.0 - params.limit_offset_bps / 10_000.0)
                    });
                }
                AutoOrderType::TrailingStop => {
                    body["trailing_percent"] =
                        serde_json::json!(params.trailing_distance_percent / 100.0)
                }
                _ => {}
            }
            "/positions/orders"
        }
    };
    let response = client
        .post(format!("{base}{endpoint}"))
        .json(&body)
        .send()
        .await
        .map_err(upstream_error)?;
    let status = response.status();
    let position_manager_ms = response
        .headers()
        .get("x-position-manager-handler-ms")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<f64>().ok());
    let response_body = response.bytes().await.map_err(upstream_error)?;
    let mut payload = serde_json::from_slice::<serde_json::Value>(&response_body).unwrap_or_else(
        |_| serde_json::json!({"error": String::from_utf8_lossy(&response_body).trim()}),
    );
    if let Some(handler_ms) = position_manager_ms {
        payload["position_manager_handler_ms"] = serde_json::json!(handler_ms);
    }
    if !status.is_success() {
        return Err((
            StatusCode::BAD_GATEWAY,
            format!("Configured paper order rejected ({status}): {payload}"),
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

fn horizon_adjusted_take_profit_percent(
    signal: &Signal,
    walk_forward: &[WalkForward],
    base_target_percent: f64,
) -> Option<f64> {
    walk_forward
        .iter()
        .filter(|item| item.symbol == signal.symbol && matches!(item.horizon_seconds, 900 | 3_600))
        .filter(|item| {
            let metrics = if signal.direction == "bullish" {
                &item.all_signal_long
            } else {
                &item.all_signal_short
            };
            metrics.sample_count >= MIN_SIDE_WALK_FORWARD_SAMPLES
                && metrics
                    .direction_match_rate
                    .is_some_and(|rate| rate >= MIN_SIDE_DIRECTION_MATCH_RATE)
                && metrics.positive_net_rate.is_some_and(|rate| rate >= 0.60)
                && metrics
                    .positive_net_rate_lower_95
                    .is_some_and(|rate| rate >= MIN_SIDE_POSITIVE_NET_LOWER_95)
                && metrics
                    .average_net_return_percent
                    .is_some_and(|average| average > base_target_percent)
        })
        .filter_map(|item| {
            if signal.direction == "bullish" {
                item.all_signal_long.average_net_return_percent
            } else {
                item.all_signal_short.average_net_return_percent
            }
        })
        .max_by(f64::total_cmp)
        .map(|average_return| {
            (average_return * 0.75)
                .max(base_target_percent)
                .min(MAX_TAKE_PROFIT_PERCENT)
        })
}

fn risk_adjusted_take_profit_percent(stop_loss_percent: f64, risk_appetite: f64) -> f64 {
    let safe_target = default_take_profit_percent()
        .max(stop_loss_percent * MIN_EXIT_REWARD_RISK_RATIO)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT);
    let risk = risk_appetite.clamp(0.0, 100.0) / 100.0;
    (safe_target * (1.0 - risk) + SMALL_NET_TAKE_PROFIT_PERCENT * risk)
        .max(safe_target)
        .clamp(MIN_TAKE_PROFIT_PERCENT, MAX_TAKE_PROFIT_PERCENT)
}

fn adapt_exit_policy(settings: &mut Settings) {
    settings.stop_loss_percent = settings
        .stop_loss_percent
        .clamp(MIN_STOP_LOSS_PERCENT, MAX_STOP_LOSS_PERCENT);
    settings.take_profit_percent =
        risk_adjusted_take_profit_percent(settings.stop_loss_percent, settings.risk_appetite);
}
