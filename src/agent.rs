pub use caracat::rate_limiter::RateLimitingMethod;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub name: String,
    pub status: AgentStatus,
    pub config: Option<AgentConfig>,
    pub health: Option<HealthStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AgentStatus {
    Active,
    Inactive,
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RateLimitingMethodSerde(pub RateLimitingMethod);

impl Serialize for RateLimitingMethodSerde {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self.0 {
            RateLimitingMethod::None => "None",
            RateLimitingMethod::Active => "Active",
            RateLimitingMethod::Auto => "Auto",
            RateLimitingMethod::Sleep => "Sleep",
        };
        serializer.serialize_str(s)
    }
}

impl<'de> Deserialize<'de> for RateLimitingMethodSerde {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let method = match s.as_str() {
            "None" => RateLimitingMethod::None,
            "Active" => RateLimitingMethod::Active,
            "Auto" => RateLimitingMethod::Auto,
            "Sleep" => RateLimitingMethod::Sleep,
            other => {
                return Err(serde::de::Error::custom(format!(
                    "Unknown rate limiting method: {}",
                    other
                )));
            }
        };
        Ok(RateLimitingMethodSerde(method))
    }
}

impl From<RateLimitingMethod> for RateLimitingMethodSerde {
    fn from(m: RateLimitingMethod) -> Self {
        RateLimitingMethodSerde(m)
    }
}
impl From<RateLimitingMethodSerde> for RateLimitingMethod {
    fn from(m: RateLimitingMethodSerde) -> Self {
        m.0
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    #[serde(default = "default_caracat_batch_size")]
    pub batch_size: u64,
    #[serde(default = "default_caracat_instance_id")]
    pub instance_id: u16,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub min_ttl: Option<u8>,
    #[serde(default)]
    pub max_ttl: Option<u8>,
    #[serde(default)]
    pub integrity_check: bool,
    #[serde(default = "default_caracat_interface")]
    pub interface: String,
    #[serde(default)]
    pub src_ipv4_addr: Option<std::net::Ipv4Addr>,
    #[serde(default)]
    pub src_ipv6_addr: Option<std::net::Ipv6Addr>,
    #[serde(default = "default_caracat_packets")]
    pub packets: u64,
    #[serde(default = "default_caracat_probing_rate")]
    pub probing_rate: u64,
    #[serde(
        default = "default_caracat_rate_limiting_method",
        serialize_with = "serialize_rlm",
        deserialize_with = "deserialize_rlm"
    )]
    pub rate_limiting_method: RateLimitingMethod,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub last_check: DateTime<Utc>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct AgentStore {
    agents: Arc<RwLock<HashMap<String, Agent>>>,
}

impl AgentStore {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_agent(&self, name: String) -> Result<(), String> {
        let now = Utc::now();
        let mut agents = self.agents.write().await;
        if agents.values().any(|a| a.name == name) {
            return Err(format!("Agent with name '{}' already exists", name));
        }
        let agent = Agent {
            name,
            status: AgentStatus::Unknown,
            config: None,
            health: None,
            created_at: now,
            updated_at: now,
        };
        agents.insert(agent.name.clone(), agent);
        Ok(())
    }

    pub async fn get(&self, name: &str) -> Option<Agent> {
        let agents = self.agents.read().await;
        agents.get(name).cloned()
    }

    pub async fn list_all(&self) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    pub async fn update_config(&self, name: &str, config: AgentConfig) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(name) {
            agent.config = Some(config);
            agent.updated_at = Utc::now();
        }
    }

    pub async fn update_health(&self, name: &str, health: HealthStatus) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(name) {
            agent.health = Some(health.clone());
            agent.status = if health.healthy {
                AgentStatus::Active
            } else {
                AgentStatus::Inactive
            };
            agent.updated_at = Utc::now();
        }
    }

    pub async fn remove_agent(&self, name: &str) -> bool {
        let mut agents = self.agents.write().await;
        agents.remove(name).is_some()
    }
}

fn default_caracat_batch_size() -> u64 {
    1000
}
fn default_caracat_instance_id() -> u16 {
    0
}
fn default_caracat_interface() -> String {
    "eth0".to_string()
}
fn default_caracat_packets() -> u64 {
    10000
}
fn default_caracat_probing_rate() -> u64 {
    1000
}
fn default_caracat_rate_limiting_method() -> RateLimitingMethod {
    RateLimitingMethod::None
}

fn serialize_rlm<S>(rlm: &RateLimitingMethod, serializer: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    RateLimitingMethodSerde::from(rlm.clone()).serialize(serializer)
}

