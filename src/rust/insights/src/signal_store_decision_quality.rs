impl SignalStore {
    async fn auto_trader_decision_quality(
        &self,
        since: Option<u64>,
    ) -> Result<AutoTraderDecisionQualitySummary> {
        let (records, decision_samples) = tokio::try_join!(
            self.load_auto_trader_execution_audit(since),
            self.count_auto_trader_decisions(since),
        )?;
        let mut summary = AutoTraderDecisionQualitySummary {
            decision_samples,
            ..Default::default()
        };
        let mut trades: HashMap<String, DecisionTradeAudit> = HashMap::new();
        for record in records {
            let Some(decision_id) = record
                .get("decision_id")
                .and_then(serde_json::Value::as_str)
            else {
                continue;
            };
            let stage = record
                .get("stage")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let trade = trades.entry(decision_id.to_owned()).or_default();
            if stage == "entry" {
                trade.entry_quantity = record
                    .get("filled_quantity")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0)
                    .max(0.0);
                trade.entry_price = record
                    .get("average_price")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite() && *v > 0.0);
                trade.entry_side = record
                    .get("side")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                trade.entry_fees = record
                    .get("fees")
                    .and_then(serde_json::Value::as_f64)
                    .unwrap_or(0.0)
                    .abs();
                trade.strategy_parameters = record
                    .get("strategy_decision")
                    .and_then(|decision| decision.get("strategy_parameters"))
                    .cloned();
                trade.exploratory = record
                    .get("exploratory")
                    .and_then(serde_json::Value::as_bool);
                trade.scout = record
                    .get("scout")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                trade.direction = record
                    .get("direction")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                trade.strength = record
                    .get("strength")
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite());
                trade.market_context = record
                    .get("market_context")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned);
                trade.strategy = record
                    .get("strategy")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| {
                        trade
                            .market_context
                            .as_deref()
                            .and_then(|context| context.rsplit(':').next())
                            .map(str::to_owned)
                    });
                trade.quote_currency = record
                    .get("symbol")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|symbol| symbol.rsplit_once('-').map(|(_, quote)| quote.to_owned()));
                trade.entry_timestamp = record.get("timestamp").and_then(serde_json::Value::as_u64);
                trade.signal_components = record.get("signal_components").cloned();
            } else if stage.starts_with("exit") {
                trade.exit_records.push(record);
            }
        }
        let mut groups: BTreeMap<(String, String), (usize, usize, f64, f64, BTreeSet<String>)> =
            BTreeMap::new();
        for trade in trades.values_mut() {
            let external_fill_ids = trade
                .exit_records
                .iter()
                .filter(|record| {
                    record
                        .get("exit_order_type")
                        .and_then(serde_json::Value::as_str)
                        == Some("external-ledger-fill")
                })
                .flat_map(|record| {
                    record
                        .get("exit_fills")
                        .and_then(serde_json::Value::as_array)
                        .into_iter()
                        .flatten()
                })
                .filter_map(|fill| {
                    fill.get("parent_order_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned)
                })
                .collect::<HashSet<_>>();
            let canonical_exit_records = trade
                .exit_records
                .iter()
                .filter(|record| {
                    let is_duplicate_stop = record
                        .get("exit_order_type")
                        .and_then(serde_json::Value::as_str)
                        == Some("StopMarket")
                        && record
                            .get("barrier_order_id")
                            .and_then(serde_json::Value::as_str)
                            .is_some_and(|id| external_fill_ids.contains(id));
                    !is_duplicate_stop
                })
                .collect::<Vec<_>>();
            let (exit_quantity, realized_pnl) =
                canonical_exit_records
                    .iter()
                    .fold((0.0, 0.0), |(quantity, pnl), record| {
                        let quantity_delta = record
                            .get("exit_filled_quantity")
                            .and_then(serde_json::Value::as_f64)
                            .unwrap_or(0.0)
                            .max(0.0);
                        let realized = record
                            .get("realized_pnl")
                            .and_then(serde_json::Value::as_f64)
                            .filter(|v| v.is_finite())
                            .unwrap_or(0.0);
                        (quantity + quantity_delta, pnl + realized)
                    });
            if trade.entry_quantity <= 0.0 {
                continue;
            }
            summary.filled_entry_samples += 1;
            if trade.scout {
                summary.scout_filled_entry_samples += 1;
            }
            if exit_quantity + 1e-12 < trade.entry_quantity {
                summary.unresolved_trade_samples += 1;
                if trade.scout {
                    summary.scout_unresolved_trade_samples += 1;
                }
                continue;
            }
            let Some(entry_price) = trade.entry_price else {
                summary.unpriced_closed_trade_samples += 1;
                if trade.scout {
                    summary.scout_unpriced_closed_trade_samples += 1;
                }
                continue;
            };
            let entry_notional = trade.entry_quantity * entry_price;
            if entry_notional <= 0.0 || !entry_notional.is_finite() {
                continue;
            }
            summary.completed_trade_samples += 1;
            if trade.scout {
                summary.scout_completed_trade_samples += 1;
            }
            let net_return = realized_pnl / entry_notional * 100.0;
            let strength_band = match trade.strength.unwrap_or(0.0) {
                v if v < 0.50 => "<0.50",
                v if v < 0.60 => "0.50–0.59",
                v if v < 0.70 => "0.60–0.69",
                _ => "0.70+",
            };
            let components = trade.signal_components.as_ref();
            let component_value = |name: &str| {
                components
                    .and_then(|values| values.get(name))
                    .and_then(serde_json::Value::as_f64)
                    .filter(|v| v.is_finite())
            };
            let momentum_band = component_value("momentum_10s_percent")
                .map(|v| {
                    if v < -0.12 {
                        "<-0.12%"
                    } else if v < -0.04 {
                        "-0.12 to -0.04%"
                    } else if v < 0.04 {
                        "-0.04 to 0.04%"
                    } else if v < 0.12 {
                        "0.04 to 0.12%"
                    } else {
                        "≥0.12%"
                    }
                })
                .unwrap_or("unavailable");
            let trend_band = component_value("trend_5m_volatility_units")
                .map(|v| {
                    if v < -1.5 {
                        "<-1.5σ"
                    } else if v < -0.5 {
                        "-1.5 to -0.5σ"
                    } else if v < 0.5 {
                        "-0.5 to 0.5σ"
                    } else if v < 1.5 {
                        "0.5 to 1.5σ"
                    } else {
                        "≥1.5σ"
                    }
                })
                .unwrap_or("unavailable");
            let volatility_band = component_value("volatility_1m_percent")
                .map(|v| {
                    if v < 0.10 {
                        "<0.10%"
                    } else if v < 0.30 {
                        "0.10 to 0.30%"
                    } else if v < 1.0 {
                        "0.30 to 1.00%"
                    } else {
                        "≥1.00%"
                    }
                })
                .unwrap_or("unavailable");
            let vwap_band = component_value("mispricing_vs_vwap_bps")
                .map(|v| {
                    if v < -8.0 {
                        "<-8 bps"
                    } else if v < -3.0 {
                        "-8 to -3 bps"
                    } else if v < 3.0 {
                        "-3 to 3 bps"
                    } else if v < 8.0 {
                        "3 to 8 bps"
                    } else {
                        "≥8 bps"
                    }
                })
                .unwrap_or("unavailable");
            let pressure_band = component_value("pressure_without_price_move")
                .map(|v| {
                    if v < -0.75 {
                        "<-0.75"
                    } else if v < 0.0 {
                        "-0.75 to 0"
                    } else if v < 0.75 {
                        "0 to 0.75"
                    } else {
                        "≥0.75"
                    }
                })
                .unwrap_or("unavailable");
            let aggressor_cost_band = component_value("aggressor_cost_bps")
                .map(|v| {
                    if v < 0.0 {
                        "<0 bps"
                    } else if v < 2.0 {
                        "0 to 2 bps"
                    } else if v < 5.0 {
                        "2 to 5 bps"
                    } else {
                        "≥5 bps"
                    }
                })
                .unwrap_or("unavailable");
            let momentum_flow_confirmation = match (
                component_value("momentum_10s_percent"),
                component_value("trade_flow_imbalance"),
                trade.direction.as_deref(),
            ) {
                (Some(momentum), Some(flow), Some(direction)) => {
                    let direction_sign = if direction == "bullish" { 1.0 } else { -1.0 };
                    let momentum_confirms = momentum * direction_sign > 0.0;
                    let flow_confirms = flow * direction_sign > 0.0;
                    match (momentum_confirms, flow_confirms) {
                        (true, true) => "both confirm",
                        (true, false) => "momentum only",
                        (false, true) => "flow only",
                        (false, false) => "neither confirms",
                    }
                }
                _ => "unavailable",
            };
            let market_regime = trade
                .market_context
                .as_deref()
                .and_then(|context| context.split(':').nth(1))
                .unwrap_or("unavailable");
            let entry_time_utc = trade
                .entry_timestamp
                .map(|timestamp| {
                    let hour = (timestamp / 3_600_000) % 24;
                    let start = (hour / 6) * 6;
                    format!("{start:02}:00–{:02}:59 UTC", start + 5)
                })
                .unwrap_or_else(|| "unavailable".to_owned());
            let dimensions = [
                (
                    "direction",
                    trade.direction.as_deref().unwrap_or("unknown").to_owned(),
                ),
                (
                    "strategy",
                    trade.strategy.as_deref().unwrap_or("unknown").to_owned(),
                ),
                (
                    "selection",
                    if trade.scout {
                        "scout"
                    } else {
                        match trade.exploratory {
                            Some(true) => "exploratory",
                            Some(false) => "strict",
                            None => "unknown",
                        }
                    }
                    .to_owned(),
                ),
                (
                    "market_context",
                    trade
                        .market_context
                        .as_deref()
                        .unwrap_or("unknown")
                        .to_owned(),
                ),
                ("market_regime", market_regime.to_owned()),
                ("entry_time_utc", entry_time_utc),
                ("strength", strength_band.to_owned()),
                ("10s_momentum", momentum_band.to_owned()),
                ("5m_trend", trend_band.to_owned()),
                ("1m_volatility", volatility_band.to_owned()),
                ("VWAP_mispricing", vwap_band.to_owned()),
                ("pressure_without_move", pressure_band.to_owned()),
                ("aggressor_cost", aggressor_cost_band.to_owned()),
                (
                    "momentum_flow_confirmation",
                    momentum_flow_confirmation.to_owned(),
                ),
            ];
            for (dimension, value) in dimensions {
                let group = groups.entry((dimension.to_owned(), value)).or_default();
                group.0 += 1;
                if net_return > 0.0 {
                    group.1 += 1;
                }
                group.2 += net_return;
                group.3 += realized_pnl;
                if let Some(currency) = trade.quote_currency.as_ref() {
                    group.4.insert(currency.clone());
                }
            }
        }
        summary.cohorts = groups
            .into_iter()
            .map(
                |((dimension, value), (samples, wins, return_total, pnl_total, currencies))| {
                    let pnl_quote_currency = (currencies.len() == 1)
                        .then(|| currencies.iter().next().cloned())
                        .flatten();
                    AutoTraderDecisionQualityCohort {
                        dimension,
                        value,
                        completed_trade_samples: samples,
                        average_net_return_percent: return_total / samples as f64,
                        win_rate: wins as f64 / samples as f64,
                        total_net_pnl: pnl_quote_currency.as_ref().map(|_| pnl_total),
                        pnl_quote_currency,
                        preliminary: samples < DECISION_QUALITY_MIN_COHORT_SAMPLES,
                    }
                },
            )
            .collect();
        Ok(summary)
    }
}
