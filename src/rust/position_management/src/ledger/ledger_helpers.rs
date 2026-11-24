fn now_ms() -> anyhow::Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis() as u64)
}
fn internal_error(error: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}
fn valid_position_symbol(symbol: &str) -> bool {
    symbol.len() <= 32
        && symbol.is_ascii()
        && symbol.split_once('-').is_some_and(|(base, quote)| {
            !base.is_empty()
                && !quote.is_empty()
                && base.bytes().all(|b| b.is_ascii_alphanumeric())
                && quote.bytes().all(|b| b.is_ascii_alphanumeric())
        })
}

fn summarize_positions(trades: &[PositionTrade]) -> PositionsResponse {
    summarize_positions_from(trades.iter())
}

fn summarize_positions_from<'a>(
    trades: impl Iterator<Item = &'a PositionTrade>,
) -> PositionsResponse {
    let mut positions = BTreeMap::<String, Position>::new();
    let mut trade_count = 0;
    for trade in trades {
        trade_count += 1;
        let position = positions.entry(trade.symbol.clone()).or_insert(Position {
            symbol: trade.symbol.clone(),
            quantity: 0.0,
            average_entry_price: 0.0,
            realized_pnl: 0.0,
            entry_fees: 0.0,
            fees_paid: 0.0,
            known_fee_fills: 0,
            unknown_fee_fills: 0,
            estimated_exit_fee_rate: 0.0,
            last_trade_timestamp: trade.timestamp,
        });
        apply_trade(position, trade);
        position.last_trade_timestamp = trade.timestamp;
    }
    PositionsResponse {
        positions: positions.into_values().collect(),
        trade_count,
    }
}
fn signed_fee_cashflow(fee: f64) -> f64 {
    // Paper trading has fees but no rebates. Normalize historical rows too:
    // earlier versions persisted positive fees for some fills.
    -fee.abs()
}
fn apply_trade(position: &mut Position, trade: &PositionTrade) {
    let old = position.quantity;
    // Unknown historical fees are excluded from numeric P&L and fee totals.
    // Their aggregate coverage is tracked by LedgerTradeIndex; the zero in a
    // legacy PositionTrade is not an observed fee amount.
    let fee = if trade.fee_known {
        position.known_fee_fills += 1;
        signed_fee_cashflow(trade.fee)
    } else {
        position.unknown_fee_fills += 1;
        0.0
    };
    position.fees_paid -= fee;
    let incoming = match trade.side {
        TradeSide::Buy => trade.quantity,
        TradeSide::Sell => -trade.quantity,
    };
    position.estimated_exit_fee_rate = if trade.fee_rate == 0.0 {
        configured_fee_rates().taker.abs()
    } else {
        trade.fee_rate.abs()
    };
    if old == 0.0 || old.signum() == incoming.signum() {
        let total = old.abs() + incoming.abs();
        position.average_entry_price = (old.abs() * position.average_entry_price
            + incoming.abs() * trade.execution_price)
            / total;
        position.quantity = old + incoming;
        position.entry_fees += fee;
        return;
    }
    let closed = old.abs().min(incoming.abs());
    let entry_fee_closed = position.entry_fees * (closed / old.abs());
    let closing_fee = fee * (closed / incoming.abs());
    let gross = closed
        * if old > 0.0 {
            trade.execution_price - position.average_entry_price
        } else {
            position.average_entry_price - trade.execution_price
        };
    // Both fee allocations are signed cash flows, so adding them deducts fees.
    position.realized_pnl += gross + entry_fee_closed + closing_fee;
    let net = old + incoming;
    if net.abs() < EPSILON {
        position.quantity = 0.0;
        position.average_entry_price = 0.0;
        position.entry_fees = 0.0;
    } else if net.signum() != old.signum() {
        position.average_entry_price = trade.execution_price;
        position.entry_fees = fee - closing_fee;
        position.quantity = net;
    } else {
        position.entry_fees -= entry_fee_closed;
        position.quantity = net;
    }
}

#[cfg(test)]
mod ledger_helper_tests {
    use super::*;

    #[test]
    fn validates_spot_symbols_without_accepting_malformed_components() {
        assert!(valid_position_symbol("BTC-USDT"));
        assert!(valid_position_symbol("A1-B2"));
        assert!(!valid_position_symbol("-USDT"));
        assert!(!valid_position_symbol("BTC-"));
        assert!(!valid_position_symbol("BTC-USDT-SWAP"));
        assert!(!valid_position_symbol("BTC/USDT"));
        assert!(!valid_position_symbol("BÉ-USDT"));
        assert!(!valid_position_symbol("A".repeat(33).as_str()));
    }

    #[test]
    fn historical_positive_fees_are_normalized_to_cash_outflows() {
        assert_eq!(signed_fee_cashflow(0.25), -0.25);
        assert_eq!(signed_fee_cashflow(-0.25), -0.25);
        assert_eq!(signed_fee_cashflow(0.0), 0.0);
    }

    #[test]
    fn unknown_fees_are_excluded_but_known_zero_fees_remain_known() {
        let make_trade = |fee_known| PositionTrade {
            id: "fee-test".to_owned(),
            symbol: "BTC-USDT".to_owned(),
            side: TradeSide::Buy,
            quantity: 1.0,
            execution_price: 100.0,
            reference_price: None,
            timestamp: 1,
            source: "manual".to_owned(),
            parent_order_id: None,
            fee: 0.0,
            fee_known,
            fee_currency: "USDT".to_owned(),
            fee_rate: 0.0,
            fee_liquidity: String::new(),
            resting_order_id: None,
            decision_timestamp: None,
            decision_price: None,
            decision_mid_price: None,
            slippage: None,
            slippage_bps: None,
        };

        let mut unknown_position = Position {
            symbol: "BTC-USDT".to_owned(), quantity: 0.0,
            average_entry_price: 0.0, realized_pnl: 0.0, entry_fees: 0.0,
            fees_paid: 0.0, known_fee_fills: 0, unknown_fee_fills: 0,
            estimated_exit_fee_rate: 0.0, last_trade_timestamp: 0,
        };
        apply_trade(&mut unknown_position, &make_trade(false));
        assert_eq!(unknown_position.fees_paid, 0.0);
        assert_eq!(unknown_position.entry_fees, 0.0);
        assert_eq!(unknown_position.unknown_fee_fills, 1);
        assert_eq!(unknown_position.known_fee_fills, 0);
        let unknown_json = serde_json::to_value(&unknown_position).unwrap();
        assert_eq!(unknown_json["unknown_fee_fills"], 1);
        assert_eq!(unknown_json["fees_paid"], 0.0);

        let mut known_zero_position = Position {
            symbol: "BTC-USDT".to_owned(), quantity: 0.0,
            average_entry_price: 0.0, realized_pnl: 0.0, entry_fees: 0.0,
            fees_paid: 0.0, known_fee_fills: 0, unknown_fee_fills: 0,
            estimated_exit_fee_rate: 0.0, last_trade_timestamp: 0,
        };
        apply_trade(&mut known_zero_position, &make_trade(true));
        assert_eq!(known_zero_position.fees_paid, 0.0);
        assert_eq!(known_zero_position.entry_fees, 0.0);
        assert_eq!(known_zero_position.known_fee_fills, 1);
        assert_eq!(known_zero_position.unknown_fee_fills, 0);
        let known_zero_json = serde_json::to_value(&known_zero_position).unwrap();
        assert_eq!(known_zero_json["known_fee_fills"], 1);
        assert_eq!(known_zero_json["unknown_fee_fills"], 0);
    }
}
