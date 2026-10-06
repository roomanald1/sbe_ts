use anyhow::Result;
use axum::{Json, Router, extract::Path, http::StatusCode, routing::get};
use rsocket_rust::prelude::*;
use rsocket_rust_transport_websocket::WebsocketServerTransport;
use sbe_schema::{SBE_BLOCK_LENGTH, SymbolPriceEncoder, WriteBuf, message_header_codec};
use serde::Serialize;
use serde_json::Value;
use std::{env, net::SocketAddr, pin::Pin, sync::Arc};
use tokio::time::{Duration, sleep};

fn encode_symbol_price(symbol: [u8; 8], price: f64) -> Vec<u8> {
    let header_length = message_header_codec::ENCODED_LENGTH;
    let mut buffer = vec![0; header_length + SBE_BLOCK_LENGTH as usize];

    let mut encoder = SymbolPriceEncoder::default().wrap(WriteBuf::new(&mut buffer), header_length);
    encoder.symbol(symbol);
    encoder.price(price);
    drop(encoder.header(0));

    buffer
}

#[tokio::main]
async fn main() {
    // Read PORT from environment (Render requirement)
    let port = env::var("PORT").unwrap_or_else(|_| "10000".to_string());
    let addr = format!("0.0.0.0:{port}");

    println!("RSocket WebSocket server on ws://{addr}");

    RSocketFactory::receive()
        .transport(WebsocketServerTransport::from(addr))
        .acceptor(Box::new(|setup, _socket| {
            println!("socket establish: setup={:?}", setup);
            Ok(Box::new(ServerResponder))
        }))
        .serve()
        .await
        .unwrap();

    let mut users = Vec::with_capacity(10000);
    for i in 0..10000 {
        users.push(User { id: i });
    }
    let users = Arc::new(users);

    let app = Router::new().route(
        "/users/{page}",
        get({
            let users = Arc::clone(&users);
            move |path| get_user_page(path, users)
        }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}

#[tokio::test]
async fn rest_main() {
    let mut users = Vec::with_capacity(10000);
    for i in 0..10000 {
        users.push(User { id: i });
    }
    let users = Arc::new(users);

    let app = Router::new().route(
        "/users/{page}",
        get({
            let users = Arc::clone(&users);
            move |path| get_user_page(path, users)
        }),
    );

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    println!("Listening on {}", addr);

    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();

    axum::serve(listener, app).await.unwrap();
}

async fn get_user_page(Path(page): Path<i32>, users: Arc<Vec<User>>) -> Result<Json<Value>, (StatusCode, String)> {
    let page_size: usize = 100;
    let start = page as usize * page_size;
    let end = start + page_size;

    if start >= users.len() {
        return Err((StatusCode::NOT_FOUND, "Page exceeds total length".into()));
    }

    let number_of_items = if end > users.len() {
        users.len() - start
    } else {
        page_size
    };

    let has_more =             number_of_items == page_size && start + page_size < users.len();
    let users = users
        .iter()
        .skip(start)
        .take(number_of_items)
        .cloned()
        .collect::<Vec<_>>();

    Ok(Json(
        serde_json::to_value(Response {
            data: users,
            has_more,
            error: None,
        })
        .unwrap(),
    ))
}
#[derive(Clone, Copy, Debug, Serialize)]
struct User {
    id: u64,
}

#[derive(Clone, Debug, Serialize)]
struct Response {
    data: Vec<User>,
    has_more: bool,
    error: Option<String>,
}

struct ServerResponder;

impl RSocket for ServerResponder {
    fn request_stream(
        &self,
        _payload: Payload,
    ) -> Pin<
        Box<
            dyn Stream<Item = Result<rsocket_rust::prelude::Payload, anyhow::Error>>
                + Send
                + 'static,
        >,
    > {
        // Emit one update for all symbols every 10 ms.
        let s =
            futures_util::stream::unfold((0_u64, 0_usize), |(round, symbol_index)| async move {
                if symbol_index == 0 {
                    sleep(Duration::from_millis(10)).await;
                }

                let symbol_text = format!("SYM{symbol_index:03}");
                let mut symbol = [b' '; 8];
                symbol[..symbol_text.len()].copy_from_slice(symbol_text.as_bytes());
                let base_price = 25.0 + symbol_index as f64 * 4.75;
                let phase = round as f64 * 0.17 + symbol_index as f64 * 1.91;
                let movement = phase.sin() * 0.004 + (phase * 0.37).cos() * 0.001;
                let price = base_price * (1.0 + movement);
                let payload = Payload::builder()
                    .set_data(encode_symbol_price(symbol, price))
                    .build();

                let next_state = if symbol_index == 99 {
                    (round.wrapping_add(1), 0)
                } else {
                    (round, symbol_index + 1)
                };

                Some((Ok(payload), next_state))
            });

        Box::pin(s)
    }

    #[doc = " Metadata-Push interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn metadata_push<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Fire and Forget interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn fire_and_forget<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<dyn ::core::future::Future<Output = Result<()>> + ::core::marker::Send + 'async_trait>,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Request-Response interaction model of RSocket."]
    #[allow(
        elided_named_lifetimes,
        clippy::type_complexity,
        clippy::type_repetition_in_bounds
    )]
    fn request_response<'life0, 'async_trait>(
        &'life0 self,
        req: Payload,
    ) -> ::core::pin::Pin<
        Box<
            dyn ::core::future::Future<Output = Result<Option<Payload>>>
                + ::core::marker::Send
                + 'async_trait,
        >,
    >
    where
        'life0: 'async_trait,
        Self: 'async_trait,
    {
        todo!()
    }

    #[doc = " Request-Channel interaction model of RSocket."]
    fn request_channel(&self, reqs: Flux<Result<Payload>>) -> Flux<Result<Payload>> {
        todo!()
    }
}
