pub async fn subscribe(instrument_data: &[Arc<str>], price_updates: Sender<Bytes>) -> Result<()> {
    let instrument_lookup = instrument_lookup(instrument_data);
    let mut retry_delay = INITIAL_RECONNECT_DELAY;

    loop {
        let attempt_started = Instant::now();
        let result = subscribe_once(
            instrument_data,
            Arc::clone(&instrument_lookup),
            price_updates.clone(),
        )
        .await;

        retry_delay = retry_delay_after_failure(retry_delay, attempt_started.elapsed());

        match result {
            Ok(()) => crate::service_log!(
                "ERROR",
                "OKX WebSocket stream ended; reconnecting in {}s",
                retry_delay.as_secs()
            ),
            Err(error) => crate::service_log!(
                "ERROR",
                "OKX WebSocket subscription failed: {error:#}; reconnecting in {}s",
                retry_delay.as_secs()
            ),
        }

        sleep(retry_delay).await;
        retry_delay = (retry_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

fn instrument_lookup(instrument_data: &[Arc<str>]) -> Arc<HashSet<Arc<str>>> {
    Arc::new(instrument_data.iter().cloned().collect())
}

#[cfg(test)]
mod market_book_stream_tests {
    use super::instrument_lookup;
    use std::sync::Arc;

    #[test]
    fn lookup_deduplicates_instrument_ids() {
        let instruments = [
            Arc::<str>::from("BTC-USDT"),
            Arc::<str>::from("ETH-USDT"),
            Arc::<str>::from("BTC-USDT"),
        ];
        let lookup = instrument_lookup(&instruments);
        assert_eq!(lookup.len(), 2);
        assert!(lookup.contains("BTC-USDT"));
        assert!(lookup.contains("ETH-USDT"));
    }
}
