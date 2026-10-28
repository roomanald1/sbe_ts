fn apply_book_side(book: &mut Vec<(f64, f64)>, rows: &[Vec<&str>]) -> Result<()> {
    for (row_index, row) in rows.iter().enumerate() {
        anyhow::ensure!(
            row.len() >= 2,
            "book row {row_index} must contain price and size"
        );
        let price = row[0]
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid price in book row {row_index}: {error}"))?;
        let size = row[1]
            .parse::<f64>()
            .map_err(|error| anyhow::anyhow!("invalid size in book row {row_index}: {error}"))?;
        anyhow::ensure!(
            price.is_finite() && price > 0.0 && size.is_finite() && size >= 0.0,
            "book row {row_index} contains an invalid price or size"
        );
        if let Some(index) = book.iter().position(|(book_price, _)| *book_price == price) {
            if size == 0.0 {
                book.remove(index);
            } else {
                book[index].1 = size;
            }
        } else if size > 0.0 {
            book.push((price, size));
        }
    }
    Ok(())
}

fn top_book_side(book: &[(f64, f64)], bids: bool) -> [(f64, f64); BOOK_DEPTH] {
    let mut top = [(0.0, 0.0); BOOK_DEPTH];
    let mut len = 0;
    for &level in book {
        let position = top[..len].iter().position(|existing| {
            if bids { level.0 > existing.0 } else { level.0 < existing.0 }
        }).unwrap_or(len);
        if position == BOOK_DEPTH { continue; }
        let new_len = (len + 1).min(BOOK_DEPTH);
        for index in (position + 1..new_len).rev() { top[index] = top[index - 1]; }
        top[position] = level;
        len = new_len;
    }
    top
}

