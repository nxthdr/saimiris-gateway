use anyhow::{Context, Result};
use reqwest::Client;
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::{certs, private_key};
use std::{fs::File, io::BufReader};

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
    pub async fn new(
        ca_cert_path: &str,
        client_cert_path: &str,
        client_key_path: &str,
    ) -> Result<Self> {
        // Ensure CryptoProvider is installed for rustls 0.23+
        rustls::crypto::CryptoProvider::install_default(rustls::crypto::ring::default_provider())
            .ok();
        let client_config =
            Self::build_client_config(ca_cert_path, client_cert_path, client_key_path)?;

        let client = Client::builder()
            .use_preconfigured_tls(client_config)
            .build()
            .context("Failed to build HTTP client")?;

        Ok(Self { client })
    }

    pub fn build_client_config(
        ca_cert_path: &str,
        client_cert_path: &str,
        client_key_path: &str,
    ) -> Result<ClientConfig> {
        let mut root_store = RootCertStore::empty();

        // Load CA certificate
        let ca_cert_file =
            File::open(ca_cert_path).context("Failed to open CA certificate file")?;
        let mut ca_cert_reader = BufReader::new(ca_cert_file);
        let ca_certs = certs(&mut ca_cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse CA certificates")?;

        for cert in ca_certs {
            root_store
                .add(cert)
                .context("Failed to add CA certificate")?;
        }

        // Load client certificate and key
        let client_cert_file =
            File::open(client_cert_path).context("Failed to open client certificate file")?;
        let mut client_cert_reader = BufReader::new(client_cert_file);
        let client_certs = certs(&mut client_cert_reader)
            .collect::<Result<Vec<_>, _>>()
            .context("Failed to parse client certificates")?;

        let client_key_file =
            File::open(client_key_path).context("Failed to open client key file")?;
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
        let response = self.client.get(&url).send().await?.error_for_status()?;

        let config = response.json::<AgentConfig>().await?;
        Ok(config)
    }

    pub async fn check_health(&self, endpoint: &str) -> Result<HealthStatus, MtlsError> {
        let url = format!("{}/health", endpoint);
        let response = self.client.get(&url).send().await?.error_for_status()?;

        let health = response.json::<HealthStatus>().await?;
        Ok(health)
    }
}
