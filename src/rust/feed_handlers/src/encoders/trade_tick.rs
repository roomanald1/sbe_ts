use anyhow::{Result, ensure};
use bytes::Bytes;
use sbe_schema::{WriteBuf, message_header_codec, trade_tick_codec};

use super::SYMBOL_LENGTH;

pub(crate) fn encode_trade_tick(
    symbol: &str,
    trade_id: &str,
    timestamp: u64,
    price: f64,
    size: f64,
    side: &str,
) -> Result<Bytes> {
    ensure!(
        symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH,
        "instrument ID does not fit the SBE symbol field: {symbol}"
    );
    ensure!(
        trade_id.is_ascii() && trade_id.len() <= SYMBOL_LENGTH,
        "trade ID does not fit the SBE trade ID field"
    );
    ensure!(
        price.is_finite() && price > 0.0 && size.is_finite() && size > 0.0,
        "trade price and size must be positive finite values"
    );
    let side = match side {
        "buy" => 1,
        "sell" => 2,
        _ => anyhow::bail!("unsupported OKX trade side: {side}"),
    };

    let mut symbol_bytes = [b' '; SYMBOL_LENGTH];
    symbol_bytes[..symbol.len()].copy_from_slice(symbol.as_bytes());
    let mut trade_id_bytes = [b' '; SYMBOL_LENGTH];
    trade_id_bytes[..trade_id.len()].copy_from_slice(trade_id.as_bytes());

    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + trade_tick_codec::SBE_BLOCK_LENGTH as usize];
    let mut encoder = trade_tick_codec::TradeTickEncoder::default()
        .wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(symbol_bytes);
    encoder.trade_id(trade_id_bytes);
    encoder.timestamp(timestamp);
    encoder.price(price);
    encoder.size(size);
    encoder.side(side);
    drop(encoder.header(0));
    Ok(Bytes::from(buffer))
}
