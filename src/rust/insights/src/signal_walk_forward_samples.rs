fn non_overlapping_samples(
    mut samples: Vec<SignalOutcome>,
    horizon_seconds: u64,
) -> Vec<SignalOutcome> {
    samples.sort_by_key(|sample| sample.signal_timestamp);
    let mut accepted = Vec::new();
    let mut next_timestamp = 0;
    for sample in samples {
        if sample.signal_timestamp < next_timestamp { continue; }
        next_timestamp = sample.evaluated_timestamp.max(
            sample.signal_timestamp.saturating_add(horizon_seconds.saturating_mul(1000)),
        );
        let has_directional_return = sample.net_return_percent.is_some()
            && matches!(sample.direction.as_str(), "bullish" | "bearish");
        if has_directional_return { accepted.push(sample); }
    }
    accepted
}
