use anyhow::{Context, Result, anyhow, ensure};

const PRICE_SCALE: u128 = 1_000_000_000_000_000_000;

pub struct Snapshot<'a> {
    pub asks: Vec<Vec<&'a str>>,
    pub bids: Vec<Vec<&'a str>>,
    pub ts: &'a str,
    pub seq: i64,
    pub prev_seq: i64,
}

pub struct Update<'a> {
    pub asks: Vec<Vec<&'a str>>,
    pub bids: Vec<Vec<&'a str>>,
    pub ts: &'a str,
    pub seq: i64,
    pub prev_seq: i64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BookLevel {
    pub price: Price,
    pub size: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Price(u128);

impl Price {
    pub const SCALE: u128 = PRICE_SCALE;

    pub const fn from_units(units: u128) -> Self {
        Self(units)
    }

    pub fn from_decimal(value: &str) -> Result<Self> {
        parse_decimal(value).map(Self)
    }

    pub fn units(self) -> u128 {
        self.0
    }

    pub fn as_f64(self) -> f64 {
        self.0 as f64 / PRICE_SCALE as f64
    }
}

#[derive(Debug, Clone, Copy)]
enum Side {
    Bid,
    Ask,
}

pub struct Book {
    tick_size: Price,
    bid_sizes: Vec<f64>,
    ask_sizes: Vec<f64>,
    bid_occupied: Vec<u64>,
    ask_occupied: Vec<u64>,
    max_levels_per_side: usize,
    parsed_bids: Vec<(usize, f64)>,
    parsed_asks: Vec<(usize, f64)>,
    best_bid: Option<usize>,
    best_ask: Option<usize>,
    seq: Option<i64>,
}

impl Book {
    // Preallocates price slots from zero to max_price.
    pub fn new(tick_size: Price, max_price: Price, max_levels_per_side: usize) -> Result<Self> {
        ensure!(tick_size.0 > 0, "tick size must be greater than zero");
        ensure!(max_price.0 > 0, "maximum price must be greater than zero");
        ensure!(
            max_levels_per_side > 0,
            "maximum levels per side must be greater than zero"
        );

        let max_tick = max_price.0 / tick_size.0;
        ensure!(
            max_tick <= usize::MAX as u128,
            "maximum price requires too many order-book slots"
        );
        let max_array_size = (max_tick as usize)
            .checked_add(1)
            .context("maximum price requires too many order-book slots")?;
        let occupancy_word_count = max_array_size.div_ceil(u64::BITS as usize);
        let parsed_bids = reserve_parsed_levels(max_levels_per_side)?;
        let parsed_asks = reserve_parsed_levels(max_levels_per_side)?;
        let bid_occupied = zeroed_occupancy(occupancy_word_count)?;
        let ask_occupied = zeroed_occupancy(occupancy_word_count)?;

        Ok(Self {
            tick_size,
            bid_sizes: zeroed_levels(max_array_size)?,
            ask_sizes: zeroed_levels(max_array_size)?,
            bid_occupied,
            ask_occupied,
            max_levels_per_side,
            parsed_bids,
            parsed_asks,
            best_bid: None,
            best_ask: None,
            seq: None,
        })
    }

    pub fn tick_size(&self) -> Price {
        self.tick_size
    }

    pub fn max_array_size(&self) -> usize {
        self.bid_sizes.len()
    }

    pub fn bid_sizes_by_tick(&self) -> &[f64] {
        &self.bid_sizes
    }

    pub fn ask_sizes_by_tick(&self) -> &[f64] {
        &self.ask_sizes
    }

    pub fn sequence(&self) -> Option<i64> {
        self.seq
    }

    pub fn bid_size_at_tick(&self, tick: usize) -> Option<f64> {
        self.bid_sizes.get(tick).copied().filter(|size| *size > 0.0)
    }

    pub fn ask_size_at_tick(&self, tick: usize) -> Option<f64> {
        self.ask_sizes.get(tick).copied().filter(|size| *size > 0.0)
    }

    pub fn set_bid_size_at_tick(&mut self, tick: usize, size: f64) -> Result<()> {
        ensure!(
            tick < self.bid_sizes.len(),
            "bid tick {tick} is outside configured order-book array"
        );
        validate_size(size)?;
        write_level(
            &mut self.bid_sizes,
            &mut self.bid_occupied,
            &mut self.best_bid,
            tick,
            size,
            Side::Bid,
        );
        Ok(())
    }

    pub fn set_ask_size_at_tick(&mut self, tick: usize, size: f64) -> Result<()> {
        ensure!(
            tick < self.ask_sizes.len(),
            "ask tick {tick} is outside configured order-book array"
        );
        validate_size(size)?;
        write_level(
            &mut self.ask_sizes,
            &mut self.ask_occupied,
            &mut self.best_ask,
            tick,
            size,
            Side::Ask,
        );
        Ok(())
    }

    pub fn best_bid(&self) -> Option<BookLevel> {
        self.best_bid.map(|tick| BookLevel {
            price: Price(tick as u128 * self.tick_size.0),
            size: self.bid_sizes[tick],
        })
    }

    pub fn best_ask(&self) -> Option<BookLevel> {
        self.best_ask.map(|tick| BookLevel {
            price: Price(tick as u128 * self.tick_size.0),
            size: self.ask_sizes[tick],
        })
    }

    pub fn update_from_snapshot(&mut self, snapshot: &Snapshot<'_>) -> Result<()> {
        if self
            .seq
            .is_some_and(|current_seq| snapshot.seq <= current_seq)
        {
            return Ok(());
        }

        parse_levels(
            &mut self.parsed_bids,
            &snapshot.bids,
            self.tick_size,
            self.bid_sizes.len(),
            self.max_levels_per_side,
        )?;
        parse_levels(
            &mut self.parsed_asks,
            &snapshot.asks,
            self.tick_size,
            self.ask_sizes.len(),
            self.max_levels_per_side,
        )?;

        clear_levels(
            &mut self.bid_sizes,
            &mut self.bid_occupied,
            &mut self.best_bid,
        );
        clear_levels(
            &mut self.ask_sizes,
            &mut self.ask_occupied,
            &mut self.best_ask,
        );
        apply_levels(
            &mut self.bid_sizes,
            &mut self.bid_occupied,
            &mut self.best_bid,
            &self.parsed_bids,
            Side::Bid,
        );
        apply_levels(
            &mut self.ask_sizes,
            &mut self.ask_occupied,
            &mut self.best_ask,
            &self.parsed_asks,
            Side::Ask,
        );
        self.seq = Some(snapshot.seq);
        Ok(())
    }

    pub fn update(&mut self, update: &Update<'_>) -> Result<()> {
        let current_seq = self
            .seq
            .context("cannot apply an incremental update before a snapshot")?;
        if update.seq <= current_seq {
            return Ok(());
        }
        ensure!(
            update.prev_seq == current_seq,
            "book sequence gap: current sequence is {current_seq}, update references {}",
            update.prev_seq
        );

        parse_levels(
            &mut self.parsed_bids,
            &update.bids,
            self.tick_size,
            self.bid_sizes.len(),
            self.max_levels_per_side,
        )?;
        parse_levels(
            &mut self.parsed_asks,
            &update.asks,
            self.tick_size,
            self.ask_sizes.len(),
            self.max_levels_per_side,
        )?;

        apply_levels(
            &mut self.bid_sizes,
            &mut self.bid_occupied,
            &mut self.best_bid,
            &self.parsed_bids,
            Side::Bid,
        );
        apply_levels(
            &mut self.ask_sizes,
            &mut self.ask_occupied,
            &mut self.best_ask,
            &self.parsed_asks,
            Side::Ask,
        );
        self.seq = Some(update.seq);
        Ok(())
    }
}

fn zeroed_levels(len: usize) -> Result<Vec<f64>> {
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(len)
        .map_err(|error| anyhow!("unable to allocate order-book array: {error}"))?;
    levels.resize(len, 0.0);
    Ok(levels)
}

fn zeroed_occupancy(len: usize) -> Result<Vec<u64>> {
    let mut occupancy = Vec::new();
    occupancy
        .try_reserve_exact(len)
        .map_err(|error| anyhow!("unable to allocate order-book occupancy map: {error}"))?;
    occupancy.resize(len, 0);
    Ok(occupancy)
}

fn reserve_parsed_levels(len: usize) -> Result<Vec<(usize, f64)>> {
    let mut levels = Vec::new();
    levels
        .try_reserve_exact(len)
        .map_err(|error| anyhow!("unable to allocate order-book parsing buffer: {error}"))?;
    Ok(levels)
}

fn parse_level(
    row: &[&str],
    row_index: usize,
    tick_size: Price,
    max_array_size: usize,
) -> Result<(usize, f64)> {
    ensure!(
        row.len() >= 2,
        "order-book row {row_index} must contain price and size"
    );
    let price = Price::from_decimal(row[0])
        .with_context(|| format!("invalid price in order-book row {row_index}"))?;
    let size = row[1]
        .parse::<f64>()
        .with_context(|| format!("invalid size in order-book row {row_index}"))?;

    ensure!(price.0 > 0, "order-book row {row_index} has invalid price");
    validate_size(size).with_context(|| format!("invalid size in order-book row {row_index}"))?;

    ensure!(
        price.0 % tick_size.0 == 0,
        "price {} is not aligned to tick size {} (scaled units)",
        row[0],
        tick_size.0
    );
    let tick = price.0 / tick_size.0;
    ensure!(
        tick < max_array_size as u128,
        "price {} is outside configured order-book array",
        row[0]
    );
    Ok((tick as usize, size))
}

fn validate_size(size: f64) -> Result<()> {
    ensure!(size.is_finite() && size >= 0.0, "invalid book size {size}");
    Ok(())
}

fn parse_levels(
    parsed: &mut Vec<(usize, f64)>,
    rows: &[Vec<&str>],
    tick_size: Price,
    max_array_size: usize,
    max_levels_per_side: usize,
) -> Result<()> {
    ensure!(
        rows.len() <= max_levels_per_side,
        "order-book side has {} levels, exceeding configured maximum {}",
        rows.len(),
        max_levels_per_side
    );
    parsed.clear();
    for (row_index, row) in rows.iter().enumerate() {
        parsed.push(parse_level(row, row_index, tick_size, max_array_size)?);
    }
    Ok(())
}

fn apply_levels(
    levels: &mut [f64],
    occupied: &mut [u64],
    best: &mut Option<usize>,
    updates: &[(usize, f64)],
    side: Side,
) {
    for &(tick, size) in updates {
        write_level(levels, occupied, best, tick, size, side);
    }
}

fn clear_levels(levels: &mut [f64], occupied: &mut [u64], best: &mut Option<usize>) {
    for (word_index, word) in occupied.iter_mut().enumerate() {
        let mut active_ticks = *word;
        while active_ticks != 0 {
            let bit = active_ticks.trailing_zeros() as usize;
            levels[word_index * u64::BITS as usize + bit] = 0.0;
            active_ticks &= active_ticks - 1;
        }
        *word = 0;
    }
    *best = None;
}

fn write_level(
    levels: &mut [f64],
    occupied: &mut [u64],
    best: &mut Option<usize>,
    tick: usize,
    size: f64,
    side: Side,
) {
    levels[tick] = size;
    let word = &mut occupied[tick / u64::BITS as usize];
    let mask = 1_u64 << (tick % u64::BITS as usize);
    if size > 0.0 {
        *word |= mask;
    } else {
        *word &= !mask;
    }

    match (side, size > 0.0) {
        (Side::Bid, true) if best.is_none_or(|best_tick| tick > best_tick) => {
            *best = Some(tick);
        }
        (Side::Ask, true) if best.is_none_or(|best_tick| tick < best_tick) => {
            *best = Some(tick);
        }
        (_, false) if *best == Some(tick) => {
            *best = find_best(occupied, side);
        }
        _ => {}
    }
}

fn find_best(occupied: &[u64], side: Side) -> Option<usize> {
    match side {
        Side::Bid => occupied
            .iter()
            .enumerate()
            .rev()
            .find_map(|(word_index, &word)| {
                (word != 0).then(|| {
                    word_index * u64::BITS as usize
                        + (u64::BITS as usize - 1 - word.leading_zeros() as usize)
                })
            }),
        Side::Ask => occupied.iter().enumerate().find_map(|(word_index, &word)| {
            (word != 0).then(|| word_index * u64::BITS as usize + word.trailing_zeros() as usize)
        }),
    }
}

fn parse_decimal(value: &str) -> Result<u128> {
    ensure!(!value.is_empty(), "decimal value cannot be empty");
    let mut whole = 0_u128;
    let mut fraction = 0_u128;
    let mut whole_digits = 0;
    let mut fraction_digits = 0;
    let mut decimal_seen = false;

    for byte in value.bytes() {
        if byte == b'.' && !decimal_seen {
            decimal_seen = true;
            continue;
        }

        ensure!(byte.is_ascii_digit(), "invalid decimal value {value}");
        let digit = u128::from(byte - b'0');
        if decimal_seen {
            fraction_digits += 1;
            ensure!(
                fraction_digits <= 18,
                "decimal value {value} has more than 18 fractional digits"
            );
            fraction = fraction
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| anyhow!("decimal value {value} is too large"))?;
        } else {
            whole_digits += 1;
            whole = whole
                .checked_mul(10)
                .and_then(|value| value.checked_add(digit))
                .ok_or_else(|| anyhow!("decimal value {value} is too large"))?;
        }
    }

    ensure!(whole_digits > 0, "invalid decimal value {value}");
    for _ in fraction_digits..18 {
        fraction *= 10;
    }
    whole
        .checked_mul(PRICE_SCALE)
        .and_then(|whole_units| whole_units.checked_add(fraction))
        .ok_or_else(|| anyhow!("decimal value {value} is too large"))
}

#[cfg(test)]
mod tests {
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
