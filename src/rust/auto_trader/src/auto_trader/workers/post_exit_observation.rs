const POST_EXIT_OBSERVATION_HORIZON_MS: u64 = 30 * 60 * 1_000;
const POST_EXIT_SAMPLE_INTERVAL_MS: u64 = 60 * 1_000;
const EXIT_TIMING_HISTORY_BACKFILL_VERSION: u32 = 1;
const EXIT_TIMING_FEEDBACK_BATCH_SIZE: usize = 32;
const EXIT_TIMING_FEEDBACK_MAX_BYTES: usize = 15 * 1024;
const EXIT_TIMING_PRE_EXIT_PATH_MAX_POINTS: usize = 96;
const EXIT_TIMING_POST_EXIT_PATH_MAX_POINTS: usize = 32;

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
    let now = current_time_ms();
    let due = state
        .settings
        .lock()
        .await
        .post_exit_observations
        .iter()
        .filter(|item| {
            item["observe_at_ms"].as_u64().unwrap_or(u64::MAX) <= now
                || item["next_sample_at_ms"].as_u64().unwrap_or(u64::MAX) <= now
        })
        .cloned()
        .collect::<Vec<_>>();
    for observation in due {
        let symbol = observation["symbol"].as_str().unwrap_or_default();
        if symbol.is_empty() {
            continue;
        }
        let market =
            crate::service_rpc::execution_get(&format!("/internal/market-mark?symbol={symbol}"))
                .await
                .map_err(|error| error.to_string())?;
        let book_updated_at = market["updated_at_ms"].as_u64().unwrap_or_default();
        if book_updated_at == 0 || now.saturating_sub(book_updated_at) > 5_000 {
            defer_post_exit_observation(state, &observation, now, "stale_market_mark").await?;
            continue;
        }
        let bid = market["bid"].as_f64();
        let ask = market["ask"].as_f64();
        let (Some(bid), Some(ask)) = (bid, ask) else {
            defer_post_exit_observation(state, &observation, now, "incomplete_market_book").await?;
            continue;
        };
        let observed_at = current_time_ms();
        let mark = (bid + ask) * 0.5;
        let exit_price = observation["exit_price"].as_f64().unwrap_or_default();
        if exit_price <= 0.0 || !mark.is_finite() || mark <= 0.0 {
            defer_post_exit_observation(
                state,
                &observation,
                observed_at,
                "invalid_observation_price",
            )
            .await?;
            continue;
        }
        let event_id = observation["event_id"].as_str().unwrap_or_default();
        let mut price_path = observation["price_path"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let sample =
            serde_json::json!({"timestamp": observed_at, "bid": bid, "ask": ask, "mid": mark});
        if price_path
            .last()
            .is_none_or(|last| last["timestamp"].as_u64() != Some(observed_at))
        {
            price_path.push(sample);
        }
        let completed = observed_at >= observation["observe_at_ms"].as_u64().unwrap_or(u64::MAX);
        if !completed {
            let mut settings = state.settings.lock().await;
            if let Some(tracker) = settings
                .post_exit_observations
                .iter_mut()
                .find(|item| item["event_id"].as_str() == Some(event_id))
            {
                tracker["price_path"] = serde_json::json!(price_path);
                tracker["next_sample_at_ms"] =
                    serde_json::json!(observed_at.saturating_add(POST_EXIT_SAMPLE_INTERVAL_MS));
                tracker["last_collection_attempt_at_ms"] = serde_json::json!(observed_at);
                tracker["last_collection_status"] = serde_json::json!("sampled");
            }
            request_settings_persist(state).map_err(|error| error.1)?;
            continue;
        }
        let direction = observation["direction"].as_f64().unwrap_or(1.0);
        let post_exit_return_percent = (mark / exit_price - 1.0) * direction * 100.0;
        let quantity = observation["entry_context"]["quantity"]
            .as_f64()
            .unwrap_or_default();
        let entry_price = observation["entry_context"]["average_price"]
            .as_f64()
            .unwrap_or_default();
        let entry_fees = observation["entry_context"]["fees"]
            .as_f64()
            .unwrap_or_default();
        let entry_notional = quantity * entry_price;
        let executable_hold_price = if direction > 0.0 { bid } else { ask };
        let taker_fee_rate = configured_taker_fee_rate();
        let hold_to_horizon_net_return_percent =
            (entry_notional > 0.0 && quantity.is_finite() && entry_fees.is_finite())
                .then(|| {
                    ((executable_hold_price - entry_price) * quantity * direction
                        - entry_fees
                        - executable_hold_price * quantity * taker_fee_rate)
                        / entry_notional
                        * 100.0
                })
                .filter(|value| value.is_finite());
        let hold_improvement_percent = hold_to_horizon_net_return_percent
            .zip(observation["exit_return_percent"].as_f64())
            .map(|(held, exited)| held - exited)
            .filter(|value| value.is_finite());
        let path_returns = price_path
            .iter()
            .filter_map(|sample| {
                let price = sample["mid"].as_f64()?;
                (price.is_finite() && price > 0.0)
                    .then_some((price / exit_price - 1.0) * direction * 100.0)
            })
            .collect::<Vec<_>>();
        let best_post_exit_return_percent = path_returns
            .iter()
            .copied()
            .reduce(f64::max)
            .unwrap_or(post_exit_return_percent);
        let worst_post_exit_return_percent = path_returns
            .iter()
            .copied()
            .reduce(f64::min)
            .unwrap_or(post_exit_return_percent);
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
                "side": observation["side"],
                "entry_context": observation["entry_context"],
                "exit_fees": observation["exit_fees"],
                "exit_filled_quantity": observation["exit_filled_quantity"],
                "effective_stop_loss_percent": observation["effective_stop_loss_percent"],
                "effective_take_profit_percent": observation["effective_take_profit_percent"],
                "pre_exit_price_path": observation["pre_exit_price_path"],
                "mark_price": mark,
                "bid": bid,
                "ask": ask,
                "exit_return_percent": observation["exit_return_percent"],
                "post_exit_return_percent": post_exit_return_percent,
                "missed_return_percent": post_exit_return_percent,
                "hold_to_horizon_net_return_percent": hold_to_horizon_net_return_percent,
                "hold_improvement_percent": hold_improvement_percent,
                "counterfactual_exit_price": executable_hold_price,
                "counterfactual_exit_fee_rate": taker_fee_rate,
                "best_post_exit_return_percent": best_post_exit_return_percent,
                "worst_post_exit_return_percent": worst_post_exit_return_percent,
                "price_path": price_path,
                "missed_pnl_quote": hold_improvement_percent
                    .map(|improvement| improvement / 100.0 * entry_notional),
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
            payload["post_exit_observation"]
                .as_object()
                .cloned()
                .unwrap_or_default(),
        );
        if !settings
            .post_exit_adaptation_history
            .iter()
            .any(|item| item["event_id"] == event_id)
        {
            settings
                .post_exit_adaptation_history
                .push(learning_observation.clone());
            if settings.post_exit_adaptation_history.len() > 500 {
                let excess = settings.post_exit_adaptation_history.len() - 500;
                settings.post_exit_adaptation_history.drain(..excess);
            }
        }
        if !settings
            .exit_timing_feedback_outbox
            .iter()
            .any(|item| item["event_id"] == event_id)
        {
            settings
                .exit_timing_feedback_outbox
                .push(learning_observation);
        }
        settings
            .post_exit_observations
            .retain(|item| item["event_id"] != event_id);
        request_settings_persist(state).map_err(|error| error.1)?;
    }
    Ok(())
}

