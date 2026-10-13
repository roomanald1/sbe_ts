use anyhow::{Result, ensure};
use bytes::Bytes;
use sbe_schema::{
    SymbolPriceEncoder, WriteBuf, message_header_codec, symbol_price_codec::SBE_BLOCK_LENGTH,
};

use super::SYMBOL_LENGTH;

pub(crate) fn encode_symbol_price(symbol: &str, price: f64) -> Result<Bytes> {
    ensure!(
        symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH,
        "instrument ID does not fit the SBE symbol field: {symbol}"
    );

    let mut symbol_bytes = [b' '; SYMBOL_LENGTH];
    symbol_bytes[..symbol.len()].copy_from_slice(symbol.as_bytes());
    Ok(Bytes::from(encode_symbol_price_bytes(&symbol_bytes, price)))
}

pub(crate) fn encode_symbol_price_bytes(symbol: &[u8; SYMBOL_LENGTH], price: f64) -> Vec<u8> {
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + SBE_BLOCK_LENGTH as usize];

    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(*symbol);
    encoder.price(price);
    drop(encoder.header(0));

    buffer
}

#[cfg(test)]
mod tests {
    use super::{encode_symbol_price, encode_symbol_price_bytes};

    #[test]
    fn encodes_full_instrument_id_and_price_in_sbe_payload() {
        let payload = encode_symbol_price("1INCH-USDT", 1.25).unwrap();
        let mut expected_symbol = [b' '; 32];
        expected_symbol[..10].copy_from_slice(b"1INCH-USDT");
        assert_eq!(&payload[8..40], expected_symbol);
        assert_eq!(
            f64::from_le_bytes(payload[40..48].try_into().unwrap()),
            1.25
        );
    }

    #[test]
    fn rejects_instrument_ids_that_exceed_the_sbe_field() {
        assert!(encode_symbol_price("123456789012345678901234567890123", 1.0).is_err());
    }

    #[test]
    fn synthetic_update_encoding_uses_the_expected_sbe_payload_length() {
        let symbol = [b' '; 32];
        assert_eq!(encode_symbol_price_bytes(&symbol, 1.0).len(), 48);
    }
}
