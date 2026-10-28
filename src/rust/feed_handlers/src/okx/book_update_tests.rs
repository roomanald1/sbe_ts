#[cfg(test)]
mod book_update_tests {
    use super::{Book, Price, Snapshot, Update};

    #[test]
    fn update_changes_and_removes_levels_by_direct_index() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["52.5", "4"], vec!["53", "5"]],
            bids: vec![vec!["52", "2"], vec!["51", "3"]],
            ts: "1",
            seq: 10,
            prev_seq: 0,
        })
        .unwrap();

        book.update(&Update {
            bids: vec![vec!["52", "0"], vec!["51.5", "6"]],
            asks: vec![vec!["52.5", "7"]],
            ts: "2",
            seq: 11,
            prev_seq: 10,
        })
        .unwrap();

        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("51.5").unwrap()
        );
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("52.5").unwrap()
        );
        assert_eq!(book.best_ask().unwrap().size, 7.0);
        assert_eq!(book.sequence(), Some(11));

        book.update(&Update {
            bids: vec![],
            asks: vec![vec!["52.5", "0"]],
            ts: "3",
            seq: 12,
            prev_seq: 11,
        })
        .unwrap();
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("53").unwrap()
        );
    }

    #[test]
    fn rejects_update_with_missing_predecessor_seq_id_and_preserves_book() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        let current_seq_id = 10;
        let missing_seq_id = 11;
        let update_seq_id = 12;

        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["52.5", "3"]],
            bids: vec![vec!["52", "2"]],
            ts: "1",
            seq: current_seq_id,
            prev_seq: 0,
        })
        .unwrap();

        let out_of_order_update = Update {
            bids: vec![vec!["51.5", "1"]],
            asks: vec![],
            ts: "2",
            seq: update_seq_id,
            prev_seq: missing_seq_id,
        };
        assert_eq!(book.sequence(), Some(current_seq_id));
        assert_eq!(out_of_order_update.prev_seq, missing_seq_id);
        assert!(book.update(&out_of_order_update).is_err());
        assert_eq!(book.sequence(), Some(current_seq_id));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("52").unwrap()
        );

        let invalid = Update {
            bids: vec![vec!["51", "1"]],
            asks: vec![vec!["52.25", "0"]],
            ts: "3",
            seq: 11,
            prev_seq: 10,
        };
        assert!(book.update(&invalid).is_err());
        assert_eq!(book.sequence(), Some(current_seq_id));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("52").unwrap()
        );
    }

    #[test]
    fn ignores_stale_updates_and_snapshots() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["52.5", "3"]],
            bids: vec![vec!["52", "2"]],
            ts: "1",
            seq: 10,
            prev_seq: 0,
        })
        .unwrap();

        book.update(&Update {
            bids: vec![vec!["51", "9"]],
            asks: vec![],
            ts: "stale update",
            seq: 9,
            prev_seq: 8,
        })
        .unwrap();
        book.update_from_snapshot(&Snapshot {
            asks: vec![],
            bids: vec![],
            ts: "1",
            seq: 9,
            prev_seq: 0,
        })
        .unwrap();

        assert_eq!(book.sequence(), Some(10));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("52").unwrap()
        );
        assert_eq!(book.best_bid().unwrap().size, 2.0);
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("52.5").unwrap()
        );
    }

    #[test]
    fn accepts_a_newer_snapshot_as_a_sequence_reset() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["52.5", "3"]],
            bids: vec![vec!["52", "2"]],
            ts: "1",
            seq: 10,
            prev_seq: 0,
        })
        .unwrap();

        book.update_from_snapshot(&Snapshot {
            asks: vec![],
            bids: vec![vec!["51", "4"]],
            ts: "1",
            seq: 12,
            prev_seq: 0,
        })
        .unwrap();

        assert_eq!(book.sequence(), Some(12));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("51").unwrap()
        );
        assert_eq!(book.best_bid().unwrap().size, 4.0);
        assert_eq!(book.best_ask(), None);
    }

    #[test]

}
