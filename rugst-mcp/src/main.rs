use rmcp::{ServiceExt, transport::stdio};

mod server;
use server::RugstServer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let service = RugstServer::new()?
        .serve(stdio())
        .await?;

    service.waiting().await?;

    Ok(())
}