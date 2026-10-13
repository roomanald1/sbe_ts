use anyhow::{Result, ensure};
use bytes::Bytes;
use sbe_schema::{WriteBuf, book_depth_codec, message_header_codec};

use super::SYMBOL_LENGTH;

const BOOK_DEPTH: usize = 7;

pub(crate) fn encode_book_depth(
    symbol: &str,
    timestamp: u64,
    bids: &[(f64, f64); BOOK_DEPTH],
    asks: &[(f64, f64); BOOK_DEPTH],
) -> Result<Bytes> {
    ensure!(
        symbol.is_ascii() && symbol.len() <= SYMBOL_LENGTH,
        "instrument ID does not fit the SBE symbol field: {symbol}"
    );

    let mut symbol_bytes = [b' '; SYMBOL_LENGTH];
    symbol_bytes[..symbol.len()].copy_from_slice(symbol.as_bytes());
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + book_depth_codec::SBE_BLOCK_LENGTH as usize];
    let mut encoder = book_depth_codec::BookDepthEncoder::default()
        .wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(symbol_bytes);
    encoder.timestamp(timestamp);
    encoder.bid_price_0(bids[0].0);
    encoder.bid_size_0(bids[0].1);
    encoder.bid_price_1(bids[1].0);
    encoder.bid_size_1(bids[1].1);
    encoder.bid_price_2(bids[2].0);
    encoder.bid_size_2(bids[2].1);
    encoder.bid_price_3(bids[3].0);
    encoder.bid_size_3(bids[3].1);
    encoder.bid_price_4(bids[4].0);
    encoder.bid_size_4(bids[4].1);
    encoder.ask_price_0(asks[0].0);
    encoder.ask_size_0(asks[0].1);
    encoder.ask_price_1(asks[1].0);
    encoder.ask_size_1(asks[1].1);
    encoder.ask_price_2(asks[2].0);
    encoder.ask_size_2(asks[2].1);
    encoder.ask_price_3(asks[3].0);
    encoder.ask_size_3(asks[3].1);
    encoder.ask_price_4(asks[4].0);
    encoder.ask_size_4(asks[4].1);
    encoder.bid_price_5(bids[5].0);
    encoder.bid_size_5(bids[5].1);
    encoder.bid_price_6(bids[6].0);
    encoder.bid_size_6(bids[6].1);
    encoder.ask_price_5(asks[5].0);
    encoder.ask_size_5(asks[5].1);
    encoder.ask_price_6(asks[6].0);
    encoder.ask_size_6(asks[6].1);
    drop(encoder.header(0));
    Ok(Bytes::from(buffer))
}

#[cfg(test)]
mod tests {
    use super::encode_book_depth;

    #[test]
    fn encodes_top_seven_book_levels_in_the_book_depth_sbe_message() {
        let bids = [
            (99.0, 1.0),
            (98.0, 2.0),
            (97.0, 3.0),
            (96.0, 4.0),
            (95.0, 5.0),
            (94.0, 6.0),
            (93.0, 7.0),
        ];
        let asks = [
            (101.0, 6.0),
            (102.0, 7.0),
            (103.0, 8.0),
            (104.0, 9.0),
            (105.0, 10.0),
            (106.0, 11.0),
            (107.0, 12.0),
        ];
        let message = encode_book_depth("BTC-USDT", 1_700_000_000_000, &bids, &asks).unwrap();

        assert_eq!(message.len(), 272);
        assert_eq!(u16::from_le_bytes([message[2], message[3]]), 2);
        assert_eq!(&message[8..16], b"BTC-USDT");
        assert_eq!(
            u64::from_le_bytes(message[40..48].try_into().unwrap()),
            1_700_000_000_000
        );
        assert_eq!(
            f64::from_le_bytes(message[48..56].try_into().unwrap()),
            99.0
        );
        assert_eq!(
            f64::from_le_bytes(message[160..168].try_into().unwrap()),
            101.0
        );
        assert_eq!(
            f64::from_le_bytes(message[264..272].try_into().unwrap()),
            12.0
        );
    }
}
