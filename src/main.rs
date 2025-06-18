use std::{net::SocketAddr, sync::Arc};
use tracing::info;

use clap::Parser;
use saimiris_gateway::{agent::AgentStore, create_app, mtls::MtlsClient, AgentMtlsState, AppState};

/// Command line arguments for the gateway
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    /// Path to the CA certificate (PEM)
    #[arg(long)]
    pub ca_cert: String,
    /// Path to the client certificate (PEM)
    #[arg(long)]
    pub client_cert: String,
    /// Path to the client private key (PEM)
    #[arg(long)]
    pub client_key: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let agent_store = AgentStore::new();
    let mtls_client =
        Arc::new(MtlsClient::new(&cli.ca_cert, &cli.client_cert, &cli.client_key).await?);

    // Create shared agent store for both client and agent APIs
    let client_state = AppState {
        agent_store: agent_store.clone(),
    };

    let agent_state = AgentMtlsState {
        agent_store: agent_store.clone(),
        mtls_client,
    };

    let app = create_app(client_state, agent_state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