async fn defer_post_exit_observation(
    state: &AutoTraderState,
    observation: &serde_json::Value,
    attempted_at_ms: u64,
    status: &str,
) -> Result<(), String> {
    let mut settings = state.settings.lock().await;
    record_post_exit_collection_status(
        &mut settings,
        observation["event_id"].as_str().unwrap_or_default(),
        attempted_at_ms,
        status,
    );
    request_settings_persist(state).map_err(|error| error.1)
}

fn record_post_exit_collection_status(
    settings: &mut Settings,
    event_id: &str,
    attempted_at_ms: u64,
    status: &str,
) {
    if let Some(tracker) = settings
        .post_exit_observations
        .iter_mut()
        .find(|item| item["event_id"].as_str() == Some(event_id))
    {
        tracker["last_collection_attempt_at_ms"] = serde_json::json!(attempted_at_ms);
        tracker["last_collection_status"] = serde_json::json!(status);
        tracker["next_sample_at_ms"] =
            serde_json::json!(attempted_at_ms.saturating_add(POST_EXIT_SAMPLE_INTERVAL_MS));
    }
}

async fn publish_exit_timing_feedback(state: &AutoTraderState) -> Result<(), String> {
    let backfilled = {
        let mut settings = state.settings.lock().await;
        backfill_exit_timing_history(&mut settings)
    };
    if backfilled {
        request_settings_persist(state).map_err(|error| error.1)?;
    }
    if std::env::var("LEARNING_RSOCKET_URL").is_err() {
        return Ok(());
    }
    let now = current_time_ms();
    let events = {
        let mut settings = state.settings.lock().await;
        let events = prepare_exit_timing_feedback(&mut settings, now);
        if !events.is_empty() {
            request_settings_persist(state).map_err(|error| error.1)?;
        }
        let (queue_depth, oldest_age_ms) = exit_timing_feedback_queue_lag(&settings, now);
        crate::service_log!(
            "INFO",
            "Exit-timing feedback queue depth={} oldest_age_ms={}",
            queue_depth,
            oldest_age_ms.unwrap_or_default()
        );
        events
    };
    if events.is_empty() {
        return Ok(());
    }

    let mut tasks = tokio::task::JoinSet::new();
    for event in events {
        let event_id = event["event_id"].as_str().unwrap_or_default().to_owned();
        if event_id.is_empty() {
            crate::service_log!(
                "WARN",
                "Skipping malformed exit-timing feedback without event_id"
            );
            continue;
        }
        tasks.spawn(async move {
            let payload = compact_exit_timing_feedback_for_delivery(&event)
                .map_err(|error| error.to_string());
            let result = async {
                let payload = payload?;
                let response = crate::service_rpc::learning_request(
                    strategy::ScoutWorkerHttpMethod::Post,
                    "/exit-timing",
                    Some(payload),
                )
                .await
                .map_err(|error| error.to_string())?;
                if !(200..300).contains(&response.status) {
                    return Err(format!(
                        "Learning rejected exit-timing event with status {}: {}",
                        response.status, response.body
                    ));
                }
                if response.body["accepted"] != true {
                    return Err("Learning returned no durable exit-timing acknowledgement".into());
                }
                Ok(())
            }
            .await;
            (event_id, result)
        });
    }

    let mut acknowledged = BTreeSet::new();
    let mut failures = BTreeMap::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok((event_id, Ok(()))) => {
                acknowledged.insert(event_id);
            }
            Ok((event_id, Err(error))) => {
                failures.insert(event_id.clone(), error.clone());
                crate::service_log!(
                    "WARN",
                    "Exit-timing feedback remains queued event_id={} error={}",
                    event_id,
                    error
                );
            }
            Err(error) => {
                crate::service_log!("WARN", "Exit-timing feedback worker task failed: {}", error)
            }
        }
    }
    if acknowledged.is_empty() && failures.is_empty() {
        return Ok(());
    }
    let mut settings = state.settings.lock().await;
    let changed = apply_exit_timing_feedback_results(
        &mut settings,
        &acknowledged,
        &failures,
        current_time_ms(),
    );
    if changed {
        request_settings_persist(state).map_err(|error| error.1)?;
    }
    Ok(())
}

