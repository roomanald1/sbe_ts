use serde_json::{Value, json};

#[derive(Deserialize)]
struct ScoutOutcomesQuery {
    page: Option<usize>,
    page_size: Option<usize>,
    source: Option<String>,
    status: Option<String>,
    learning_symbol: Option<String>,
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

fn instance_row(audit: &Value, now: u64) -> Option<Value> {
    let decision_id = string(audit, "decision_id")?;
    let symbol = string(audit, "symbol")?;
    let strategy = string(audit, "strategy").unwrap_or_else(|| "live-insights".into());
    let direction = string(audit, "direction")?;
    if !matches!(direction.as_str(), "bullish" | "bearish") { return None; }
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
        "scoutLevelPercent": number(audit,"scout_level_percent"),
        "scoutParameterKey": string(audit,"scout_parameter_key"),
        "scoutParameterValue": number(audit,"scout_parameter_value"),
        "scoutParameterEntryEligible": audit.get("scout_parameter_entry_eligible").and_then(Value::as_bool),
        "comparisonRole": string(audit,"scout_comparison_role"), "pairedDeltaPercent": paired_delta,
        "pairedDeltaTone": paired_delta.map(tone),
        "exitScenario": string(audit,"scout_exit_scenario").unwrap_or_else(|| "strategy".into()),
        "direction": direction, "strength": number(audit,"strength").unwrap_or(0.0), "openedAt": opened_at,
        "durationLabel": (status == "Open").then(|| duration_label(now.saturating_sub(opened_at))),
        "closedAt": evaluated_at, "entryPrice": entry_price, "entryBid": entry_bid, "entryAsk": entry_ask,
        "markPrice": null, "takerFeeRate": taker_fee_rate, "status": status, "netReturnPercent": net_return,
        "openNetReturnPercent": open_net_return, "returnBasis": return_basis,
        "displayNetReturnPercent": display_return, "netReturnLabel": net_return_label,
        "netReturnEstimated": status == "Open", "netVsBreakevenLeft": bar_left,
        "netVsBreakevenWidth": bar_width, "netVsBreakevenTone": bar_tone,
        "scoutComparisonId": comparison_id
    }))
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
) -> Json<Value> {
    let now = scout_now_ms();
    let since = now.saturating_sub(7 * 86_400_000);
    let state = service.scout.data.lock().await;
    let mut rows = state.scout_instances.values()
        .filter(|audit| audit["timestamp"].as_u64().unwrap_or_default() >= since)
        .filter_map(|audit| instance_row(audit, now))
        .collect::<Vec<_>>();
    rows.sort_by(|left,right| right["openedAt"].as_u64().cmp(&left["openedAt"].as_u64()));
    rows.truncate(5_000);
    let all_summary=count_json(count_rows(&rows,false));
    let live_summary=count_json(count_rows(&rows,true));
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
    Json(json!({"rows":page_rows,"summary":{"all":all_summary,"live":live_summary},"learningRows":visible_learning_rows,"learningOutcomes":learning_outcomes,"symbols":symbols,"learningSymbol":requested_symbol,"updatedAt":now,"source":source,"status":status.unwrap_or("all"),"page":page,"pageSize":page_size,"pageCount":page_count,"totalRows":total,"pageNumber":page+1,"hasPrevious":page>0,"hasNext":page+1<page_count}))
}

#[cfg(test)]
mod scout_report_tests {
    use super::*;

    fn audit() -> Value {
        json!({"decision_id":"sample-1","symbol":"BTC-USDT","strategy":"live-trade-flow","direction":"bullish","timestamp":1000,"strength":0.8,"scout_entry_price":100.0,"scout_exit_scenario":"strategy","scout":true})
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
    fn reports_expire_unresolved_instances_after_seven_days() {
        let row = instance_row(&audit(), 8 * 86_400_000).unwrap();
        assert_eq!(row["status"], "Outcome unavailable");
    }
}
