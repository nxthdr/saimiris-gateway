use axum_test::TestServer;
use saimiris_gateway::agent::{AgentConfig, HealthStatus};
use saimiris_gateway::{AppState, agent::AgentStore, create_app, kafka, database::Database};
use serde_json::json;
use std::net::{Ipv4Addr, Ipv6Addr};

async fn create_mock_database() -> Database {
    Database::new_mock()
}

#[tokio::test]
async fn test_agent_api_scenario() {
    // Create a mock Kafka setup
    let kafka_config = kafka::KafkaConfig {
        brokers: "localhost:9092".to_string(),
        topic: "probes".to_string(),
        auth: kafka::KafkaAuth::PlainText,
    };

    // Create a mock Kafka producer
    let kafka_producer = rdkafka::config::ClientConfig::new()
        .create()
        .expect("Failed to create mock Kafka producer");

    // Set up the app state
    let agent_store = AgentStore::new();
    let state = AppState {
        agent_store,
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: false,
        database: create_mock_database().await,
    };
    let app = create_app(state.clone());
    let server = TestServer::new(app).unwrap();

    // 1. Register agent
    let register_body = json!({"id": "agent1", "secret": "s3cr3t"});
    let response = server
        .post("/agent-api/agent/register")
        .add_header("authorization", "Bearer test-key")
        .json(&register_body)
        .await;
    assert_eq!(response.status_code(), 200);

    // 2. Update config
    let configs = vec![
        AgentConfig {
            batch_size: 100,
            instance_id: 1,
            dry_run: false,
            min_ttl: Some(10),
            max_ttl: Some(255),
            integrity_check: true,
            interface: "eth0".to_string(),
            src_ipv4_addr: Some("192.168.1.1".parse::<Ipv4Addr>().unwrap()),
            src_ipv6_addr: Some("::1".parse::<Ipv6Addr>().unwrap()),
            packets: 1000,
            probing_rate: 100,
            rate_limiting_method: "None".to_string(),
        },
        AgentConfig {
            batch_size: 200,
            instance_id: 2,
            dry_run: true,
            min_ttl: Some(20),
            max_ttl: Some(200),
            integrity_check: false,
            interface: "eth1".to_string(),
            src_ipv4_addr: Some("192.168.1.2".parse::<Ipv4Addr>().unwrap()),
            src_ipv6_addr: Some("::2".parse::<Ipv6Addr>().unwrap()),
            packets: 2000,
            probing_rate: 200,
            rate_limiting_method: "auto".to_string(),
        },
    ];
    let response = server
        .post("/agent-api/agent/agent1/config")
        .add_header("authorization", "Bearer test-key")
        .json(&configs)
        .await;
    assert_eq!(response.status_code(), 200);

    // 3. Update health
    let health = HealthStatus {
        healthy: true,
        last_check: chrono::Utc::now(),
        message: Some("All systems operational".to_string()),
    };
    let response = server
        .post("/agent-api/agent/agent1/health")
        .add_header("authorization", "Bearer test-key")
        .json(&health)
        .await;
    assert_eq!(response.status_code(), 200);

    // 4. Fetch list of agents
    let response = server.get("/api/agents").await;
    assert_eq!(response.status_code(), 200);
    let agents: Vec<serde_json::Value> = response.json();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["id"], "agent1");

    // 5. Fetch agent by id
    let response = server.get("/api/agent/agent1").await;
    assert_eq!(response.status_code(), 200);
    let agent: serde_json::Value = response.json();
    assert_eq!(agent["id"], "agent1");

    // 6. Fetch agent config
    let response = server.get("/api/agent/agent1/config").await;
    assert_eq!(response.status_code(), 200);
    let fetched_configs: Vec<AgentConfig> = response.json();
    assert_eq!(fetched_configs.len(), 2);
    assert_eq!(fetched_configs[0].batch_size, 100);
    assert_eq!(fetched_configs[1].batch_size, 200);
}
