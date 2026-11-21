use serde_json::{Value, json};

#[derive(Deserialize)]
struct ScoutOutcomesQuery {
    page: Option<usize>,
    page_size: Option<usize>,
    source: Option<String>,
    status: Option<String>,
    learning_symbol: Option<String>,
    sort_by: Option<String>,
    sort_direction: Option<String>,
}

#[derive(Deserialize, Default)]
struct ScoutReplayQuery {
    origin_at_ms: Option<u64>,
    page: Option<usize>,
    page_size: Option<usize>,
}

#[derive(Default)]
struct ScoutCounts {
    total: usize,
    open: usize,
    wins: usize,
    losses: usize,
    flats: usize,
    unavailable: usize,
}

#[derive(Default)]
struct LearningAggregate {
    kind: String,
    symbol: String,
    strategy: String,
    scenario: String,
    parameter: Option<String>,
    value: Option<f64>,
    entry_eligible: Option<bool>,
    level: Option<f64>,
    count: usize,
    wins: usize,
    returns: Vec<f64>,
    paired_deltas: Vec<f64>,
}

fn string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn tone(value: f64) -> &'static str {
    if value > 1e-12 { "is-positive" } else if value < -1e-12 { "is-negative" } else { "is-flat" }
}

fn win_rate_color(rate: f64) -> String {
    let progress = rate.clamp(0.0, 100.0) / 100.0;
    let red = [240.0, 110.0, 114.0];
    let green = [69.0, 201.0, 137.0];
    let channels = (0..3).map(|index| (red[index] + (green[index] - red[index]) * progress).round() as u8).collect::<Vec<_>>();
    format!("rgb({}, {}, {})", channels[0], channels[1], channels[2])
}

fn duration_label(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    if seconds < 60 { return format!("{seconds}s"); }
    let minutes = seconds / 60;
    if minutes < 60 { return format!("{minutes}m"); }
    let hours = minutes / 60;
    let remainder = minutes % 60;
    if remainder == 0 { format!("{hours}h") } else { format!("{hours}h {remainder}m") }
}

fn scout_horizon_markouts(audit: &Value, direction: &str) -> Value {
    let strategy_exit = audit["scout_markout_observations"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|observation| observation["kind"] == "strategy_exit");
    let mut markouts = BTreeMap::new();
    for observation in audit["scout_markout_observations"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|observation| observation["kind"] == "horizon")
    {
        let Some(horizon) = observation["horizon_seconds"].as_u64() else {
            continue;
        };
        if !matches!(horizon, 60 | 300 | 900 | 3_600) {
            continue;
        }
        let net = match direction {
            "bullish" => number(observation, "long_net_percent"),
            "bearish" => number(observation, "short_net_percent"),
            _ => None,
        };
        let Some(net) = net else {
            continue;
        };
        let entry = number(observation, "reference_entry_price")
            .filter(|price| *price > 0.0)
            .or_else(|| number(audit, "scout_entry_price").filter(|price| *price > 0.0));
        let favorable = entry.and_then(|entry| {
            let extreme = match direction {
                "bullish" => number(observation, "path_high"),
                "bearish" => number(observation, "path_low"),
                _ => None,
            }?;
            Some(if direction == "bullish" {
                (extreme / entry - 1.0) * 100.0
            } else {
                (entry - extreme) / entry * 100.0
            })
        });
        let adverse = entry.and_then(|entry| {
            let extreme = match direction {
                "bullish" => number(observation, "path_low"),
                "bearish" => number(observation, "path_high"),
                _ => None,
            }?;
            Some(if direction == "bullish" {
                (extreme / entry - 1.0) * 100.0
            } else {
                (entry - extreme) / entry * 100.0
            })
        });
        let gross = match direction {
            "bullish" => number(observation, "long_gross_percent"),
            "bearish" => number(observation, "short_gross_percent"),
            _ => None,
        };
        let time_to_positive = match direction {
            "bullish" => observation["long_time_to_positive_net_seconds"].as_u64(),
            "bearish" => observation["short_time_to_positive_net_seconds"].as_u64(),
            _ => None,
        };
        let post_exit_net = strategy_exit.and_then(|exit| {
            if observation["evaluated_timestamp_ms"].as_u64()?
                <= exit["evaluated_timestamp_ms"].as_u64()?
            {
                return None;
            }
            let (entry, close) = match direction {
                "bullish" => (number(exit, "exit_ask")?, number(observation, "exit_bid")?),
                "bearish" => (number(exit, "exit_bid")?, number(observation, "exit_ask")?),
                _ => return None,
            };
            if entry <= 0.0 || close <= 0.0 {
                return None;
            }
            let gross = match direction {
                "bullish" => (close / entry - 1.0) * 100.0,
                "bearish" => ((entry - close) / entry) * 100.0,
                _ => return None,
            };
            let fee_rate = number(exit, "taker_fee_rate")?;
            Some(gross - 2.0 * fee_rate * 100.0)
        });
        markouts.insert(
            horizon.to_string(),
            json!({
                "horizonSeconds": horizon,
                "netReturnPercent": net,
                "grossReturnPercent": gross,
                "executionCostPercent": gross.map(|gross| gross - net),
                "bestFavorablePercent": favorable.filter(|value| value.is_finite()),
                "worstAdversePercent": adverse.filter(|value| value.is_finite()),
                "timeToPositiveNetSeconds": time_to_positive,
                "postExitNetReturnPercent": post_exit_net.filter(|value| value.is_finite()),
                "observedAt": observation["evaluated_timestamp_ms"],
                "takerFeeRate": observation["taker_fee_rate"],
            }),
        );
    }
    json!(markouts)
}