fn backfill_exit_timing_history(settings: &mut Settings) -> bool {
    if settings.exit_timing_history_backfill_version >= EXIT_TIMING_HISTORY_BACKFILL_VERSION {
        return false;
    }

    let mut queued_ids = BTreeSet::new();
    settings.exit_timing_feedback_outbox.retain(|item| {
        item["event_id"]
            .as_str()
            .is_none_or(|event_id| event_id.is_empty() || queued_ids.insert(event_id.to_owned()))
    });
    for item in &settings.post_exit_adaptation_history {
        let Some(event_id) = item["event_id"].as_str().filter(|id| !id.is_empty()) else {
            continue;
        };
        if queued_ids.insert(event_id.to_owned()) {
            settings.exit_timing_feedback_outbox.push(item.clone());
        }
    }
    settings.exit_timing_history_backfill_version = EXIT_TIMING_HISTORY_BACKFILL_VERSION;
    true
}

fn remove_acknowledged_exit_timing_feedback(
    settings: &mut Settings,
    acknowledged: &BTreeSet<String>,
) -> bool {
    let original_len = settings.exit_timing_feedback_outbox.len();
    settings.exit_timing_feedback_outbox.retain(|item| {
        item["event_id"]
            .as_str()
            .is_none_or(|event_id| !acknowledged.contains(event_id))
    });
    settings.exit_timing_feedback_outbox.len() != original_len
}

