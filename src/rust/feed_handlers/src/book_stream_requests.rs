fn requested_book_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    let request = std::str::from_utf8(data.as_ref()).ok()?;
    request.strip_prefix("book:")
}

fn book_request_stream(
    instrument: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>> {
    let subscription = subscribe_shared_books(&instrument);
    Box::pin(futures_util::stream::unfold(
        subscription,
        |mut subscription| async move {
            loop {
                match subscription.recv().await {
                    Ok(bytes) => {
                        return Some((Ok(Payload::new(Some(bytes), None)), subscription));
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return None,
                }
            }
        },
    ))
}
