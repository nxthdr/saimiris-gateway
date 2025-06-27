use rdkafka::config::ClientConfig;
use rdkafka::message::OwnedHeaders;
use rdkafka::producer::FutureProducer;
use rdkafka::{error::KafkaError, producer::FutureRecord};
use std::time::Duration;
use tracing::{debug, error};

/// SASL authentication configuration
#[derive(Clone)]
pub struct SaslAuth {
    pub username: String,
    pub password: String,
    pub mechanism: String,
}

/// Kafka authentication options
#[derive(Clone)]
pub enum KafkaAuth {
    SaslPlainText(SaslAuth),
    PlainText,
}

/// Represents a Kafka configuration
#[derive(Clone)]
pub struct KafkaConfig {
    pub brokers: String,
    pub topic: String,
    pub auth: KafkaAuth,
}

impl Default for KafkaConfig {
    fn default() -> Self {
        Self {
            brokers: "localhost:9092".into(),
            topic: "probes".into(),
            auth: KafkaAuth::PlainText,
        }
    }
}

/// Creates a new Kafka producer
pub fn create_producer(config: &KafkaConfig) -> Result<FutureProducer, KafkaError> {
    let mut client_config = ClientConfig::new();
    client_config
        .set("bootstrap.servers", &config.brokers)
        .set("message.timeout.ms", "5000");

    match &config.auth {
        KafkaAuth::PlainText => {
            // No additional configuration needed for plaintext
        }
        KafkaAuth::SaslPlainText(sasl_auth) => {
            client_config
                .set("sasl.username", &sasl_auth.username)
                .set("sasl.password", &sasl_auth.password)
                .set("sasl.mechanisms", &sasl_auth.mechanism)
                .set("security.protocol", "SASL_PLAINTEXT");
        }
    }

    client_config.create()
}

/// Sends a message to Kafka
pub async fn send_to_kafka(
    producer: &FutureProducer,
    topic: &str,
    key: &str,
    payload: &[u8],
    headers: Option<OwnedHeaders>,
) -> Result<(), KafkaError> {
    let mut record = FutureRecord::to(topic).payload(payload).key(key);

    if let Some(headers) = headers {
        record = record.headers(headers);
    }

    let delivery_status = producer.send(record, Duration::from_secs(0)).await;

    match delivery_status {
        Ok((partition, offset)) => {
            debug!(
                "Message sent to topic '{}', partition {} at offset {}",
                topic, partition, offset
            );
            Ok(())
        }
        Err((err, _)) => {
            error!("Failed to send message to Kafka: {}", err);
            Err(err)
        }
    }
}
