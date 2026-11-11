use super::*;

fn next_retry_delay(retry: Duration) -> Duration {
    (retry * 2).min(Duration::from_secs(30))
}

pub(super) async fn feed_consumer(sender: mpsc::Sender<SignalEvent>) {
    let mut retry = Duration::from_secs(1);
    loop {
        let url =
            std::env::var("INSIGHTS_RSOCKET_URL").unwrap_or_else(|_| "ws://127.0.0.1:10004".into());
        let client = RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(url.as_str()))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await;
        match client {
            Ok(client) => {
                let mut stream = client
                    .request_stream(Payload::builder().set_data_utf8("signal-events:*").build());
                crate::service_log!(
                    "INFO",
                    "Live Signals connected to Insights enriched market-event stream"
                );
                retry = Duration::from_secs(1);
                if !forward_feed_stream(&mut stream, &sender).await {
                    return;
                }
                crate::service_log!("WARN", "Live Signals feed stream disconnected");
            }
            Err(error) => crate::service_log!(
                "WARN",
                "Live Signals could not connect to Insights: {error:#}"
            ),
        }
        tokio::time::sleep(retry).await;
        retry = next_retry_delay(retry);
    }
}

async fn forward_feed_stream<S, E>(mut stream: S, sender: &mpsc::Sender<SignalEvent>) -> bool
where
    S: futures_util::Stream<Item = Result<Payload, E>> + Unpin,
    E: std::fmt::Display,
{
    while let Some(item) = stream.next().await {
        match item {
            Ok(payload) => match payload.data().cloned() {
                Some(bytes) => match decode_event(&bytes) {
                    Some(event) => {
                        if sender.send(event).await.is_err() {
                            return false;
                        }
                    }
                    None => {
                        crate::service_log!("WARN", "Live Signals rejected malformed market event")
                    }
                },
                None => crate::service_log!("WARN", "Live Signals received an empty market event"),
            },
            Err(error) => crate::service_log!(
                "WARN",
                "Live Signals received a failed market event: {error}"
            ),
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    fn price_payload() -> Payload {
        let mut bytes = vec![0; 72];
        bytes[0..2].copy_from_slice(&64_u16.to_le_bytes());
        bytes[2..4].copy_from_slice(&4_u16.to_le_bytes());
        bytes[4..6].copy_from_slice(&1_u16.to_le_bytes());
        bytes[6..8].copy_from_slice(&9_u16.to_le_bytes());
        bytes[8..16].copy_from_slice(b"BTC-USDT");
        bytes[40..48].copy_from_slice(&100.0_f64.to_le_bytes());
        bytes[48..56].copy_from_slice(&1_000_u64.to_le_bytes());
        bytes[56..64].copy_from_slice(&0.1_f64.to_le_bytes());
        Payload::new(Some(Bytes::from(bytes)), None)
    }

    #[tokio::test]
    async fn forwards_decoded_events_and_skips_malformed_or_empty_payloads() {
        let (sender, mut receiver) = mpsc::channel(4);
        let stream = futures_util::stream::iter(vec![
            Ok::<_, std::io::Error>(price_payload()),
            Ok(Payload::builder().set_data_utf8("bad").build()),
            Ok(Payload::builder().build()),
        ]);

        assert!(forward_feed_stream(stream, &sender).await);
        let event = receiver.recv().await.unwrap();
        assert_eq!(event.symbol, "BTC-USDT");
        assert!(matches!(
            event.event,
            MarketEvent::Price {
                timestamp: 1_000,
                price: 100.0,
                ..
            }
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[tokio::test]
    async fn feed_forwarding_stops_when_the_receiver_is_gone() {
        let (sender, receiver) = mpsc::channel(1);
        drop(receiver);
        let stream = futures_util::stream::iter(vec![Ok::<_, std::io::Error>(price_payload())]);

        assert!(!forward_feed_stream(stream, &sender).await);
    }

    #[test]
    fn feed_retry_delay_doubles_and_caps_at_thirty_seconds() {
        assert_eq!(
            next_retry_delay(Duration::from_secs(1)),
            Duration::from_secs(2)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(20)),
            Duration::from_secs(30)
        );
        assert_eq!(
            next_retry_delay(Duration::from_secs(30)),
            Duration::from_secs(30)
        );
    }
}
