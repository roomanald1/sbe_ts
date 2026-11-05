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

#[cfg(test)]
mod book_update_helper_tests {
    use super::{BOOK_DEPTH, BookLevel, apply_book_side, load_book_snapshot, top_book_side};

    #[test]
    fn applies_sorted_bid_and_ask_changes_and_removes_zero_sized_levels() {
        let mut bids = vec![(99.0, 1.0), (97.0, 1.0)];
        apply_book_side(
            &mut bids,
            &[BookLevel { price: "98", size: "2" }, BookLevel { price: "99", size: "0" }],
            true,
        )
        .unwrap();
        assert_eq!(bids, [(98.0, 2.0), (97.0, 1.0)]);

        let mut asks = vec![(101.0, 1.0), (103.0, 1.0)];
        apply_book_side(
            &mut asks,
            &[BookLevel { price: "102", size: "3" }, BookLevel { price: "101", size: "4" }],
            false,
        )
        .unwrap();
        assert_eq!(asks, [(101.0, 4.0), (102.0, 3.0), (103.0, 1.0)]);
    }

    #[test]
    fn rejects_invalid_updates_and_snapshot_loads_only_positive_sizes() {
        let mut book = vec![(99.0, 1.0)];
        assert!(apply_book_side(&mut book, &[BookLevel { price: "bad", size: "1" }], true).is_err());
        assert!(apply_book_side(&mut book, &[BookLevel { price: "100", size: "NaN" }], true).is_err());

        load_book_snapshot(
            &mut book,
            &[BookLevel { price: "101", size: "0" }, BookLevel { price: "102", size: "2" }],
        )
        .unwrap();
        assert_eq!(book, [(102.0, 2.0)]);
        assert!(load_book_snapshot(&mut book, &[BookLevel { price: "NaN", size: "1" }]).is_err());
        load_book_snapshot(&mut book, &[BookLevel { price: "102", size: "2" }]).unwrap();

        let top = top_book_side(&book);
        assert_eq!(top[0], (102.0, 2.0));
        assert_eq!(top[1..], [(0.0, 0.0); BOOK_DEPTH - 1]);
    }
}
