mod encoders;
mod okx;
mod positions;
mod rest;
mod websocket;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tokio::try_join!(rest::serve(), websocket::serve())?;
    Ok(())
}
