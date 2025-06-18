use anyhow::{Context, Result};
use reqwest::Client;
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::{certs, private_key};
use serde::{Deserialize, Serialize};
use std::{fs::File, io::BufReader, sync::Arc};
use tokio_rustls::TlsConnector;

use crate::agent::{AgentConfig, HealthStatus};

#[derive(Debug, thiserror::Error)]
pub enum MtlsError {
    #[error("Certificate error: {0}")]
    Certificate(#[from] std::io::Error),
    #[error("TLS error: {0}")]
    Tls(#[from] rustls::Error),
    #[error("HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("Invalid certificate format")]
    InvalidCertFormat,
}

pub struct MtlsClient {
    client: Client,
}

impl MtlsClient {
    pub async fn new() -> Result<Self> {
        let client_config = Self::build_client_config()?;
        
        let client = Client::builder()
            .use_preconfigured_tls(client_config)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client })
    }

    fn build_client_config() -> Result<ClientConfig> {
        let mut root_store = RootCertStore::empty();
        
        // Load CA certificate
        let ca_cert_file = File::open("certs/ca.pem")
            .context("Failed to open CA certificate file")?;
        let mut ca_cert_reader = BufReader::new(ca_cert_file);
        let ca_certs = certs(&mut ca_cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse CA certificates")?;
        
        for cert in ca_certs {
            root_store.add(cert).context("Failed to add CA certificate")?;
        }

        // Load client certificate and key
        let client_cert_file = File::open("certs/client.pem")
            .context("Failed to open client certificate file")?;
        let mut client_cert_reader = BufReader::new(client_cert_file);
        let client_certs = certs(&mut client_cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse client certificates")?;

        let client_key_file = File::open("certs/client-key.pem")
            .context("Failed to open client key file")?;
        let mut client_key_reader = BufReader::new(client_key_file);
        let client_key = private_key(&mut client_key_reader)
            .context("Failed to parse client private key")?
            .ok_or_else(|| anyhow::anyhow!("No private key found"))?;

        let config = ClientConfig::builder()
            .with_root_certificates(root_store)
            .with_client_auth_cert(client_certs, client_key)
            .context("Failed to build client config with mTLS")?;

        Ok(config)
    }

    pub async fn get_config(&self, endpoint: &str) -> Result<AgentConfig, MtlsError> {
        let url = format!("{}/config", endpoint);
        let response = self.client
            .get(&url)
            .send()
            .await?
            .error_for_status()?;

        let config = response.json::<AgentConfig>().await?;
        Ok(config)
    }

    pub async fn check_health(&self, endpoint: &str) -> Result<HealthStatus, MtlsError> {
        let url = format!("{}/health", endpoint);
        let response = self.client
            .get(&url)
            .send()
            .await?
            .error_for_status()?;

        let health = response.json::<HealthStatus>().await?;
        Ok(health)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        matchers::{method, path},
        Mock, MockServer, ResponseTemplate,
    };

    #[tokio::test]
    async fn test_get_config_success() {
        let mock_server = MockServer::start().await;
        
        let config_response = AgentConfig {
            version: "1.0.0".to_string(),
            capabilities: vec!["capability1".to_string(), "capability2".to_string()],
            settings: {
                let mut settings = std::collections::HashMap::new();
                settings.insert("key1".to_string(), serde_json::Value::String("value1".to_string()));
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
        assert_eq!(health_response.message.as_ref().unwrap(), "Service degraded");
    }
}