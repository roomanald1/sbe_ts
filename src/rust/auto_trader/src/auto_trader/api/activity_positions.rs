struct ActivityPositionSnapshot {
    orders: Vec<TraderOrder>,
    auto_positions: Vec<(String, TraderPosition)>,
    strategy_positions: Vec<OpenStrategyPosition>,
    open_position_symbols: Vec<String>,
    position_exit_policies: Vec<PositionExitPolicy>,
}

fn snapshot_activity_positions(settings: &Settings, today_start: u64) -> ActivityPositionSnapshot {
    let today = utc_day();
    let orders = settings
        .orders
        .iter()
        // A protective order can be armed on a prior day and execute today.
        // Keep it so the position ledger's since-today barrier P&L is attributed.
        .filter(|order| order.timestamp >= today_start
            || order.purpose == "stop-loss"
            || order.strategy_realized_pnl_day.as_deref() == Some(today.as_str()))
        .rev()
        .take(MAX_RETAINED_AUTO_TRADER_ORDERS)
        .cloned()
        .map(|mut order| {
            if order.target_price.is_none() && order.purpose == "stop-loss" {
                if let Some(position) = settings
                    .positions
                    .get(&order.symbol)
                    .filter(|position| position.quantity.abs() > 1e-12)
                {
                    let quantity = position.quantity.abs();
                    let loss_fraction = settings.stop_loss_percent / 100.0;
                    let fee_rate = configured_taker_fee_rate();
                    order.target_price = Some(if position.quantity > 0.0 {
                        (quantity * position.average_entry_price * (1.0 - loss_fraction)
                            + position.entry_fees)
                            / (quantity * (1.0 - fee_rate))
                    } else {
                        (quantity * position.average_entry_price * (1.0 + loss_fraction)
                            - position.entry_fees)
                            / (quantity * (1.0 + fee_rate))
                    });
                }
            }
            if order.quantity > 0.0
                && order.filled_quantity >= order.quantity
                && matches!(order.status.as_str(), "armed" | "resting")
            {
                order.status = "filled".into();
            }
            order
        })
        .collect::<Vec<_>>();
    let auto_positions = settings
        .positions
        .iter()
        .filter(|(_, position)| position.quantity.abs() > 1e-12)
        .map(|(symbol, position)| {
            let mut position = position.clone();
            if position.strategy.is_none() {
                // Reconciliation can leave zero-quantity strategy lots behind. They must
                // not make an otherwise unambiguous live position lose its attribution.
                let active_lots = settings.strategy_positions.get(symbol).into_iter()
                    .flat_map(|lots| lots.iter())
                    .filter(|(_, lot)| lot.quantity.abs() > 1e-12)
                    .collect::<Vec<_>>();
                if let [(strategy, lot)] = active_lots.as_slice() {
                    position.strategy = Some((*strategy).clone());
                    if position.entry_decision_id.is_none() {
                        position.entry_decision_id = lot.entry_decision_id.clone();
                    }
                }
            }
            (symbol.clone(), position)
        })
        .collect::<Vec<_>>();
    let strategy_positions = settings.strategy_positions.iter().flat_map(|(symbol, lots)| {
        lots.iter().filter(|(_, position)| position.quantity.abs() > 1e-12).map(|(strategy, position)| {
            OpenStrategyPosition {
                symbol: symbol.clone(),
                strategy: strategy.clone(),
                decision_id: position.entry_decision_id.clone(),
                quantity: position.quantity,
                average_entry_price: position.average_entry_price,
                entry_fees: -position.entry_fees.abs(),
                entry_timestamp_ms: position.entry_timestamp_ms,
                stop_loss_percent: position.stop_loss_percent,
                take_profit_percent: position.take_profit_percent,
                horizon_take_profit_percent: position.horizon_take_profit_percent,
            }
        })
    }).collect::<Vec<_>>();
    let open_position_symbols = auto_positions
        .iter()
        .map(|(symbol, _)| symbol.clone())
        .collect::<Vec<_>>();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let position_exit_policies = auto_positions
        .iter()
        .map(|(symbol, position)| {
            let take_profit_candidate = position
                .take_profit_percent
                .or_else(|| position.horizon_take_profit_percent.map(|target| target.max(settings.take_profit_percent)))
                .unwrap_or(settings.take_profit_percent);
            let take_profit_percent = confidence_adjusted_take_profit_percent(
                &settings.closed_trade_returns_percent,
                take_profit_candidate,
            );
            PositionExitPolicy {
                symbol: symbol.clone(),
                stop_loss_percent: position.stop_loss_percent.unwrap_or(settings.stop_loss_percent),
                take_profit_percent: age_adjusted_take_profit_percent(
                    position.strategy.as_deref(),
                    position.entry_timestamp_ms,
                    now_ms,
                    take_profit_percent,
                ),
            }
        })
        .collect();
    ActivityPositionSnapshot {
        orders,
        auto_positions,
        strategy_positions,
        open_position_symbols,
        position_exit_policies,
    }
}

