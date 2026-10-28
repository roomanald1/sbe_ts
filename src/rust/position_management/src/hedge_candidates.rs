async fn get_hedge_candidates(
    State(store): State<Arc<PositionStore>>,
) -> Json<HedgeCandidatesResponse> {
    let positions = {
        let ledger = store.ledger.lock().await;
        cached_position_summary(&store, &ledger, None)
            .positions
            .into_iter()
            .filter(|position| position.quantity.abs() > EPSILON)
            .collect::<Vec<_>>()
    };
    for position in &positions {
        store.ensure_subscription(&position.symbol);
    }
    let histories = store.hedge_samples.lock().await.clone();
    let mut pair_correlations = Vec::new();
    for left_index in 0..positions.len() {
        for right_index in (left_index + 1)..positions.len() {
            let left = &positions[left_index];
            let right = &positions[right_index];
            let left_key = hedge_history_key(&left.symbol);
            let right_key = hedge_history_key(&right.symbol);
            let left_samples = histories.get(&left_key).map(Vec::as_slice).unwrap_or(&[]);
            let right_samples = histories.get(&right_key).map(Vec::as_slice).unwrap_or(&[]);
            let (correlation, _) = return_correlation(left_samples, right_samples);
            pair_correlations.push(HedgePairCorrelation {
                symbol_a: left.symbol.clone(),
                symbol_b: right.symbol.clone(),
                correlation,
            });
        }
    }
    let response = positions
        .into_iter()
        .map(|position| {
            let position_key = hedge_history_key(&position.symbol);
            let quote = position_key.split_once('-').map(|(_, quote)| quote);
            let position_samples = histories
                .get(&position_key)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut candidates: Vec<HedgeCandidate> = store
                .spot_symbols
                .iter()
                .filter(|symbol| {
                    symbol.as_str() != position.symbol
                        && quote.is_some_and(|quote| {
                            symbol
                                .split_once('-')
                                .is_some_and(|(_, candidate_quote)| candidate_quote == quote)
                        })
                })
                .map(|symbol| {
                    let samples = histories.get(symbol).map(Vec::as_slice).unwrap_or(&[]);
                    let (correlation, paired_samples) =
                        return_correlation(position_samples, samples);
                    HedgeCandidate {
                        symbol: symbol.clone(),
                        correlation,
                        paired_samples,
                        latest_price: samples.last().map(|(_, price)| *price),
                    }
                })
                .collect();
            candidates.sort_by(|left: &HedgeCandidate, right: &HedgeCandidate| {
                match (left.correlation, right.correlation) {
                    (Some(left), Some(right)) => left.total_cmp(&right),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => right.paired_samples.cmp(&left.paired_samples),
                }
                .then_with(|| left.symbol.cmp(&right.symbol))
            });
            candidates.truncate(10);
            HedgePosition {
                symbol: position.symbol,
                quantity: position.quantity,
                average_entry_price: position.average_entry_price,
                latest_price: position_samples.last().map(|(_, price)| *price),
                candidates,
            }
        })
        .collect();
    Json(HedgeCandidatesResponse {
        positions: response,
        pair_correlations,
    })
}

fn hedge_history_key(symbol: &str) -> String {
    let mut parts = symbol.split('-');
    match (parts.next(), parts.next()) {
        (Some(base), Some(quote)) => format!("{base}-{quote}"),
        _ => symbol.to_owned(),
    }
}
fn return_correlation(a: &[(u64, f64)], b: &[(u64, f64)]) -> (Option<f64>, usize) {
    const BUCKET_MS: u64 = 10_000;
    let bucket_prices = |samples: &[(u64, f64)]| {
        samples
            .iter()
            .filter(|(_, price)| price.is_finite() && *price > 0.0)
            .map(|(timestamp, price)| (timestamp / BUCKET_MS, *price))
            .collect::<HashMap<_, _>>()
    };
    let left = bucket_prices(a);
    let right = bucket_prices(b);
    let mut shared_buckets = left
        .keys()
        .copied()
        .filter(|bucket| right.contains_key(bucket))
        .collect::<Vec<_>>();
    shared_buckets.sort_unstable();
    let returns = shared_buckets
        .windows(2)
        .map(|buckets| {
            let previous = buckets[0];
            let current = buckets[1];
            (
                (left[&current] / left[&previous]).ln(),
                (right[&current] / right[&previous]).ln(),
            )
        })
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect::<Vec<_>>();
    let count = returns.len();
    if count < 10 {
        return (None, count);
    }
    let mean_a = returns.iter().map(|(x, _)| x).sum::<f64>() / count as f64;
    let mean_b = returns.iter().map(|(_, y)| y).sum::<f64>() / count as f64;
    let covariance = returns
        .iter()
        .map(|(x, y)| (x - mean_a) * (y - mean_b))
        .sum::<f64>();
    let variance_a = returns
        .iter()
        .map(|(x, _)| (x - mean_a).powi(2))
        .sum::<f64>();
    let variance_b = returns
        .iter()
        .map(|(_, y)| (y - mean_b).powi(2))
        .sum::<f64>();
    let correlation = if variance_a > 0.0 && variance_b > 0.0 {
        Some((covariance / (variance_a * variance_b).sqrt()).clamp(-1.0, 1.0))
    } else {
        None
    };
    (correlation.filter(|value| value.is_finite()), count)
}