fn instance_row(audit: &Value, now: u64) -> Option<Value> {
    let decision_id = string(audit, "decision_id")?;
    let symbol = string(audit, "symbol")?;
    let strategy = string(audit, "strategy").unwrap_or_else(|| "unattributed".into());
    // Simulated outcomes can lack direction and entry-price audit data. Keep
    // them visible as Scout evidence without fabricating a fill or position.
    let direction = string(audit, "direction").unwrap_or_else(|| "unknown".into());
    if !matches!(direction.as_str(), "bullish" | "bearish" | "unknown") { return None; }
    let opened_at = audit["timestamp"].as_u64().or_else(|| audit["signal_timestamp"].as_u64()).unwrap_or_default();
    let net_return = number(audit, "scout_net_return_percent");
    let expired = net_return.is_none() && now.saturating_sub(opened_at) > 7 * 86_400_000;
    let status = if let Some(value) = net_return {
        if value > 1e-12 { "Won" } else if value < -1e-12 { "Lost" } else { "Flat" }
    } else if expired {
        "Outcome unavailable"
    } else {
        "Open"
    };
    let entry_bid = number(audit, "scout_entry_bid");
    let entry_ask = number(audit, "scout_entry_ask");
    let entry_price = number(audit, "scout_entry_price")
        .or_else(|| number(audit, "price"))
        .or_else(|| number(&audit["strategy_decision"], "observed_close"));
    let missing_live_entry = strategy.starts_with("live-") && entry_price.is_none();
    let status = if missing_live_entry && status == "Open" { "Outcome unavailable" } else { status };
    let comparison_id = string(audit, "scout_comparison_id");
    let paired_delta = number(audit, "paired_delta_percent");
    let evaluated_at = audit["scout_evaluated_timestamp_ms"].as_u64();
    let taker_fee_rate = number(audit, "scout_taker_fee_rate").unwrap_or(0.001);
    let open_net_return = None::<f64>;
    let display_return = net_return;
    let bar_position = display_return.map(|value| (50.0 + value / 2.0 * 40.0).clamp(10.0, 90.0));
    let bar_left = bar_position.map(|position| position.min(50.0));
    let bar_width = bar_position.map(|position| (position - 50.0).abs());
    let bar_tone = display_return.map(tone);
    let return_basis = net_return.map(|_| "strategy exit");
    let net_return_label = display_return.map(|value| format!("{}{value:.3}%", if value > 0.0 { "+" } else { "" }));
    Some(json!({
        "decisionId": decision_id, "symbol": symbol, "strategy": strategy, "source": "Scout",
        "profileSymbol": string(audit, "scout_parameter_profile_symbol")
            .or_else(|| string(audit, "scout_comparison_parameter_profile_symbol")),
        "strategyParameters": audit.get("strategy_parameters").cloned().unwrap_or_else(|| json!({})),
        "scoutLevelPercent": number(audit,"scout_level_percent"),
        "scoutParameterKey": string(audit,"scout_parameter_key"),
        "scoutParameterValue": number(audit,"scout_parameter_value"),
        "scoutParameterEntryEligible": audit.get("scout_parameter_entry_eligible").and_then(Value::as_bool),
        "comparisonRole": string(audit,"scout_comparison_role"), "pairedDeltaPercent": paired_delta,
        "scoutComparisonId": comparison_id,
        "scoutComparisonIds": audit.get("scout_comparison_ids").cloned().unwrap_or_else(|| json!([])),
        "scoutComparisonZeroRole": string(audit,"scout_comparison_zero_role"),
        "scoutComparisonParameterKey": string(audit,"scout_comparison_parameter_key"),
        "scoutComparisonParameterValue": number(audit,"scout_comparison_parameter_value"),
        "scoutComparisonParameterProfileSymbol": string(audit,"scout_comparison_parameter_profile_symbol"),
        "pairedDeltaTone": paired_delta.map(tone),
        "exitScenario": string(audit,"scout_exit_scenario").unwrap_or_else(|| "strategy".into()),
        "direction": direction, "strength": number(audit,"strength").unwrap_or(0.0), "openedAt": opened_at,
        "durationLabel": (status == "Open").then(|| duration_label(now.saturating_sub(opened_at))),
        "durationMs": (status == "Open").then(|| now.saturating_sub(opened_at)),
        "closedAt": evaluated_at, "entryPrice": entry_price, "entryBid": entry_bid, "entryAsk": entry_ask,
        "markPrice": null, "takerFeeRate": taker_fee_rate, "status": status, "netReturnPercent": net_return,
        "openNetReturnPercent": open_net_return, "returnBasis": return_basis,
        "horizonMarkouts": scout_horizon_markouts(audit, &direction),
        "displayNetReturnPercent": display_return, "netReturnLabel": net_return_label,
        "netReturnEstimated": status == "Open", "netVsBreakevenLeft": bar_left,
        "netVsBreakevenWidth": bar_width, "netVsBreakevenTone": bar_tone,
        "simulated": audit["simulated"].as_bool().unwrap_or(false)
    }))
}

fn outcome_instance(outcome: &ScoutOutcomeRequest) -> Value {
    json!({
        "decision_id": outcome.decision_id,
        "symbol": outcome.symbol,
        "strategy": outcome.strategy,
        "direction": "unknown",
        "timestamp": outcome.signal_timestamp_ms,
        "scout_net_return_percent": outcome.net_return_percent,
        "scout_evaluated_timestamp_ms": outcome.evaluated_timestamp_ms,
        "scout_taker_fee_rate": outcome.taker_fee_rate,
        "scout_learning_context": outcome.learning_context,
        "scout_exit_scenario": outcome.exit_scenario,
        "scout_sample_group_id": outcome.sample_group_id,
        "strategy_parameters": outcome.strategy_parameters,
        "scout_parameter_key": outcome.scout_parameter_key,
        "scout_parameter_value": outcome.scout_parameter_value,
        "scout_parameter_profile_symbol": outcome.scout_parameter_profile_symbol,
        "scout_comparison_id": outcome.scout_comparison_id,
        "scout_comparison_ids": outcome.scout_comparison_ids,
        "scout_comparison_role": outcome.scout_comparison_role,
        "scout_comparison_zero_role": outcome.scout_comparison_zero_role,
        "scout_comparison_parameter_key": outcome.scout_comparison_parameter_key,
        "scout_comparison_parameter_value": outcome.scout_comparison_parameter_value,
        "scout_comparison_parameter_profile_symbol": outcome.scout_comparison_parameter_profile_symbol,
        "scout_parameter_entry_eligible": false,
        "simulated": true
    })
}

fn count_rows(rows: &[Value], live_only: bool) -> ScoutCounts {
    let mut counts = ScoutCounts::default();
    for row in rows {
        if live_only && !string(row, "strategy").is_some_and(|strategy| strategy.starts_with("live-")) { continue; }
        counts.total += 1;
        match row["status"].as_str().unwrap_or_default() {
            "Open" => counts.open += 1,
            "Won" => counts.wins += 1,
            "Lost" => counts.losses += 1,
            "Flat" => counts.flats += 1,
            "Outcome unavailable" => counts.unavailable += 1,
            _ => {}
        }
    }
    counts
}

fn sort_rows(rows: &mut [Value], sort_by: &str, descending: bool) {
    let sort_key = match sort_by {
        "durationLabel" => "durationMs",
        "netReturnLabel" => "displayNetReturnPercent",
        _ => sort_by,
    };
    rows.sort_by(|left, right| {
        let ordering = match sort_by {
            "symbol" | "strategy" | "status" | "exitScenario" | "comparisonRole" => left[sort_key].as_str().cmp(&right[sort_key].as_str()),
            "durationLabel" => left[sort_key].as_u64().cmp(&right[sort_key].as_u64()),
            "scoutLevelPercent" | "displayNetReturnPercent" | "netReturnLabel" | "pairedDeltaPercent" | "strength" | "entryPrice" | "markPrice" => left[sort_key].as_f64().partial_cmp(&right[sort_key].as_f64()).unwrap_or(std::cmp::Ordering::Equal),
            _ => left["openedAt"].as_u64().cmp(&right["openedAt"].as_u64()),
        };
        let ordering = if descending { ordering.reverse() } else { ordering };
        ordering.then_with(|| left["decisionId"].as_str().cmp(&right["decisionId"].as_str()))
    });
}

