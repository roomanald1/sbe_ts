const TRADE_PRICE_PATH_SAMPLE_INTERVAL_MS: u64 = 1_000;
const TRADE_PRICE_PATH_MAX_GAP_MS: u64 = 1_000;

fn trade_price_path_sample_due(last_timestamp_ms: Option<u64>, observed_at_ms: u64) -> bool {
    last_timestamp_ms.is_none_or(|last| {
        observed_at_ms.saturating_sub(last) >= TRADE_PRICE_PATH_SAMPLE_INTERVAL_MS
    })
}

fn gross_executable_return_percent(
    signed_quantity: f64,
    entry_price: f64,
    bid: f64,
    ask: f64,
) -> Option<f64> {
    if !signed_quantity.is_finite()
        || signed_quantity.abs() <= f64::EPSILON
        || !entry_price.is_finite()
        || entry_price <= 0.0
        || !bid.is_finite()
        || bid <= 0.0
        || !ask.is_finite()
        || ask < bid
    {
        return None;
    }
    let value = if signed_quantity > 0.0 {
        (bid / entry_price - 1.0) * 100.0
    } else {
        (entry_price / ask - 1.0) * 100.0
    };
    value.is_finite().then_some(value)
}

fn record_event_driven_gross_excursion(
    position: &mut TraderPosition,
    return_percent: Option<f64>,
    quote_identity_complete: bool,
    freshness_complete: bool,
    cadence_complete: bool,
    first_sample: bool,
) {
    if let Some(value) = return_percent.filter(|value| value.is_finite()) {
        position.price_path_gross_mfe_percent = Some(
            position
                .price_path_gross_mfe_percent
                .map_or(value, |prior| prior.max(value)),
        );
        position.price_path_gross_mae_percent = Some(
            position
                .price_path_gross_mae_percent
                .map_or(value, |prior| prior.min(value)),
        );
        position.price_path_event_sample_count =
            position.price_path_event_sample_count.saturating_add(1);
    }
    if !quote_identity_complete || !freshness_complete || !cadence_complete {
        position.price_path_event_metrics_complete = false;
    } else if first_sample {
        position.price_path_event_metrics_complete = true;
    }
}

fn price_path_threshold_crossings_for_event(
    position: &TraderPosition,
    observed_at_ms: u64,
    bid: f64,
    ask: f64,
    source_fresh: Option<bool>,
    quote_source_system: Option<&str>,
    quote_source_epoch: Option<&str>,
    quote_source_cursor: Option<u64>,
    quote_source_record_id: Option<&str>,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    immediate_net_return_percent: f64,
    profit_net_return_percent: f64,
    immediate_exit_fee_rate: f64,
    profit_exit_fee_rate: f64,
    stop_loss_source: &str,
    take_profit_source: &str,
) -> Vec<serde_json::Value> {
    let Some(quote_source_system) = quote_source_system.filter(|value| !value.trim().is_empty())
    else {
        return Vec::new();
    };
    let Some(quote_source_epoch) = quote_source_epoch.filter(|value| !value.trim().is_empty())
    else {
        return Vec::new();
    };
    let Some(quote_source_cursor) = quote_source_cursor.filter(|cursor| *cursor > 0) else {
        return Vec::new();
    };
    let Some(quote_source_record_id) =
        quote_source_record_id.filter(|value| !value.trim().is_empty())
    else {
        return Vec::new();
    };
    let Some(path_epoch) = position.price_path_epoch.as_deref() else {
        return Vec::new();
    };
    if source_fresh != Some(true) || !bid.is_finite() || bid <= 0.0 || !ask.is_finite() || ask < bid
    {
        return Vec::new();
    }
    let close_mark = if position.quantity > 0.0 { bid } else { ask };
    let mut crossings = Vec::with_capacity(2);
    if stop_loss_percent.is_finite()
        && stop_loss_percent > 0.0
        && immediate_net_return_percent <= -stop_loss_percent
    {
        crossings.push(serde_json::json!({
            "schema_version": 1,
            "threshold_type": "stop_loss",
            "threshold_source": stop_loss_source,
            "threshold_percent": -stop_loss_percent,
            "observed_net_return_percent": immediate_net_return_percent,
            "threshold_comparator": "less_than_or_equal",
            "timestamp_ms": observed_at_ms,
            "position_path_epoch": path_epoch,
            "quote_side": if position.quantity > 0.0 { "bid" } else { "ask" },
            "close_mark": close_mark,
            "quote_source_system": quote_source_system,
            "quote_source_epoch": quote_source_epoch,
            "quote_source_cursor": quote_source_cursor,
            "quote_source_record_id": quote_source_record_id,
            "fee_rate": immediate_exit_fee_rate,
            "fee_rate_attribution": "local_estimate_unverified_against_effective_paper_exchange_schedule",
            "crossing_evidence_status": "observed_but_fee_schedule_unverified"
        }));
    }
    if take_profit_percent.is_finite()
        && take_profit_percent > 0.0
        && profit_net_return_percent >= take_profit_percent
    {
        crossings.push(serde_json::json!({
            "schema_version": 1,
            "threshold_type": "take_profit",
            "threshold_source": take_profit_source,
            "threshold_percent": take_profit_percent,
            "observed_net_return_percent": profit_net_return_percent,
            "threshold_comparator": "greater_than_or_equal",
            "timestamp_ms": observed_at_ms,
            "position_path_epoch": path_epoch,
            "quote_side": if position.quantity > 0.0 { "bid" } else { "ask" },
            "close_mark": close_mark,
            "quote_source_system": quote_source_system,
            "quote_source_epoch": quote_source_epoch,
            "quote_source_cursor": quote_source_cursor,
            "quote_source_record_id": quote_source_record_id,
            "fee_rate": profit_exit_fee_rate,
            "fee_rate_attribution": "local_estimate_unverified_against_effective_paper_exchange_schedule",
            "crossing_evidence_status": "observed_but_fee_schedule_unverified"
        }));
    }
    crossings
}

