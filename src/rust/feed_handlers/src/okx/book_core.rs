
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

