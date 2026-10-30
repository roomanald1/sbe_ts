impl SignalStore {
    async fn auto_trader_execution_quality(
        &self,
        since: Option<u64>,
    ) -> Result<AutoTraderExecutionQualitySummary> {
        let records = self.load_auto_trader_execution_audit(since, 50_000).await?;
        let mut summary = AutoTraderExecutionQualitySummary::default();
        let (mut requested_quantity, mut filled_quantity, mut measured_notional) = (0.0, 0.0, 0.0);
        let (mut shortfall_total, mut fee_total, mut all_in_total, mut fee_notional) =
            (0.0, 0.0, 0.0, 0.0);
        let (mut response_latency_total, mut response_latency_samples) = (0u64, 0usize);
        let external_exit_ids = records
            .iter()
            .filter(|record| {
                record
                    .get("exit_order_type")
                    .and_then(serde_json::Value::as_str)
                    == Some("external-ledger-fill")
            })
            .flat_map(|record| {
                let trade_id = record
                    .get("trade_id")
                    .or_else(|| record.get("decision_id"))
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_owned();
                record
                    .get("exit_fills")
                    .and_then(serde_json::Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(move |fill| {
                        fill.get("parent_order_id")
                            .and_then(serde_json::Value::as_str)
                            .map(|id| (trade_id.clone(), id.to_owned()))
                    })
            })
            .collect::<HashSet<_>>();

        for record in records {
            let stage = record
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if stage != "entry" && !stage.starts_with("exit") {
                continue;
            }
            let trade_id = record
                .get("trade_id")
                .or_else(|| record.get("decision_id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if record
                .get("exit_order_type")
                .and_then(serde_json::Value::as_str)
                == Some("StopMarket")
                && record
                    .get("barrier_order_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| external_exit_ids.contains(&(trade_id.to_owned(), id.to_owned())))
            {
                continue;
            }
            summary.order_samples += 1;
            summary.last_updated_at = summary.last_updated_at.max(
                record
                    .get("response_received_at")
                    .or_else(|| record.get("observed_at"))
                    .or_else(|| record.get("timestamp"))
                    .and_then(serde_json::Value::as_u64),
            );
            if let Some(latency_ms) = record
                .get("submit_response_ms")
                .and_then(serde_json::Value::as_f64)
                .filter(|value| value.is_finite() && *value >= 0.0)
            {
                response_latency_total += (latency_ms * 1_000.0) as u64;
                response_latency_samples += 1;
            }
            let quantity = (if stage == "entry" {
                record.get("order_quantity")
            } else {
                record.get("exit_order_quantity")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .max(0.0);
            let filled = (if stage == "entry" {
                record.get("filled_quantity")
            } else {
                record.get("exit_filled_quantity")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .clamp(0.0, quantity);
            requested_quantity += quantity;
            filled_quantity += filled;
            if filled <= 0.0 {
                summary.unfilled_order_samples += 1;
                continue;
            }
            summary.filled_order_samples += 1;
            if quantity > 0.0 && filled + 1e-12 < quantity {
                summary.partial_fill_samples += 1;
            }

            let average_price = (if stage == "entry" {
                record.get("average_price")
            } else {
                record.get("exit_average_price")
            })
            .and_then(serde_json::Value::as_f64)
            .filter(|price| price.is_finite() && *price > 0.0);
            let fee = (if stage == "entry" {
                record.get("fees")
            } else {
                record.get("exit_fees")
            })
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0)
            .abs();
            let Some(execution_price) = average_price else {
                continue;
            };
            let notional = filled * execution_price;
            if notional <= 0.0 || !notional.is_finite() {
                continue;
            }
            let fee_bps = fee / notional * 10_000.0;
            fee_total += fee_bps * notional;
            fee_notional += notional;

            let Some(mid) = record
                .get("decision_mid_price")
                .and_then(serde_json::Value::as_f64)
                .filter(|price| price.is_finite() && *price > 0.0)
            else {
                continue;
            };
            let Some(side) = record.get("side").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let shortfall_bps = if side == "buy" {
                (execution_price / mid - 1.0) * 10_000.0
            } else {
                (1.0 - execution_price / mid) * 10_000.0
            };
            measured_notional += notional;
            shortfall_total += shortfall_bps * notional;
            all_in_total += (shortfall_bps + fee_bps) * notional;
            summary.measured_fill_samples += 1;
        }

        summary.average_fill_rate =
            (requested_quantity > 0.0).then_some(filled_quantity / requested_quantity);
        summary.reference_price_coverage_percent = (summary.filled_order_samples > 0).then_some(
            summary.measured_fill_samples as f64 / summary.filled_order_samples as f64 * 100.0,
        );
        summary.average_implementation_shortfall_bps =
            (measured_notional > 0.0).then_some(shortfall_total / measured_notional);
        summary.average_all_in_cost_bps =
            (measured_notional > 0.0).then_some(all_in_total / measured_notional);
        summary.average_fee_bps = (fee_notional > 0.0).then_some(fee_total / fee_notional);
        summary.response_latency_samples = response_latency_samples;
        summary.average_submit_response_ms = (response_latency_samples > 0)
            .then_some(response_latency_total as f64 / response_latency_samples as f64 / 1_000.0);
        Ok(summary)
    }
}
