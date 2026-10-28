#[cfg(test)]
mod book_snapshot_tests {
    use super::{Book, Price, Snapshot, Update};

    #[test]
    fn snapshot_directly_indexes_levels_by_tick() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            400,
        )
        .unwrap();
        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["53", "4"], vec!["52.5", "5"]],
            bids: vec![vec!["51", "2"], vec!["52", "3"], vec!["51.5", "1"]],
            ts: "1",
            seq: 10,
            prev_seq: 0,
        })
        .unwrap();

        assert_eq!(book.max_array_size(), 107);
        assert_eq!(book.bid_size_at_tick(102), Some(2.0));
        assert_eq!(book.bid_size_at_tick(103), Some(1.0));
        assert_eq!(book.bid_size_at_tick(104), Some(3.0));
        assert_eq!(book.ask_size_at_tick(105), Some(5.0));
        assert_eq!(book.ask_size_at_tick(106), Some(4.0));
        assert_eq!(
            book.best_bid(),
            Some(super::BookLevel {
                price: Price::from_decimal("52").unwrap(),
                size: 3.0
            })
        );
        assert_eq!(
            book.best_ask(),
            Some(super::BookLevel {
                price: Price::from_decimal("52.5").unwrap(),
                size: 5.0
            })
        );
    }

    #[test]
    fn direct_tick_writes_update_values_and_best_prices() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("53").unwrap(),
            4,
        )
        .unwrap();
        book.set_bid_size_at_tick(104, 2.0).unwrap();
        book.set_ask_size_at_tick(105, 3.0).unwrap();

        assert_eq!(book.bid_size_at_tick(104), Some(2.0));
        assert_eq!(book.ask_size_at_tick(105), Some(3.0));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("52").unwrap()
        );
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("52.5").unwrap()
        );

        book.set_bid_size_at_tick(104, 0.0).unwrap();
        book.set_ask_size_at_tick(105, 0.0).unwrap();
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), None);
        assert!(book.set_bid_size_at_tick(107, 1.0).is_err());
        assert!(book.set_ask_size_at_tick(105, f64::NAN).is_err());
    }

    #[test]
    fn deleting_best_levels_finds_next_occupied_tick_across_words() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("1000").unwrap(),
            4,
        )
        .unwrap();
        for tick in [1, 65, 130, 2000] {
            book.set_bid_size_at_tick(tick, 1.0).unwrap();
        }
        for tick in [1, 65, 130, 2000] {
            book.set_ask_size_at_tick(tick, 1.0).unwrap();
        }

        book.set_bid_size_at_tick(2000, 0.0).unwrap();
        book.set_ask_size_at_tick(1, 0.0).unwrap();
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("65").unwrap()
        );
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("32.5").unwrap()
        );

        book.set_bid_size_at_tick(130, 0.0).unwrap();
        book.set_ask_size_at_tick(65, 0.0).unwrap();
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("32.5").unwrap()
        );
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("65").unwrap()
        );
    }

    #[test]
    fn sparse_snapshot_clears_old_levels_and_occupancy() {
        let mut book = Book::new(
            Price::from_decimal("0.5").unwrap(),
            Price::from_decimal("1000").unwrap(),
            4,
        )
        .unwrap();
        book.set_bid_size_at_tick(2000, 1.0).unwrap();
        book.set_bid_size_at_tick(130, 2.0).unwrap();
        book.set_ask_size_at_tick(1, 3.0).unwrap();
        book.set_ask_size_at_tick(2000, 4.0).unwrap();

        book.update_from_snapshot(&Snapshot {
            asks: vec![vec!["32.5", "5"]],
            bids: vec![vec!["999.5", "6"]],
            ts: "1",
            seq: 1,
            prev_seq: 0,
        })
        .unwrap();

        assert_eq!(book.bid_size_at_tick(130), None);
        assert_eq!(book.ask_size_at_tick(1), None);
        assert_eq!(book.bid_size_at_tick(1999), Some(6.0));
        assert_eq!(book.ask_size_at_tick(65), Some(5.0));
        assert_eq!(
            book.best_bid().unwrap().price,
            Price::from_decimal("999.5").unwrap()
        );
        assert_eq!(
            book.best_ask().unwrap().price,
            Price::from_decimal("32.5").unwrap()
        );

        book.set_bid_size_at_tick(1999, 0.0).unwrap();
        book.set_ask_size_at_tick(65, 0.0).unwrap();
        assert_eq!(book.best_bid(), None);
        assert_eq!(book.best_ask(), None);
    }


}
