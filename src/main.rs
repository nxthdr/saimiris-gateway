use anyhow::Result;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::net::SocketAddr;
use tracing::info;

use clap::Parser;
use saimiris_gateway::{AppState, agent::AgentStore, create_app};

/// Command line arguments for the gateway
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    /// Agent key for agent authentication
    #[arg(long = "agent-key")]
    pub agent_key: String,

    /// Verbosity level
    #[clap(flatten)]
    verbose: Verbosity<InfoLevel>,
}

fn set_tracing(cli: &Cli) -> Result<()> {
    let subscriber = tracing_subscriber::fmt()
        .compact()
        .with_file(true)
        .with_line_number(true)
        .with_max_level(cli.verbose)
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;
    Ok(())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    set_tracing(&cli)?;

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