async fn await_activity_realized_pnl(
    client: &reqwest::Client,
    position_url: &str,
    orders: &mut [TraderOrder],
) -> (
    Option<f64>,
    Option<BTreeMap<String, f64>>,
    Vec<PnlBreakdown>,
) {
    let ledger_pnl = fetch_ledger_realized_pnl(client, position_url).await.ok();
    let (realized_pnl, realized_pnl_by_quote, pnl_breakdown) = if let Some(pnl) = ledger_pnl {
        for order in orders.iter_mut() {
            let realized = if order.purpose == "stop-loss" {
                pnl.realized_pnl_by_barrier_created_at
                    .get(&order.timestamp.to_string())
                    .copied()
            } else if order.order_type.starts_with("Twap") || order.order_type.starts_with("Vwap") {
                order
                    .execution_order_id
                    .as_ref()
                    .and_then(|id| pnl.realized_pnl_by_parent_order.get(id))
                    .copied()
            } else {
                pnl.realized_pnl_by_parent_order
                    .get(&order.timestamp.to_string())
                    .copied()
            };
            order.realized_pnl = realized.unwrap_or(0.0);
        }
        let mut grouped = BTreeMap::<(String, String, String, String), (f64, usize)>::new();
        for order in orders.iter() {
            let today = utc_day();
            let strategy_realized = if order.strategy_realized_pnl_day.as_deref() == Some(today.as_str()) {
                order.strategy_realized_pnl_today.unwrap_or_default()
            } else if order.purpose == "stop-loss" {
                // For legacy or pre-midnight barriers, `realized_pnl` has just
                // been replaced with the ledger's since-UTC-midnight amount.
                order.realized_pnl
            } else {
                order.strategy_realized_pnl.unwrap_or(order.realized_pnl)
            };
            if strategy_realized.abs() <= 1e-12 {
                continue;
            }
            let direction = if order.side.eq_ignore_ascii_case("sell") {
                "LONG"
            } else {
                "SHORT"
            };
            let total = grouped
                .entry((
                    order.symbol.clone(),
                    direction.to_owned(),
                    order.purpose.clone(),
                    order.strategy.clone().unwrap_or_else(|| "legacy-unknown".into()),
                ))
                .or_default();
            total.0 += strategy_realized;
            total.1 += 1;
        }
        let mut breakdown = grouped
            .into_iter()
            .map(
                |((symbol, direction, purpose, strategy), (realized_pnl, outcomes))| PnlBreakdown {
                    symbol,
                    direction,
                    purpose,
                    strategy,
                    realized_pnl,
                    outcomes,
                },
            )
            .collect::<Vec<_>>();
        breakdown
            .sort_by(|left, right| right.realized_pnl.abs().total_cmp(&left.realized_pnl.abs()));
        (
            pnl.realized_pnl_all_time_usd,
            Some(pnl.realized_pnl_by_quote),
            breakdown,
        )
    } else {
        for order in orders.iter_mut() {
            order.realized_pnl = 0.0;
        }
        (None, None, Vec::new())
    };
    (realized_pnl, realized_pnl_by_quote, pnl_breakdown)
}
