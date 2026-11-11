use anyhow::Context;
use http_body_util::BodyExt;
use tower::ServiceExt;

struct InsightsMarketEventResponder {
    events: broadcast::Sender<Bytes>,
    scout_router: Option<Router>,
}

use sbe_schema::{
    WriteBuf, enriched_symbol_price_codec::EnrichedSymbolPriceEncoder,
    enriched_symbol_price_codec::SBE_BLOCK_LENGTH as ENRICHED_PRICE_BLOCK_LENGTH,
    message_header_codec,
};

fn enrich_raw_price(raw: &Bytes, volatility_1m: f64, volatility_5m: f64) -> Option<Bytes> {
    if raw.len() < 56
        || u16::from_le_bytes(raw.get(0..2)?.try_into().ok()?) != 48
        || u16::from_le_bytes(raw.get(2..4)?.try_into().ok()?) != 1
        || u16::from_le_bytes(raw.get(4..6)?.try_into().ok()?) != 1
        || u16::from_le_bytes(raw.get(6..8)?.try_into().ok()?) != 9
    {
        return None;
    }
    let symbol = raw.get(8..40)?.try_into().ok()?;
    let price = f64::from_le_bytes(raw.get(40..48)?.try_into().ok()?);
    let timestamp = u64::from_le_bytes(raw.get(48..56)?.try_into().ok()?);
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut enriched = vec![0; header_length + ENRICHED_PRICE_BLOCK_LENGTH as usize];
    let mut encoder = EnrichedSymbolPriceEncoder::default()
        .wrap(WriteBuf::new(&mut enriched), header_length);
    encoder.symbol(symbol);
    encoder.price(price);
    encoder.timestamp(timestamp);
    encoder.volatility_1m(volatility_1m);
    encoder.volatility_5m(volatility_5m);
    drop(encoder.header(0));
    Some(Bytes::from(enriched))
}

impl RSocket for InsightsMarketEventResponder {
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket metadata push is unsupported")) })
    }

    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("RSocket fire-and-forget is unsupported")) })
    }

    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        request: Payload,
    ) -> Pin<
        Box<
            dyn futures_util::Future<Output = anyhow::Result<Option<Payload>>> + Send + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            let router = self
                .scout_router
                .as_ref()
                .context("Insights Scout RSocket route is not configured")?;
            let bytes = request.data().context("empty Scout service route request")?;
            let request: strategies::ScoutCoordinatorRequest =
                serde_json::from_slice(bytes).context("invalid Scout service route request")?;
            let strategies::ScoutCoordinatorRequest::WorkerRoute(command) = request else {
                anyhow::bail!("Insights accepts Scout service route operations only");
            };
            let path = command.path.split('?').next().unwrap_or(&command.path);
            let method = match command.method {
                strategies::ScoutWorkerHttpMethod::Get => http::Method::GET,
                strategies::ScoutWorkerHttpMethod::Post => http::Method::POST,
                strategies::ScoutWorkerHttpMethod::Put => http::Method::PUT,
                strategies::ScoutWorkerHttpMethod::Delete => http::Method::DELETE,
            };
            let allowed = match (method.clone(), path) {
                (http::Method::GET, "/internal/live-context") => true,
                (http::Method::GET, "/internal/live-news-context") => true,
                (http::Method::POST, "/internal/scouts/register")
                | (http::Method::POST, "/internal/signals/counterfactual")
                | (http::Method::POST, "/internal/signals/scout-strategy-status") => true,
                _ => false,
            };
            if !allowed {
                anyhow::bail!("unsupported Insights Scout RSocket route");
            }
            let path_and_query = command.path;
            let body = if method == http::Method::GET || method == http::Method::DELETE {
                Vec::new()
            } else {
                serde_json::to_vec(&command.payload)?
            };
            let response = router
                .clone()
                .oneshot(
                    http::Request::builder()
                        .method(method)
                        .uri(path_and_query)
                        .header(http::header::CONTENT_TYPE, "application/json")
                        .body(axum::body::Body::from(body))?,
                )
                .await?;
            let status = response.status().as_u16();
            let body = response.into_body().collect().await?.to_bytes();
            let body = serde_json::from_slice(&body).unwrap_or_else(|_| {
                serde_json::Value::String(String::from_utf8_lossy(&body).into())
            });
            let result = strategies::ScoutCoordinatorResponse::WorkerRoute(
                strategies::ScoutCoordinatorRouteResult {
                    request_id: command.request_id,
                    status,
                    body,
                },
            );
            Ok(Some(
                Payload::builder()
                    .set_data(serde_json::to_vec(&result)?)
                    .build(),
            ))
        })
    }

    fn request_stream(
        &self,
        payload: Payload,
    ) -> Pin<
        Box<
            dyn futures_util::Stream<Item = Result<Payload, anyhow::Error>> + Send + 'static,
        >,
    > {
        let request = payload
            .data()
            .and_then(|data| std::str::from_utf8(data.as_ref()).ok());
        let only_prices = match request {
            Some("signal-events:*") => false,
            Some("tickers:*") => true,
            _ => {
                return Box::pin(futures_util::stream::once(async {
                    Err(anyhow::anyhow!("unsupported Insights RSocket stream request"))
                }));
            }
        };
        let events = self.events.subscribe();
        Box::pin(futures_util::stream::unfold(
            (events, only_prices),
            |(mut events, only_prices)| async move {
                loop {
                    match events.recv().await {
                        Ok(data) => {
                            let is_price = data.get(2..4).is_some_and(|template| {
                                u16::from_le_bytes([template[0], template[1]]) == 4
                            });
                            if only_prices && !is_price {
                                continue;
                            }
                            break Some((Ok(Payload::new(Some(data), None)), (events, only_prices)));
                        }
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            crate::service_log!("WARN", "Insights market-event subscriber lagged; skipped {skipped} events");
                        }
                        Err(broadcast::error::RecvError::Closed) => break None,
                    }
                }
            },
        ))
    }

    fn request_channel(&self, _requests: Flux<Result<Payload>>) -> Flux<Result<Payload>> {
        futures_util::stream::once(async {
            Err(anyhow::anyhow!("RSocket request-channel is unsupported"))
        })
        .boxed()
    }
}