fn prepare_exit_timing_feedback(settings: &mut Settings, now_ms: u64) -> Vec<serde_json::Value> {
    settings
        .exit_timing_feedback_outbox
        .iter_mut()
        .take(EXIT_TIMING_FEEDBACK_BATCH_SIZE)
        .map(|event| {
            let attempts = event["delivery_attempts"].as_u64().unwrap_or_default() + 1;
            event["delivery_attempts"] = serde_json::json!(attempts);
            event["last_attempt_at_ms"] = serde_json::json!(now_ms);
            event["delivery_status"] = serde_json::json!("delivering");
            event["last_delivery_error"] = serde_json::Value::Null;
            event.clone()
        })
        .collect()
}

fn compact_exit_timing_feedback_for_delivery(
    event: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let mut payload = event.clone();
    if let Some(object) = payload.as_object_mut() {
        for field in [
            "delivery_attempts",
            "last_attempt_at_ms",
            "delivery_status",
            "last_delivery_error",
            "acknowledged_at_ms",
        ] {
            object.remove(field);
        }
    }

    let pre_exit_path = payload["pre_exit_price_path"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let post_exit_path = payload["price_path"].as_array().cloned().unwrap_or_default();
    let mut pre_exit_cap = pre_exit_path.len().min(EXIT_TIMING_PRE_EXIT_PATH_MAX_POINTS);
    let mut post_exit_cap = post_exit_path.len().min(EXIT_TIMING_POST_EXIT_PATH_MAX_POINTS);
    loop {
        payload["pre_exit_price_path"] = serde_json::Value::Array(
            sample_exit_timing_path(&pre_exit_path, pre_exit_cap),
        );
        payload["price_path"] =
            serde_json::Value::Array(sample_exit_timing_path(&post_exit_path, post_exit_cap));
        let compacted = pre_exit_cap < pre_exit_path.len() || post_exit_cap < post_exit_path.len();
        if compacted {
            payload["history_path_compacted"] = serde_json::json!(true);
            payload["pre_exit_path_original_count"] = serde_json::json!(pre_exit_path.len());
            payload["post_exit_path_original_count"] = serde_json::json!(post_exit_path.len());
        }
        let encoded = serde_json::to_vec(&payload)
            .map_err(|error| format!("could not encode exit-timing feedback: {error}"))?;
        if encoded.len() <= EXIT_TIMING_FEEDBACK_MAX_BYTES {
            return Ok(payload);
        }

        if pre_exit_cap >= post_exit_cap && pre_exit_cap > 8 {
            pre_exit_cap = (pre_exit_cap / 2).max(8);
        } else if post_exit_cap > 8 {
            post_exit_cap = (post_exit_cap / 2).max(8);
        } else {
            return Err(format!(
                "exit-timing feedback remains {} bytes after path compaction",
                encoded.len()
            ));
        }
    }
}

fn sample_exit_timing_path(
    path: &[serde_json::Value],
    maximum_points: usize,
) -> Vec<serde_json::Value> {
    if path.len() <= maximum_points {
        return path.to_vec();
    }
    if maximum_points <= 1 {
        return path.first().cloned().into_iter().collect();
    }
    let last_index = path.len() - 1;
    (0..maximum_points)
        .map(|sample| {
            let index = sample * last_index / (maximum_points - 1);
            path[index].clone()
        })
        .collect()
}

fn apply_exit_timing_feedback_results(
    settings: &mut Settings,
    acknowledged: &BTreeSet<String>,
    failures: &BTreeMap<String, String>,
    acknowledged_at_ms: u64,
) -> bool {
    let created_at_by_id = settings
        .exit_timing_feedback_outbox
        .iter()
        .filter_map(|event| {
            Some((
                event["event_id"].as_str()?.to_owned(),
                event["created_at_ms"]
                    .as_u64()
                    .or_else(|| event["observed_at_ms"].as_u64())
                    .or_else(|| event["timestamp"].as_u64())?,
            ))
        })
        .collect::<BTreeMap<_, _>>();
    for event in &mut settings.post_exit_adaptation_history {
        let Some(event_id) = event["event_id"].as_str().map(str::to_owned) else {
            continue;
        };
        if acknowledged.contains(&event_id) {
            event["delivery_status"] = serde_json::json!("acknowledged");
            event["acknowledged_at_ms"] = serde_json::json!(acknowledged_at_ms);
            event["last_delivery_error"] = serde_json::Value::Null;
            if let Some(created_at) = created_at_by_id.get(&event_id) {
                event["delivery_age_ms"] =
                    serde_json::json!(acknowledged_at_ms.saturating_sub(*created_at));
            }
        }
    }
    for event in &mut settings.exit_timing_feedback_outbox {
        let Some(event_id) = event["event_id"].as_str() else {
            continue;
        };
        if let Some(error) = failures.get(event_id) {
            event["delivery_status"] = serde_json::json!(if error.contains("rejected") {
                "rejected"
            } else {
                "error"
            });
            event["last_delivery_error"] = serde_json::json!(error);
        }
    }
    remove_acknowledged_exit_timing_feedback(settings, acknowledged)
        || !failures.is_empty()
        || !acknowledged.is_empty()
}

fn exit_timing_feedback_queue_lag(settings: &Settings, now_ms: u64) -> (usize, Option<u64>) {
    let oldest_created_at = settings
        .exit_timing_feedback_outbox
        .iter()
        .filter_map(|event| {
            event["created_at_ms"]
                .as_u64()
                .or_else(|| event["observed_at_ms"].as_u64())
                .or_else(|| event["timestamp"].as_u64())
        })
        .min();
    (
        settings.exit_timing_feedback_outbox.len(),
        oldest_created_at.map(|created_at| now_ms.saturating_sub(created_at)),
    )
}

#[cfg(test)]
mod post_exit_feedback_tests {
    use super::*;

    #[test]
    fn deferred_observation_records_reason_and_next_retry_time() {
        let mut settings = Settings::default();
        settings.post_exit_observations = vec![serde_json::json!({"event_id": "event-a"})];

        record_post_exit_collection_status(&mut settings, "event-a", 10_000, "stale_market_mark");

        let observation = &settings.post_exit_observations[0];
        assert_eq!(observation["last_collection_attempt_at_ms"], 10_000);
        assert_eq!(observation["last_collection_status"], "stale_market_mark");
        assert_eq!(
            observation["next_sample_at_ms"],
            10_000 + POST_EXIT_SAMPLE_INTERVAL_MS
        );
    }

    #[test]
    fn oversized_exit_timing_feedback_is_compacted_below_learning_limit() {
        let path = (0..2_618)
            .map(|index| {
                serde_json::json!({
                    "timestamp": 1_000 + index * 1_000,
                    "bid": 0.30553 + index as f64 * 0.00001,
                    "ask": 0.30555 + index as f64 * 0.00001
                })
            })
            .collect::<Vec<_>>();
        let event = serde_json::json!({
            "event_id": "exit-timing:large-event",
            "symbol": "BNB-USDT",
            "strategy": "live-vwap-dislocation",
            "exit_return_percent": -0.5,
            "post_exit_return_percent": 0.1,
            "missed_return_percent": 0.1,
            "pre_exit_price_path": path,
            "price_path": [{"timestamp": 3_000_000, "bid": 0.3, "ask": 0.31}],
            "delivery_attempts": 1,
            "last_attempt_at_ms": 4_000_000,
            "delivery_status": "delivering"
        });
        assert!(serde_json::to_vec(&event).unwrap().len() > 16 * 1024);

        let payload = compact_exit_timing_feedback_for_delivery(&event).unwrap();
        let encoded = serde_json::to_vec(&payload).unwrap();

        assert!(encoded.len() <= EXIT_TIMING_FEEDBACK_MAX_BYTES);
        assert_eq!(payload["event_id"], "exit-timing:large-event");
        assert_eq!(payload["pre_exit_path_original_count"], 2_618);
        assert_eq!(payload["history_path_compacted"], true);
        assert_eq!(payload["pre_exit_price_path"][0], event["pre_exit_price_path"][0]);
        assert_eq!(
            payload["pre_exit_price_path"][payload["pre_exit_price_path"].as_array().unwrap().len() - 1],
            event["pre_exit_price_path"][2_617]
        );
        assert!(payload.get("delivery_attempts").is_none());
    }

    #[test]
    fn exit_timing_outbox_persists_attempt_error_ack_and_queue_lag() {
        let mut settings = Settings::default();
        settings.exit_timing_feedback_outbox = vec![serde_json::json!({
            "event_id": "event-a",
            "timestamp": 1_000,
        })];
        settings.post_exit_adaptation_history = settings.exit_timing_feedback_outbox.clone();

        let events = prepare_exit_timing_feedback(&mut settings, 2_000);
        assert_eq!(events[0]["delivery_attempts"], 1);
        assert_eq!(events[0]["delivery_status"], "delivering");
        let (depth, oldest_age_ms) = exit_timing_feedback_queue_lag(&settings, 2_000);
        assert_eq!(depth, 1);
        assert_eq!(oldest_age_ms, Some(1_000));

        let failures = BTreeMap::from([("event-a".to_owned(), "Learning rejected event".into())]);
        apply_exit_timing_feedback_results(&mut settings, &BTreeSet::new(), &failures, 2_100);
        assert_eq!(
            settings.exit_timing_feedback_outbox[0]["delivery_status"],
            "rejected"
        );
        assert_eq!(
            settings.exit_timing_feedback_outbox[0]["last_delivery_error"],
            "Learning rejected event"
        );

        apply_exit_timing_feedback_results(
            &mut settings,
            &BTreeSet::from(["event-a".to_owned()]),
            &BTreeMap::new(),
            3_000,
        );
        assert!(settings.exit_timing_feedback_outbox.is_empty());
        assert_eq!(
            settings.post_exit_adaptation_history[0]["delivery_status"],
            "acknowledged"
        );
        assert_eq!(
            settings.post_exit_adaptation_history[0]["acknowledged_at_ms"],
            3_000
        );
        assert_eq!(
            settings.post_exit_adaptation_history[0]["delivery_age_ms"],
            2_000
        );
    }

    #[test]
    fn backfill_is_one_time_and_deduplicates_existing_queue_and_history_ids() {
        let mut settings = Settings::default();
        settings.post_exit_adaptation_history = vec![
            serde_json::json!({"event_id": "event-a", "value": 1}),
            serde_json::json!({"event_id": "event-b", "value": 2}),
            serde_json::json!({"event_id": "event-a", "value": 3}),
            serde_json::json!({"value": 4}),
        ];
        settings.exit_timing_feedback_outbox = vec![
            serde_json::json!({"event_id": "event-a", "value": 9}),
            serde_json::json!({"event_id": "event-a", "value": 10}),
        ];

        assert!(backfill_exit_timing_history(&mut settings));
        let ids = settings
            .exit_timing_feedback_outbox
            .iter()
            .filter_map(|item| item["event_id"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, vec!["event-a", "event-b"]);
        assert_eq!(settings.exit_timing_history_backfill_version, 1);
        let persisted = rmp_serde::to_vec_named(&settings).unwrap();
        let mut restarted: Settings = rmp_serde::from_slice(&persisted).unwrap();
        let queued = restarted.exit_timing_feedback_outbox.clone();
        assert!(!backfill_exit_timing_history(&mut restarted));
        assert_eq!(restarted.exit_timing_feedback_outbox, queued);
    }

    #[test]
    fn partial_acknowledgement_removes_only_durably_accepted_events() {
        let mut settings = Settings::default();
        settings.exit_timing_feedback_outbox = vec![
            serde_json::json!({"event_id": "accepted-a"}),
            serde_json::json!({"event_id": "retry-b"}),
            serde_json::json!({"event_id": "accepted-c"}),
        ];
        let acknowledged = BTreeSet::from(["accepted-a".to_owned(), "accepted-c".to_owned()]);

        assert!(remove_acknowledged_exit_timing_feedback(
            &mut settings,
            &acknowledged
        ));
        assert_eq!(settings.exit_timing_feedback_outbox.len(), 1);
        assert_eq!(
            settings.exit_timing_feedback_outbox[0]["event_id"],
            "retry-b"
        );
    }
}
