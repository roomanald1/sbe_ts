fn requested_book_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    let request = std::str::from_utf8(data.as_ref()).ok()?;
    request.strip_prefix("book:")
}

fn book_request_stream(
    instrument: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
    let feeds = BOOK_FEEDS.get_or_init(|| StdMutex::new(HashMap::new()));
    let sender = {
        let mut feeds = feeds.lock().expect("book feed map poisoned");
        feeds
            .entry(instrument.clone())
            .or_insert_with(|| {
                let (output, _) = broadcast::channel(BOOK_UPDATE_CAPACITY);
                let task_output = output.clone();
                tokio::spawn(async move {
                    let mut delay = Duration::from_secs(1);
                    loop {
                        let (tx, mut rx) = mpsc::channel(BOOK_UPDATE_CAPACITY);
                        let sender = task_output.clone();
                        let forward = tokio::spawn(async move {
                            while let Some(Ok(bytes)) = rx.recv().await {
                                let _ = sender.send(bytes);
                            }
                        });
                        let attempt_started = std::time::Instant::now();
                        match subscribe_book_once(&instrument, tx).await {
                            Ok(()) => break,
                            Err(error) => {
                                if attempt_started.elapsed() >= Duration::from_secs(60) {
                                    delay = Duration::from_secs(1);
                                }
                                crate::service_log!("ERROR", 
                                    "Shared book subscription for {instrument} failed: {error:#}; retrying in {}s",
                                    delay.as_secs()
                                );
                                forward.abort();
                                sleep(delay).await;
                                delay = (delay * 2).min(Duration::from_secs(30));
                            }
                        }
                    }
                });
                output
            })
            .clone()
    };
    Box::pin(futures_util::stream::unfold(
        sender.subscribe(),
        |mut rx| async move {
            loop {
                match rx.recv().await {
                    Ok(bytes) => break Some((Ok(Payload::new(Some(bytes), None)), rx)),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break None,
                }
            }
        },
    ))
}

