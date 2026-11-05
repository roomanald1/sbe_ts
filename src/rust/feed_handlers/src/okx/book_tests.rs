include!("book_snapshot_tests.rs");
include!("book_update_tests.rs");
include!("book_price_tests.rs");
#[test]
fn parses_decimal_prices_and_rejects_malformed_values() {
    assert_eq!(Price::from_decimal("12.34").unwrap().units(), 12_340_000_000_000_000_000);
    for invalid in ["", ".5", "-1", "1.2.3", "1e3", "1.1234567890123456789"] {
        assert!(Price::from_decimal(invalid).is_err(), "accepted {invalid:?}");
    }
    assert!(Price::from_decimal("340282366920938463463374607431768211456").is_err());
    assert!(Price::from_decimal("999999999999999999999").is_err());
}

#[test]
fn validates_and_parses_order_book_levels() {
    let tick = Price::from_decimal("0.01").unwrap();
    assert_eq!(parse_level(&["1.25", "3.5"], 0, tick, 200).unwrap(), (125, 3.5));
    assert!(parse_level(&["1.251", "1"], 0, tick, 200).is_err());
    assert!(parse_level(&["1.25"], 0, tick, 200).is_err());
    assert!(parse_level(&["1.25", "NaN"], 0, tick, 200).is_err());
    assert!(parse_level(&["0", "1"], 0, tick, 200).is_err());
    assert!(parse_level(&["1.25", "-1"], 0, tick, 200).is_err());
    assert!(parse_level(&["2.01", "1"], 0, tick, 200).is_err());

    let mut parsed = vec![(999, 999.0)];
    assert!(parse_levels(&mut parsed, &[vec!["1.25", "1"]], tick, 200, 0).is_err());
    parse_levels(&mut parsed, &[vec!["1.25", "1"]], tick, 200, 1).unwrap();
    assert_eq!(parsed, [(125, 1.0)]);
}

#[test]
fn clears_levels_and_tracks_best_bid_and_ask() {
    let mut levels = zeroed_levels(130).unwrap();
    let mut occupied = zeroed_occupancy(3).unwrap();
    let mut best_bid = None;
    apply_levels(
        &mut levels,
        &mut occupied,
        &mut best_bid,
        &[(2, 1.0), (70, 2.0)],
        Side::Bid,
    );
    assert_eq!(best_bid, Some(70));
    write_level(&mut levels, &mut occupied, &mut best_bid, 70, 0.0, Side::Bid);
    assert_eq!(best_bid, Some(2));

    let mut best_ask = None;
    write_level(&mut levels, &mut occupied, &mut best_ask, 70, 2.0, Side::Ask);
    write_level(&mut levels, &mut occupied, &mut best_ask, 2, 1.0, Side::Ask);
    assert_eq!(best_ask, Some(2));
    assert_eq!(find_best(&occupied, Side::Ask), Some(2));
    clear_levels(&mut levels, &mut occupied, &mut best_ask);
    assert_eq!(best_ask, None);
    assert!(levels.iter().all(|level| *level == 0.0));
    assert!(occupied.iter().all(|word| *word == 0));
}