fn counterfactual_exit_fee_attribution(
    symbol: &str,
    exit_timestamp_ms: u64,
    entry_fill_links: &serde_json::Value,
    exit_fill_evidence: &serde_json::Value,
    quotes: &[serde_json::Value],
) -> Option<serde_json::Value> {
    let quote_currency = symbol.rsplit_once('-')?.1;
    if quotes.is_empty() || exit_fill_evidence["fees_complete"].as_bool() != Some(true) {
        return None;
    }
    let entry_fills = entry_fill_links.as_array()?;
    let exit_fills = exit_fill_evidence["fills"].as_array()?;
    if entry_fills.is_empty() || exit_fills.is_empty() {
        return None;
    }
    let fill_schedule_matches = |fill: &serde_json::Value| {
        let event = &fill["paper_exchange_fee_schedule_evidence"];
        fill["paper_exchange_fee_currency"].as_str() == Some(quote_currency)
            && event["fee_schedule_id"].as_str() == fill["paper_exchange_fee_schedule_id"].as_str()
            && event["fee_schedule_version"].as_str()
                == fill["paper_exchange_fee_schedule_version"].as_str()
            && fill["paper_exchange_fee_amount"]
                .as_f64()
                .is_some_and(|fee| fee.is_finite() && fee >= 0.0)
            && fill["paper_exchange_fee_order_class"]
                .as_str()
                .is_some_and(|class| matches!(class, "maker" | "taker"))
    };
    if entry_fills
        .iter()
        .chain(exit_fills)
        .any(|fill| !fill_schedule_matches(fill))
    {
        return None;
    }
    let mut schedules = std::collections::BTreeMap::<(String, String), serde_json::Value>::new();
    for event in entry_fill_links
        .as_array()?
        .iter()
        .filter_map(|fill| fill.get("paper_exchange_fee_schedule_evidence"))
        .filter(|event| !event.is_null())
        .chain(
            exit_fill_evidence["fills"]
                .as_array()?
                .iter()
                .filter_map(|fill| {
                    fill["paper_exchange_fee_schedule_evidence"]
                        .as_object()
                        .map(|_| &fill["paper_exchange_fee_schedule_evidence"])
                }),
        )
    {
        if event["schema_version"].as_str() != Some("paper_exchange_financial_evidence:v1")
            || event["event_type"].as_str() != Some("fee_schedule")
            || event["source_system"].as_str() != Some("paper_exchange_simulator")
        {
            return None;
        }
        let schedule_id = event["fee_schedule_id"].as_str()?.to_owned();
        let schedule_version = event["fee_schedule_version"].as_str()?.to_owned();
        let maker_rate = event["maker_rate"].as_f64()?;
        let taker_rate = event["taker_rate"].as_f64()?;
        let effective_from = event["effective_from_utc_ms"].as_u64()?;
        let source_timestamp = event["source_timestamp_utc_ms"].as_u64()?;
        if event["symbol"].as_str() != Some(symbol)
            || event["source_epoch"].as_str().is_none_or(str::is_empty)
            || event["source_record_id"].as_str().is_none_or(str::is_empty)
            || event["source_cursor"]
                .as_u64()
                .is_none_or(|cursor| cursor == 0)
            || event["source_timestamp_utc_ms"]
                .as_u64()
                .is_none_or(|ts| ts == 0)
            || !maker_rate.is_finite()
            || !taker_rate.is_finite()
            || maker_rate < 0.0
            || taker_rate < 0.0
            || effective_from == 0
            || source_timestamp == 0
            || event["effective_to_utc_ms"]
                .as_u64()
                .is_some_and(|end| end <= effective_from)
        {
            return None;
        }
        let normalized = serde_json::json!({
            "source_schema_version": event["schema_version"],
            "source_event_id": event["event_id"],
            "source_system": event["source_system"],
            "source_epoch": event["source_epoch"],
            "source_cursor": event["source_cursor"],
            "source_record_id": event["source_record_id"],
            "source_timestamp_utc_ms": event["source_timestamp_utc_ms"],
            "fee_schedule_id": schedule_id,
            "fee_schedule_version": schedule_version,
            "symbol": event["symbol"],
            "effective_from_utc_ms": event["effective_from_utc_ms"],
            "effective_to_utc_ms": event["effective_to_utc_ms"],
            "maker_rate": event["maker_rate"],
            "taker_rate": event["taker_rate"],
        });
        let key = (schedule_id, schedule_version);
        if schedules
            .get(&key)
            .is_some_and(|prior| prior != &normalized)
        {
            return None;
        }
        schedules.insert(key, normalized);
    }
    if schedules.is_empty() {
        return None;
    }
    let covers = |schedule: &&serde_json::Value, timestamp: u64| {
        schedule["effective_from_utc_ms"]
            .as_u64()
            .is_some_and(|from| from <= timestamp)
            && schedule["effective_to_utc_ms"]
                .as_u64()
                .is_none_or(|to| timestamp < to)
    };
    let mut quote_observations = Vec::with_capacity(quotes.len());
    for quote in quotes {
        let timestamp = quote["timestamp"].as_u64()?;
        let covering = schedules
            .values()
            .filter(|schedule| covers(schedule, timestamp))
            .collect::<Vec<_>>();
        if covering.len() != 1
            || quote["source_fresh"] != serde_json::Value::Bool(true)
            || quote["freshness_contract_version"].as_u64() != Some(1)
        {
            return None;
        }
        let schedule = covering[0];
        quote_observations.push(serde_json::json!([
            timestamp,
            schedule["fee_schedule_id"],
            schedule["fee_schedule_version"],
            "taker",
            schedule["taker_rate"],
            quote_currency,
        ]));
    }
    let exit_schedules = schedules
        .values()
        .filter(|schedule| covers(schedule, exit_timestamp_ms))
        .collect::<Vec<_>>();
    if exit_schedules.len() != 1 {
        return None;
    }
    let schedule = exit_schedules[0];
    if exit_fills.iter().any(|fill| {
        fill["paper_exchange_fee_schedule_id"].as_str() != schedule["fee_schedule_id"].as_str()
            || fill["paper_exchange_fee_schedule_version"].as_str()
                != schedule["fee_schedule_version"].as_str()
    }) {
        return None;
    }
    Some(serde_json::json!({
        "schema_version": 1,
        "fee_schedules": schedules.values().collect::<Vec<_>>(),
        "exit_fee_schedule_ref": {
            "timestamp_ms": exit_timestamp_ms,
            "fee_schedule_id": schedule["fee_schedule_id"],
            "fee_schedule_version": schedule["fee_schedule_version"],
            "exit_order_class": "taker",
            "applied_rate": schedule["taker_rate"],
            "fee_currency": quote_currency,
            "fee_currency_conversion": serde_json::Value::Null,
        },
        "quote_observations": quote_observations,
    }))
}

