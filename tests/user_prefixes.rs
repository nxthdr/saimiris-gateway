use axum_test::TestServer;
use saimiris_gateway::{
    AppState,
    agent::{AgentConfig, AgentStore},
    create_app,
    database::Database,
    kafka,
};

async fn create_mock_database() -> Database {
    Database::new_mock()
}

/// Test the new /api/user/prefixes endpoint
#[tokio::test]
async fn test_user_prefixes_endpoint() {
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
        agent_store: agent_store.clone(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: true,
        database: create_mock_database().await,
    };

    // Add a test agent with IPv6 prefix configuration
    agent_store
        .add_agent("test-agent-1".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config = vec![AgentConfig {
        src_ipv6_prefix: Some("2001:db8:1234::/48".to_string()),
        ..Default::default()
    }];
    agent_store.update_config("test-agent-1", config).await;

    // Add another agent without IPv6 prefix (should not appear in results)
    agent_store
        .add_agent("test-agent-2".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_no_ipv6 = vec![AgentConfig {
        src_ipv6_prefix: None,
        ..Default::default()
    }];
    agent_store
        .update_config("test-agent-2", config_no_ipv6)
        .await;

    let app = create_app(state);
    let server = TestServer::new(app).unwrap();

    // Test the user prefixes endpoint
    let response = server
        .get("/api/user/prefixes")
        .add_header("authorization", "Bearer test-token")
        .await;

    assert_eq!(response.status_code(), 200);
    let response_body: serde_json::Value = response.json();

    // Verify the response structure
    assert!(response_body.get("user_id").is_some());
    assert!(response_body.get("agents").is_some());

    let user_id = response_body["user_id"].as_u64().unwrap();
    let agents = response_body["agents"].as_array().unwrap();

    // Should only have one agent (the one with IPv6 prefix)
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    assert_eq!(agent["agent_id"], "test-agent-1");

    let prefixes = agent["prefixes"].as_array().unwrap();
    assert_eq!(prefixes.len(), 1);

    let prefix = &prefixes[0];
    assert_eq!(prefix["agent_prefix"], "2001:db8:1234::/48");

    // Verify the user prefix calculation
    let user_prefix = prefix["user_prefix"].as_str().unwrap();
    assert!(user_prefix.ends_with("/80"));

    println!("✅ User prefixes endpoint test passed:");
    println!("   User ID: {}", user_id);
    println!("   Agent: {}", agent["agent_id"]);
    println!("   Agent Prefix: {}", prefix["agent_prefix"]);
    println!("   User Prefix: {}", user_prefix);
}

/// Test the calculate_user_prefix function directly
#[tokio::test]
async fn test_calculate_user_prefix_function() {
    use saimiris_gateway::calculate_user_prefix;

    let user_id = 0x12345678u32; // Use a static test user ID
    let agent_prefix = "2001:db8:1234::/48";

    let user_prefix = calculate_user_prefix(agent_prefix, user_id);
    assert!(user_prefix.is_some());

    let user_prefix = user_prefix.unwrap();
    assert!(user_prefix.starts_with("2001:db8:1234:"));
    assert!(user_prefix.ends_with("/80"));

    println!("✅ Prefix calculation test passed:");
    println!("   User ID: 0x{:08x}", user_id);
    println!("   Agent Prefix: {}", agent_prefix);
    println!("   User Prefix: {}", user_prefix);

    // Test with invalid prefix
    let invalid_result = calculate_user_prefix("invalid-prefix", user_id);
    assert!(invalid_result.is_none());
}

/// Test with multiple agents and multiple prefixes per agent
#[tokio::test]
async fn test_multiple_agents_and_prefixes() {
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
        agent_store: agent_store.clone(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        logto_jwks_uri: Some("https://test.logto.app/oidc/jwks".to_string()),
        logto_issuer: Some("https://test.logto.app/oidc".to_string()),
        bypass_jwt_validation: true,
        database: create_mock_database().await,
    };

    // Add multiple agents with different prefix configurations
    agent_store
        .add_agent("agent-eu".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_eu = vec![
        AgentConfig {
            src_ipv6_prefix: Some("2001:db8:1000::/48".to_string()),
            ..Default::default()
        },
        AgentConfig {
            src_ipv6_prefix: Some("2001:db8:2000::/48".to_string()),
            ..Default::default()
        },
    ];
    agent_store.update_config("agent-eu", config_eu).await;

    agent_store
        .add_agent("agent-us".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_us = vec![AgentConfig {
        src_ipv6_prefix: Some("2001:db8:3000::/48".to_string()),
        ..Default::default()
    }];
    agent_store.update_config("agent-us", config_us).await;

    let app = create_app(state);
    let server = TestServer::new(app).unwrap();

    // Test the user prefixes endpoint
    let response = server
        .get("/api/user/prefixes")
        .add_header("authorization", "Bearer test-token")
        .await;

    assert_eq!(response.status_code(), 200);
    let response_body: serde_json::Value = response.json();

    let agents = response_body["agents"].as_array().unwrap();

    // Should have two agents
    assert_eq!(agents.len(), 2);

    // Find the EU agent (should have 2 prefixes)
    let eu_agent = agents.iter().find(|a| a["agent_id"] == "agent-eu").unwrap();
    let eu_prefixes = eu_agent["prefixes"].as_array().unwrap();
    assert_eq!(eu_prefixes.len(), 2);

    // Find the US agent (should have 1 prefix)
    let us_agent = agents.iter().find(|a| a["agent_id"] == "agent-us").unwrap();
    let us_prefixes = us_agent["prefixes"].as_array().unwrap();
    assert_eq!(us_prefixes.len(), 1);

    println!("✅ Multiple agents test passed:");
    println!("   EU agent has {} prefixes", eu_prefixes.len());
    println!("   US agent has {} prefixes", us_prefixes.len());
}
