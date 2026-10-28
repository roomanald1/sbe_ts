fn decode_book(bytes: &Bytes) -> Option<BookSnapshot> {
    if bytes.len() < 272 {
        return None;
    }
    let symbol = std::str::from_utf8(&bytes[8..40]).ok()?.trim();
    if !valid_position_symbol(symbol) {
        return None;
    }
    let mut bids = Vec::new();
    let mut asks = Vec::new();
    for index in 0..BOOK_DEPTH {
        let bid_offset = 48 + index * 16;
        let ask_offset = 160 + index * 16;
        let bid = read_level(bytes, bid_offset)?;
        let ask = read_level(bytes, ask_offset)?;
        if bid.size > 0.0 {
            bids.push(bid);
        }
        if ask.size > 0.0 {
            asks.push(ask);
        }
    }
    Some(BookSnapshot {
        bids,
        asks,
        updated_at_ms: now_ms().ok()?,
    })
}
fn read_level(bytes: &[u8], offset: usize) -> Option<BookLevel> {
    let price = f64::from_le_bytes(bytes.get(offset..offset + 8)?.try_into().ok()?);
    let size = f64::from_le_bytes(bytes.get(offset + 8..offset + 16)?.try_into().ok()?);
    (price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0)
        .then_some(BookLevel { price, size })
}
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
    let fee = signed_fee_cashflow(trade.fee);
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
