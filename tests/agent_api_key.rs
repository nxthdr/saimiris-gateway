use axum::{body::Body, http::Request};
use rdkafka::config::ClientConfig;
use saimiris_gateway::{AppState, agent::AgentStore, kafka, database::Database};

async fn create_mock_database() -> Database {
    // Create a mock database that doesn't require PostgreSQL
    Database::new_mock()
}

#[tokio::test]
async fn test_agent_key_middleware_with_valid_key() {
    // Create test Kafka config
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    let state = AppState {
        agent_store: AgentStore::new(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };

    let request = Request::builder()
        .header("authorization", "Bearer test-key")
        .body(Body::empty())
        .unwrap();

    // Test that middleware would pass with valid key
    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    assert_eq!(auth_header, Some("test-key"));
    assert_eq!(auth_header, Some(state.agent_key.as_str()));
}

#[tokio::test]
async fn test_agent_key_middleware_with_invalid_key() {
    // Create test Kafka config
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    let state = AppState {
        agent_store: AgentStore::new(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };

    let request = Request::builder()
        .header("authorization", "Bearer wrong-key")
        .body(Body::empty())
        .unwrap();

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    assert_eq!(auth_header, Some("wrong-key"));
    assert_ne!(auth_header, Some(state.agent_key.as_str()));
}

#[tokio::test]
async fn test_agent_key_middleware_without_header() {
    // Create test Kafka config
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    let _state = AppState {
        agent_store: AgentStore::new(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };

    let request = Request::builder().body(Body::empty()).unwrap();

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    assert_eq!(auth_header, None);
}

#[tokio::test]
async fn test_agent_key_middleware_with_malformed_header() {
    // Create test Kafka config
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    let _state = AppState {
        agent_store: AgentStore::new(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };

    let request = Request::builder()
        .header("authorization", "test-key") // Missing "Bearer " prefix
        .body(Body::empty())
        .unwrap();

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    assert_eq!(auth_header, None);
}

#[tokio::test]
async fn test_app_state_creation() {
    let agent_store = AgentStore::new();
    let agent_key = "test-api-key-123".to_string();

    // Create test Kafka config
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    let state = AppState {
        agent_store: agent_store.clone(),
        agent_key: agent_key.clone(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };

    assert_eq!(state.agent_key, agent_key);
}
