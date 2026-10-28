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

