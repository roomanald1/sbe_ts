impl SignalStore {
    pub async fn walk_forward(&self) -> Result<Vec<SignalWalkForward>> {
        let cache = self.walk_forward_cache.lock().await;
        if let Some((created, value)) = cache.as_ref() {
            // Walk-forward training scans and evaluates a large history. Keep
            // serving the last complete result and refresh at a low frequency.
            if created.elapsed() < Duration::from_secs(15 * 60) {
                return Ok(value.clone());
            }
            // Walk-forward training is deliberately expensive. Keep serving
            // the last complete result while a background refresh runs, so
            // a cache expiry cannot stall the AutoTrader's signal path.
            let stale = value.clone();
            if self
                .walk_forward_refreshing
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                let store = self.clone();
                tokio::spawn(async move {
                    let refreshed = store.compute_walk_forward().await;
                    if let Ok(value) = refreshed {
                        *store.walk_forward_cache.lock().await = Some((Instant::now(), value));
                    }
                    store
                        .walk_forward_refreshing
                        .store(false, Ordering::Release);
                });
            }
            return Ok(stale);
        }
        drop(cache);
        if self
            .walk_forward_refreshing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            let store = self.clone();
            tokio::spawn(async move {
                if let Ok(value) = store.compute_walk_forward().await {
                    *store.walk_forward_cache.lock().await = Some((Instant::now(), value));
                }
                store
                    .walk_forward_refreshing
                    .store(false, Ordering::Release);
            });
        }
        Ok(Vec::new())
    }

}