fn entry_fill_identity_complete(position: &TraderPosition) -> bool {
    let unique_ids = position
        .entry_fill_ids
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let linked_fee_currencies = position
        .entry_fill_links
        .iter()
        .filter_map(|fill| fill.fee_currency.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let recorded_fee_currencies = position
        .entry_fee_currencies
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    !position.entry_fill_ids.is_empty()
        && unique_ids.len() == position.entry_fill_ids.len()
        && position.entry_fill_costs_complete
        && !position.entry_fee_currencies.is_empty()
        && linked_fee_currencies == recorded_fee_currencies
        && position.entry_fill_links.len() == position.entry_fill_ids.len()
        && position
            .entry_fill_ids
            .iter()
            .all(|fill_id| stable_pm_exit_fill_id(fill_id))
        && position.entry_fill_links.iter().all(|fill| {
            unique_ids.contains(&fill.fill_id)
                && !fill.client_order_id.trim().is_empty()
                && !fill.symbol.trim().is_empty()
                && matches!(fill.side.as_str(), "buy" | "sell")
                && fill.quantity.is_finite()
                && fill.quantity > 0.0
                && fill.execution_price.is_finite()
                && fill.execution_price > 0.0
                && fill.fee_known
                && fill.fee_amount.is_some_and(f64::is_finite)
                && fill
                    .fee_currency
                    .as_deref()
                    .is_some_and(|currency| !currency.trim().is_empty())
        })
}

fn price_path_excursion_evidence(
    position: &TraderPosition,
    exit_observed_at_ms: u64,
) -> serde_json::Value {
    let mut evidence = price_path_excursion_evidence_for_samples(
        &position.price_path,
        position.price_path_total_samples,
        position.price_path_truncated,
        position.price_path_restart_interrupted,
        position.average_entry_price,
        position.quantity,
        position.entry_timestamp_ms,
        exit_observed_at_ms,
    );
    evidence["threshold_crossings"] =
        serde_json::Value::Array(position.price_path_threshold_crossings.clone());
    evidence["threshold_crossing_evidence_status"] =
        serde_json::json!(if position.price_path_threshold_crossings.is_empty() {
            "no_source_backed_threshold_crossing_recorded"
        } else {
            "observed_with_unverified_fee_schedule"
        });
    evidence["event_driven_gross_mfe_percent"] =
        serde_json::json!(position.price_path_gross_mfe_percent);
    evidence["event_driven_gross_mae_percent"] =
        serde_json::json!(position.price_path_gross_mae_percent);
    evidence["event_driven_sample_count"] =
        serde_json::json!(position.price_path_event_sample_count);
    evidence["event_driven_metrics_complete"] =
        serde_json::json!(position.price_path_event_metrics_complete);
    evidence["event_driven_metrics_attribution"] =
        serde_json::json!(if position.price_path_event_metrics_complete {
            "fresh_source_identity_within_one_second_cadence"
        } else {
            "incomplete_or_unverified_source_cadence"
        });
    let identity = position.lifecycle_identity.as_ref();
    let identity_complete = identity.is_some_and(|identity| {
        identity.schema_version == 1
            && !identity.trade_id.trim().is_empty()
            && !identity.episode_id.trim().is_empty()
            && !identity.opportunity_id.trim().is_empty()
            && position.trade_id.as_deref() == Some(identity.trade_id.as_str())
            && entry_fill_identity_complete(position)
    });
    let mut missing_coverage = evidence["missing_coverage"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let active_epoch_matches = position.price_path_epoch.as_deref().is_some_and(|active| {
        position
            .price_path
            .last()
            .and_then(|sample| sample.epoch.as_deref())
            == Some(active)
    });
    if !active_epoch_matches {
        missing_coverage.push(serde_json::json!("active_path_epoch_mismatch_or_missing"));
    }
    let continuity_complete = evidence["complete"].as_bool() == Some(true) && active_epoch_matches;
    if !identity_complete {
        missing_coverage.push(serde_json::json!(
            "position_trade_identity_or_entry_fills_missing"
        ));
    }
    evidence["continuity_complete"] = serde_json::json!(continuity_complete);
    evidence["complete"] = serde_json::json!(continuity_complete && identity_complete);
    evidence["missing_coverage"] = serde_json::Value::Array(missing_coverage);
    let initial_risk_complete = position
        .initial_risk_reference
        .as_ref()
        .is_some_and(|risk| {
            risk.schema_version == 1
                && risk.status == "complete"
                && risk
                    .quote_amount
                    .is_some_and(|amount| amount.is_finite() && amount > 0.0)
                && risk
                    .quote_currency
                    .as_deref()
                    .is_some_and(|currency| !currency.is_empty())
                && !risk.fill_ids.is_empty()
        });
    evidence["identity_complete"] = serde_json::json!(identity_complete);
    evidence["trade_id"] = serde_json::json!(position.trade_id);
    evidence["source_episode_id"] =
        serde_json::json!(identity.map(|identity| &identity.episode_id));
    evidence["opportunity_id"] =
        serde_json::json!(identity.map(|identity| &identity.opportunity_id));
    evidence["initial_risk_complete"] = serde_json::json!(initial_risk_complete);
    let mut policy_evaluation_blockers = Vec::new();
    if !continuity_complete {
        policy_evaluation_blockers.push("executable_price_path_incomplete");
    }
    if !identity_complete {
        policy_evaluation_blockers.push("position_trade_identity_or_entry_fills_missing");
    }
    if !initial_risk_complete {
        policy_evaluation_blockers.push("authoritative_initial_risk_missing_or_incomplete");
    }
    evidence["policy_evaluation_eligible"] =
        serde_json::json!(evidence["complete"].as_bool() == Some(true) && initial_risk_complete);
    evidence["policy_evaluation_blockers"] = serde_json::json!(policy_evaluation_blockers);
    evidence
}

fn price_path_excursion_evidence_for_samples(
    price_path: &[TradePriceSample],
    total_sample_count: u64,
    path_truncated: bool,
    restart_interrupted: bool,
    entry_price: f64,
    signed_quantity: f64,
    entry_timestamp_ms: u64,
    exit_observed_at_ms: u64,
) -> serde_json::Value {
    let is_long = signed_quantity > 0.0;
    let mut excursions = Vec::with_capacity(price_path.len());
    let mut missing = Vec::new();
    if entry_timestamp_ms == 0 {
        missing.push("entry_timestamp_unknown");
    }
    if price_path.is_empty() {
        missing.push("price_path_empty");
    }
    if path_truncated {
        missing.push("price_path_retention_truncated");
    }
    if restart_interrupted {
        missing.push("price_path_restart_interrupted");
    }
    if total_sample_count > price_path.len() as u64 {
        missing.push("price_path_has_unretained_samples");
    }
    let mut maximum_gap_ms = 0_u64;
    let mut freshness_unknown_samples = 0_usize;
    let mut stale_samples = 0_usize;
    let mut invalid_quote_samples = 0_usize;
    let mut non_monotonic = false;
    let mut sequence_discontinuities = 0_usize;
    let mut unknown_sequence_samples = 0_usize;
    let mut missing_epoch_samples = 0_usize;
    let mut path_epochs = std::collections::BTreeSet::new();

    for (index, sample) in price_path.iter().enumerate() {
        if index > 0 {
            let previous = &price_path[index - 1];
            if sample.timestamp <= previous.timestamp {
                non_monotonic = true;
            } else {
                maximum_gap_ms = maximum_gap_ms.max(sample.timestamp - previous.timestamp);
            }
            if previous.sequence == 0 || sample.sequence == 0 {
                unknown_sequence_samples += 1;
            } else if sample.sequence != previous.sequence.saturating_add(1) {
                sequence_discontinuities += 1;
            }
        }
        if sample.sequence == 0 {
            unknown_sequence_samples += usize::from(index == 0);
        }
        if let Some(epoch) = sample.epoch.as_deref().filter(|epoch| !epoch.is_empty()) {
            path_epochs.insert(epoch.to_owned());
        } else {
            missing_epoch_samples += 1;
        }
        if sample.source_fresh != Some(true)
            || sample.freshness_contract_version != Some(1)
            || sample.freshness_max_age_ms.is_none()
            || sample.source_updated_at_ms.is_none()
        {
            if sample.source_fresh == Some(false) {
                stale_samples += 1;
            } else {
                freshness_unknown_samples += 1;
            }
            continue;
        }
        let executable_price = if is_long { sample.bid } else { sample.ask };
        if !executable_price.is_finite() || executable_price <= 0.0 || entry_price <= 0.0 {
            invalid_quote_samples += 1;
            continue;
        }
        let signed_return_percent = if is_long {
            (executable_price / entry_price - 1.0) * 100.0
        } else {
            (entry_price / executable_price - 1.0) * 100.0
        };
        if signed_return_percent.is_finite() {
            excursions.push(signed_return_percent);
        } else {
            invalid_quote_samples += 1;
        }
    }

    if non_monotonic {
        missing.push("price_path_timestamps_not_monotonic");
    }
    if maximum_gap_ms > TRADE_PRICE_PATH_MAX_GAP_MS {
        missing.push("price_path_gap_exceeds_bound");
    }
    if stale_samples > 0 {
        missing.push("stale_source_marks");
    }
    if freshness_unknown_samples > 0 {
        missing.push("source_freshness_unknown");
    }
    if invalid_quote_samples > 0 {
        missing.push("invalid_executable_quotes");
    }
    if unknown_sequence_samples > 0 {
        missing.push("path_sequence_unknown");
    }
    if sequence_discontinuities > 0 {
        missing.push("path_sequence_discontinuity");
    }
    if missing_epoch_samples > 0 {
        missing.push("path_epoch_missing");
    }
    if path_epochs.len() > 1 {
        missing.push("path_spans_multiple_epochs");
    }

    let first_timestamp = price_path.first().map(|sample| sample.timestamp);
    let last_timestamp = price_path.last().map(|sample| sample.timestamp);
    if first_timestamp.is_some_and(|first| {
        first < entry_timestamp_ms
            || first.saturating_sub(entry_timestamp_ms) > TRADE_PRICE_PATH_MAX_GAP_MS
    }) {
        missing.push("entry_to_first_sample_gap_exceeds_bound");
    }
    if last_timestamp.is_some_and(|last| {
        last > exit_observed_at_ms
            || exit_observed_at_ms.saturating_sub(last) > TRADE_PRICE_PATH_MAX_GAP_MS
    }) {
        missing.push("last_sample_to_exit_gap_exceeds_bound");
    }

    let mfe = excursions.iter().copied().reduce(f64::max);
    let mae = excursions.iter().copied().reduce(f64::min);
    serde_json::json!({
        "schema_version": 1,
        "sampling_interval_ms": TRADE_PRICE_PATH_SAMPLE_INTERVAL_MS,
        "maximum_allowed_gap_ms": TRADE_PRICE_PATH_MAX_GAP_MS,
        "retained_sample_count": price_path.len(),
        "total_sample_count": total_sample_count.max(price_path.len() as u64),
        "source_fresh_sample_count": excursions.len(),
        "source_stale_sample_count": stale_samples,
        "source_freshness_unknown_sample_count": freshness_unknown_samples,
        "invalid_quote_sample_count": invalid_quote_samples,
        "maximum_observed_gap_ms": maximum_gap_ms,
        "sequence_discontinuity_count": sequence_discontinuities,
        "unknown_sequence_sample_count": unknown_sequence_samples,
        "missing_epoch_sample_count": missing_epoch_samples,
        "path_epochs": path_epochs,
        "first_sample_at_ms": first_timestamp,
        "last_sample_at_ms": last_timestamp,
        "gross_executable_mfe_percent": mfe,
        "gross_executable_mae_percent": mae,
        "fee_adjusted_mfe_mae_status": "not_computed_without_path_level_execution_costs",
        "complete": missing.is_empty(),
        "missing_coverage": missing,
    })
}

fn stable_pm_exit_fill_id(value: &str) -> bool {
    value
        .strip_prefix("pm-fill:v1:")
        .and_then(|identity| identity.split_once(':'))
        .is_some_and(|(epoch, sequence)| {
            !epoch.trim().is_empty() && sequence.parse::<u64>().is_ok()
        })
}

fn realized_r_evidence(
    position: &TraderPosition,
    symbol: &str,
    trade_closed: bool,
    cumulative_realized_pnl: f64,
    fee_currency: Option<&str>,
    fee_coverage_complete: bool,
) -> serde_json::Value {
    let expected_quote = symbol
        .split_once('-')
        .map(|(_, quote)| quote)
        .filter(|quote| !quote.trim().is_empty());
    let reference = position.initial_risk_reference.as_ref();
    let quote_amount = reference
        .and_then(|reference| reference.quote_amount)
        .filter(|amount| amount.is_finite() && *amount > 0.0);
    let quote_currency = reference.and_then(|reference| reference.quote_currency.as_deref());
    let identity_complete = position
        .lifecycle_identity
        .as_ref()
        .is_some_and(|identity| {
            identity.schema_version == 1
                && !identity.episode_id.trim().is_empty()
                && !identity.opportunity_id.trim().is_empty()
                && position.trade_id.as_deref() == Some(identity.trade_id.as_str())
        });
    let eligible = trade_closed
        && cumulative_realized_pnl.is_finite()
        && fee_coverage_complete
        && fee_currency == expected_quote
        && identity_complete
        && reference.is_some_and(|reference| {
            reference.schema_version == 1
                && reference.status == "complete"
                && quote_amount.is_some()
                && expected_quote.is_some()
                && quote_currency == expected_quote
                && !reference.fill_ids.is_empty()
                && reference
                    .fill_ids
                    .iter()
                    .all(|fill_id| position.entry_fill_ids.contains(fill_id))
                && reference.fill_ids.len() == position.entry_fill_ids.len()
                && entry_fill_identity_complete(position)
        });
    let mut missing_coverage = Vec::new();
    if !trade_closed {
        missing_coverage.push("trade_not_closed");
    } else {
        if !identity_complete {
            missing_coverage.push("canonical_trade_episode_identity_missing_or_mismatched");
        }
        if !fee_coverage_complete || fee_currency != expected_quote {
            missing_coverage.push("known_quote_currency_entry_exit_fees_incomplete");
        }
        if !reference.is_some_and(|reference| {
            reference.schema_version == 1
                && reference.status == "complete"
                && quote_amount.is_some()
                && quote_currency == expected_quote
                && !reference.fill_ids.is_empty()
                && reference.fill_ids.len() == position.entry_fill_ids.len()
                && reference
                    .fill_ids
                    .iter()
                    .all(|fill_id| position.entry_fill_ids.contains(fill_id))
                && entry_fill_identity_complete(position)
        }) {
            missing_coverage
                .push("authoritative_initial_risk_or_entry_fills_missing_or_mismatched");
        }
    }
    serde_json::json!({
        "schema_version": 1,
        "status": if eligible { "complete" } else if trade_closed { "unavailable" } else { "not_closed" },
        "net_realized_r_multiple": if eligible { Some(cumulative_realized_pnl / quote_amount.unwrap()) } else { None },
        "cumulative_realized_pnl_quote": trade_closed.then_some(cumulative_realized_pnl),
        "initial_risk_quote_amount": quote_amount,
        "quote_currency": quote_currency,
        "expected_quote_currency": expected_quote,
        "fee_coverage_complete": fee_coverage_complete,
        "fee_currency": fee_currency,
        "identity_complete": identity_complete,
        "initial_risk_fill_ids": reference.map(|reference| &reference.fill_ids),
        "missing_coverage": missing_coverage,
    })
}

fn exit_fill_lineage_evidence(
    response: &serde_json::Value,
    expected_client_order_id: Option<&str>,
    expected_symbol: &str,
    expected_side: &str,
) -> serde_json::Value {
    let reported_filled = response["filled_quantity"].as_f64().unwrap_or(f64::NAN);
    let fills = response["fill_evidence"].as_array();
    if reported_filled.is_finite()
        && reported_filled <= 1e-12
        && fills.is_none_or(|rows| rows.is_empty())
    {
        return serde_json::json!({
            "schema_version": 1,
            "expected_client_order_id": expected_client_order_id,
            "expected_symbol": expected_symbol,
            "expected_side": expected_side,
            "coverage_status": "not_applicable",
            "fee_coverage_status": "not_applicable",
            "fill_lineage_complete": true,
            "fees_complete": true,
            "complete": true,
            "exit_fill_ids": [],
            "fills": [],
            "missing_coverage": [],
        });
    }

    let mut missing = Vec::new();
    let expected_client_order_id =
        expected_client_order_id.filter(|value| !value.trim().is_empty());
    if expected_client_order_id.is_none() {
        missing.push("exit_client_order_id_missing");
    }
    let mut output_fills = Vec::new();
    let mut exit_fill_ids = Vec::new();
    let mut seen_fill_ids = std::collections::BTreeSet::new();
    let mut filled_quantity = 0.0;
    let mut known_fee_total = 0.0;
    let mut all_order_links_valid = expected_client_order_id.is_some();
    let mut all_fill_ids_valid = true;
    let mut all_fees_complete = response["fee_complete"].as_bool() == Some(true);
    if !all_fees_complete {
        missing.push("response_fee_completeness_false_or_missing");
    }

    let Some(fills) = fills else {
        missing.push("exit_fill_evidence_missing");
        return serde_json::json!({
            "schema_version": 1,
            "expected_client_order_id": expected_client_order_id,
            "expected_symbol": expected_symbol,
            "expected_side": expected_side,
            "coverage_status": "missing",
            "fee_coverage_status": "missing",
            "fill_lineage_complete": false,
            "fees_complete": false,
            "complete": false,
            "exit_fill_ids": [],
            "fills": [],
            "missing_coverage": missing,
        });
    };
    if fills.is_empty() {
        missing.push("exit_fill_evidence_empty_for_filled_order");
    }

    for fill in fills {
        let fill_id = fill["fill_id"].as_str();
        let valid_fill_id = fill_id.is_some_and(stable_pm_exit_fill_id);
        if !valid_fill_id {
            all_fill_ids_valid = false;
            missing.push("exit_fill_id_missing_or_unsupported");
        }
        let unique_fill_id = fill_id.is_some_and(|id| seen_fill_ids.insert(id.to_owned()));
        if valid_fill_id && !unique_fill_id {
            all_fill_ids_valid = false;
            missing.push("exit_fill_id_duplicate");
        }
        let parent_order_id = fill["parent_order_id"].as_str();
        let client_order_id = fill["client_order_id"].as_str();
        let order_link_valid = expected_client_order_id.is_some_and(|expected| {
            parent_order_id == Some(expected) && client_order_id == Some(expected)
        });
        let market_identity_valid = fill["symbol"].as_str() == Some(expected_symbol)
            && fill["side"].as_str() == Some(expected_side);
        if !order_link_valid {
            all_order_links_valid = false;
            missing.push("exit_fill_order_identity_missing_or_mismatched");
        }
        if !market_identity_valid {
            all_order_links_valid = false;
            missing.push("exit_fill_symbol_or_side_mismatched");
        }
        if order_link_valid && market_identity_valid && valid_fill_id && unique_fill_id {
            exit_fill_ids.push(fill_id.unwrap().to_owned());
        }

        let quantity = fill["quantity"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        let execution_price = fill["execution_price"]
            .as_f64()
            .filter(|value| value.is_finite() && *value > 0.0);
        if let Some(quantity) = quantity {
            filled_quantity += quantity;
        } else {
            missing.push("exit_fill_quantity_missing_or_invalid");
        }
        if execution_price.is_none() {
            missing.push("exit_fill_execution_price_missing_or_invalid");
        }

        let raw_fee = fill["fee"].as_f64().filter(|value| value.is_finite());
        let fee_known = fill["fee_known"].as_bool() == Some(true) && raw_fee.is_some();
        let fee_currency = fill["fee_currency"]
            .as_str()
            .filter(|value| !value.trim().is_empty());
        let fee_currency_known = fee_currency.is_some();
        if fee_known {
            known_fee_total += raw_fee.unwrap().abs();
        }
        if !fee_known || !fee_currency_known {
            all_fees_complete = false;
            missing.push(if fee_known {
                "exit_fill_fee_currency_missing"
            } else {
                "exit_fill_fee_amount_unknown"
            });
        }
        let fee_status = match (fee_known, fee_currency_known) {
            (true, true) => "known",
            (true, false) => "currency_unknown",
            _ => "unknown",
        };
        output_fills.push(serde_json::json!({
            "pm_trade_id": fill["id"],
            "fill_id": fill_id,
            "parent_order_id": parent_order_id,
            "client_order_id": client_order_id,
            "symbol": fill["symbol"],
            "side": fill["side"],
            "order_link_status": if order_link_valid && market_identity_valid { "linked" } else { "mismatched" },
            "quantity": quantity,
            "execution_price": execution_price,
            "fee_known": fee_known,
            "fee_amount": fee_known.then_some(raw_fee).flatten(),
            "fee_currency": fee_currency,
            "fee_currency_known": fee_currency_known,
            "fee_status": fee_status,
            "paper_exchange_fee_amount": fill["paper_exchange_fee_amount"],
            "paper_exchange_fee_currency": fill["paper_exchange_fee_currency"],
            "paper_exchange_fee_schedule_id": fill["paper_exchange_fee_schedule_id"],
            "paper_exchange_fee_schedule_version": fill["paper_exchange_fee_schedule_version"],
            "paper_exchange_fee_order_class": fill["paper_exchange_fee_order_class"],
            "paper_exchange_fee_schedule_evidence": fill["paper_exchange_fee_schedule_evidence"],
        }));
    }

    let quantity_matches = reported_filled.is_finite()
        && reported_filled >= 0.0
        && (filled_quantity - reported_filled).abs()
            <= 1e-9 * filled_quantity.abs().max(reported_filled.abs()).max(1.0);
    if !quantity_matches {
        missing.push("exit_fill_quantity_does_not_match_response");
    }
    let reported_fee = response["estimated_fee"]
        .as_f64()
        .filter(|value| value.is_finite())
        .map(f64::abs);
    let fee_total_matches = reported_fee.is_some_and(|reported| {
        (known_fee_total - reported).abs()
            <= 1e-9 * known_fee_total.abs().max(reported.abs()).max(1.0)
    });
    if !fee_total_matches {
        all_fees_complete = false;
        missing.push("per_fill_fee_total_does_not_match_response");
    }
    let fill_lineage_complete = all_order_links_valid && all_fill_ids_valid && quantity_matches;
    let fees_complete = all_fees_complete && fill_lineage_complete;
    let coverage_status = if fills.is_empty() {
        "missing"
    } else if fill_lineage_complete {
        "complete"
    } else {
        "incomplete"
    };
    let fee_coverage_status = if fills.is_empty() {
        "missing"
    } else if fees_complete {
        "complete"
    } else {
        "incomplete"
    };
    serde_json::json!({
        "schema_version": 1,
        "expected_client_order_id": expected_client_order_id,
        "expected_symbol": expected_symbol,
        "expected_side": expected_side,
        "coverage_status": coverage_status,
        "fee_coverage_status": fee_coverage_status,
        "fill_lineage_complete": fill_lineage_complete,
        "fees_complete": fees_complete,
        "complete": fill_lineage_complete && fees_complete,
        "reported_filled_quantity": reported_filled.is_finite().then_some(reported_filled),
        "fill_evidence_quantity": filled_quantity,
        "reported_estimated_fee": reported_fee,
        "known_fee_amount_abs_total": known_fee_total,
        "fee_total_matches": fee_total_matches,
        "exit_fill_ids": exit_fill_ids,
        "fills": output_fills,
        "missing_coverage": missing,
    })
}

fn position_exit_audit_payload(
    decision_id: &str,
    observed_at: u64,
    submitted_at: u64,
    symbol: &str,
    reason: &str,
    side: &str,
    order_type: &str,
    response: &serde_json::Value,
    expected_client_order_id: Option<&str>,
    lifecycle_identity: Option<&CanonicalLifecycleIdentity>,
    position: &TraderPosition,
    quantity: f64,
    filled: f64,
    average_price: Option<f64>,
    fee: f64,
    realized_pnl: f64,
    entry_notional: f64,
    return_fraction: f64,
    bid: f64,
    ask: f64,
    submit_response_ms: f64,
    stop_loss_percent: f64,
    take_profit_percent: f64,
    exit_decision_context: &serde_json::Value,
) -> serde_json::Value {
    let mid = bid + (ask - bid) * 0.5;
    let exit_fill_evidence =
        exit_fill_lineage_evidence(response, expected_client_order_id, symbol, side);
    let trade_id = lifecycle_identity
        .map(|identity| identity.trade_id.as_str())
        .or(position.trade_id.as_deref())
        .unwrap_or(decision_id);
    let canonical_identity_status = match lifecycle_identity {
        None => "missing",
        Some(identity)
            if identity.schema_version == 1
                && !identity.episode_id.trim().is_empty()
                && !identity.opportunity_id.trim().is_empty()
                && position.trade_id.as_deref() == Some(identity.trade_id.as_str()) =>
        {
            "complete"
        }
        Some(_) => "mismatched",
    };
    let lineage_status = if canonical_identity_status == "complete"
        && entry_fill_identity_complete(position)
        && exit_fill_evidence["complete"].as_bool() == Some(true)
    {
        "complete"
    } else {
        "NeedsAudit"
    };
    let mut payload = serde_json::json!({
        "record_schema_version": 2,
        "decision_id": decision_id,
        "trade_id": trade_id,
        "canonical_lifecycle_identity": lifecycle_identity,
        "canonical_identity_status": canonical_identity_status,
        "lineage_status": lineage_status,
        "exit_client_order_id": expected_client_order_id,
        "exit_fill_evidence": exit_fill_evidence,
        "trade_closed": filled + 1e-12 >= quantity,
        "strategy": position.strategy,
        "stage": format!("exit:{submitted_at}"),
        "timestamp": observed_at,
        "symbol": symbol,
        "status": exit_fill_status(filled, quantity),
        "reason": reason,
        "exit_order_type": order_type,
        "exit_order_quantity": quantity,
        "exit_filled_quantity": filled,
        "exit_average_price": average_price,
        "exit_fees": fee,
        "realized_pnl": realized_pnl,
        "execution_response": response,
        "net_return_percent": (entry_notional > 0.0).then_some(realized_pnl / entry_notional * 100.0),
        "side": side,
        "decision_mid_price": mid,
        "submitted_at": submitted_at,
        "response_received_at": observed_at,
        "submit_response_ms": submit_response_ms,
        "fill_rate": (quantity > 0.0).then_some((filled / quantity).clamp(0.0, 1.0)),
        "implementation_shortfall_bps": average_price.map(|price| if side == "buy" { (price / mid - 1.0) * 10_000.0 } else { (1.0 - price / mid) * 10_000.0 }),
        "fee_bps": (filled > 0.0).then_some(fee / (filled * mid) * 10_000.0),
        "exit_bid": bid,
        "exit_ask": ask,
        "effective_stop_loss_percent": stop_loss_percent,
        "effective_take_profit_percent": take_profit_percent,
        "exit_parameter_provenance": position.exit_parameter_provenance,
        "selected_stop_loss_percent": position
            .exit_parameter_provenance
            .as_ref()
            .and_then(|provenance| provenance["stop_loss"]["selected_percent"].as_f64()),
        "selected_take_profit_percent": position
            .exit_parameter_provenance
            .as_ref()
            .and_then(|provenance| provenance["take_profit"]["selected_percent"].as_f64()),
    });
    let trade_closed = filled + 1e-12 >= quantity;
    let exit_fee_currencies = response["fill_evidence"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|fill| fill["fee_currency"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let fee_currency = (position.entry_fee_currencies.len() == 1
        && exit_fee_currencies.len() == 1
        && position.entry_fee_currencies[0].as_str()
            == *exit_fee_currencies.iter().next().unwrap())
    .then(|| position.entry_fee_currencies[0].as_str());
    payload["realized_r_evidence"] = realized_r_evidence(
        position,
        symbol,
        trade_closed,
        position.realized_pnl + realized_pnl,
        fee_currency,
        exit_fill_evidence["fees_complete"].as_bool() == Some(true)
            && response["fee_complete"].as_bool() == Some(true),
    );
    let identity = position.lifecycle_identity.as_ref();
    payload["price_path"] = serde_json::json!(
        position
            .price_path
            .iter()
            .map(|sample| {
                serde_json::json!({
                    "timestamp": sample.timestamp,
                    "sequence": sample.sequence,
                    "epoch": sample.epoch,
                    "trade_id": position.trade_id,
                    "episode_id": identity.map(|identity| &identity.episode_id),
                    "opportunity_id": identity.map(|identity| &identity.opportunity_id),
                    "bid": sample.bid,
                    "ask": sample.ask,
                    "source_updated_at_ms": sample.source_updated_at_ms,
                    "freshness_contract_version": sample.freshness_contract_version,
                    "freshness_max_age_ms": sample.freshness_max_age_ms,
                    "source_fresh": sample.source_fresh,
                    "quote_source_system": sample.quote_source_system,
                    "quote_source_epoch": sample.quote_source_epoch,
                    "quote_source_cursor": sample.quote_source_cursor,
                    "quote_source_record_id": sample.quote_source_record_id,
                })
            })
            .collect::<Vec<_>>()
    );
    payload["price_path_evidence"] = price_path_excursion_evidence(position, observed_at);
    let executable_quotes = payload["price_path"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let entry_fill_links_full = serde_json::json!(position.entry_fill_links);
    payload["counterfactual_exit_fee_attribution"] = counterfactual_exit_fee_attribution(
        symbol,
        observed_at,
        &entry_fill_links_full,
        &payload["exit_fill_evidence"],
        &executable_quotes,
    )
    .unwrap_or(serde_json::Value::Null);
    payload["entry_context"] = serde_json::json!({
        "trade_id": trade_id,
        "strategy_decision": position.strategy_decision,
        "average_price": position.average_entry_price,
        "fees": position.entry_fees,
        "initial_entry_notional": position.initial_entry_notional,
        "total_entry_fees_paid": position.total_entry_fees_paid,
        "total_fees_paid": position.total_fees_paid + fee,
        "learning_context": position.entry_context,
        "timestamp_ms": position.entry_timestamp_ms,
        "duration_ms": (position.entry_timestamp_ms > 0).then_some(observed_at.saturating_sub(position.entry_timestamp_ms)),
        "quantity": position.quantity.abs(),
        "entry_fill_ids": position.entry_fill_ids,
        "entry_fill_links": entry_fill_links_full,
        "entry_fee_currencies": position.entry_fee_currencies,
        "entry_fill_costs_complete": position.entry_fill_costs_complete,
        "initial_risk_reference": position.initial_risk_reference,
        "exit_parameter_provenance": position.exit_parameter_provenance,
        "lifecycle_identity": lifecycle_identity.or(position.lifecycle_identity.as_ref()),
    });
    payload["exit_decision_context"] = exit_decision_context.clone();
    payload["exit_rule_trace"] = serde_json::json!({
        "reason": reason,
        "net_open_pnl": return_fraction * entry_notional,
        "net_return_fraction": return_fraction,
        "stop_loss_percent": stop_loss_percent,
        "take_profit_percent": take_profit_percent,
        "entry_notional": entry_notional,
        "entry_average_price": position.average_entry_price,
        "entry_fees": position.entry_fees,
        "observed_bid": bid,
        "observed_ask": ask,
    });
    let protection = profit_protection_parameters(position);
    payload["exit_rule_trace"]["profit_protection"] = serde_json::json!({
        "activation_percent": protection.activation_percent,
        "giveback_percent": protection.giveback_percent,
        "minimum_lock_percent": protection.minimum_lock_percent,
        "peak_return_percent": position.profit_protection_peak_return_percent,
        "triggered": reason == "profit-protection",
        "learning_profile_id": position.strategy_decision.as_ref()
            .and_then(|decision| decision.get("learning_profile_id")),
        "learning_profile_version": position.strategy_decision.as_ref()
            .and_then(|decision| decision.get("learning_profile_version")),
        "learned_exit_protection_policy": position.strategy_decision.as_ref()
            .and_then(|decision| decision.get("learned_exit_protection_policy")),
    });
    payload
}

fn queue_post_exit_observation(settings: &mut Settings, payload: &serde_json::Value) {
    let reason = payload["reason"].as_str().unwrap_or_default();
    if !matches!(
        reason,
        "take-profit" | "stop-loss" | "profit-protection" | "thesis-failure"
    ) || payload["trade_closed"].as_bool() != Some(true)
    {
        return;
    }
    let Some(exit_price) = payload["exit_average_price"]
        .as_f64()
        .filter(|value| *value > 0.0)
    else {
        return;
    };
    let direction = if payload["side"].as_str() == Some("sell") {
        1.0
    } else {
        -1.0
    };
    let now = current_time_ms();
    let event_id = format!(
        "exit-timing:{}",
        payload["trade_id"].as_str().unwrap_or_default()
    );
    if settings
        .post_exit_observations
        .iter()
        .any(|item| item["event_id"] == event_id)
    {
        return;
    }
    settings.post_exit_observations.push(serde_json::json!({
        "event_id": event_id,
        "decision_id": payload["decision_id"],
        "trade_id": payload["trade_id"],
        "symbol": payload["symbol"],
        "strategy": payload["strategy"],
        "reason": reason,
        "side": payload["side"],
        "direction": direction,
        "exit_price": exit_price,
        "exit_return_percent": payload["net_return_percent"],
        "exit_fees": payload["exit_fees"],
        "exit_filled_quantity": payload["exit_filled_quantity"],
        "effective_stop_loss_percent": payload["effective_stop_loss_percent"],
        "effective_take_profit_percent": payload["effective_take_profit_percent"],
        "pre_exit_price_path": payload["price_path"],
        "pre_exit_price_path_evidence": payload["price_path_evidence"],
        "counterfactual_exit_fee_attribution": payload["counterfactual_exit_fee_attribution"],
        "exit_fill_evidence": payload["exit_fill_evidence"],
        "realized_r_evidence": payload["realized_r_evidence"],
        "canonical_lifecycle_identity": payload["canonical_lifecycle_identity"],
        "exit_parameter_provenance": payload["exit_parameter_provenance"],
        "canonical_identity_status": payload["canonical_identity_status"],
        "lineage_status": payload["lineage_status"],
        "exit_timestamp_ms": now,
        "observe_at_ms": now.saturating_add(POST_EXIT_OBSERVATION_HORIZON_MS),
        "next_sample_at_ms": now,
        "price_path": [],
        "entry_context": payload["entry_context"],
        "strategy_decision": payload["entry_context"]["strategy_decision"],
        "learning_context": payload["entry_context"]["learning_context"]
    }));
}

fn exit_fill_status(filled: f64, quantity: f64) -> &'static str {
    if filled + 1e-12 >= quantity {
        "filled"
    } else if filled > 0.0 {
        "partially_filled_closed"
    } else {
        "expired_or_cancelled"
    }
}

fn new_position_exit_order(
    timestamp: u64,
    symbol: &str,
    side: &str,
    reason: &str,
    position: &TraderPosition,
    quantity: f64,
    notional: f64,
) -> TraderOrder {
    TraderOrder {
        timestamp,
        triggered_at: None,
        symbol: symbol.to_owned(),
        side: side.to_owned(),
        order_type: if position.exploratory && matches!(reason, "take-profit" | "profit-protection")
        {
            "PostOnlyLimit · exit".into()
        } else {
            "Ioc".into()
        },
        purpose: reason.into(),
        exploratory: position.exploratory,
        strategy: Some(
            position
                .strategy
                .clone()
                .unwrap_or_else(|| "unattributed".into()),
        ),
        // Link the protective exit to the decision that opened this position.
        decision_id: position.entry_decision_id.clone(),
        revalidation_snapshot: None,
        strength: 0.0,
        ioc_fallback_for: None,
        quantity,
        filled_quantity: 0.0,
        fill_rate: 0.0,
        average_price: None,
        fees_paid: 0.0,
        reserved_notional: notional,
        execution_order_id: None,
        processed_fill_trade_ids: Vec::new(),
        learning_context: None,
        book_bid_depth_10bps: None,
        book_ask_depth_10bps: None,
        book_depth_imbalance_10bps: None,
        estimated_sweep_bps: None,
        estimated_ioc_all_in_bps: None,
        decision_bid: None,
        decision_ask: None,
        decision_mid: None,
        submitted_limit_price: None,
        target_price: None,
        horizon_take_profit_percent: None,
        limit_to_mid_bps: None,
        limit_to_crossing_quote_bps: None,
        expires_at: None,
        failure_reason: None,
        stage_timings_ms: BTreeMap::new(),
        status: "submitting".into(),
        realized_pnl: 0.0,
        strategy_realized_pnl: None,
        strategy_realized_pnl_today: None,
        strategy_realized_pnl_day: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trade_price_path_samples_no_slower_than_one_second_when_events_arrive() {
        assert!(trade_price_path_sample_due(None, 1_000));
        assert!(!trade_price_path_sample_due(Some(1_000), 1_999));
        assert!(trade_price_path_sample_due(Some(1_000), 2_000));
        assert!(!trade_price_path_sample_due(Some(2_000), 1_000));
    }

    #[test]
    fn event_driven_gross_excursion_uses_executable_side_for_long_and_short() {
        assert_eq!(
            gross_executable_return_percent(1.0, 100.0, 103.0, 104.0),
            Some(3.0)
        );
        assert!(
            (gross_executable_return_percent(-1.0, 100.0, 96.0, 97.0).unwrap()
                - (100.0 / 97.0 - 1.0) * 100.0)
                .abs()
                < 1e-12
        );
        assert_eq!(
            gross_executable_return_percent(1.0, 0.0, 103.0, 104.0),
            None
        );
    }

    #[test]
    fn event_driven_excursion_aggregates_monotonically_and_fails_closed_on_gap_or_identity() {
        let mut position = TraderPosition::default();
        record_event_driven_gross_excursion(&mut position, Some(1.0), true, true, true, true);
        record_event_driven_gross_excursion(&mut position, Some(-2.0), true, true, true, false);
        record_event_driven_gross_excursion(&mut position, Some(0.5), false, true, true, false);
        assert_eq!(position.price_path_gross_mfe_percent, Some(1.0));
        assert_eq!(position.price_path_gross_mae_percent, Some(-2.0));
        assert_eq!(position.price_path_event_sample_count, 3);
        assert!(!position.price_path_event_metrics_complete);
    }

    #[test]
    fn event_threshold_crossings_keep_executable_quote_lineage_and_fee_uncertainty() {
        let position = TraderPosition {
            quantity: 1.0,
            price_path_epoch: Some("path-epoch".into()),
            ..TraderPosition::default()
        };
        let crossings = price_path_threshold_crossings_for_event(
            &position,
            2_000,
            98.0,
            99.0,
            Some(true),
            Some("paper_exchange.market_data_cache"),
            Some("pex-epoch"),
            Some(11),
            Some("paper-book:v1:pex-epoch:BTC-USDT:11"),
            2.0,
            3.0,
            -2.5,
            3.5,
            0.001,
            0.001,
            "entry_profile",
            "entry_profile",
        );
        assert_eq!(crossings.len(), 2);
        assert_eq!(crossings[0]["threshold_type"], "stop_loss");
        assert_eq!(crossings[0]["quote_side"], "bid");
        assert_eq!(crossings[0]["quote_source_cursor"], 11);
        assert_eq!(
            crossings[0]["fee_rate_attribution"],
            "local_estimate_unverified_against_effective_paper_exchange_schedule"
        );
        assert_eq!(crossings[1]["threshold_type"], "take_profit");
        assert!(
            price_path_threshold_crossings_for_event(
                &position,
                2_000,
                98.0,
                99.0,
                Some(false),
                Some("paper_exchange.market_data_cache"),
                Some("pex-epoch"),
                Some(12),
                Some("paper-book:v1:pex-epoch:BTC-USDT:12"),
                2.0,
                3.0,
                -2.5,
                3.5,
                0.001,
                0.001,
                "entry_profile",
                "entry_profile",
            )
            .is_empty()
        );
    }

    fn fee_schedule_event(from_ms: u64, to_ms: Option<u64>) -> serde_json::Value {
        serde_json::json!({
            "schema_version":"paper_exchange_financial_evidence:v1",
            "event_type":"fee_schedule",
            "event_id":"schedule-event-1",
            "source_system":"paper_exchange_simulator",
            "source_epoch":"paper-epoch-1",
            "source_cursor":7,
            "source_record_id":"paper-record-7",
            "source_timestamp_utc_ms":from_ms,
            "fee_schedule_id":"schedule-1",
            "fee_schedule_version":"v1",
            "symbol":"BTC-USDT",
            "effective_from_utc_ms":from_ms,
            "effective_to_utc_ms":to_ms,
            "maker_rate":0.0008,
            "taker_rate":0.001
        })
    }

    #[test]
    fn counterfactual_fee_attribution_requires_exact_schedule_and_quote_provenance() {
        let schedule = fee_schedule_event(1_000, None);
        let entry_links = vec![PositionEntryFillLink {
            paper_exchange_fee_amount: Some(0.01),
            paper_exchange_fee_currency: Some("USDT".into()),
            paper_exchange_fee_schedule_id: Some("schedule-1".into()),
            paper_exchange_fee_schedule_version: Some("v1".into()),
            paper_exchange_fee_order_class: Some("taker".into()),
            paper_exchange_fee_schedule_evidence: Some(schedule.clone()),
            ..PositionEntryFillLink::default()
        }];
        let exit_evidence = serde_json::json!({
            "fees_complete":true,
            "fills":[{
                "paper_exchange_fee_amount":0.01,
                "paper_exchange_fee_currency":"USDT",
                "paper_exchange_fee_schedule_id":"schedule-1",
                "paper_exchange_fee_schedule_version":"v1",
                "paper_exchange_fee_order_class":"taker",
                "paper_exchange_fee_schedule_evidence":schedule
            }]
        });
        let quotes = vec![serde_json::json!({
            "timestamp":1_500,
            "sequence":1,
            "epoch":"path-epoch-1",
            "source_updated_at_ms":1_490,
            "freshness_contract_version":1,
            "freshness_max_age_ms":5_000,
            "source_fresh":true,
            "quote_source_system":"paper_exchange.market_data_cache",
            "quote_source_epoch":"market-epoch-1",
            "quote_source_cursor":9,
            "quote_source_record_id":"paper-book-9"
        })];

        let attribution = counterfactual_exit_fee_attribution(
            "BTC-USDT",
            2_000,
            &serde_json::json!(entry_links),
            &exit_evidence,
            &quotes,
        )
        .expect("exact schedule and quote sources allow a fee contract");
        assert_eq!(attribution["schema_version"], 1);
        assert_eq!(attribution["fee_schedules"][0]["source_cursor"], 7);
        assert_eq!(attribution["quote_observations"][0][2], "v1");
        assert_eq!(attribution["exit_fee_schedule_ref"]["applied_rate"], 0.001);

        let mut stale_quotes = quotes;
        stale_quotes[0]["source_fresh"] = serde_json::Value::Bool(false);
        assert!(
            counterfactual_exit_fee_attribution(
                "BTC-USDT",
                2_000,
                &serde_json::json!(entry_links),
                &exit_evidence,
                &stale_quotes,
            )
            .is_none()
        );
    }

    #[test]
    fn position_exit_audit_includes_execution_and_entry_context() {
        let position = TraderPosition {
            quantity: 2.0,
            average_entry_price: 100.0,
            entry_fees: 0.2,
            trade_id: Some("trade-1".into()),
            lifecycle_identity: Some(CanonicalLifecycleIdentity {
                schema_version: 1,
                opportunity_id: "opportunity-1".into(),
                episode_id: "episode-1".into(),
                trade_id: "trade-1".into(),
                profile_id: Some("profile-1".into()),
                profile_hash: Some("sha256:profile-1".into()),
                profile_version: Some(1),
                execution_mode: "paper".into(),
                entry_policy: "strict".into(),
                source: "auto_trader_entry_reservation".into(),
            }),
            entry_timestamp_ms: 1_000,
            price_path_epoch: Some("path-epoch-1".into()),
            entry_fill_ids: vec!["pm-fill:v1:entry-1:1".into(), "pm-fill:v1:entry-1:2".into()],
            entry_fill_links: vec![
                PositionEntryFillLink {
                    fill_id: "pm-fill:v1:entry-1:1".into(),
                    client_order_id: "entry-order-1".into(),
                    symbol: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: 1.0,
                    execution_price: 100.0,
                    timestamp_ms: Some(1_000),
                    fee_known: true,
                    fee_amount: Some(0.05),
                    fee_currency: Some("USDT".into()),
                    ..PositionEntryFillLink::default()
                },
                PositionEntryFillLink {
                    fill_id: "pm-fill:v1:entry-1:2".into(),
                    client_order_id: "entry-order-1".into(),
                    symbol: "BTC-USDT".into(),
                    side: "buy".into(),
                    quantity: 1.0,
                    execution_price: 100.0,
                    timestamp_ms: Some(1_100),
                    fee_known: true,
                    fee_amount: Some(0.05),
                    fee_currency: Some("USDT".into()),
                    ..PositionEntryFillLink::default()
                },
            ],
            entry_fee_currencies: vec!["USDT".into()],
            entry_fill_costs_complete: true,
            initial_risk_reference: Some(InitialRiskReference {
                schema_version: 1,
                quote_currency: Some("USDT".into()),
                stop_loss_percent: Some(0.5),
                quote_amount: Some(10.0),
                fill_ids: vec!["pm-fill:v1:entry-1:1".into(), "pm-fill:v1:entry-1:2".into()],
                status: "complete".into(),
            }),
            strategy_decision: Some(serde_json::json!({"strategy": "test"})),
            price_path: vec![TradePriceSample {
                timestamp: 1_500,
                bid: 101.0,
                ask: 102.0,
                sequence: 1,
                epoch: Some("path-epoch-1".into()),
                source_updated_at_ms: Some(1_490),
                freshness_contract_version: Some(1),
                freshness_max_age_ms: Some(5_000),
                source_fresh: Some(true),
                ..TradePriceSample::default()
            }],
            ..TraderPosition::default()
        };
        let payload = position_exit_audit_payload(
            "decision-1",
            2_000,
            1_800,
            "BTC-USDT",
            "take-profit",
            "sell",
            "PostOnlyLimit",
            &serde_json::json!({
                "filled_quantity":2.0,
                "estimated_fee":0.1,
                "fee_complete":true,
                "fill_evidence":[
                    {"id":"pm-row-1","fill_id":"pm-fill:v1:epoch-1:1",
                     "parent_order_id":"1800","client_order_id":"1800",
                     "symbol":"BTC-USDT","side":"sell",
                     "quantity":1.0,"execution_price":109.0,"fee":-0.05,
                     "fee_known":true,"fee_currency":"USDT"},
                    {"id":"pm-row-2","fill_id":"pm-fill:v1:epoch-1:2",
                     "parent_order_id":"1800","client_order_id":"1800",
                     "symbol":"BTC-USDT","side":"sell",
                     "quantity":1.0,"execution_price":110.0,"fee":-0.05,
                     "fee_known":true,"fee_currency":"USDT"}
                ]
            }),
            Some("1800"),
            position.lifecycle_identity.as_ref(),
            &position,
            2.0,
            2.0,
            Some(110.0),
            0.1,
            19.7,
            200.0,
            0.0985,
            109.0,
            111.0,
            2.0,
            5.0,
            10.0,
            &serde_json::json!({"signal_active": true}),
        );

        assert_eq!(payload["stage"], "exit:1800");
        assert_eq!(payload["status"], "filled");
        assert_eq!(payload["net_return_percent"], 9.85);
        assert_eq!(payload["entry_context"]["duration_ms"], 1_000);
        assert_eq!(
            payload["entry_context"]["entry_fill_links"][0]["timestamp_ms"],
            1_000
        );
        assert_eq!(
            payload["entry_context"]["entry_fill_links"][1]["timestamp_ms"],
            1_100
        );
        assert_eq!(payload["price_path"][0]["timestamp"], 1_500);
        assert_eq!(payload["price_path"][0]["sequence"], 1);
        assert_eq!(payload["price_path"][0]["epoch"], "path-epoch-1");
        assert_eq!(payload["price_path"][0]["episode_id"], "episode-1");
        assert_eq!(payload["price_path_evidence"]["identity_complete"], true);
        assert_eq!(
            payload["price_path_evidence"]["initial_risk_complete"],
            true
        );
        assert_eq!(
            payload["price_path_evidence"]["policy_evaluation_eligible"],
            true
        );
        assert_eq!(payload["realized_r_evidence"]["status"], "complete");
        assert_eq!(
            payload["realized_r_evidence"]["net_realized_r_multiple"],
            1.97
        );
        let partial_r = realized_r_evidence(&position, "BTC-USDT", false, 10.0, Some("USDT"), true);
        assert_eq!(partial_r["status"], "not_closed");
        assert_eq!(
            partial_r["net_realized_r_multiple"],
            serde_json::Value::Null
        );
        let foreign_fee_r =
            realized_r_evidence(&position, "BTC-USDT", true, 19.7, Some("BNB"), true);
        assert_eq!(foreign_fee_r["status"], "unavailable");
        assert_eq!(
            foreign_fee_r["net_realized_r_multiple"],
            serde_json::Value::Null
        );
        assert_eq!(payload["exit_rule_trace"]["reason"], "take-profit");
        assert_eq!(payload["exit_order_type"], "PostOnlyLimit");
        assert_eq!(payload["exit_decision_context"]["signal_active"], true);
        assert_eq!(payload["trade_id"], "trade-1");
        assert_eq!(
            payload["canonical_lifecycle_identity"]["episode_id"],
            "episode-1"
        );
        assert_eq!(payload["canonical_identity_status"], "complete");
        assert_eq!(payload["lineage_status"], "complete");
        assert_eq!(payload["exit_fill_evidence"]["coverage_status"], "complete");
        assert_eq!(
            payload["exit_fill_evidence"]["fee_coverage_status"],
            "complete"
        );
        assert_eq!(
            payload["exit_fill_evidence"]["exit_fill_ids"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            payload["exit_fill_evidence"]["fills"][0]["fee_amount"],
            -0.05
        );
        assert_eq!(
            payload["exit_fill_evidence"]["fills"][0]["fee_currency"],
            "USDT"
        );
    }

    #[test]
    fn position_exit_audit_handles_zero_quantity_metrics() {
        let payload = position_exit_audit_payload(
            "decision-2",
            2,
            1,
            "ETH-USDT",
            "stop-loss",
            "buy",
            "Ioc",
            &serde_json::json!({
                "filled_quantity":0.0,
                "estimated_fee":0.0,
                "fee_complete":true,
                "fill_evidence":[]
            }),
            None,
            None,
            &TraderPosition::default(),
            0.0,
            0.0,
            None,
            0.0,
            0.0,
            0.0,
            0.0,
            99.0,
            101.0,
            0.0,
            5.0,
            10.0,
            &serde_json::Value::Null,
        );
        assert_eq!(payload["status"], "filled");
        assert_eq!(payload["fill_rate"], serde_json::Value::Null);
        assert_eq!(payload["net_return_percent"], serde_json::Value::Null);
        assert_eq!(payload["fee_bps"], serde_json::Value::Null);
        assert_eq!(
            payload["exit_fill_evidence"]["coverage_status"],
            "not_applicable"
        );
    }

    #[test]
    fn exit_fill_evidence_keeps_unknown_fees_null_and_rejects_mismatched_parent() {
        let response = serde_json::json!({
            "filled_quantity":2.0,
            "estimated_fee":0.04,
            "fee_complete":false,
            "fill_evidence":[
                {"id":"pm-row-1","fill_id":"pm-fill:v1:epoch-1:1",
                 "parent_order_id":"order-7","client_order_id":"order-7",
                 "symbol":"BTC-USDT","side":"sell",
                 "quantity":1.0,"execution_price":109.0,"fee":-0.05,
                 "fee_known":true,"fee_currency":"USDT"},
                {"id":"pm-row-2","fill_id":"pm-fill:v1:epoch-1:2",
                 "parent_order_id":"different-order","client_order_id":"order-7",
                 "symbol":"ETH-USDT","side":"buy",
                 "quantity":1.0,"execution_price":110.0,"fee":0.0,
                 "fee_known":false,"fee_currency":""}
            ]
        });
        let evidence = exit_fill_lineage_evidence(&response, Some("order-7"), "BTC-USDT", "sell");
        assert_eq!(evidence["coverage_status"], "incomplete");
        assert_eq!(evidence["fee_coverage_status"], "incomplete");
        assert_eq!(evidence["complete"], false);
        assert_eq!(
            evidence["exit_fill_ids"],
            serde_json::json!(["pm-fill:v1:epoch-1:1"])
        );
        assert_eq!(evidence["fills"][0]["fee_amount"], -0.05);
        assert_eq!(evidence["fills"][0]["fee_currency"], "USDT");
        assert_eq!(evidence["fills"][1]["fee_amount"], serde_json::Value::Null);
        assert_eq!(
            evidence["fills"][1]["fee_currency"],
            serde_json::Value::Null
        );
        assert_eq!(evidence["fills"][1]["fee_status"], "unknown");
        assert_eq!(evidence["fee_total_matches"], false);
        assert!(
            evidence["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "exit_fill_order_identity_missing_or_mismatched")
        );
        assert!(
            evidence["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "exit_fill_symbol_or_side_mismatched")
        );
        assert!(
            evidence["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "per_fill_fee_total_does_not_match_response")
        );

        let missing = exit_fill_lineage_evidence(
            &serde_json::json!({"filled_quantity":1.0,"estimated_fee":0.0,"fee_complete":false}),
            Some("order-7"),
            "BTC-USDT",
            "sell",
        );
        assert_eq!(missing["coverage_status"], "missing");
        assert_eq!(missing["exit_fill_ids"], serde_json::json!([]));
    }

    #[test]
    fn known_fee_amount_without_currency_remains_unconvertible_and_incomplete() {
        let evidence = exit_fill_lineage_evidence(
            &serde_json::json!({
                "filled_quantity":1.0,
                "estimated_fee":0.05,
                "fee_complete":false,
                "fill_evidence":[{
                    "id":"pm-row-1","fill_id":"pm-fill:v1:epoch-1:1",
                    "parent_order_id":"order-7","client_order_id":"order-7",
                    "symbol":"BTC-USDT","side":"sell",
                    "quantity":1.0,"execution_price":109.0,"fee":-0.05,
                    "fee_known":true,"fee_currency":""
                }]
            }),
            Some("order-7"),
            "BTC-USDT",
            "sell",
        );
        assert_eq!(evidence["fees_complete"], false);
        assert_eq!(evidence["fills"][0]["fee_amount"], -0.05);
        assert_eq!(
            evidence["fills"][0]["fee_currency"],
            serde_json::Value::Null
        );
        assert_eq!(evidence["fills"][0]["fee_status"], "currency_unknown");
    }

    #[test]
    fn executable_mfe_mae_uses_side_prices_and_marks_missing_coverage() {
        let fresh_sample = |timestamp, bid, ask| TradePriceSample {
            timestamp,
            bid,
            ask,
            sequence: timestamp / 5_000,
            epoch: Some("test-path-epoch".into()),
            source_updated_at_ms: Some(timestamp.saturating_sub(100)),
            freshness_contract_version: Some(1),
            freshness_max_age_ms: Some(5_000),
            source_fresh: Some(true),
            ..TradePriceSample::default()
        };
        let long = TraderPosition {
            quantity: 1.0,
            average_entry_price: 100.0,
            entry_timestamp_ms: 10_000,
            price_path_epoch: Some("test-path-epoch".into()),
            price_path: vec![
                fresh_sample(10_000, 101.0, 102.0),
                fresh_sample(15_000, 98.0, 99.0),
                fresh_sample(20_000, 103.0, 104.0),
            ],
            price_path_total_samples: 3,
            ..TraderPosition::default()
        };
        let evidence = price_path_excursion_evidence(&long, 25_000);
        assert_eq!(evidence["continuity_complete"], true);
        assert_eq!(evidence["complete"], false);
        assert_eq!(evidence["identity_complete"], false);
        assert!((evidence["gross_executable_mfe_percent"].as_f64().unwrap() - 3.0).abs() < 1e-12);
        assert!((evidence["gross_executable_mae_percent"].as_f64().unwrap() + 2.0).abs() < 1e-12);
        assert_eq!(
            evidence["fee_adjusted_mfe_mae_status"],
            "not_computed_without_path_level_execution_costs"
        );

        let mut discontinuous = long.clone();
        discontinuous.price_path[2].sequence = 5;
        let discontinuity_evidence = price_path_excursion_evidence(&discontinuous, 25_000);
        assert_eq!(discontinuity_evidence["complete"], false);
        assert!(
            discontinuity_evidence["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "path_sequence_discontinuity")
        );

        let mut short = TraderPosition {
            quantity: -1.0,
            average_entry_price: 100.0,
            entry_timestamp_ms: 10_000,
            price_path_epoch: Some("test-path-epoch".into()),
            price_path: vec![
                fresh_sample(10_000, 98.0, 99.0),
                fresh_sample(15_000, 101.0, 102.0),
                fresh_sample(20_000, 96.0, 97.0),
            ],
            price_path_total_samples: 3,
            ..TraderPosition::default()
        };
        let short_evidence = price_path_excursion_evidence(&short, 25_000);
        assert_eq!(short_evidence["continuity_complete"], true);
        assert_eq!(short_evidence["complete"], false);
        assert!(
            (short_evidence["gross_executable_mfe_percent"]
                .as_f64()
                .unwrap()
                - ((100.0 / 97.0 - 1.0) * 100.0))
                .abs()
                < 1e-12
        );
        assert!(
            (short_evidence["gross_executable_mae_percent"]
                .as_f64()
                .unwrap()
                - ((100.0 / 102.0 - 1.0) * 100.0))
                .abs()
                < 1e-12
        );

        short.price_path[1].source_fresh = Some(false);
        let incomplete = price_path_excursion_evidence(&short, 25_000);
        assert_eq!(incomplete["complete"], false);
        assert_eq!(incomplete["source_stale_sample_count"], 1);
        assert!(
            incomplete["missing_coverage"]
                .as_array()
                .unwrap()
                .iter()
                .any(|reason| reason == "stale_source_marks")
        );
    }

    #[test]
    fn thesis_failure_exits_enter_the_existing_post_exit_counterfactual_loop() {
        let mut settings = Settings::default();
        let payload = serde_json::json!({
            "reason": "thesis-failure",
            "trade_closed": true,
            "exit_average_price": 99.5,
            "trade_id": "trade-early-exit",
            "decision_id": "decision-early-exit",
            "symbol": "BTC-USDT",
            "strategy": "live-momentum",
            "side": "sell",
            "exit_fees": 0.1,
            "exit_filled_quantity": 1.0,
            "effective_stop_loss_percent": 0.5,
            "effective_take_profit_percent": 0.8,
            "price_path": [{"timestamp": 123, "bid": 99.4, "ask": 99.6}],
            "exit_fill_evidence": {"coverage_status":"complete", "exit_fill_ids":["pm-fill:v1:e:1"]},
            "canonical_lifecycle_identity": {"trade_id":"trade-early-exit"},
            "canonical_identity_status":"complete",
            "lineage_status":"complete",
            "entry_context": {
                "learning_context": "btc:live-momentum:short",
                "average_price": 100.0,
                "fees": 0.1,
                "quantity": 1.0
            }
        });

        queue_post_exit_observation(&mut settings, &payload);

        assert_eq!(settings.post_exit_observations.len(), 1);
        assert_eq!(
            settings.post_exit_observations[0]["reason"],
            "thesis-failure"
        );
        let observation = &settings.post_exit_observations[0];
        assert_eq!(observation["entry_context"]["average_price"], 100.0);
        assert_eq!(observation["entry_context"]["fees"], 0.1);
        assert_eq!(observation["exit_fees"], 0.1);
        assert_eq!(observation["exit_filled_quantity"], 1.0);
        assert_eq!(observation["effective_stop_loss_percent"], 0.5);
        assert_eq!(observation["pre_exit_price_path"][0]["bid"], 99.4);
        assert_eq!(
            observation["exit_fill_evidence"]["coverage_status"],
            "complete"
        );
        assert_eq!(
            observation["canonical_lifecycle_identity"]["trade_id"],
            "trade-early-exit"
        );
        assert_eq!(observation["lineage_status"], "complete");
    }
}