async fn run_insights_market_event_server(events: broadcast::Sender<Bytes>) -> anyhow::Result<()> {
    let port = std::env::var("INSIGHTS_RSOCKET_PORT").unwrap_or_else(|_| "10004".to_owned());
    let address = format!("0.0.0.0:{port}");
    crate::service_log!("INFO", "Insights RSocket market-event server listening on ws://{address}");
    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address))
        .acceptor(Box::new(move |setup, _socket| {
            crate::service_log!("INFO", "Insights RSocket client connected: setup={setup:?}");
            Ok(Box::new(InsightsMarketEventResponder {
                events: events.clone(),
                scout_router: None,
            }))
        }))
        .serve()
        .await?;
    Ok(())
}

/// Starts the service-owned RSocket request-response endpoint for durable
/// Scout registration callbacks. `router` is the already configured local
/// Insights router; handlers run in-process and no HTTP network call is used.
pub async fn run_insights_scout_rpc_server(
    router: Router,
    address: String,
) -> anyhow::Result<()> {
    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address.clone()))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(InsightsMarketEventResponder {
                events: broadcast::channel(1).0,
                scout_router: Some(router.clone()),
            }))
        }))
        .serve()
        .await
        .map_err(anyhow::Error::from)?;
    Ok(())
}

#[cfg(test)]
mod market_event_rsocket_tests {
    use super::enrich_raw_price;
    use super::run_insights_scout_rpc_server;
    use axum::{Json, Router, routing::post};
    use bytes::Bytes;
    use sbe_schema::{
        ReadBuf, enriched_symbol_price_codec::EnrichedSymbolPriceDecoder,
        message_header_codec::MessageHeaderDecoder,
    };

    #[test]
    fn enriched_price_uses_its_typed_sbe_template_and_preserves_values() {
        let mut raw = vec![0; 56];
        raw[0..2].copy_from_slice(&48_u16.to_le_bytes());
        raw[2..4].copy_from_slice(&1_u16.to_le_bytes());
        raw[4..6].copy_from_slice(&1_u16.to_le_bytes());
        raw[6..8].copy_from_slice(&9_u16.to_le_bytes());
        raw[8..16].copy_from_slice(b"BTC-USDT");
        raw[40..48].copy_from_slice(&100.5_f64.to_le_bytes());
        raw[48..56].copy_from_slice(&1234_u64.to_le_bytes());
        let enriched = enrich_raw_price(&Bytes::from(raw), 0.2, 0.4).unwrap();
        assert_eq!(enriched.len(), 72);
        assert_eq!(u16::from_le_bytes(enriched[0..2].try_into().unwrap()), 64);
        assert_eq!(u16::from_le_bytes(enriched[2..4].try_into().unwrap()), 4);
        assert_eq!(u16::from_le_bytes(enriched[4..6].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(enriched[6..8].try_into().unwrap()), 9);
        let header = MessageHeaderDecoder::default().wrap(ReadBuf::new(&enriched), 0);
        let decoded = EnrichedSymbolPriceDecoder::default().header(header);
        assert_eq!(&decoded.symbol()[..8], b"BTC-USDT");
        assert_eq!(decoded.price(), 100.5);
        assert_eq!(decoded.timestamp(), 1234);
        assert_eq!(decoded.volatility_1m(), 0.2);
        assert_eq!(decoded.volatility_5m(), 0.4);
    }

    #[test]
    fn rejects_non_price_or_truncated_payloads() {
        assert!(enrich_raw_price(&Bytes::from(vec![0; 55]), 0.2, 0.4).is_none());
        let mut book = vec![0; 56];
        book[0..2].copy_from_slice(&48_u16.to_le_bytes());
        book[2..4].copy_from_slice(&2_u16.to_le_bytes());
        book[4..6].copy_from_slice(&1_u16.to_le_bytes());
        book[6..8].copy_from_slice(&9_u16.to_le_bytes());
        assert!(enrich_raw_price(&Bytes::from(book), 0.2, 0.4).is_none());
    }

    #[tokio::test]
    async fn scout_registration_route_is_available_over_rsocket() {
        let app = Router::new().route(
            "/internal/scouts/register",
            post(|Json(payload): Json<serde_json::Value>| async move {
                Json(serde_json::json!({"registered": payload["decision_id"]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let server = tokio::spawn(run_insights_scout_rpc_server(app, address.to_string()));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let response = strategies::request_scout_service_route(
            &format!("ws://{address}"),
            &strategies::ScoutCoordinatorRouteCommand {
                request_id: "registration-1".into(),
                strategy: None,
                method: strategies::ScoutWorkerHttpMethod::Post,
                path: "/internal/scouts/register".into(),
                payload: serde_json::json!({"decision_id":"decision-1"}),
            },
        )
        .await
        .unwrap();
        assert_eq!(response.status, 200);
        assert_eq!(response.body["registered"], "decision-1");
        server.abort();
    }
}
