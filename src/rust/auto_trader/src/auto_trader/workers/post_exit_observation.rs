const POST_EXIT_OBSERVATION_HORIZON_MS: u64 = 30 * 60 * 1_000;
const POST_EXIT_SAMPLE_INTERVAL_MS: u64 = 60 * 1_000;

fn start_post_exit_observation_worker(state: &AutoTraderState) {
    let worker_state = state.clone();
    tokio::spawn(async move {
        let mut ticker = interval(Duration::from_secs(15));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Err(error) = collect_due_post_exit_observations(&worker_state).await {
                crate::service_log!("WARN", "Post-exit observation collection failed: {}", error);
            }
            if let Err(error) = publish_exit_timing_feedback(&worker_state).await {
                crate::service_log!("WARN", "Post-exit learning feedback failed: {}", error);
            }
        }
    });
}

async fn collect_due_post_exit_observations(state: &AutoTraderState) -> Result<(), String> {
    let position_url = std::env::var("POSITION_MANAGEMENT_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:3002".into());
    let now = current_time_ms();
    let due = state.settings.lock().await.post_exit_observations.iter()
        .filter(|item| item["observe_at_ms"].as_u64().unwrap_or(u64::MAX) <= now
            || item["next_sample_at_ms"].as_u64().unwrap_or(u64::MAX) <= now)
        .cloned().collect::<Vec<_>>();
    for observation in due {
        let symbol = observation["symbol"].as_str().unwrap_or_default();
        if symbol.is_empty() { continue; }
        let response = state.client.get(format!("{position_url}/internal/market-book?symbol={symbol}"))
            .timeout(Duration::from_secs(2)).send().await.map_err(|error| error.to_string())?;
        let market = response.error_for_status().map_err(|error| error.to_string())?
            .json::<serde_json::Value>().await.map_err(|error| error.to_string())?;
        let book_updated_at = market["updated_at_ms"].as_u64().unwrap_or_default();
        if book_updated_at == 0 || now.saturating_sub(book_updated_at) > 5_000 { continue; }
        let bid = market["bids"].as_array().and_then(|levels| levels.first())
            .and_then(|level| level["price"].as_f64());
        let ask = market["asks"].as_array().and_then(|levels| levels.first())
            .and_then(|level| level["price"].as_f64());
        let (Some(bid), Some(ask)) = (bid, ask) else { continue; };
        let observed_at = current_time_ms();
        let mark = (bid + ask) * 0.5;
        let exit_price = observation["exit_price"].as_f64().unwrap_or_default();
        if exit_price <= 0.0 || !mark.is_finite() || mark <= 0.0 { continue; }
        let event_id = observation["event_id"].as_str().unwrap_or_default();
        let mut price_path = observation["price_path"].as_array().cloned().unwrap_or_default();
        let sample = serde_json::json!({"timestamp": observed_at, "bid": bid, "ask": ask, "mid": mark});
        if price_path.last().is_none_or(|last| last["timestamp"].as_u64() != Some(observed_at)) {
            price_path.push(sample);
        }
        let completed = observed_at >= observation["observe_at_ms"].as_u64().unwrap_or(u64::MAX);
        if !completed {
            let mut settings = state.settings.lock().await;
            if let Some(tracker) = settings.post_exit_observations.iter_mut()
                .find(|item| item["event_id"].as_str() == Some(event_id))
            {
                tracker["price_path"] = serde_json::json!(price_path);
                tracker["next_sample_at_ms"] = serde_json::json!(observed_at.saturating_add(POST_EXIT_SAMPLE_INTERVAL_MS));
            }
            request_settings_persist(state).map_err(|error| error.1)?;
            continue;
        }
        let direction = observation["direction"].as_f64().unwrap_or(1.0);
        let post_exit_return_percent = (mark / exit_price - 1.0) * direction * 100.0;
        let path_returns = price_path.iter().filter_map(|sample| {
            let price = sample["mid"].as_f64()?;
            (price.is_finite() && price > 0.0).then_some((price / exit_price - 1.0) * direction * 100.0)
        }).collect::<Vec<_>>();
        let best_post_exit_return_percent = path_returns.iter().copied().reduce(f64::max).unwrap_or(post_exit_return_percent);
        let worst_post_exit_return_percent = path_returns.iter().copied().reduce(f64::min).unwrap_or(post_exit_return_percent);
        let payload = serde_json::json!({
            "record_schema_version": 1,
            "decision_id": observation["decision_id"],
            "trade_id": observation["trade_id"],
            "stage": format!("exit:post:{event_id}"),
            "timestamp": observed_at,
            "symbol": symbol,
            "strategy": observation["strategy"],
            "reason": observation["reason"],
            "status": "observed",
            "post_exit_observation": {
                "event_id": event_id,
                "horizon_ms": POST_EXIT_OBSERVATION_HORIZON_MS,
                "exit_timestamp_ms": observation["exit_timestamp_ms"],
                "observed_at_ms": observed_at,
                "exit_price": exit_price,
                "mark_price": mark,
                "bid": bid,
                "ask": ask,
                "exit_return_percent": observation["exit_return_percent"],
                "post_exit_return_percent": post_exit_return_percent,
                "missed_return_percent": post_exit_return_percent,
                "best_post_exit_return_percent": best_post_exit_return_percent,
                "worst_post_exit_return_percent": worst_post_exit_return_percent,
                "price_path": price_path,
                "missed_pnl_quote": (mark - exit_price) * observation["direction"].as_f64().unwrap_or(1.0)
                    * observation["entry_context"]["quantity"].as_f64().unwrap_or(0.0),
                "learning_context": observation["learning_context"],
                "strategy_decision": observation["strategy_decision"]
            }
        });
        queue_auto_trader_audit(state, payload.clone());
        let mut settings = state.settings.lock().await;
        let mut learning_observation = serde_json::json!({
            "event_id": event_id,
            "symbol": symbol,
            "strategy": observation["strategy"],
            "reason": observation["reason"],
            "trade_id": observation["trade_id"],
        });
        learning_observation.as_object_mut().unwrap().extend(
            payload["post_exit_observation"].as_object().cloned().unwrap_or_default()
        );
        if !settings.exit_timing_feedback_outbox.iter().any(|item| item["event_id"] == event_id) {
            settings.exit_timing_feedback_outbox.push(learning_observation);
        }
        settings.post_exit_observations.retain(|item| item["event_id"] != event_id);
        request_settings_persist(state).map_err(|error| error.1)?;
    }
    Ok(())
}

async fn publish_exit_timing_feedback(state: &AutoTraderState) -> Result<(), String> {
    let Some(base) = std::env::var("LEARNING_SERVICE_URL").ok() else { return Ok(()); };
    let Some(event) = state.settings.lock().await.exit_timing_feedback_outbox.first().cloned() else { return Ok(()); };
    let response = state.client.post(format!("{}/exit-timing", base.trim_end_matches('/')))
        .timeout(Duration::from_secs(3)).json(&event).send().await.map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("learning service returned {}", response.status()));
    }
    let event_id = event["event_id"].as_str().unwrap_or_default();
    let mut settings = state.settings.lock().await;
    settings.exit_timing_feedback_outbox.retain(|item| item["event_id"] != event_id);
    request_settings_persist(state).map_err(|error| error.1)
}
