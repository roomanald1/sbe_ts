pub async fn subscribe(instrument_data: &[Arc<str>], price_updates: Sender<Bytes>) -> Result<()> {
    let instrument_lookup = Arc::new(instrument_data.iter().cloned().collect::<HashSet<_>>());
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
