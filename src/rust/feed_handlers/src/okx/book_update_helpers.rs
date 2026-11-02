fn apply_book_side(book: &mut Vec<(f64, f64)>, rows: &[BookLevel<'_>], bids: bool) -> Result<()> {
    for (row_index, row) in rows.iter().enumerate() {
        let price = row
            .price
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid price in book row {row_index}: {error}"))?;
        let size = row
            .size
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid size in book row {row_index}: {error}"))?;
        anyhow::ensure!(
            price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0,
            "book row {row_index} contains an invalid price or size"
        );
        let position = book.binary_search_by(|(level_price, _)| {
            if bids {
                price.total_cmp(level_price)
            } else {
                level_price.total_cmp(&price)
            }
        });
        match position {
            Ok(index) if size == 0.0 => {
                book.remove(index);
            }
            Ok(index) => book[index].1 = size,
            Err(index) if size > 0.0 => book.insert(index, (price, size)),
            Err(_) => {}
        }
    }
    Ok(())
}

fn load_book_snapshot(book: &mut Vec<(f64, f64)>, rows: &[BookLevel<'_>]) -> Result<()> {
    book.clear();
    book.reserve(rows.len());
    for (row_index, row) in rows.iter().enumerate() {
        let price = row
            .price
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid price in book row {row_index}: {error}"))?;
        let size = row
            .size
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid size in book row {row_index}: {error}"))?;
        anyhow::ensure!(
            price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0,
            "book row {row_index} contains an invalid price or size"
        );
        if size > 0.0 {
            // OKX snapshots are unique and ordered by price on each side.
            book.push((price, size));
        }
    }
    Ok(())
}

fn top_book_side(book: &[(f64, f64)]) -> [(f64, f64); BOOK_DEPTH] {
    let mut top = [(0.0, 0.0); BOOK_DEPTH];
    for (target, &level) in top.iter_mut().zip(book.iter()) {
        *target = level;
    }
    top
}
