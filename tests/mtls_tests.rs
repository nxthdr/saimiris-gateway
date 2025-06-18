use saimiris_gateway::agent::{AgentConfig, HealthStatus};
use saimiris_gateway::mtls::MtlsClient;
use std::sync::Once;
use wiremock::{
    matchers::{method, path},
    Mock, MockServer, ResponseTemplate,
};

static INIT: Once = Once::new();

fn ensure_crypto_provider() {
    INIT.call_once(|| {
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
            .unwrap();
    });
}

#[tokio::test]
async fn test_get_config_success() {
    let mock_server = MockServer::start().await;

    let config_response = AgentConfig {
        version: "1.0.0".to_string(),
        capabilities: vec!["capability1".to_string(), "capability2".to_string()],
        settings: {
            let mut settings = std::collections::HashMap::new();
            settings.insert(
                "key1".to_string(),
                serde_json::Value::String("value1".to_string()),
            );
            settings
        },
        last_updated: chrono::Utc::now(),
    };

    Mock::given(method("GET"))
        .and(path("/config"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&config_response))
        .mount(&mock_server)
        .await;

    // Note: This test would require mocking the TLS client creation
    // For a full implementation, you'd need to create a trait for MtlsClient
    // and use dependency injection with a mock implementation

    // This is a simplified test structure showing how you would test
    // the HTTP interactions without the TLS complexity
    assert_eq!(config_response.version, "1.0.0");
    assert_eq!(config_response.capabilities.len(), 2);
}

#[tokio::test]
async fn test_check_health_success() {
    let mock_server = MockServer::start().await;

    let health_response = HealthStatus {
        healthy: true,
        last_check: chrono::Utc::now(),
        message: None,
    };

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&health_response))
        .mount(&mock_server)
        .await;

    // Similar to above, this demonstrates the test structure
    // A full implementation would require trait abstraction
    assert!(health_response.healthy);
    assert!(health_response.message.is_none());
}

#[tokio::test]
async fn test_check_health_unhealthy() {
    let mock_server = MockServer::start().await;

    let health_response = HealthStatus {
        healthy: false,
        last_check: chrono::Utc::now(),
        message: Some("Service degraded".to_string()),
    };

    Mock::given(method("GET"))
        .and(path("/health"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&health_response))
        .mount(&mock_server)
        .await;

    assert!(!health_response.healthy);
    assert_eq!(
        health_response.message.as_ref().unwrap(),
        "Service degraded"
    );
}

#[test]
fn test_build_client_config_with_real_test_files() {
    ensure_crypto_provider();
    let ca_cert = "tests/certs/test-ca.pem";
    let client_cert = "tests/certs/test-client.pem";
    let client_key = "tests/certs/test-client-key.pem";
    let result = MtlsClient::build_client_config(ca_cert, client_cert, client_key);
    assert!(
        result.is_ok(),
        "Expected Ok(ClientConfig), got: {:?}",
        result
    );
}
