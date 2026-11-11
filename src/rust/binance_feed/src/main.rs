#[path = "../../service_docs.rs"]
mod service_docs;

use axum::{Router, routing::get};
use bytes::Bytes;
use futures_util::future::join_all;
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use serde::Serialize;
use std::{collections::BTreeMap, pin::Pin, sync::OnceLock, time::Duration};

#[derive(Serialize)]
struct DailyCandle {
    close_time_ms: u64,
    high: f64,
    close: f64,
}

static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();

async fn daily_candles(symbols: Vec<String>) -> anyhow::Result<BTreeMap<String, Vec<DailyCandle>>> {
    let pairs = [
        ("BTC-USDT", "BTCUSDT"),
        ("ETH-USDT", "ETHUSDT"),
        ("BNB-USDT", "BNBUSDT"),
        ("XRP-USDT", "XRPUSDT"),
        ("SOL-USDT", "SOLUSDT"),
        ("ADA-USDT", "ADAUSDT"),
        ("DOGE-USDT", "DOGEUSDT"),
        ("LTC-USDT", "LTCUSDT"),
        ("TRX-USDT", "TRXUSDT"),
        ("LINK-USDT", "LINKUSDT"),
        ("BCH-USDT", "BCHUSDT"),
        ("XLM-USDT", "XLMUSDT"),
        ("ETC-USDT", "ETCUSDT"),
        ("FIL-USDT", "FILUSDT"),
        ("AVAX-USDT", "AVAXUSDT"),
        ("DOT-USDT", "DOTUSDT"),
    ];
    let mut requested = Vec::new();
    for symbol in &symbols {
        let Some((_, pair)) = pairs
            .iter()
            .find(|(name, pair)| *name == symbol || *pair == symbol)
        else {
            anyhow::bail!("unsupported symbol: {symbol}");
        };
        requested.push((symbol.to_owned(), (*pair).to_owned()));
    }
    if requested.is_empty() {
        anyhow::bail!("at least one symbol is required");
    }
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .timeout(Duration::from_secs(8))
                .build()
                .unwrap_or_else(|_| reqwest::Client::new())
        })
        .clone();
    let responses = join_all(requested.into_iter().map(|(symbol, pair)| {
        let client = client.clone();
        async move {
            let url = format!(
                "https://data-api.binance.vision/api/v3/klines?symbol={pair}&interval=1d&limit=260"
            );
            let response = client
                .get(url)
                .send()
                .await
                .map_err(|error| error.to_string())?
                .error_for_status()
                .map_err(|error| error.to_string())?;
            let rows = response
                .json::<Vec<Vec<serde_json::Value>>>()
                .await
                .map_err(|error| error.to_string())?;
            Ok::<_, String>((symbol, rows))
        }
    }))
    .await;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut result = BTreeMap::new();
    for response in responses {
        let (symbol, rows) = match response {
            Ok(value) => value,
            Err(error) => anyhow::bail!("Binance daily candle request failed: {error}"),
        };
        let candles = rows
            .into_iter()
            .filter_map(|row| {
                let close_time_ms = row.get(6)?.as_u64()?;
                if close_time_ms >= now_ms {
                    return None;
                }
                let high = row.get(2)?.as_str()?.parse::<f64>().ok()?;
                let close = row.get(4)?.as_str()?.parse::<f64>().ok()?;
                (high.is_finite() && high > 0.0 && close.is_finite() && close > 0.0).then_some(
                    DailyCandle {
                        close_time_ms,
                        high,
                        close,
                    },
                )
            })
            .collect::<Vec<_>>();
        result.insert(symbol, candles);
    }
    Ok(result)
}

#[derive(serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    DailyCandles { symbols: Vec<String> },
}

struct BinanceResponder;

impl RSocket for BinanceResponder {
    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        request: Payload,
    ) -> Pin<
        Box<
            dyn futures_util::Future<Output = anyhow::Result<Option<Payload>>>
                + Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async move {
            let bytes = request
                .data()
                .ok_or_else(|| anyhow::anyhow!("missing request data"))?;
            let request: Request = serde_json::from_slice(bytes.as_ref())?;
            let response = match request {
                Request::DailyCandles { symbols } => daily_candles(symbols).await?,
            };
            Ok(Some(Payload::new(
                Some(Bytes::from(serde_json::to_vec(&response)?)),
                None,
            )))
        })
    }

    fn request_stream(
        &self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(futures_util::stream::once(async {
            Err(anyhow::anyhow!(
                "Binance Feed does not support request-stream"
            ))
        }))
    }

    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("metadata-push is unsupported")) })
    }

    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        _request: Payload,
    ) -> Pin<Box<dyn futures_util::Future<Output = anyhow::Result<()>> + Send + 'async_trait>>
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        Box::pin(async { Err(anyhow::anyhow!("fire-and-forget is unsupported")) })
    }

    fn request_channel(
        &self,
        _requests: Pin<
            Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>,
        >,
    ) -> Pin<Box<dyn futures_util::Stream<Item = anyhow::Result<Payload>> + Send + 'static>> {
        Box::pin(futures_util::stream::once(async {
            Err(anyhow::anyhow!("request-channel is unsupported"))
        }))
    }
}

async fn serve_rsocket() -> anyhow::Result<()> {
    let port = std::env::var("RSOCKET_PORT").unwrap_or_else(|_| "10002".to_owned());
    let address = format!("0.0.0.0:{port}");
    eprintln!("Binance Feed RSocket API listening on ws://{address}");
    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(address))
        .acceptor(Box::new(|_setup, _socket| Ok(Box::new(BinanceResponder))))
        .serve()
        .await?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let app = Router::new()
        .merge(service_docs::routes(
            "Binance Feed",
            &[("/health", &["get"])],
        ))
        .route("/health", get(|| async { "ok" }));
    let port = std::env::var("PORT").unwrap_or_else(|_| "3012".to_owned());
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port.parse::<u16>()?)).await?;
    eprintln!("Binance Feed health API listening on {listener:?}");
    let http = async {
        axum::serve(listener, app)
            .await
            .map_err(anyhow::Error::from)
    };
    tokio::try_join!(http, serve_rsocket())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{DailyCandle, Request};

    #[test]
    fn daily_candle_request_and_response_contract_round_trip() {
        let request: Request =
            serde_json::from_str(r#"{"type":"daily_candles","symbols":["BTCUSDT"]}"#).unwrap();
        assert!(
            matches!(request, Request::DailyCandles { symbols } if symbols == vec!["BTCUSDT".to_owned()])
        );

        let response = std::collections::BTreeMap::from([(
            "BTCUSDT",
            vec![DailyCandle {
                close_time_ms: 1_700_000_000_000,
                high: 42.0,
                close: 41.0,
            }],
        )]);
        let json = serde_json::to_value(response).unwrap();
        assert_eq!(json["BTCUSDT"][0]["close_time_ms"], 1_700_000_000_000u64);
        assert_eq!(json["BTCUSDT"][0]["high"], 42.0);
        assert_eq!(json["BTCUSDT"][0]["close"], 41.0);
    }
}
