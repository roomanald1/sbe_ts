use anyhow::{Context, bail};
use futures_util::{Stream, stream};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::HashMap, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::watch;

use super::{SignalStore, get_position_insights, signal_ui_snapshot};

#[derive(Deserialize)]
struct BrowserRequest {
    method: String,
    path: String,
    body: Option<Value>,
}

#[derive(Serialize)]
struct BrowserResponse {
    status: u16,
    body: Value,
}

#[derive(Clone)]
struct InsightsBrowserResponder {
    store: SignalStore,
    news: watch::Receiver<Arc<Value>>,
}

impl InsightsBrowserResponder {
    async fn respond(&self, request: BrowserRequest) -> anyhow::Result<BrowserResponse> {
        let (path, query) = split_path_query(&request.path)?;
        let method = request.method.to_ascii_uppercase();
        let result = match (method.as_str(), path.as_str()) {
            ("GET", "/positions/insights") => {
                let response = get_position_insights()
                    .await
                    .map_err(|(_, message)| anyhow::anyhow!(message))?;
                serde_json::to_value(response.0)?
            }
            ("GET", "/signals/dashboard") => serde_json::to_value(self.store.dashboard().await?)?,
            ("GET", "/signals/history") => serde_json::to_value(self.store.recent(200).await?)?,
            ("GET", "/signals/performance") => {
                serde_json::to_value(self.store.performance().await?)?
            }
            ("GET", "/signals/auto-trader-dashboard") => {
                serde_json::to_value(self.store.auto_trader_dashboard().await?)?
            }
            ("GET", "/signals/auto-trader-decision-quality") => serde_json::to_value(
                self.store
                    .auto_trader_decision_quality(number(&query, "since"))
                    .await?,
            )?,
            ("GET", "/signals/auto-trader-execution-quality") => serde_json::to_value(
                self.store
                    .auto_trader_execution_quality(number(&query, "since"))
                    .await?,
            )?,
            ("GET", "/signals/auto-trader-execution-audit") => serde_json::to_value(
                self.store
                    .load_auto_trader_execution_audit_page(
                        signed_number(&query, "cursor"),
                        number(&query, "since"),
                        usize_number(&query, "limit").unwrap_or(500),
                    )
                    .await?,
            )?,
            ("GET", "/signals/auto-trader-audit") => serde_json::to_value(
                self.store
                    .load_auto_trader_audit_page(
                        number(&query, "since"),
                        signed_number(&query, "cursor"),
                        usize_number(&query, "limit").unwrap_or(10_000),
                        query.get("stage").cloned(),
                    )
                    .await?,
            )?,
            ("POST", "/signals/auto-trader-audit") => {
                self.store
                    .save_auto_trader_audit(request.body.context("audit request body is required")?)
                    .await?;
                return Ok(BrowserResponse {
                    status: 204,
                    body: Value::Null,
                });
            }
            ("GET", "/signals/best") => serde_json::to_value(self.store.best(10).await?)?,
            ("GET", "/signals/evaluations") => {
                let horizons = query.get("horizons").map(|value| {
                    value
                        .split(',')
                        .filter_map(|item| item.parse::<u64>().ok())
                        .collect::<Vec<_>>()
                });
                serde_json::to_value(
                    self.store
                        .evaluations(
                            number(&query, "since"),
                            usize_number(&query, "limit")
                                .unwrap_or(10_000)
                                .clamp(1, 150_000),
                            horizons.as_deref(),
                        )
                        .await?,
                )?
            }
            ("GET", "/signals/book-snapshots") => serde_json::to_value(
                self.store
                    .book_snapshots(
                        number(&query, "since"),
                        usize_number(&query, "limit")
                            .unwrap_or(10_000)
                            .clamp(1, 50_000),
                    )
                    .await?,
            )?,
            ("GET", "/signals/research-features") => serde_json::to_value(
                self.store
                    .research_features(
                        number(&query, "since"),
                        usize_number(&query, "limit")
                            .unwrap_or(10_000)
                            .clamp(1, 50_000),
                    )
                    .await?,
            )?,
            ("GET", "/signals/external-market") => serde_json::to_value(
                self.store
                    .external_market_features(
                        number(&query, "since"),
                        usize_number(&query, "limit")
                            .unwrap_or(10_000)
                            .clamp(1, 50_000),
                    )
                    .await?,
            )?,
            _ => bail!("unsupported Insights browser request: {method} {path}"),
        };
        Ok(BrowserResponse {
            status: 200,
            body: result,
        })
    }
}

fn news_snapshot_stream(
    news: watch::Receiver<Arc<Value>>,
) -> impl Stream<Item = anyhow::Result<Payload>> + Send + 'static {
    stream::unfold((news, true), |(mut news, initial)| async move {
        if !initial && news.changed().await.is_err() {
            return None;
        }
        let snapshot = news.borrow_and_update().clone();
        let payload = Payload::builder()
            .set_data(serde_json::to_vec(snapshot.as_ref()).unwrap_or_default())
            .build();
        Some((Ok(payload), (news, false)))
    })
}

#[rsocket_rust::async_trait]
impl RSocket for InsightsBrowserResponder {
    async fn metadata_push(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Insights browser endpoint accepts request-response and request-stream")
    }

    async fn fire_and_forget(&self, _req: Payload) -> anyhow::Result<()> {
        bail!("Insights browser mutations require a response")
    }