fn count_json(counts: ScoutCounts) -> Value {
    json!({"total":counts.total,"open":counts.open,"resolved":counts.wins+counts.losses+counts.flats,"wins":counts.wins,"losses":counts.losses,"flats":counts.flats,"unavailable":counts.unavailable})
}

fn add_learning_result(groups: &mut BTreeMap<String, LearningAggregate>, key: String, mut group: LearningAggregate, row: &Value) {
    let aggregate = groups.entry(key).or_insert_with(|| { group.returns = Vec::new(); group.paired_deltas = Vec::new(); group });
    let status = row["status"].as_str().unwrap_or_default();
    if !matches!(status, "Won" | "Lost" | "Flat") { return; }
    aggregate.count += 1;
    aggregate.wins += usize::from(status == "Won");
    if let Some(value) = number(row, "netReturnPercent") { aggregate.returns.push(value); }
    if row["comparisonRole"] == "variant" {
        if let Some(value) = number(row, "pairedDeltaPercent") { aggregate.paired_deltas.push(value); }
    }
}

fn learning_rows(rows: &[Value]) -> Vec<Value> {
    let mut groups = BTreeMap::<String, LearningAggregate>::new();
    for row in rows {
        let (Some(symbol), Some(strategy)) = (string(row,"symbol"), string(row,"strategy")) else { continue; };
        let scenario = string(row,"exitScenario").unwrap_or_else(|| "strategy".into());
        let comparison = string(row,"comparisonRole");
        if comparison.is_none() {
            add_learning_result(&mut groups,format!("summary|{symbol}|{strategy}|{scenario}"),LearningAggregate{kind:"Scout summary".into(),symbol:symbol.clone(),strategy:strategy.clone(),scenario:scenario.clone(),..LearningAggregate::default()},row);
        }
        if let (Some(parameter),Some(value))=(string(row,"scoutParameterKey"),number(row,"scoutParameterValue")) {
            let eligible=row.get("scoutParameterEntryEligible").and_then(Value::as_bool);
            add_learning_result(&mut groups,format!("parameter|{symbol}|{strategy}|{scenario}|{parameter}|{value:.8}|{eligible:?}"),LearningAggregate{kind:"Parameter probe".into(),symbol:symbol.clone(),strategy:strategy.clone(),scenario:scenario.clone(),parameter:Some(parameter),value:Some(value),entry_eligible:eligible,..LearningAggregate::default()},row);
        }
        if let Some(level)=number(row,"scoutLevelPercent") {
            add_learning_result(&mut groups,format!("level|{symbol}|{strategy}|{scenario}|{level:.8}"),LearningAggregate{kind:"Breakout distance".into(),symbol,strategy,scenario,level:Some(level),..LearningAggregate::default()},row);
        }
    }
    groups.into_values().filter(|group| group.count > 0).map(|group| {
        let average=(!group.returns.is_empty()).then(||group.returns.iter().sum::<f64>()/group.returns.len() as f64);
        let paired=(!group.paired_deltas.is_empty()).then(||group.paired_deltas.iter().sum::<f64>()/group.paired_deltas.len() as f64);
        let win_rate=(group.count>0).then(||group.wins as f64*100.0/group.count as f64);
        json!({"kind":group.kind,"symbol":group.symbol,"strategy":group.strategy,"scenario":group.scenario,"parameter":group.parameter,"value":group.value,"entryEligible":group.entry_eligible,"level":group.level,"count":group.count,"wins":group.wins,"winRatePercent":win_rate,"winRateColor":win_rate.map(win_rate_color),"average":average,"averageTone":average.map(tone),"pairedDeltaAverage":paired,"pairedDeltaTone":paired.map(tone)})
    }).collect()
}

