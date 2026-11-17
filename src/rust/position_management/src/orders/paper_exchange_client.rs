use anyhow::Context;
use rsocket_rust::{Client, prelude::*, utils::EchoRSocket};
use rsocket_rust_transport_websocket::WebsocketClientTransport;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::{Arc, Mutex as StdMutex, OnceLock};

#[derive(Serialize)]
struct PaperExchangeRequest {
    method: String,
    path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    body: Option<Value>,
}

#[derive(Deserialize)]
struct PaperExchangeResponse {
    status: u16,
    body: Value,
}

fn paper_exchange_client() -> &'static StdMutex<Option<Arc<Client>>> {
    static CLIENT: OnceLock<StdMutex<Option<Arc<Client>>>> = OnceLock::new();
    CLIENT.get_or_init(|| StdMutex::new(None))
}

async fn get_paper_exchange_client() -> anyhow::Result<Arc<Client>> {
    if let Some(client) = paper_exchange_client()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
    {
        return Ok(client);
    }
    let endpoint = std::env::var("PAPER_EXCHANGE_RSOCKET_URL")
        .unwrap_or_else(|_| "ws://127.0.0.1:10006".to_owned());
    let connected = Arc::new(
        RSocketFactory::connect()
            .transport(WebsocketClientTransport::from(endpoint.as_str()))
            .acceptor(Box::new(|| Box::new(EchoRSocket)))
            .start()
            .await
            .with_context(|| format!("could not connect to Paper Exchange at {endpoint}"))?,
    );
    let mut client = paper_exchange_client()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    Ok(client.get_or_insert(connected).clone())
}

async fn paper_exchange_request(
    method: &str,
    path: &str,
    body: Option<Value>,
) -> anyhow::Result<Value> {
    let client = get_paper_exchange_client().await?;
    let request = PaperExchangeRequest {
        method: method.to_owned(),
        path: path.to_owned(),
        body,
    };
    let reply = match client
        .request_response(
            Payload::builder()
                .set_data(serde_json::to_vec(&request)?)
                .build(),
        )
        .await
    {
        Ok(Some(reply)) => reply,
        Ok(None) => anyhow::bail!("Paper Exchange returned an empty response"),
        Err(error) => {
            let mut slot = paper_exchange_client()
                .lock()
                .unwrap_or_else(|p| p.into_inner());
            if slot
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, &client))
            {
                *slot = None;
            }
            return Err(error).context("Paper Exchange RSocket request failed");
        }
    };
    let response: PaperExchangeResponse = serde_json::from_slice(
        reply
            .data()
            .context("Paper Exchange response had no data")?
            .as_ref(),
    )
    .context("Paper Exchange response was invalid")?;
    anyhow::ensure!(
        (200..300).contains(&response.status),
        "Paper Exchange returned status {}: {}",
        response.status,
        response.body
    );
    Ok(response.body)
}

pub(super) async fn paper_exchange_get(path: &str) -> anyhow::Result<Value> {
    paper_exchange_request("GET", path, None).await
}

pub(super) async fn paper_exchange_post(path: &str, body: Value) -> anyhow::Result<Value> {
    paper_exchange_request("POST", path, Some(body)).await
}
