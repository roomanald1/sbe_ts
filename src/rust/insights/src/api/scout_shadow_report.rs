use super::{Instant, SignalRuntime, now_ms};
use axum::{
    Json,
    extract::{Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap};

const MAX_SCOUT_TRADE_RECORDS: usize = 5_000;
const MAX_SCOUT_AUDIT_RECORDS: usize = 10_000;
const MAX_SCOUT_EVALUATION_RECORDS: usize = 20_000;

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
struct LearningGroup {
    row: ScoutLearningRow,
    returns: Vec<f64>,
    paired_deltas: Vec<f64>,
}

#[derive(Deserialize)]
pub(super) struct ScoutShadowQuery {
    page: Option<usize>,
    page_size: Option<usize>,
    source: Option<String>,
    status: Option<String>,
    learning_symbol: Option<String>,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct ScoutLearningRow {
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
    win_rate_percent: Option<f64>,
    win_rate_color: Option<String>,
    average: Option<f64>,
    average_tone: Option<String>,
    paired_delta_average: Option<f64>,
    paired_delta_tone: Option<String>,
}

fn string(record: &Value, key: &str) -> Option<String> {
    record.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number(record: &Value, key: &str) -> Option<f64> {
    record
        .get(key)
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn positive_number(record: &Value, key: &str) -> Option<f64> {
    number(record, key).filter(|value| *value > 0.0)
}

fn decision_sample_key(audit: &Value) -> String {
    let part = |key: &str, fallback: &str| {
        audit
            .get(key)
            .filter(|value| !value.is_null())
            .map(Value::to_string)
            .unwrap_or_else(|| {
                if key == "shadow_exit_scenario" {
                    Value::String(fallback.to_owned()).to_string()
                } else {
                    fallback.to_owned()
                }
            })
    };
    [
        part("symbol", ""),
        part("strategy", ""),
        part("direction", ""),
        part("scout_level_percent", "exploratory"),
        format!(
            "{}:{}",
            part("scout_parameter_key", ""),
            part("scout_parameter_value", "")
        ),
        part("shadow_exit_scenario", "strategy"),
        part("scout_comparison_id", ""),
    ]
    .join("|")
}

fn finite_average(values: &[f64]) -> Option<f64> {
    (!values.is_empty()).then(|| values.iter().sum::<f64>() / values.len() as f64)
}

fn learning_group<'a>(
    groups: &'a mut BTreeMap<String, LearningGroup>,
    key: String,
    row: ScoutLearningRow,
) -> &'a mut LearningGroup {
    let group = groups.entry(key).or_default();
    if group.row.kind.is_empty() {
        group.row = row;
    }
    group
}

fn accumulate_learning_result(group: &mut LearningGroup, row: &Value) {
    let status = string(row, "status").unwrap_or_default();
    if matches!(status.as_str(), "Won" | "Lost" | "Flat") {
        group.row.count += 1;
        group.row.wins += usize::from(status == "Won");
        if let Some(value) = number(row, "netReturnPercent") {
            group.returns.push(value);
        }
        if string(row, "comparisonRole").as_deref() == Some("variant") {
            if let Some(value) = number(row, "pairedDeltaPercent") {
                group.paired_deltas.push(value);
            }
        }
    }
}

fn build_learning_rows(rows: &[Value]) -> Vec<ScoutLearningRow> {
    let mut groups = BTreeMap::<String, LearningGroup>::new();
    for row in rows {
        if string(row, "source").as_deref() != Some("Scout") {
            continue;
        }
        let symbol = string(row, "symbol").unwrap_or_default();
        let strategy = string(row, "strategy").unwrap_or_default();
        let scenario = string(row, "exitScenario").unwrap_or_else(|| "strategy".into());
        let comparison = string(row, "comparisonRole");
        if comparison.is_none() {
            let key = format!("outcome|{symbol}|{strategy}|{scenario}");
            let group = learning_group(
                &mut groups,
                key,
                ScoutLearningRow {
                    kind: "Scout summary".into(),
                    symbol: symbol.clone(),
                    strategy: strategy.clone(),
                    scenario: scenario.clone(),
                    ..ScoutLearningRow::default()
                },
            );
            accumulate_learning_result(group, row);
        }
        if let (Some(parameter), Some(value)) = (
            string(row, "scoutParameterKey"),
            number(row, "scoutParameterValue"),
        ) {
            let entry_eligible = row
                .get("scoutParameterEntryEligible")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let key = format!(
                "parameter|{symbol}|{strategy}|{parameter}|{value:.8}|{entry_eligible}|{scenario}"
            );
            let group = learning_group(
                &mut groups,
                key,
                ScoutLearningRow {
                    kind: "Parameter probe".into(),
                    symbol: symbol.clone(),
                    strategy: strategy.clone(),
                    scenario: scenario.clone(),
                    parameter: Some(parameter),
                    value: Some(value),
                    entry_eligible: Some(entry_eligible),
                    ..ScoutLearningRow::default()
                },
            );
            accumulate_learning_result(group, row);
        }
        if let Some(level) = number(row, "scoutLevelPercent") {
            let key = format!("level|{symbol}|{strategy}|{scenario}|{level:.8}");
            let group = learning_group(
                &mut groups,
                key,
                ScoutLearningRow {
                    kind: "Breakout distance".into(),
                    symbol,
                    strategy,
                    scenario,
                    level: Some(level),
                    ..ScoutLearningRow::default()
                },
            );
            accumulate_learning_result(group, row);
        }
    }
    groups
        .into_values()
        .map(|mut group| {
            group.row.average = finite_average(&group.returns);
            group.row.paired_delta_average = finite_average(&group.paired_deltas);
            group.row.win_rate_percent = (group.row.count > 0)
                .then(|| group.row.wins as f64 * 100.0 / group.row.count as f64);
            group.row.win_rate_color = group.row.win_rate_percent.map(win_rate_color);
            group.row.average_tone = group.row.average.map(return_tone).map(str::to_owned);
            group.row.paired_delta_tone = group
                .row
                .paired_delta_average
                .map(return_tone)
                .map(str::to_owned);
            group.row
        })
        .collect()
}

fn win_rate_color(rate: f64) -> String {
    let progress = (rate.clamp(0.0, 100.0)) / 100.0;
    let red = [240.0, 110.0, 114.0];
    let green = [69.0, 201.0, 137.0];
    let channels = (0..3)
        .map(|index| (red[index] + (green[index] - red[index]) * progress).round() as u8)
        .collect::<Vec<_>>();
    format!("rgb({}, {}, {})", channels[0], channels[1], channels[2])
}

fn return_tone(value: f64) -> &'static str {
    if value > 1e-12 {
        "is-positive"
    } else if value < -1e-12 {
        "is-negative"
    } else {
        "is-flat"
    }
}

fn duration_label(milliseconds: u64) -> String {
    let seconds = milliseconds / 1_000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    let remainder_seconds = seconds % 60;
    if minutes < 60 {
        return if remainder_seconds > 0 {
            format!("{minutes}m {remainder_seconds}s")
        } else {
            format!("{minutes}m")
        };
    }
    let hours = minutes / 60;
    let remainder_minutes = minutes % 60;
    if remainder_minutes > 0 {
        format!("{hours}h {remainder_minutes}m")
    } else {
        format!("{hours}h")
    }
}

fn scout_counts(rows: &[Value], live_only: bool) -> ScoutCounts {
    let mut result = ScoutCounts::default();
    for row in rows {
        if live_only
            && !string(row, "strategy").is_some_and(|strategy| strategy.starts_with("live-"))
        {
            continue;
        }
        result.total += 1;
        match string(row, "status").as_deref() {
            Some("Open") => result.open += 1,
            Some("Won") => result.wins += 1,
            Some("Lost") => result.losses += 1,
            Some("Flat") => result.flats += 1,
            Some("Outcome unavailable") => result.unavailable += 1,
            _ => {}
        }
    }
    result
}

fn counts_json(counts: ScoutCounts) -> Value {
    json!({
        "total": counts.total,
        "open": counts.open,
        "resolved": counts.wins + counts.losses + counts.flats,
        "wins": counts.wins,
        "losses": counts.losses,
        "flats": counts.flats,
        "unavailable": counts.unavailable,
    })
}

async fn build_scout_shadow_data(runtime: &SignalRuntime) -> Result<Value, String> {
    let now = now_ms();
    let since = now.saturating_sub(7 * 24 * 60 * 60_000);
    let mut audit_rows = Vec::new();
    let mut cursor = None;
    loop {
        let remaining = MAX_SCOUT_AUDIT_RECORDS.saturating_sub(audit_rows.len());
        if remaining == 0 {
            break;
        }
        let page = runtime
            .store
            .load_auto_trader_audit_page(Some(since), cursor, remaining.min(10_000))
            .await
            .map_err(|error| error.to_string())?;
        audit_rows.extend(page.items);
        let Some(next) = page.next_cursor else {
            break;
        };
        cursor = Some(next);
    }
    let evaluations = runtime
        .store
        .evaluations(Some(since), MAX_SCOUT_EVALUATION_RECORDS, None)
        .await
        .map_err(|error| error.to_string())?;
    let mut evaluations_by_signal = HashMap::new();
    for evaluation in evaluations {
        evaluations_by_signal.insert(
            (evaluation.symbol.clone(), evaluation.signal_timestamp),
            evaluation,
        );
    }

    let mut outcomes_by_decision = HashMap::<String, Value>::new();
    let mut paired_returns = HashMap::<String, (Option<f64>, Option<f64>)>::new();
    let mut paired_deltas = HashMap::<String, f64>::new();
    for audit in &audit_rows {
        if string(audit, "stage").as_deref() != Some("scout-outcome") {
            continue;
        }
        if let Some(decision_id) = string(audit, "shadow_decision_id") {
            outcomes_by_decision.insert(decision_id, audit.clone());
            if let Some(id) = string(audit, "scout_comparison_id") {
                if let Some(delta) = number(audit, "paired_delta_percent") {
                    paired_deltas.insert(id.clone(), delta);
                }
                if let Some(value) = number(audit, "shadow_net_return_percent") {
                    let pair = paired_returns.entry(id).or_default();
                    match string(audit, "scout_comparison_role").as_deref() {
                        Some("baseline") => pair.0 = Some(value),
                        Some("variant") => pair.1 = Some(value),
                        _ => {}
                    }
                }
            }
        }
    }
    for (id, (baseline, variant)) in paired_returns {
        if !paired_deltas.contains_key(&id) {
            if let (Some(baseline), Some(variant)) = (baseline, variant) {
                paired_deltas.insert(id, variant - baseline);
            }
        }
    }

    let mut decisions = audit_rows
        .iter()
        .filter(|audit| {
            string(audit, "stage").as_deref() == Some("decision")
                && string(audit, "status").as_deref() == Some("shadowed")
                && audit.get("scout").and_then(Value::as_bool) == Some(true)
                && string(audit, "decision_id").is_some()
        })
        .cloned()
        .collect::<Vec<_>>();
    decisions.sort_by_key(|audit| {
        audit
            .get("timestamp")
            .and_then(Value::as_u64)
            .or_else(|| audit.get("signal_timestamp").and_then(Value::as_u64))
            .unwrap_or_default()
    });

    let mut latest_by_sample = HashMap::<String, u64>::new();
    let mut latest_decisions = BTreeMap::<String, Value>::new();
    for audit in decisions {
        let timestamp = audit
            .get("timestamp")
            .and_then(Value::as_u64)
            .or_else(|| audit.get("signal_timestamp").and_then(Value::as_u64))
            .unwrap_or_default();
        let key = decision_sample_key(&audit);
        if latest_by_sample
            .get(&key)
            .is_some_and(|prior| timestamp.saturating_sub(*prior) < 60 * 60_000)
        {
            continue;
        }
        latest_by_sample.insert(key, timestamp);
        if let Some(decision_id) = string(&audit, "decision_id") {
            latest_decisions.insert(decision_id, audit);
        }
    }

    let mut rows = Vec::new();
    for (decision_id, audit) in latest_decisions {
        let Some(signal_timestamp) = audit.get("signal_timestamp").and_then(Value::as_u64) else {
            continue;
        };
        let Some(symbol) = string(&audit, "symbol") else {
            continue;
        };
        let Some(direction) = string(&audit, "direction")
            .filter(|direction| direction == "bullish" || direction == "bearish")
        else {
            continue;
        };
        let strategy = string(&audit, "strategy").unwrap_or_else(|| "live-insights".into());
        let outcome = outcomes_by_decision.get(&decision_id);
        let comparison_id = outcome
            .and_then(|row| string(row, "scout_comparison_id"))
            .or_else(|| string(&audit, "scout_comparison_id"));
        let comparison_role = outcome
            .and_then(|row| string(row, "scout_comparison_role"))
            .or_else(|| string(&audit, "scout_comparison_role"));
        let evaluation = evaluations_by_signal.get(&(symbol.clone(), signal_timestamp));
        let net_return = outcome
            .and_then(|row| number(row, "shadow_net_return_percent"))
            .or_else(|| {
                evaluation.and_then(|row| {
                    if direction == "bullish" {
                        row.long_net_percent
                    } else {
                        row.short_net_percent
                    }
                })
            });
        let entry_bid = positive_number(&audit, "shadow_entry_bid");
        let entry_ask = positive_number(&audit, "shadow_entry_ask");
        let entry_price = if direction == "bullish" {
            entry_ask.or_else(|| positive_number(&audit, "shadow_entry_price"))
        } else {
            entry_bid.or_else(|| positive_number(&audit, "shadow_entry_price"))
        };
        let missing_live_entry = strategy.starts_with("live-") && entry_price.is_none();
        let status = if let Some(value) = net_return {
            if value > 1e-12 {
                "Won"
            } else if value < -1e-12 {
                "Lost"
            } else {
                "Flat"
            }
        } else if outcome.is_some() || evaluation.is_some() || missing_live_entry {
            "Outcome unavailable"
        } else {
            "Open"
        };
        let opened_at = audit
            .get("timestamp")
            .and_then(Value::as_u64)
            .unwrap_or(signal_timestamp);
        let evaluated_at = outcome
            .and_then(|row| row.get("evaluated_timestamp").and_then(Value::as_u64))
            .or_else(|| evaluation.map(|row| row.evaluated_timestamp));
        let taker_fee = outcome
            .and_then(|row| number(row, "shadow_taker_fee_rate"))
            .or_else(|| evaluation.map(|row| row.taker_fee_rate))
            .or_else(|| number(&audit, "shadow_taker_fee_rate"))
            .unwrap_or(0.001);
        let mark_price = if status == "Open" {
            runtime.latest_prices.get(&symbol).and_then(|price| {
                (now.saturating_sub(price.value().0) < 60_000).then_some(price.value().1)
            })
        } else {
            None
        };
        let entry_spread_percent = entry_bid
            .zip(entry_ask)
            .zip(entry_price)
            .map(|((bid, ask), entry)| (ask - bid) / entry * 100.0)
            .unwrap_or(0.0);
        let open_net_return = mark_price
            .zip(entry_price)
            .map(|(mark, entry)| {
                let gross = if direction == "bullish" {
                    (mark / entry - 1.0) * 100.0
                } else {
                    (entry / mark - 1.0) * 100.0
                };
                gross - taker_fee * 2.0 * 100.0 - entry_spread_percent
            })
            .filter(|value| value.is_finite());
        let display_return = if status == "Open" {
            open_net_return
        } else {
            net_return
        };
        let bar_position =
            display_return.map(|value| (50.0 + value / 2.0 * 40.0).clamp(10.0, 90.0));
        let bar_left = bar_position.map(|position| position.min(50.0));
        let bar_width = bar_position.map(|position| (position - 50.0).abs());
        let bar_tone = display_return.map(|value| {
            if value > 1e-12 {
                "is-positive"
            } else if value < -1e-12 {
                "is-negative"
            } else {
                "is-flat"
            }
        });
        let return_basis = if status == "Open" && open_net_return.is_some() {
            Some("ticker estimate")
        } else if net_return.is_some() {
            Some("strategy exit")
        } else {
            None
        };
        let net_return_label = display_return.map(|value| {
            format!(
                "{}{value:.3}%{}",
                if value > 0.0 { "+" } else { "" },
                if status == "Open" || return_basis == Some("ticker estimate") {
                    " est."
                } else {
                    ""
                }
            )
        });
        rows.push(json!({
            "decisionId": decision_id, "symbol": symbol, "strategy": strategy, "source": "Scout",
            "scoutLevelPercent": number(&audit,"scout_level_percent"), "scoutParameterKey": string(&audit,"scout_parameter_key"),
            "scoutParameterValue": number(&audit,"scout_parameter_value"), "scoutParameterEntryEligible": audit.get("scout_parameter_entry_eligible").and_then(Value::as_bool),
            "comparisonRole": comparison_role, "pairedDeltaPercent": comparison_id.as_ref().and_then(|id| paired_deltas.get(id).copied()),
            "pairedDeltaTone": comparison_id.as_ref().and_then(|id| paired_deltas.get(id).copied()).map(return_tone),
            "exitScenario": string(&audit,"shadow_exit_scenario").unwrap_or_else(|| "strategy".into()), "direction": direction,
            "strength": number(&audit,"strength").unwrap_or(0.0), "openedAt": opened_at,
            "durationLabel": (status == "Open").then(|| duration_label(now.saturating_sub(opened_at))),
            "closedAt": evaluated_at, "entryPrice": entry_price, "entryBid": entry_bid, "entryAsk": entry_ask,
            "markPrice": mark_price, "takerFeeRate": taker_fee, "status": status, "netReturnPercent": net_return,
            "openNetReturnPercent": open_net_return, "returnBasis": return_basis,
            "displayNetReturnPercent": display_return, "netReturnLabel": net_return_label,
            "netReturnEstimated": status == "Open" || return_basis == Some("ticker estimate"),
            "netVsBreakevenLeft": bar_left, "netVsBreakevenWidth": bar_width, "netVsBreakevenTone": bar_tone,
        }));
    }
    rows.sort_by(|left, right| right["openedAt"].as_u64().cmp(&left["openedAt"].as_u64()));
    rows.truncate(MAX_SCOUT_TRADE_RECORDS);
    let all_summary = counts_json(scout_counts(&rows, false));
    let live_summary = counts_json(scout_counts(&rows, true));
    let learning_rows = build_learning_rows(&rows);
    let symbols = learning_rows
        .iter()
        .map(|row| row.symbol.clone())
        .collect::<BTreeSet<_>>();
    let learning_outcomes = audit_rows
        .iter()
        .filter_map(|audit| {
            (string(audit, "stage").as_deref() == Some("scout-outcome")
                && string(audit, "strategy").is_some()
                && string(audit, "scout_comparison_id").is_none())
            .then(|| {
                number(audit, "shadow_net_return_percent").map(|net_return| {
                    json!({
                        "strategy": string(audit,"strategy").unwrap_or_default(),
                        "netReturnPercent": net_return,
                        "parameterKey": string(audit,"scout_parameter_key"),
                        "parameterValue": number(audit,"scout_parameter_value"),
                    })
                })
            })
            .flatten()
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "rows": rows,
        "summary": { "all": all_summary, "live": live_summary },
        "learningRows": learning_rows,
        "learningOutcomes": learning_outcomes,
        "symbols": symbols,
        "updatedAt": now,
    }))
}