async fn get_scout_outcomes(
    State(service): State<ServiceState>,
    query: axum::extract::Query<ScoutOutcomesQuery>,
) -> axum::response::Response {
    let cache_key = format!(
        "{}|{}|{}|{}|{}|{}|{}",
        query.source.as_deref().unwrap_or(""),
        query.status.as_deref().unwrap_or(""),
        query.page.unwrap_or_default(),
        query.page_size.unwrap_or_default(),
        query.learning_symbol.as_deref().unwrap_or(""),
        query.sort_by.as_deref().unwrap_or(""),
        query.sort_direction.as_deref().unwrap_or(""),
    );
    let mut cache = service.scout_outcomes_cache.lock().await;
    let cache_ttl = std::time::Duration::from_secs(30);
    cache.retain(|_, (created, _)| created.elapsed() < cache_ttl);
    if let Some((_, bytes)) = cache.get(&cache_key) {
        return axum::response::Response::builder()
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(axum::body::Body::from(bytes.clone()))
            .expect("valid cached Scout outcomes response");
    }
    let now = scout_now_ms();
    let since = now.saturating_sub(7 * 86_400_000);
    let instances = match load_scout_instances_since(&service.scout.path, since) {
        Ok(instances) => instances,
        Err(error) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(error.to_string()))
                .expect("valid Scout outcomes error response");
        }
    };
    let mut rows = instances.values()
        .filter_map(|audit| instance_row(audit, now))
        .collect::<Vec<_>>();
    let instance_ids = rows
        .iter()
        .filter_map(|row| row["decisionId"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let simulated_outcomes = match load_scout_outcomes_since(&service.scout.path, since) {
        Ok(outcomes) => outcomes,
        Err(error) => {
            return axum::response::Response::builder()
                .status(axum::http::StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(error.to_string()))
                .expect("valid Scout outcomes error response");
        }
    };
    let simulated_outcomes = simulated_outcomes.into_iter()
        .filter(|outcome| !instance_ids.contains(&outcome.decision_id))
        .collect::<Vec<_>>();
    rows.extend(simulated_outcomes.iter().filter_map(|outcome| {
        instance_row(&outcome_instance(outcome), now)
    }));
    let sort_by = query.sort_by.as_deref().filter(|field| matches!(*field, "symbol" | "strategy" | "status" | "exitScenario" | "scoutLevelPercent" | "openedAt" | "durationLabel" | "entryPrice" | "markPrice" | "netReturnLabel" | "displayNetReturnPercent" | "comparisonRole" | "pairedDeltaPercent" | "strength")).unwrap_or("openedAt");
    let descending = query.sort_direction.as_deref() != Some("asc");
    sort_rows(&mut rows, sort_by, descending);
    let all_summary = count_json(count_rows(&rows, false));
    let live_summary = count_json(count_rows(&rows, true));
    let mut pending_parameters = BTreeMap::<
        (String, String, String, String),
        BTreeSet<String>,
    >::new();
    for row in rows.iter().filter(|row| row["status"] == "Open") {
        let Some(strategy) = row["strategy"].as_str().filter(|strategy| !strategy.starts_with("live-")) else { continue; };
        let Some(comparison_id) = row["scoutComparisonId"].as_str() else { continue; };
        if row["comparisonRole"] != "variant" && row["scoutComparisonZeroRole"] != "variant" {
            continue;
        }
        let parameter = row["scoutParameterKey"]
            .as_str()
            .or_else(|| row["scoutComparisonParameterKey"].as_str());
        let candidate = row["scoutParameterValue"]
            .as_f64()
            .or_else(|| row["scoutComparisonParameterValue"].as_f64());
        let (Some(parameter), Some(candidate)) = (parameter, candidate) else { continue; };
        let symbol = row["scoutParameterProfileSymbol"]
            .as_str()
            .or_else(|| row["scoutComparisonParameterProfileSymbol"].as_str())
            .or_else(|| row["symbol"].as_str())
            .unwrap_or_default();
        pending_parameters
            .entry((symbol.to_owned(), strategy.to_owned(), parameter.to_owned(), format!("{candidate:.8}")))
            .or_default()
            .insert(comparison_id.to_owned());
    }
    let pending_parameter_probes = pending_parameters
        .into_iter()
        .filter_map(|((symbol, strategy, parameter, value), comparisons)| {
            Some(json!({
                "symbol": symbol,
                "strategy": strategy,
                "parameter": parameter,
                "candidateValue": value.parse::<f64>().ok()?,
                "openSamples": comparisons.len(),
            }))
        })
        .collect::<Vec<_>>();
    let all_learning_rows=learning_rows(&rows);
    let symbols=all_learning_rows.iter().filter_map(|row| row["symbol"].as_str().map(str::to_owned)).collect::<BTreeSet<_>>();
    let requested_symbol=query.learning_symbol.as_deref().filter(|symbol| symbols.contains(*symbol));
    let visible_learning_rows=all_learning_rows.into_iter().filter(|row| requested_symbol.is_none_or(|symbol| row["symbol"].as_str()==Some(symbol))).collect::<Vec<_>>();
    let learning_outcomes=rows.iter().filter(|row| requested_symbol.is_none_or(|symbol| row["symbol"].as_str()==Some(symbol)))
        .filter(|row| row["status"]!="Open" && row.get("scoutComparisonId").is_none_or(Value::is_null))
        .filter_map(|row| number(row,"netReturnPercent").map(|value|json!({"strategy":row["strategy"],"netReturnPercent":value,"parameterKey":row["scoutParameterKey"],"parameterValue":row["scoutParameterValue"]}))).collect::<Vec<_>>();
    let source=if query.source.as_deref()==Some("all"){"all"}else{"live"};
    let status=query.status.as_deref().filter(|status|matches!(*status,"open"|"closed"));
    let mut filtered=rows.into_iter().filter(|row| source=="all" || row["strategy"].as_str().is_some_and(|strategy|strategy.starts_with("live-")))
        .filter(|row| status.is_none_or(|status| if status=="open" {row["status"]=="Open"}else{row["status"]!="Open"})).collect::<Vec<_>>();
    let page_size=query.page_size.unwrap_or(50).clamp(1,100);
    let total=filtered.len();
    let page_count=total.div_ceil(page_size);
    let page=query.page.unwrap_or(0).min(page_count.saturating_sub(1));
    let page_rows=filtered.drain(page.saturating_mul(page_size)..).take(page_size).collect::<Vec<_>>();
    let body = serde_json::to_vec(&json!({"rows":page_rows,"summary":{"all":all_summary,"live":live_summary},"learningRows":visible_learning_rows,"learningOutcomes":learning_outcomes,"pendingParameterProbes":pending_parameter_probes,"symbols":symbols,"learningSymbol":requested_symbol,"updatedAt":now,"source":source,"status":status.unwrap_or("all"),"sortBy":sort_by,"sortDirection":if descending {"desc"} else {"asc"},"page":page,"pageSize":page_size,"pageCount":page_count,"totalRows":total,"pageNumber":page+1,"hasPrevious":page>0,"hasNext":page+1<page_count}))
        .unwrap_or_default();
    let bytes = axum::body::Bytes::from(body);
    cache_scout_outcomes_response(&mut cache, cache_key, bytes.clone());
    axum::response::Response::builder()
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(axum::body::Body::from(bytes))
        .expect("valid Scout outcomes response")
}

/// Read one bounded page from the durable Scout outcome collection. This
/// bypasses the runtime cache while keeping each RSocket response bounded.
async fn get_scout_replay_outcomes(
    State(service): State<ServiceState>,
    query: axum::extract::Query<ScoutReplayQuery>,
) -> Result<axum::Json<Value>, (axum::http::StatusCode, String)> {
    const DEFAULT_PAGE_SIZE: usize = 25;
    const MAX_PAGE_SIZE: usize = 100;
    let page = query.page.unwrap_or_default();
    let page_size = query.page_size.unwrap_or(DEFAULT_PAGE_SIZE).clamp(1, MAX_PAGE_SIZE);
    let (outcomes, total_records, malformed_records, first_timestamp_ms, last_timestamp_ms) =
        load_scout_replay_outcome_page(&service.scout.path, page, page_size)
        .map_err(|error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let decision_ids = outcomes.iter().map(|(_, outcome)| outcome.decision_id.as_str()).collect::<Vec<_>>();
    let instance_rows = load_scout_replay_instances(
        &service.scout.path,
        &decision_ids,
    ).map_err(|error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let page_start = page.saturating_mul(page_size).min(total_records);
    let replay_outcomes = outcomes
        .iter()
        .take(page_size)
        .map(|(record_key, outcome)| {
            replay_outcome_payload(
                record_key,
                outcome,
                query.origin_at_ms,
                instance_rows.get(&outcome.decision_id),
            )
        })
        .collect::<Vec<_>>();
    let before_origin_records = query.origin_at_ms.map(|origin| {
        outcomes.iter().filter(|(_, outcome)| outcome.evaluated_timestamp_ms < origin).count()
    });
    let at_or_after_origin_records = query.origin_at_ms.map(|origin| {
        outcomes.iter().filter(|(_, outcome)| outcome.evaluated_timestamp_ms >= origin).count()
    });
    let missing_regime_records = outcomes.iter().filter(|(_, outcome)| {
        canonical_market_regime(outcome.market_regime.as_deref()).is_none()
    }).count();
    let fee_rate_and_net_return_records = outcomes.iter().filter(|(_, outcome)| {
        outcome.taker_fee_rate.is_finite()
            && outcome.taker_fee_rate >= 0.0
            && outcome.net_return_percent.is_finite()
    }).count();
    let explicit_cost_breakdown_records = replay_outcomes
        .iter()
        .filter(|outcome| outcome["cost_breakdown"].is_object())
        .count();
    Ok(axum::Json(json!({
        "datasetVersion": 2,
        "source": "scout_records.outcomes",
        "ordering": "evaluated_timestamp_ms_asc_then_record_key_asc",
        "coverage": {
            "status": if malformed_records == 0 && page == 0 && replay_outcomes.len() == total_records { "complete" } else { "incomplete" },
            "complete": malformed_records == 0 && page == 0 && replay_outcomes.len() == total_records,
            "totalDurableRecords": total_records,
            "returnedRecords": replay_outcomes.len(),
            "page": page,
            "pageSize": page_size,
            "hasMore": page_start.saturating_add(page_size) < total_records,
            "malformedRecords": malformed_records,
            "firstEvaluatedTimestampMs": first_timestamp_ms,
            "lastEvaluatedTimestampMs": last_timestamp_ms,
            "runtimeHydrationLimit": MAX_CACHED_SCOUT_OUTCOMES,
            "originAtMs": query.origin_at_ms,
            "completedBeforeOriginRecords": before_origin_records,
            "atOrAfterOriginRecords": at_or_after_origin_records,
            "missingRegimeRecords": missing_regime_records,
            "feeRateAndNetReturnReportedRecords": fee_rate_and_net_return_records,
            "explicitCostBreakdownRecords": explicit_cost_breakdown_records,
            "explicitCostBreakdown": explicit_cost_breakdown_records == replay_outcomes.len(),
        },
        "outcomes": replay_outcomes,
    })))
}

async fn get_scout_replay_searches(
    State(service): State<ServiceState>,
) -> Result<axum::Json<Value>, (axum::http::StatusCode, String)> {
    let (searches, total_records, malformed_records) =
        load_all_scout_searches(&service.scout.path)
            .map_err(|error| (axum::http::StatusCode::INTERNAL_SERVER_ERROR, error.to_string()))?;
    let complete = malformed_records == 0 && searches.len() == total_records;
    let first_as_of_timestamp_ms = searches
        .first()
        .map(|(_, search)| search.as_of_timestamp_ms);
    let last_as_of_timestamp_ms = searches
        .last()
        .map(|(_, search)| search.as_of_timestamp_ms);
    let searches = searches
        .into_iter()
        .map(|(record_key, search)| {
            let mut value = serde_json::to_value(search).unwrap_or(Value::Null);
            value["search_id"] = json!(record_key);
            value
        })
        .collect::<Vec<_>>();
    Ok(axum::Json(json!({
        "datasetVersion": 1,
        "source": "scout_records.scout_search_ledger",
        "ordering": "as_of_timestamp_ms_asc_then_record_key_asc",
        "coverage": {
            "status": if complete { "complete" } else { "incomplete" },
            "complete": complete,
            "totalDurableRecords": total_records,
            "returnedRecords": searches.len(),
            "malformedRecords": malformed_records,
            "firstAsOfTimestampMs": first_as_of_timestamp_ms,
            "lastAsOfTimestampMs": last_as_of_timestamp_ms,
            "runtimeCacheLimit": MAX_CACHED_SCOUT_SEARCH_LEDGER,
        },
        "searches": searches,
    })))
}

fn load_scout_replay_outcome_page(
    path: &std::path::Path,
    page: usize,
    page_size: usize,
) -> anyhow::Result<(
    Vec<(String, ScoutOutcomeRequest)>,
    usize,
    usize,
    Option<u64>,
    Option<u64>,
)> {
    let connection = open_scout_read_connection(path)?;
    let total_records: usize = connection.query_row(
        "SELECT COUNT(*) FROM scout_records WHERE collection='outcomes'",
        [],
        |row| row.get::<_, i64>(0),
    )?.max(0) as usize;
    let (first_timestamp_ms, last_timestamp_ms): (Option<i64>, Option<i64>) = connection.query_row(
        "SELECT MIN(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.evaluated_timestamp_ms') AS INTEGER) END), MAX(CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.evaluated_timestamp_ms') AS INTEGER) END) FROM scout_records WHERE collection='outcomes'",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='outcomes' ORDER BY CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.evaluated_timestamp_ms') AS INTEGER) ELSE 0 END, record_key LIMIT ?1 OFFSET ?2",
    )?;
    let offset = page.saturating_mul(page_size).min(i64::MAX as usize) as i64;
    let rows = statement.query_map(rusqlite::params![page_size as i64, offset], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut outcomes = Vec::with_capacity(page_size);
    let mut malformed_records = 0;
    for row in rows {
        match row.and_then(|(record_key, payload)| {
            serde_json::from_str::<ScoutOutcomeRequest>(&payload)
                .map_err(|error| rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                ))
                .map(|outcome| (record_key, outcome))
        }) {
            Ok(outcome) => outcomes.push(outcome),
            Err(_) => malformed_records += 1,
        }
    }
    Ok((
        outcomes,
        total_records,
        malformed_records,
        first_timestamp_ms.map(|value| value.max(0) as u64),
        last_timestamp_ms.map(|value| value.max(0) as u64),
    ))
}

fn load_scout_replay_instances(
    path: &std::path::Path,
    decision_ids: &[&str],
) -> anyhow::Result<BTreeMap<String, Value>> {
    let connection = open_scout_read_connection(path)?;
    let mut statement = connection.prepare(
        "SELECT payload FROM scout_records WHERE collection='scout_instances' AND record_key=?1",
    )?;
    let mut instances = BTreeMap::new();
    for decision_id in decision_ids {
        let payload = statement
            .query_row([decision_id], |row| row.get::<_, String>(0))
            .optional()?;
        if let Some(payload) = payload {
            instances.insert((*decision_id).to_owned(), serde_json::from_str(&payload)?);
        }
    }
    Ok(instances)
}

fn load_all_scout_searches(
    path: &std::path::Path,
) -> anyhow::Result<(Vec<(String, ScoutSearchLedger)>, usize, usize)> {
    let connection = open_scout_read_connection(path)?;
    let total_records: usize = connection
        .query_row(
            "SELECT COUNT(*) FROM scout_records WHERE collection='scout_search_ledger'",
            [],
            |row| row.get::<_, i64>(0),
        )?
        .max(0) as usize;
    let mut statement = connection.prepare(
        "SELECT record_key, payload FROM scout_records WHERE collection='scout_search_ledger' ORDER BY CASE WHEN json_valid(payload) THEN CAST(json_extract(payload, '$.as_of_timestamp_ms') AS INTEGER) ELSE 0 END, record_key",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    let mut searches = Vec::with_capacity(total_records);
    let mut malformed_records = 0;
    for row in rows {
        match row.and_then(|(record_key, payload)| {
            serde_json::from_str::<ScoutSearchLedger>(&payload)
                .map(|search| (record_key, search))
                .map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })
        }) {
            Ok(search) => searches.push(search),
            Err(_) => malformed_records += 1,
        }
    }
    Ok((searches, total_records, malformed_records))
}

fn replay_outcome_payload(
    record_key: &str,
    outcome: &ScoutOutcomeRequest,
    origin_at_ms: Option<u64>,
    instance: Option<&Value>,
) -> Value {
    let profile_symbol = outcome
        .scout_parameter_profile_symbol
        .as_deref()
        .or(outcome.scout_comparison_parameter_profile_symbol.as_deref());
    let profile_symbol_used = profile_symbol.unwrap_or(&outcome.symbol);
    let mut parameters = outcome.strategy_parameters.clone();
    let tested_parameters = parameter_keys(outcome);
    if let Some(tested) = &tested_parameters {
        parameters.extend(tested.clone());
    }
    let profile_identity_complete = is_liquid_scout_symbol(profile_symbol_used)
        && is_liquid_scout_symbol(&outcome.symbol)
        && strategies::Strategy::from_name(&outcome.strategy).is_some()
        && outcome.strategy_parameters.values().all(|value| value.is_finite())
        && tested_parameters.is_some();
    let profile_id = profile_identity_complete.then(|| {
        strategies::profile_id(profile_symbol_used, &outcome.strategy, &parameters)
    });
    let regime_status = match canonical_market_regime(outcome.market_regime.as_deref()) {
        Some(_) => "canonical_regime_recorded",
        None => "regime_missing_or_legacy_unconditioned",
    };
    let fee_status = if outcome.taker_fee_rate.is_finite()
        && outcome.taker_fee_rate >= 0.0
        && outcome.net_return_percent.is_finite()
    {
        "net_return_and_fee_rate_reported"
    } else {
        "fee_or_net_return_missing_or_invalid"
    };
    let (evidence_group_id, evidence_group_source) = if !outcome.sample_group_id.is_empty() {
        (outcome.sample_group_id.clone(), "sample_group_id")
    } else if let Some(comparison_id) = outcome
        .scout_comparison_id
        .as_deref()
        .or_else(|| outcome.scout_comparison_ids.first().map(String::as_str))
    {
        (comparison_id.to_owned(), "comparison_id")
    } else {
        (outcome.decision_id.clone(), "decision_id_fallback")
    };
    let eligible_before_origin = origin_at_ms.map(|origin| outcome.evaluated_timestamp_ms < origin);
    let eligibility_status = match (eligible_before_origin, profile_identity_complete, fee_status) {
        (None, _, _) => "origin_not_supplied",
        (Some(false), _, _) => "outcome_not_completed_before_origin",
        (Some(true), false, _) => "profile_identity_incomplete",
        (Some(true), _, "fee_or_net_return_missing_or_invalid") => "fee_or_return_incomplete",
        (Some(true), true, _) => "eligible_before_origin",
    };
    let cost_breakdown = scout_replay_cost_breakdown(outcome, instance);
    let mut payload = serde_json::to_value(outcome).unwrap_or(Value::Null);
    payload["outcome_id"] = json!(record_key);
    payload["label_window_start_timestamp_ms"] = json!(outcome.signal_timestamp_ms);
    payload["label_window_end_timestamp_ms"] = json!(outcome.evaluated_timestamp_ms);
    payload["label_window_status"] = json!(
        if outcome.evaluated_timestamp_ms >= outcome.signal_timestamp_ms {
            "observed_signal_to_outcome_completion"
        } else {
            "invalid_timestamp_order"
        }
    );
    payload["profile_id"] = json!(profile_id);
    payload["profile_symbol"] = json!(profile_symbol_used);
    payload["profile_symbol_source"] = json!(if profile_symbol.is_some() {
        "explicit_profile_symbol"
    } else {
        "market_symbol_fallback"
    });
    payload["evidence_group_id"] = json!(evidence_group_id);
    payload["evidence_group_source"] = json!(evidence_group_source);
    payload["notional_usd"] = instance.and_then(|audit| audit["scout_notional_usd"].as_f64()).into();
    payload["portfolio_weight"] = instance
        .and_then(|audit| {
            audit["strategy_order_size_multiplier"]
                .as_f64()
                .or_else(|| audit["strategy_decision"]["strategy_order_size_multiplier"].as_f64())
        })
        .into();
    payload["cost_breakdown"] = cost_breakdown.clone().unwrap_or(Value::Null);
    payload["eligibility"] = json!({
        "status": eligibility_status,
        "eligibleBeforeOrigin": eligible_before_origin,
        "profileIdentity": if profile_identity_complete { "reconstructed" } else { "incomplete" },
        "marketSymbol": if is_liquid_scout_symbol(&outcome.symbol) { "allowed" } else { "missing_or_disallowed" },
        "profileSymbol": if is_liquid_scout_symbol(profile_symbol_used) { "allowed" } else { "missing_or_disallowed" },
        "regime": regime_status,
        "cost": fee_status,
        "costBreakdown": if cost_breakdown.is_some() { "recorded_on_scout_markout" } else { "not_recorded" },
        "evidenceGroup": evidence_group_source,
    });
    payload
}

fn scout_replay_cost_breakdown(
    outcome: &ScoutOutcomeRequest,
    instance: Option<&Value>,
) -> Option<Value> {
    let observations = instance?.get("scout_markout_observations")?.as_array()?;
    let observation = if let Some(horizon) = outcome
        .exit_scenario
        .strip_prefix("horizon-")
        .and_then(|seconds| seconds.parse::<u64>().ok())
    {
        observations.iter().find(|observation| {
            observation["kind"] == "horizon" && observation["horizon_seconds"].as_u64() == Some(horizon)
        })?
    } else {
        observations
            .iter()
            .find(|observation| observation["kind"] == "strategy_exit")?
    };
    let (gross_return_percent, net_return_percent) = match instance?["direction"].as_str()? {
        "bullish" => (
            observation["long_gross_percent"].as_f64(),
            observation["long_net_percent"].as_f64(),
        ),
        "bearish" => (
            observation["short_gross_percent"].as_f64(),
            observation["short_net_percent"].as_f64(),
        ),
        _ => return None,
    };
    Some(json!({
        "coverageStatus": "partial",
        "includesAllExecutionCosts": false,
        "missingCostComponents": ["itemized_entry_fee", "itemized_exit_fee", "funding", "borrow", "carry", "historical_portfolio_valuation"],
        "takerFeeRate": observation["taker_fee_rate"],
        "grossReturnPercent": gross_return_percent,
        "markoutNetReturnPercent": net_return_percent,
        "outcomeNetReturnPercent": outcome.net_return_percent,
        "executionCostPercent": gross_return_percent.zip(net_return_percent).map(|(gross, net)| gross - net),
        "referenceEntryPrice": observation["reference_entry_price"],
        "referenceExitPrice": observation["reference_exit_price"],
        "entryBid": observation["entry_bid"],
        "entryAsk": observation["entry_ask"],
        "exitBid": observation["exit_bid"],
        "exitAsk": observation["exit_ask"],
    }))
}

const MAX_CACHED_SCOUT_OUTCOMES_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const MAX_CACHED_SCOUT_OUTCOMES_BYTES: usize = 8 * 1024 * 1024;
const MAX_CACHED_SCOUT_OUTCOMES_RESPONSES: usize = 4;

fn cache_scout_outcomes_response(
    cache: &mut BTreeMap<String, (std::time::Instant, axum::body::Bytes)>,
    key: String,
    bytes: axum::body::Bytes,
) {
    if bytes.len() > MAX_CACHED_SCOUT_OUTCOMES_RESPONSE_BYTES {
        return;
    }
    cache.insert(key, (std::time::Instant::now(), bytes));
    while cache.len() > MAX_CACHED_SCOUT_OUTCOMES_RESPONSES
        || cache.values().map(|(_, bytes)| bytes.len()).sum::<usize>()
            > MAX_CACHED_SCOUT_OUTCOMES_BYTES
    {
        let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, (created, _))| *created)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        cache.remove(&oldest_key);
    }
}

