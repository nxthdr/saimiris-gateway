use anyhow::Result;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::{env, net::SocketAddr, path::Path};
use tracing::{info, warn};

use clap::Parser;
use dotenv;
use saimiris_gateway::{AppState, agent::AgentStore, create_app};

/// Command line arguments for the gateway
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    /// API listen address (e.g. 0.0.0.0:8080 or [::]:8080)
    #[arg(long = "address", default_value = "0.0.0.0:8080")]
    pub address: String,

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
    // Load environment variables from the appropriate .env file
    let env_file = match env::var("ENVIRONMENT")
        .unwrap_or_else(|_| "development".to_string())
        .as_str()
    {
        "production" => ".env.production",
        _ => ".env.development",
    };

    if Path::new(env_file).exists() {
        dotenv::from_path(env_file).ok();
        info!("Loaded environment from {}", env_file);
    } else {
        warn!(
            "Environment file {} not found. Using default values.",
            env_file
        );
    }

    let cli = Cli::parse();
    set_tracing(&cli)?;

    let agent_store = AgentStore::new();

    // Display the JWT-related environment variables
    match env::var("LOGTO_JWKS_URI") {
        Ok(uri) => info!("LOGTO_JWKS_URI is set to: {}", uri),
        Err(_) => warn!("LOGTO_JWKS_URI is not set!"),
    }

    match env::var("LOGTO_ISSUER") {
        Ok(issuer) => info!("LOGTO_ISSUER is set to: {}", issuer),
        Err(_) => warn!("LOGTO_ISSUER is not set!"),
    }

    // Create app state with agent key for authentication
    let state = AppState {
        agent_store,
        agent_key: cli.agent_key,
    };

    let app = create_app(state);

    let addr: SocketAddr = cli.address.parse()?;
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