    async fn request_response(&self, req: Payload) -> anyhow::Result<Option<Payload>> {
        let data = req.data().context("Insights browser request is empty")?;
        let request: BrowserRequest =
            serde_json::from_slice(data).context("invalid Insights browser request")?;
        let response = self.respond(request).await?;
        Ok(Some(
            Payload::builder()
                .set_data(serde_json::to_vec(&response)?)
                .build(),
        ))
    }

    fn request_stream(
        &self,
        req: Payload,
    ) -> Pin<Box<dyn Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        let route = req
            .data()
            .and_then(|data| serde_json::from_slice::<Value>(data).ok())
            .and_then(|value| {
                value
                    .get("route")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        if route.as_deref() == Some("news.snapshot") {
            return Box::pin(news_snapshot_stream(self.news.clone()));
        }
        if route.as_deref() != Some("ui.snapshot") {
            return Box::pin(stream::once(async {
                Err(anyhow::anyhow!("unsupported Insights browser stream"))
            }));
        }
        let store = self.store.clone();
        Box::pin(stream::unfold(
            (store, tokio::time::interval(Duration::from_secs(15)), true),
            |(store, mut interval, initial)| async move {
                if !initial {
                    interval.tick().await;
                }
                let state = signal_ui_snapshot(&store).await;
                let message = serde_json::json!({"type":"snapshot","state":state});
                let payload = Payload::builder()
                    .set_data(serde_json::to_vec(&message).unwrap_or_default())
                    .build();
                Some((Ok(payload), (store, interval, false)))
            },
        ))
    }

    fn request_channel(
        &self,
        _requests: Flux<anyhow::Result<Payload>>,
    ) -> Flux<anyhow::Result<Payload>> {
        Box::pin(stream::once(async {
            Err(anyhow::anyhow!(
                "Insights browser request-channel is unsupported"
            ))
        }))
    }
}

#[cfg(test)]
mod news_snapshot_stream_tests {
    use super::news_snapshot_stream;
    use futures_util::StreamExt;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::watch;

    #[tokio::test]
    async fn emits_current_snapshot_then_each_changed_snapshot() {
        let initial = Arc::new(json!({"updated_at_ms": 1, "articles": []}));
        let (sender, receiver) = watch::channel(initial);
        let mut stream = Box::pin(news_snapshot_stream(receiver));
        let first = stream.next().await.unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(first.data().unwrap()).unwrap()["updated_at_ms"],
            1
        );

        sender.send_replace(Arc::new(json!({"updated_at_ms": 2, "articles": []})));
        let second = stream.next().await.unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(second.data().unwrap()).unwrap()["updated_at_ms"],
            2
        );
    }
}

pub async fn serve_insights_browser_rsocket(
    store: SignalStore,
    news: watch::Receiver<Arc<Value>>,
) -> anyhow::Result<()> {
    let port =
        std::env::var("INSIGHTS_BROWSER_RSOCKET_PORT").unwrap_or_else(|_| "10011".to_owned());
    let address = format!("0.0.0.0:{port}");
    eprintln!("Insights browser RSocket listening on ws://{address}");
    let responder = InsightsBrowserResponder { store, news };
    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address))
        .acceptor(Box::new(move |_setup, _socket| {
            Ok(Box::new(responder.clone()))
        }))
        .serve()
        .await?;
    Ok(())
}

fn split_path_query(path: &str) -> anyhow::Result<(String, HashMap<String, String>)> {
    let uri = path
        .parse::<axum::http::Uri>()
        .context("invalid request path")?;
    let mut query = HashMap::new();
    if let Some(raw) = uri.query() {
        for part in raw.split('&').filter(|part| !part.is_empty()) {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            query.insert(decode_query_component(key)?, decode_query_component(value)?);
        }
    }
    Ok((uri.path().to_owned(), query))
}

fn decode_query_component(value: &str) -> anyhow::Result<String> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => decoded.push(b' '),
            b'%' if index + 2 < bytes.len() => {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                let (Some(high), Some(low)) = (high, low) else {
                    bail!("invalid percent-encoding in query string");
                };
                decoded.push(((high << 4) | low) as u8);
                index += 2;
            }
            byte => decoded.push(byte),
        }
        index += 1;
    }
    Ok(String::from_utf8(decoded).context("query string is not UTF-8")?)
}

fn number(query: &HashMap<String, String>, key: &str) -> Option<u64> {
    query.get(key)?.parse().ok()
}

fn signed_number(query: &HashMap<String, String>, key: &str) -> Option<i64> {
    query.get(key)?.parse().ok()
}

fn usize_number(query: &HashMap<String, String>, key: &str) -> Option<usize> {
    query.get(key)?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::{decode_query_component, split_path_query};

    #[test]
    fn parses_and_decodes_browser_query_parameters() {
        let (path, query) =
            split_path_query("/signals/auto-trader-audit?stage=strategy%3Aevaluation&limit=1000")
                .unwrap();
        assert_eq!(path, "/signals/auto-trader-audit");
        assert_eq!(
            query.get("stage").map(String::as_str),
            Some("strategy:evaluation")
        );
        assert_eq!(query.get("limit").map(String::as_str), Some("1000"));
        assert_eq!(decode_query_component("a+b").unwrap(), "a b");
    }

    #[test]
    fn rejects_malformed_query_encoding() {
        assert!(decode_query_component("%XZ").is_err());
        assert!(decode_query_component("%E0%A4").is_err());
    }
}