#[cfg(test)]
mod scout_report_tests {
    use super::*;
    use http_body_util::BodyExt;

    fn audit() -> Value {
        json!({"decision_id":"sample-1","symbol":"BTC-USDT","strategy":"live-trade-flow","direction":"bullish","timestamp":1000,"strength":0.8,"scout_entry_price":100.0,"scout_exit_scenario":"strategy","scout":true})
    }

    #[test]
    fn replay_outcome_identity_and_origin_cutoff_are_explicit() {
        let outcome: ScoutOutcomeRequest = serde_json::from_value(json!({
            "decision_id":"decision-1",
            "learning_context":"scout:trend",
            "simulated_net_pnl_usd":1.0,
            "net_return_percent":0.5,
            "signal_timestamp_ms":100,
            "evaluated_timestamp_ms":200,
            "taker_fee_rate":0.001,
            "symbol":"BTC-USDT",
            "strategy":"live-momentum",
            "strategy_parameters":{},
            "exit_scenario":"strategy"
        }))
        .unwrap();

        let at_origin = replay_outcome_payload("decision-1|strategy|single", &outcome, Some(200), None);
        assert_eq!(at_origin["outcome_id"], "decision-1|strategy|single");
        assert_eq!(at_origin["eligibility"]["eligibleBeforeOrigin"], false);
        assert_eq!(at_origin["eligibility"]["status"], "outcome_not_completed_before_origin");
        assert_eq!(at_origin["eligibility"]["regime"], "regime_missing_or_legacy_unconditioned");
        assert_eq!(at_origin["eligibility"]["costBreakdown"], "not_recorded");
        assert!(at_origin["cost_breakdown"].is_null());
        assert_eq!(at_origin["evidence_group_id"], "decision-1");
        assert_eq!(at_origin["evidence_group_source"], "decision_id_fallback");

        let after_origin = replay_outcome_payload("decision-1|strategy|single", &outcome, Some(201), None);
        assert_eq!(after_origin["eligibility"]["eligibleBeforeOrigin"], true);
        assert_eq!(after_origin["eligibility"]["status"], "eligible_before_origin");
    }

