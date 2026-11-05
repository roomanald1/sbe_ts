struct StreamEndLog(String);

impl Drop for StreamEndLog {
    fn drop(&mut self) {
        crate::service_log!("INFO", "{}", self.0);
    }
}

fn log_stream_end<S>(
    stream: S,
    message: String,
) -> Pin<Box<dyn Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static>>
where
    S: Stream<Item = Result<Payload, anyhow::Error>> + Send + Unpin + 'static,
{
    Box::pin(futures_util::stream::unfold(
        (stream, StreamEndLog(message)),
        |(mut stream, end_log)| async move { stream.next().await.map(|item| (item, (stream, end_log))) },
    ))
}

fn requested_signal_instrument(payload: &Payload) -> Option<&str> {
    let data = payload.data()?;
    std::str::from_utf8(data.as_ref())
        .ok()?
        .strip_prefix("signals:")
}

