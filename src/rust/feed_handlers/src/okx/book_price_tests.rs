#[cfg(test)]
mod book_price_tests {
    use super::{Book, Price, Snapshot, Update};

    fn prices_outside_the_configured_array_are_rejected() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        assert_eq!(book.bid_size_at_tick(107), None);
        assert!(
            book.update_from_snapshot(&Snapshot {
                asks: vec![],
                bids: vec![vec!["53.5", "1"]],
                ts: "1",
                seq: 1,
                prev_seq: 0,
            })
            .is_err()
        );
    }

    #[test]
    fn array_is_zero_anchored_and_preallocated_to_max_price() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        assert_eq!(book.bid_sizes_by_tick().len(), 107);

        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["52.5", "3"]],
            bids: vec![vec!["52", "2"]],
            ts: "1",
            seq: 1,
            prev_seq: 0,
        })
        .unwrap();
        assert_eq!(book.bid_sizes_by_tick()[104], 2.0);
        assert_eq!(book.ask_sizes_by_tick()[105], 3.0);
    }

    #[test]
    fn update_requires_an_initial_snapshot() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        let update = Update {
            bids: vec![],
            asks: vec![],
            ts: "1",
            seq: 1,
            prev_seq: 0,
        };

        assert!(book.update(&update).is_err());
    }

    #[test]
    fn prices_use_exact_fixed_point_units() {
        let parsed_price = Price::from_decimal("123.450067").unwrap();
        assert_eq!(parsed_price.units(), 123_450_067_000_000_000_000);
        assert_eq!(parsed_price.as_f64(), 123.450067);
        assert_eq!(
            Price::from_decimal("1.2300").unwrap(),
            Price::from_decimal("1.23").unwrap()
        );
        assert!(Price::from_decimal("1.1234567890123456789").is_err());
    }

    #[test]
    fn fixed_point_price_parser_rejects_malformed_decimals() {
        for value in ["", ".5", "1..2", "1.2.3", "-1", "+1", "1e3", "a1"] {
            assert!(Price::from_decimal(value).is_err(), "{value}");
        }
        assert_eq!(
            Price::from_decimal("1.").unwrap(),
            Price::from_decimal("1").unwrap()
        );
    }

}