#[cfg(test)]
mod scout_shadow_report_tests {
    use super::*;

    #[test]
    fn number_helpers_reject_nonfinite_values_and_nonpositive_prices() {
        assert_eq!(string(&json!({"v":"x"}), "v").as_deref(), Some("x"));
        assert_eq!(number(&json!({"v":1.5}), "v"), Some(1.5));
        assert_eq!(number(&json!({"v":f64::INFINITY}), "v"), None);
        assert_eq!(positive_number(&json!({"v":0.0}), "v"), None);
    }

    #[test]
    fn decision_key_includes_comparison_and_defaults_missing_fields() {
        let key = decision_sample_key(
            &json!({"symbol":"BTC","strategy":"trend","scout_comparison_id":"pair"}),
        );
        assert!(key.starts_with("\"BTC\"|\"trend\"|"));
        assert!(key.ends_with("|\"strategy\"|\"pair\""));
    }

    #[test]
    fn learning_rows_group_outcomes_parameter_variants_and_pair_deltas() {
        let rows = vec![
            json!({"source":"Scout","symbol":"BTC","strategy":"trend","status":"Won","netReturnPercent":2.0,"exitScenario":"strategy","scoutParameterKey":"threshold","scoutParameterValue":0.4,"scoutParameterEntryEligible":true,"comparisonRole":"variant","pairedDeltaPercent":1.0}),
            json!({"source":"Scout","symbol":"BTC","strategy":"trend","status":"Lost","netReturnPercent":-1.0,"exitScenario":"strategy","scoutParameterKey":"threshold","scoutParameterValue":0.4,"scoutParameterEntryEligible":true,"comparisonRole":"variant","pairedDeltaPercent":-0.2}),
            json!({"source":"AutoTrader","symbol":"BTC","strategy":"trend","status":"Won"}),
        ];
        let output = build_learning_rows(&rows);
        assert_eq!(output.len(), 1);
        let row = &output[0];
        assert_eq!(row.kind, "Parameter probe");
        assert_eq!(row.count, 2);
        assert_eq!(row.wins, 1);
        assert_eq!(row.average, Some(0.5));
        assert_eq!(row.paired_delta_average, Some(0.4));
    }

