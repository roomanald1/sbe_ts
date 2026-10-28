fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn retry_delay_after_failure(current_delay: Duration, connection_duration: Duration) -> Duration {
    if connection_duration >= STABLE_CONNECTION_DURATION {
        INITIAL_RECONNECT_DELAY
    } else {
        current_delay
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BOOK_DEPTH, BookMessage, INITIAL_RECONNECT_DELAY, MAX_RECONNECT_DELAY,
        STABLE_CONNECTION_DURATION, TickerMessage, apply_book_side, retry_delay_after_failure,
        select_top_instruments, top_book_side,
    };
    use serde_json::json;
    use std::time::Duration;

    #[test]
    fn reconnect_delay_resets_after_a_stable_connection() {
        assert_eq!(
            retry_delay_after_failure(
                MAX_RECONNECT_DELAY,
                STABLE_CONNECTION_DURATION + Duration::from_secs(1)
            ),
            INITIAL_RECONNECT_DELAY
        );
    }

    #[test]
    fn reconnect_delay_is_retained_after_a_short_connection() {
        assert_eq!(
            retry_delay_after_failure(Duration::from_secs(8), Duration::from_secs(5)),
            Duration::from_secs(8)
        );
    }

    #[test]
    fn selects_top_spot_instruments_by_24_hour_quote_volume() {
        let tickers = vec![
            json!({"instId": "LOW-USDT", "volCcy24h": "10"}),
            json!({"instId": "HIGH-USDT", "volCcy24h": "1000"}),
            json!({"instId": "MID-USDT", "volCcy24h": "100"}),
        ];

        assert_eq!(
            select_top_instruments(&tickers).unwrap(),
            ["HIGH-USDT", "MID-USDT", "LOW-USDT"]
        );
    }

    #[test]
    fn selects_no_more_than_the_top_100_instruments() {
        let tickers = (0..110)
            .map(|index| {
                json!({
                    "instId": format!("COIN{index:03}-USDT"),
                    "volCcy24h": index.to_string()
                })
            })
            .collect::<Vec<_>>();

        let selected = select_top_instruments(&tickers).unwrap();
        assert_eq!(selected.len(), 100);
        assert_eq!(selected[0], "COIN109-USDT");
        assert_eq!(selected[99], "COIN010-USDT");
    }

    #[test]
    fn ticker_updates_borrow_instrument_and_price_from_json() {
        let text =
            r#"{"arg":{"channel":"tickers"},"data":[{"instId":"BTC-USDT","last":"84000.5"}]}"#;
        let message: TickerMessage<'_> = serde_json::from_str(text).unwrap();
        let ticker = &message.data.unwrap()[0];

        assert_eq!(ticker.instrument, "BTC-USDT");
        assert_eq!(ticker.last, "84000.5");
    }

    #[test]
    fn parses_okx_book_snapshot_and_keeps_seven_best_levels() {
        let text = r#"{"arg":{"channel":"books","instId":"BTC-USDT"},"action":"snapshot","data":[{"asks":[["101","1"],["102","2"]],"bids":[["99","3"],["98","4"]],"ts":"1700000000000"}]}"#;
        let message: BookMessage<'_> = serde_json::from_str(text).unwrap();
        let update = &message.data.unwrap()[0];
        let mut bid_book = Vec::new();
        let mut ask_book = Vec::new();
        apply_book_side(&mut bid_book, &update.bids).unwrap();
        apply_book_side(&mut ask_book, &update.asks).unwrap();
        let bids = top_book_side(&bid_book, true);
        let asks = top_book_side(&ask_book, false);

        assert_eq!(message.arg.unwrap().instrument, "BTC-USDT");
        assert_eq!(update.ts, "1700000000000");
        assert_eq!(bids[0], (99.0, 3.0));
        assert_eq!(bids[1], (98.0, 4.0));
        assert_eq!(bids[2..], [(0.0, 0.0); BOOK_DEPTH - 2]);
        assert_eq!(asks[0], (101.0, 1.0));
        assert_eq!(asks[1], (102.0, 2.0));
    }

    #[test]
    fn rejects_invalid_okx_book_levels() {
        assert!(apply_book_side(&mut Vec::new(), &[vec!["NaN", "1"]]).is_err());
        assert!(apply_book_side(&mut Vec::new(), &[vec!["100", "-1"]]).is_err());
        assert!(apply_book_side(&mut Vec::new(), &[vec!["100"]]).is_err());
    }
}
