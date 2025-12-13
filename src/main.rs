use anyhow::Result;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use std::net::SocketAddr;
use tracing::{error, info, warn};

use clap::Parser;
use saimiris_gateway::{
    AppState,
    agent::AgentStore,
    create_app,
    database::{Database, DatabaseConfig},
    kafka,
};

/// Command line arguments for the gateway
#[derive(Parser, Debug)]
#[command(author, version, about)]
pub struct Cli {
    /// API listen address (e.g. 0.0.0.0:8080 or [::]:8080)
    #[arg(long = "address", default_value = "0.0.0.0:8080")]
    pub address: String,

    /// Agent key for agent authentication
    #[arg(long = "agent-key", default_value = "agent-key")]
    pub agent_key: String,

    /// Kafka broker addresses (comma-separated list)
    #[arg(long = "kafka-brokers", default_value = "localhost:9092")]
    pub kafka_brokers: String,

    /// Kafka topic for probes
    #[arg(long = "kafka-topic", default_value = "saimiris-probes")]
    pub kafka_topic: String,

    /// Kafka authentication protocol (PLAINTEXT or SASL_PLAINTEXT)
    #[arg(long = "kafka-auth-protocol", default_value = "PLAINTEXT")]
    pub kafka_auth_protocol: String,

    /// Kafka SASL username (required for SASL_PLAINTEXT)
    #[arg(long = "kafka-sasl-username")]
    pub kafka_sasl_username: Option<String>,

    /// Kafka SASL password (required for SASL_PLAINTEXT)
    #[arg(long = "kafka-sasl-password")]
    pub kafka_sasl_password: Option<String>,

    /// Kafka SASL mechanism (default: SCRAM-SHA-512)
    #[arg(long = "kafka-sasl-mechanism", default_value = "SCRAM-SHA-512")]
    pub kafka_sasl_mechanism: String,

    /// Auth0 JWKS URI for JWT validation
    #[arg(long = "auth0-jwks-uri")]
    pub auth0_jwks_uri: Option<String>,
    /// Auth0 issuer for JWT validation
    #[arg(long = "auth0-issuer")]
    pub auth0_issuer: Option<String>,

    /// Bypass JWT validation (for development only)
    #[arg(long = "bypass-jwt", default_value = "false")]
    pub bypass_jwt: bool,

    /// PostgreSQL database URL
    #[arg(
        long = "database-url",
        default_value = "postgresql://localhost/saimiris_gateway"
    )]
    pub database_url: String,

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
    // Parse command line arguments
    let cli = Cli::parse();

    set_tracing(&cli)?;

    let agent_store = AgentStore::new();

    // Log JWT configuration from CLI parameters
    if let Some(ref jwks_uri) = cli.auth0_jwks_uri {
        info!("Auth0 JWKS URI is set to: {}", jwks_uri);
    } else {
        warn!("Auth0 JWKS URI is not set");
    }

    if let Some(ref issuer) = cli.auth0_issuer {
        info!("Auth0 issuer is set to: {}", issuer);
    } else {
        warn!("Auth0 issuer is not set");
    }

    // Initialize Kafka configuration from CLI parameters
    let kafka_auth = match cli.kafka_auth_protocol.as_str() {
        "PLAINTEXT" => kafka::KafkaAuth::PlainText,
        "SASL_PLAINTEXT" => {
            let username = cli.kafka_sasl_username.ok_or_else(|| {
                anyhow::anyhow!("kafka-sasl-username is required for SASL_PLAINTEXT")
            })?;
            let password = cli.kafka_sasl_password.ok_or_else(|| {
                anyhow::anyhow!("kafka-sasl-password is required for SASL_PLAINTEXT")
            })?;
            kafka::KafkaAuth::SaslPlainText(kafka::SaslAuth {
                username,
                password,
                mechanism: cli.kafka_sasl_mechanism,
            })
        }
        _ => {
            return Err(anyhow::anyhow!(
                "Invalid Kafka authentication protocol: {}. Use PLAINTEXT or SASL_PLAINTEXT",
                cli.kafka_auth_protocol
            ));
        }
    };

    let kafka_config = kafka::KafkaConfig {
        brokers: cli.kafka_brokers.clone(),
        topic: cli.kafka_topic.clone(),
        auth: kafka_auth,
    };

    // Create Kafka producer
    let kafka_producer = match kafka::create_producer(&kafka_config) {
        Ok(producer) => {
            info!("Connected to Kafka brokers: {}", kafka_config.brokers);
            info!("Using Kafka topic: {}", kafka_config.topic);
            producer
        }
        Err(err) => {
            error!("Failed to create Kafka producer: {}", err);
            return Err(anyhow::anyhow!("Failed to create Kafka producer: {}", err));
        }
    };

    // Initialize database
    let database_config = DatabaseConfig::new(cli.database_url.clone());
    let database = match Database::new(&database_config).await {
        Ok(db) => {
            info!("Connected to database: {}", cli.database_url);

            // Run database migrations automatically
            info!("Running database migrations...");
            if let Err(err) = db.initialize().await {
                error!("Failed to run database migrations: {}", err);
                return Err(anyhow::anyhow!(
                    "Failed to run database migrations: {}",
                    err
                ));
            }
            info!("Database migrations completed successfully");
            db
        }
        Err(err) => {
            error!("Failed to connect to database: {}", err);
            return Err(anyhow::anyhow!("Failed to connect to database: {}", err));
        }
    };

    // Create app state with agent key for authentication
    let state = AppState {
        agent_store: agent_store.clone(),
        agent_key: cli.agent_key,
        kafka_config,
        kafka_producer,
        auth0_jwks_uri: cli.auth0_jwks_uri.clone(),
        auth0_issuer: cli.auth0_issuer.clone(),
        bypass_jwt_validation: cli.bypass_jwt,
        database,
    };

    if cli.bypass_jwt {
        warn!("⚠️ JWT validation bypass is enabled!");
    }

    // Spawn background task to clean up stale agents every 5 minutes
    let cleanup_agent_store = agent_store.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(300)); // 5 minutes
        loop {
            interval.tick().await;
            let max_age = chrono::Duration::minutes(10);
            let removed = cleanup_agent_store.remove_stale_agents(max_age).await;
            if !removed.is_empty() {
                info!("Removed {} stale agents: {:?}", removed.len(), removed);
            }
        }
    });

    let app = create_app(state);

    let addr: SocketAddr = cli.address.parse()?;
    info!("Starting server on {}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}