fn deserialize_rlm<'de, D>(deserializer: D) -> Result<RateLimitingMethod, D::Error>
where
    D: serde::Deserializer<'de>,
{
    RateLimitingMethodSerde::deserialize(deserializer).map(|w| w.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json;

    #[test]
    fn test_rate_limiting_method_serialization() {
        let method = RateLimitingMethod::Active;
        let wrapper = RateLimitingMethodSerde(method);
        let serialized = serde_json::to_string(&wrapper).unwrap();
        assert_eq!(serialized, "\"Active\"");
        let deserialized: RateLimitingMethodSerde = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.0, RateLimitingMethod::Active);
    }

    #[test]
    fn test_rate_limiting_method_serialization_none() {
        let method = RateLimitingMethod::None;
        let wrapper = RateLimitingMethodSerde(method);
        let serialized = serde_json::to_string(&wrapper).unwrap();
        assert_eq!(serialized, "\"None\"");
        let deserialized: RateLimitingMethodSerde = serde_json::from_str(&serialized).unwrap();
        assert_eq!(deserialized.0, RateLimitingMethod::None);
    }

    #[tokio::test]
    async fn test_agent_store_add_and_get() {
        let store = AgentStore::new();
        store.add_agent("test-agent".to_string()).await.unwrap();

        let agent = store.get("test-agent").await.unwrap();
        assert_eq!(agent.name, "test-agent");
        assert!(matches!(agent.status, AgentStatus::Unknown));
    }

    #[tokio::test]
    async fn test_agent_store_list_all() {
        let store = AgentStore::new();
        store.add_agent("agent1".to_string()).await.unwrap();
        store.add_agent("agent2".to_string()).await.unwrap();

        let agents = store.list_all().await;
        assert_eq!(agents.len(), 2);

        let agent_names: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
        assert!(agent_names.contains(&"agent1".to_string()));
        assert!(agent_names.contains(&"agent2".to_string()));
    }

    #[tokio::test]
    async fn test_agent_store_update_config() {
        let store = AgentStore::new();
        store.add_agent("test-agent".to_string()).await.unwrap();

        let config = AgentConfig {
            batch_size: 100,
            instance_id: 1,
            dry_run: false,
            min_ttl: Some(10),
            max_ttl: Some(255),
            integrity_check: true,
            interface: "eth0".to_string(),
            src_ipv4_addr: Some("192.168.1.1".parse().unwrap()),
            src_ipv6_addr: Some("::1".parse().unwrap()),
            packets: 1000,
            probing_rate: 100,
            rate_limiting_method: RateLimitingMethod::None,
        };

        store.update_config("test-agent", config.clone()).await;

        let agent = store.get("test-agent").await.unwrap();
        assert!(agent.config.is_some());
        let agent_config = agent.config.unwrap();
        assert_eq!(agent_config.batch_size, 100);
        assert_eq!(agent_config.instance_id, 1);
        assert!(!agent_config.dry_run);
        assert_eq!(agent_config.min_ttl, Some(10));
        assert_eq!(agent_config.max_ttl, Some(255));
        assert!(agent_config.integrity_check);
        assert_eq!(agent_config.interface, "eth0");
        assert_eq!(
            agent_config.src_ipv4_addr,
            Some("192.168.1.1".parse().unwrap())
        );
        assert_eq!(agent_config.src_ipv6_addr, Some("::1".parse().unwrap()));
        assert_eq!(agent_config.packets, 1000);
        assert_eq!(agent_config.probing_rate, 100);
        assert_eq!(agent_config.rate_limiting_method, RateLimitingMethod::None);
    }

    #[tokio::test]
    async fn test_agent_store_update_health() {
        let store = AgentStore::new();
        store.add_agent("test-agent".to_string()).await.unwrap();

        let health = HealthStatus {
            healthy: true,
            last_check: Utc::now(),
            message: None,
        };

        store.update_health("test-agent", health).await;

        let agent = store.get("test-agent").await.unwrap();
        assert!(agent.health.is_some());
        assert!(agent.health.unwrap().healthy);
        assert!(matches!(agent.status, AgentStatus::Active));
    }

    #[tokio::test]
    async fn test_agent_store_update_health_unhealthy() {
        let store = AgentStore::new();
        store.add_agent("test-agent".to_string()).await.unwrap();

        let health = HealthStatus {
            healthy: false,
            last_check: Utc::now(),
            message: Some("Service unavailable".to_string()),
        };

        store.update_health("test-agent", health).await;

        let agent = store.get("test-agent").await.unwrap();
        assert!(agent.health.is_some());
        assert!(!agent.health.unwrap().healthy);
        assert!(matches!(agent.status, AgentStatus::Inactive));
    }

    #[tokio::test]
    async fn test_agent_store_remove_agent() {
        let store = AgentStore::new();
        store.add_agent("test-agent".to_string()).await.unwrap();

        let removed = store.remove_agent("test-agent").await;
        assert!(removed);

        let agent = store.get("test-agent").await;
        assert!(agent.is_none());
    }

    #[tokio::test]
    async fn test_agent_store_get_nonexistent() {
        let store = AgentStore::new();
        let fake_name = "nonexistent-agent";

        let agent = store.get(fake_name).await;
        assert!(agent.is_none());
    }

    #[tokio::test]
    async fn test_agent_store_remove_nonexistent() {
        let store = AgentStore::new();
        let fake_name = "nonexistent-agent";

        let removed = store.remove_agent(fake_name).await;
        assert!(!removed);
    }
}
