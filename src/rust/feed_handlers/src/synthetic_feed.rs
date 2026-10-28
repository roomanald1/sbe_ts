fn synthetic_price_update(round: u64, symbol_index: usize) -> ([u8; SYMBOL_LENGTH], f64) {
    let mut symbol = [b' '; SYMBOL_LENGTH];
    symbol[..3].copy_from_slice(b"SYM");
    symbol[3] = b'0' + (symbol_index / 100) as u8;
    symbol[4] = b'0' + ((symbol_index / 10) % 10) as u8;
    symbol[5] = b'0' + (symbol_index % 10) as u8;
    let base_price = 25.0 + symbol_index as f64 * 4.75;
    let phase = round as f64 * 0.17 + symbol_index as f64 * 1.91;
    let movement = phase.sin() * 0.004 + (phase * 0.37).cos() * 0.001;
    (symbol, base_price * (1.0 + movement))
}

fn is_synthetic_request(payload: &Payload) -> bool {
    payload
        .data()
        .is_some_and(|data| data.as_ref() == b"synthetic")
}

#[cfg(test)]
mod tests {
    use super::{is_synthetic_request, requested_book_instrument, synthetic_price_update};
    use crate::encoders::encode_symbol_price_bytes;
    use rsocket_rust::prelude::Payload;

    #[test]
    fn synthetic_updates_produce_fast_feed_symbols_and_vary_by_round() {
        let (symbol, first_price) = synthetic_price_update(0, 0);
        let (_, next_price) = synthetic_price_update(1, 0);

        assert_eq!(&symbol[..6], b"SYM000");
        assert!(symbol[6..].iter().all(|byte| *byte == b' '));
        assert_ne!(first_price, next_price);
        assert!(first_price.is_finite() && first_price > 0.0);
        assert_eq!(
            encode_symbol_price_bytes(&symbol, first_price, f64::NAN, f64::NAN).len(),
            56
        );
    }

    #[test]
    fn selects_synthetic_stream_only_for_synthetic_request_payload() {
        let synthetic = Payload::builder().set_data(b"synthetic".to_vec()).build();
        let okx = Payload::builder().set_data(b"okx".to_vec()).build();
        let book = Payload::builder()
            .set_data(b"book:BTC-USDT".to_vec())
            .build();

        assert!(is_synthetic_request(&synthetic));
        assert!(!is_synthetic_request(&okx));
        assert!(!is_synthetic_request(&Payload::new(None, None)));
        assert_eq!(requested_book_instrument(&book), Some("BTC-USDT"));
    }
}