    #[test]
    fn duration_tones_colors_and_counts_cover_boundaries() {
        assert_eq!(duration_label(59_999), "59s");
        assert_eq!(duration_label(60_000), "1m");
        assert_eq!(duration_label(3_661_000), "1h 1m");
        assert_eq!(return_tone(0.1), "is-positive");
        assert_eq!(return_tone(-0.1), "is-negative");
        assert_eq!(return_tone(0.0), "is-flat");
        assert_eq!(win_rate_color(0.0), "rgb(240, 110, 114)");
        assert_eq!(win_rate_color(100.0), "rgb(69, 201, 137)");
        let counts = scout_counts(
            &[
                json!({"strategy":"live-trend","status":"Open"}),
                json!({"strategy":"trend","status":"Won"}),
            ],
            true,
        );
        assert_eq!(counts.total, 1);
        assert_eq!(counts.open, 1);
        assert_eq!(counts_json(counts)["resolved"], 0);
    }
}

pub(super) async fn get_scout_shadow_outcomes(
    State(runtime): State<std::sync::Arc<SignalRuntime>>,
    Query(query): Query<ScoutShadowQuery>,
) -> Result<Json<Value>, (StatusCode, String)> {
    let core = {
        let mut cache = runtime.scout_shadow_cache.lock().await;
        if let Some((created, data)) = cache.as_ref() {
            if created.elapsed() < std::time::Duration::from_secs(30) {
                data.clone()
            } else {
                let data = build_scout_shadow_data(&runtime)
                    .await
                    .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
                *cache = Some((Instant::now(), data.clone()));
                data
            }
        } else {
            let data = build_scout_shadow_data(&runtime)
                .await
                .map_err(|error| (StatusCode::INTERNAL_SERVER_ERROR, error))?;
            *cache = Some((Instant::now(), data.clone()));
            data
        }
    };
    let source = query
        .source
        .as_deref()
        .filter(|value| *value == "all")
        .unwrap_or("live");
    let status = query
        .status
        .as_deref()
        .filter(|value| matches!(*value, "open" | "closed"))
        .unwrap_or("all");
    let matching_rows = core["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|row| {
            let source_matches = source == "all"
                || row["strategy"]
                    .as_str()
                    .is_some_and(|strategy| strategy.starts_with("live-"));
            let is_open = row["status"].as_str() == Some("Open");
            let status_matches = status == "all"
                || (status == "open" && is_open)
                || (status == "closed" && !is_open);
            source_matches && status_matches
        })
        .collect::<Vec<_>>();
    let page_size = query.page_size.unwrap_or(50).clamp(1, 100);
    let total_rows = matching_rows.len();
    let page_count = total_rows.div_ceil(page_size);
    let page = query.page.unwrap_or(0).min(page_count.saturating_sub(1));
    let page_rows = matching_rows
        .into_iter()
        .skip(page.saturating_mul(page_size))
        .take(page_size)
        .collect::<Vec<_>>();
    let symbols = core["symbols"].as_array().cloned().unwrap_or_default();
    let learning_symbol = query
        .learning_symbol
        .as_deref()
        .filter(|value| symbols.iter().any(|symbol| symbol.as_str() == Some(value)));
    let visible_learning_rows = core["learningRows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|row| learning_symbol.is_none_or(|symbol| row["symbol"].as_str() == Some(symbol)))
        .cloned()
        .collect::<Vec<_>>();

    Ok(Json(json!({
        "rows": page_rows,
        "summary": core["summary"],
        "learningRows": visible_learning_rows,
        "learningOutcomes": core["learningOutcomes"],
        "symbols": symbols,
        "learningSymbol": learning_symbol,
        "updatedAt": core["updatedAt"],
        "source": source,
        "status": status,
        "page": page,
        "pageSize": page_size,
        "pageCount": page_count,
        "totalRows": total_rows,
        "pageNumber": page + 1,
        "hasPrevious": page > 0,
        "hasNext": page + 1 < page_count,
    })))
}
