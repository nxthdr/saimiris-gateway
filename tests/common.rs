use rdkafka::config::ClientConfig;
use saimiris_gateway::{AppState, agent::AgentStore, database::Database, kafka};

/// Create a mock database for testing
/// This creates a test database that won't actually persist data
pub async fn create_mock_database() -> Database {
    // Create a mock database that doesn't require PostgreSQL
    Database::new_mock()
}

pub async fn create_test_app_state() -> AppState {
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    // Use mock database instead of real PostgreSQL connection
    let database = create_mock_database().await;

    AppState {
        agent_store: AgentStore::new(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database,
    }
}