    #[test]
    fn replay_exposes_modeled_gross_net_and_execution_cost_when_markout_exists() {
        let outcome: ScoutOutcomeRequest = serde_json::from_value(json!({
            "decision_id":"decision-cost",
            "learning_context":"scout:trend",
            "simulated_net_pnl_usd":0.8,
            "net_return_percent":0.8,
            "signal_timestamp_ms":100,
            "evaluated_timestamp_ms":200,
            "taker_fee_rate":0.001,
            "symbol":"BTC-USDT",
            "strategy":"live-momentum",
            "strategy_parameters":{},
            "exit_scenario":"strategy"
        }))
        .unwrap();
        let instance = json!({
            "direction":"bullish",
            "scout_notional_usd":100.0,
            "strategy_order_size_multiplier":0.5,
            "scout_markout_observations":[{
                "kind":"strategy_exit",
                "taker_fee_rate":0.001,
                "long_gross_percent":1.0,
                "long_net_percent":0.8,
                "reference_entry_price":100.0,
                "reference_exit_price":100.8
            }]
        });
        let replay = replay_outcome_payload("decision-cost|strategy|single", &outcome, Some(300), Some(&instance));
        assert_eq!(replay["notional_usd"], 100.0);
        assert_eq!(replay["portfolio_weight"], 0.5);
        assert_eq!(replay["label_window_start_timestamp_ms"], 100);
        assert_eq!(replay["label_window_end_timestamp_ms"], 200);
        assert_eq!(replay["cost_breakdown"]["grossReturnPercent"], 1.0);
        assert_eq!(replay["cost_breakdown"]["markoutNetReturnPercent"], 0.8);
        assert_eq!(replay["cost_breakdown"]["coverageStatus"], "partial");
        assert_eq!(replay["cost_breakdown"]["includesAllExecutionCosts"], false);
        assert!(
            (replay["cost_breakdown"]["executionCostPercent"]
                .as_f64()
                .unwrap()
                - 0.2)
                .abs()
                < 1e-12
        );
        assert_eq!(replay["eligibility"]["costBreakdown"], "recorded_on_scout_markout");
    }

