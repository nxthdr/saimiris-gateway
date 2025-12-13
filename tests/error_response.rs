use axum_test::TestServer;
use saimiris_gateway::{AppState, agent::AgentStore, create_app, database::Database, kafka};
use serde_json::json;

async fn create_mock_database() -> Database {
    Database::new_mock()
}

/// Test that error responses are properly formatted as JSON
#[tokio::test]
async fn test_json_error_responses() {
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
        auth0_jwks_uri: Some("https://test.auth0.com/.well-known/jwks.json".to_string()),
        auth0_issuer: Some("https://test.auth0.com/".to_string()),
        bypass_jwt_validation: true, // Bypass JWT validation for testing
        database: create_mock_database().await,
    };
    let app = create_app(state.clone());
    let server = TestServer::new(app).unwrap();

    // Test 1: Empty probe list should return JSON error
    let empty_probe_request = json!({
        "probes": [],
        "metadata": []
    });
    let response = server
        .post("/api/probes")
        .add_header("authorization", "Bearer test-token")
        .json(&empty_probe_request)
        .await;

    assert_eq!(response.status_code(), 400);
    let response_body: serde_json::Value = response.json();
    assert!(response_body.get("error").is_some());
    assert!(response_body.get("message").is_some());
    assert_eq!(response_body["error"], 400);

    // Test 2: Probe submission with missing IP address should return JSON error
    let probe_request_no_ip = json!({
        "probes": [
            ["8.8.8.8", 12345, 80, 64, "tcp"]
        ],
        "metadata": [
            {
                "id": "test-agent-1"
                // Missing ip_address field
            }
        ]
    });
    let response = server
        .post("/api/probes")
        .add_header("authorization", "Bearer test-token")
        .json(&probe_request_no_ip)
        .await;

    assert_eq!(response.status_code(), 400);
    let response_body: serde_json::Value = response.json();
    assert!(response_body.get("error").is_some());
    assert!(response_body.get("message").is_some());
    assert_eq!(response_body["error"], 400);

    // Test 3: User info should work with bypass JWT validation
    let response = server
        .get("/api/user/me")
        .add_header("authorization", "Bearer test-token")
        .await;

    assert_eq!(response.status_code(), 200);
    let response_body: serde_json::Value = response.json();
    assert!(response_body.get("user_id").is_some());
    assert!(response_body.get("used").is_some());
    assert!(response_body.get("limit").is_some());
}

/// Test that all error responses include consistent error structure
#[tokio::test]
async fn test_error_structure_consistency() {
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
        auth0_jwks_uri: Some("https://test.auth0.com/.well-known/jwks.json".to_string()),
        auth0_issuer: Some("https://test.auth0.com/".to_string()),
        bypass_jwt_validation: true,
        database: create_mock_database().await,
    };
    let app = create_app(state.clone());
    let server = TestServer::new(app).unwrap();

    // Test various error conditions and ensure they all have consistent structure
    let test_cases = vec![
        // Empty probe list
        (
            json!({
                "probes": [],
                "metadata": []
            }),
            400,
        ),
        // Missing metadata
        (
            json!({
                "probes": [
                    ["8.8.8.8", 12345, 80, 64, "tcp"]
                ],
                "metadata": []
            }),
            400,
        ),
    ];

    for (request_body, expected_status) in test_cases {
        let response = server
            .post("/api/probes")
            .add_header("authorization", "Bearer test-token")
            .json(&request_body)
            .await;

        assert_eq!(response.status_code(), expected_status);

        // Check if response is JSON and has expected structure
        let response_body: serde_json::Value = response.json();
        assert!(
            response_body.get("error").is_some(),
            "Error response should have 'error' field"
        );
        assert!(
            response_body.get("message").is_some(),
            "Error response should have 'message' field"
        );

        // Ensure error and message are present and correctly typed
        assert!(
            response_body["error"].is_number(),
            "Error field should be a number"
        );
        assert!(
            response_body["message"].is_string(),
            "Message field should be a string"
        );

        // Ensure message is not empty
        assert!(
            !response_body["message"].as_str().unwrap().is_empty(),
            "Message should not be empty"
        );
    }
}
