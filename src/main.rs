use std::net::SocketAddr;
use tracing::info;

use clap::Parser;
use saimiris_gateway::{agent::AgentStore, create_app, AppState};

/// Command line arguments for the gateway
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    /// Agent key for agent authentication
    #[arg(long = "agent-key")]
    pub agent_key: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    let agent_store = AgentStore::new();

    // Create app state with agent key for authentication
    let state = AppState {
        agent_store,
        agent_key: cli.agent_key,
    };

    let app = create_app(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