    #[test]
    fn replay_search_reader_returns_all_durable_rows_in_as_of_order() {
        let path = std::env::temp_dir().join(format!(
            "scout-replay-search-reader-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE scout_records (collection TEXT NOT NULL, record_key TEXT NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(collection, record_key));",
            )
            .unwrap();
        let search = |request_id: &str, as_of_timestamp_ms| ScoutSearchLedger {
            request_id: request_id.into(),
            as_of_timestamp_ms,
            outcome_cutoff_timestamp_ms: as_of_timestamp_ms,
            selection_metric: Some("effective_sample_shortfall_to_10_then_rotation".into()),
            selection_metric_status: "recorded".into(),
            selection_target_effective_sample_size: Some(10.0),
            baseline_candidates: Vec::new(),
            baseline_status: "not_provided".into(),
            evaluation_window: ScoutSearchEvaluationWindow {
                start_timestamp_ms: as_of_timestamp_ms,
                end_timestamp_ms: None,
                end_rule: "outcome completion".into(),
                fixed_horizon: false,
            },
            candidates: Vec::new(),
            selected_candidate_ids: Vec::new(),
            selected_candidates: Vec::new(),
            response: ScoutPlanResponse::default(),
        };
        for (key, value) in [
            ("later", serde_json::to_string(&search("later", 200)).unwrap()),
            ("earlier", serde_json::to_string(&search("earlier", 100)).unwrap()),
            ("malformed", "not-json".into()),
        ] {
            connection
                .execute(
                    "INSERT INTO scout_records(collection, record_key, payload) VALUES('scout_search_ledger', ?1, ?2)",
                    rusqlite::params![key, value],
                )
                .unwrap();
        }
        drop(connection);

        let (searches, total, malformed) = load_all_scout_searches(&path).unwrap();
        assert_eq!(total, 3);
        assert_eq!(malformed, 1);
        assert_eq!(searches.len(), 2);
        assert_eq!(searches[0].0, "earlier");
        assert_eq!(searches[0].1.as_of_timestamp_ms, 100);
        assert_eq!(searches[1].0, "later");
        assert_eq!(searches[1].1.as_of_timestamp_ms, 200);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
        let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
    }

    #[test]
    fn report_rows_are_built_from_scout_owned_instances_and_outcomes() {
        let row = instance_row(&audit(), 2000).unwrap();
        assert_eq!(row["status"], "Open");
        assert_eq!(row["source"], "Scout");
        assert_eq!(row["decisionId"], "sample-1");

        let mut completed = audit();
        completed["scout_net_return_percent"] = json!(-0.25);
        completed["scout_evaluated_timestamp_ms"] = json!(1500);
        let row = instance_row(&completed, 2000).unwrap();
        assert_eq!(row["status"], "Lost");
        assert_eq!(row["netReturnPercent"], -0.25);
        assert_eq!(row["closedAt"], 1500);
    }

    #[test]
    fn scout_outcomes_response_cache_is_bounded_by_bytes_and_entry_count() {
        let mut cache = BTreeMap::new();
        let cached_size = 3 * 1024 * 1024;
        for index in 0..4 {
            cache_scout_outcomes_response(
                &mut cache,
                format!("response-{index}"),
                axum::body::Bytes::from(vec![0; cached_size]),
            );
        }
        assert_eq!(cache.len(), 2);
        assert!(!cache.contains_key("response-0"));
        assert!(!cache.contains_key("response-1"));
        assert!(cache.contains_key("response-2"));
        assert!(cache.contains_key("response-3"));
        assert!(cache.values().map(|(_, bytes)| bytes.len()).sum::<usize>()
            <= MAX_CACHED_SCOUT_OUTCOMES_BYTES);

        cache_scout_outcomes_response(
            &mut cache,
            "oversized".into(),
            axum::body::Bytes::from(vec![0; MAX_CACHED_SCOUT_OUTCOMES_RESPONSE_BYTES + 1]),
        );
        assert!(!cache.contains_key("oversized"));
        assert_eq!(cache.len(), 2);
    }

    #[tokio::test]
    async fn persisted_scout_report_rows_survive_worker_restart() {
        let path = std::env::temp_dir().join(format!(
            "scout-report-restart-{}-{}.sqlite",
            std::process::id(),
            scout_now_ms()
        ));
        let now = scout_now_ms();
        let mut state = PersistedState::default();
        let mut stored = audit();
        stored["timestamp"] = json!(now);
        state.scout_instances.insert("sample-1".into(), stored.clone());
        persist_scout_state(&path, &state, &["scout_instances"]).unwrap();

        let restarted = load_scout_state(&path).unwrap();
        assert!(restarted.scout_instances.is_empty());
        let service = ServiceState {
            learning_rsocket_url: String::new(),
            insights_rsocket_url: String::new(),
            strategy_outcomes_by_profile: Arc::clone(&restarted.strategy_outcomes_by_profile),
            portfolio_instances_by_group: Arc::clone(&restarted.portfolio_instances_by_group),
            strategy_outcome_profile_index_ready: Arc::new(
                std::sync::atomic::AtomicBool::new(true),
            ),
            live_scout_maximum_duration_ms: Arc::new(std::sync::RwLock::new(BTreeMap::new())),
            live_profile_snapshot_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout_outcomes_cache: Arc::new(Mutex::new(BTreeMap::new())),
            scout: ScoutState {
                data: Arc::new(Mutex::new(restarted)),
                path: path.clone(),
                flush_lock: Arc::new(Mutex::new(())),
                flush_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            },
        };
        let response = get_scout_outcomes(
            State(service),
            axum::extract::Query(ScoutOutcomesQuery {
                page: Some(0),
                page_size: Some(50),
                source: Some("all".into()),
                status: None,
                learning_symbol: None,
                sort_by: None,
                sort_direction: None,
            }),
        )
        .await;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        let report: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(report["totalRows"], 1);
        assert_eq!(report["rows"][0]["decisionId"], "sample-1");
        assert_eq!(report["rows"][0]["symbol"], "BTC-USDT");
        assert_eq!(report["rows"][0]["status"], "Open");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn horizon_markouts_report_directional_net_and_excursion_values() {
        let mut bullish = audit();
        bullish["scout_markout_observations"] = json!([{
            "kind": "horizon",
            "horizon_seconds": 60,
            "evaluated_timestamp_ms": 61_000,
            "reference_entry_price": 100.0,
            "long_gross_percent": 0.1,
            "short_gross_percent": -0.1,
            "long_net_percent": 0.05,
            "short_net_percent": -0.15,
            "path_high": 102.0,
            "path_low": 99.0,
            "exit_bid": 100.0,
            "exit_ask": 100.2,
            "long_time_to_positive_net_seconds": 25,
            "short_time_to_positive_net_seconds": null,
            "taker_fee_rate": 0.001
        },{
            "kind": "strategy_exit",
            "evaluated_timestamp_ms": 30_000,
            "exit_bid": 100.0,
            "exit_ask": 100.2,
            "taker_fee_rate": 0.001
        }]);
        let markout = instance_row(&bullish, 70_000).unwrap()["horizonMarkouts"]["60"].clone();
        assert_eq!(markout["netReturnPercent"], 0.05);
        assert_eq!(markout["executionCostPercent"], 0.05);
        assert!((markout["bestFavorablePercent"].as_f64().unwrap() - 2.0).abs() < 1e-9);
        assert!((markout["worstAdversePercent"].as_f64().unwrap() + 1.0).abs() < 1e-9);
        assert_eq!(markout["timeToPositiveNetSeconds"], 25);
        assert!(
            (markout["postExitNetReturnPercent"].as_f64().unwrap() + 0.3996007984).abs() < 1e-8
        );

        bullish["direction"] = json!("bearish");
        let short_markout =
            instance_row(&bullish, 70_000).unwrap()["horizonMarkouts"]["60"].clone();
        assert_eq!(short_markout["netReturnPercent"], -0.15);
        assert!((short_markout["bestFavorablePercent"].as_f64().unwrap() - 1.0).abs() < 1e-9);
        assert!((short_markout["worstAdversePercent"].as_f64().unwrap() + 2.0).abs() < 1e-9);
    }

    #[test]
    fn reports_expire_unresolved_instances_after_seven_days() {
        let row = instance_row(&audit(), 8 * 86_400_000).unwrap();
        assert_eq!(row["status"], "Outcome unavailable");
    }

    #[test]
    fn sorts_before_pagination_using_requested_backend_field_and_direction() {
        let mut rows = vec![
            json!({"decisionId":"older","openedAt":10,"displayNetReturnPercent":2.0}),
            json!({"decisionId":"newer","openedAt":20,"displayNetReturnPercent":-1.0}),
            json!({"decisionId":"middle","openedAt":15,"displayNetReturnPercent":0.5}),
        ];
        sort_rows(&mut rows, "displayNetReturnPercent", true);
        assert_eq!(rows.iter().map(|row| row["decisionId"].as_str().unwrap()).collect::<Vec<_>>(), vec!["older", "middle", "newer"]);
        sort_rows(&mut rows, "openedAt", false);
        assert_eq!(rows.iter().map(|row| row["decisionId"].as_str().unwrap()).collect::<Vec<_>>(), vec!["older", "middle", "newer"]);
    }

    #[test]
    fn scout_history_can_exceed_five_thousand_rows() {
        let rows = (0..5_002)
            .map(|index| {
                let mut row = audit();
                row["decision_id"] = json!(format!("live-{index}"));
                row["strategy"] = json!("live-trade-flow");
                instance_row(&row, 2_000).unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(rows.len(), 5_002);
        assert_eq!(count_rows(&rows, true).total, 5_002);
    }
}
