use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    pub endpoint: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub version: String,
    pub capabilities: Vec<String>,
    pub settings: HashMap<String, serde_json::Value>,
    pub last_updated: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthStatus {
    pub healthy: bool,
    pub last_check: DateTime<Utc>,
    pub message: Option<String>,
}

#[derive(Clone)]
pub struct AgentStore {
    agents: Arc<RwLock<HashMap<Uuid, Agent>>>,
}

impl AgentStore {
    pub fn new() -> Self {
        Self {
            agents: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn add_agent(&self, name: String, endpoint: String) -> Uuid {
        let id = Uuid::new_v4();
        let now = Utc::now();
        
        let agent = Agent {
            id,
            name,
            endpoint,
            status: AgentStatus::Unknown,
            config: None,
            health: None,
            created_at: now,
            updated_at: now,
        };

        let mut agents = self.agents.write().await;
        agents.insert(id, agent);
        id
    }

    pub async fn get(&self, id: &Uuid) -> Option<Agent> {
        let agents = self.agents.read().await;
        agents.get(id).cloned()
    }

    pub async fn list_all(&self) -> Vec<Agent> {
        let agents = self.agents.read().await;
        agents.values().cloned().collect()
    }

    pub async fn update_config(&self, id: &Uuid, config: AgentConfig) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.config = Some(config);
            agent.updated_at = Utc::now();
        }
    }

    pub async fn update_health(&self, id: &Uuid, health: HealthStatus) {
        let mut agents = self.agents.write().await;
        if let Some(agent) = agents.get_mut(id) {
            agent.health = Some(health.clone());
            agent.status = if health.healthy {
                AgentStatus::Active
            } else {
                AgentStatus::Inactive
            };
            agent.updated_at = Utc::now();
        }
    }

    pub async fn remove_agent(&self, id: &Uuid) -> bool {
        let mut agents = self.agents.write().await;
        agents.remove(id).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_agent_store_add_and_get() {
        let store = AgentStore::new();
        let id = store.add_agent("test-agent".to_string(), "https://example.com".to_string()).await;
        
        let agent = store.get(&id).await.unwrap();
        assert_eq!(agent.name, "test-agent");
        assert_eq!(agent.endpoint, "https://example.com");
        assert!(matches!(agent.status, AgentStatus::Unknown));
    }

    #[tokio::test]
    async fn test_agent_store_list_all() {
        let store = AgentStore::new();
        let id1 = store.add_agent("agent1".to_string(), "https://agent1.com".to_string()).await;
        let id2 = store.add_agent("agent2".to_string(), "https://agent2.com".to_string()).await;
        
        let agents = store.list_all().await;
        assert_eq!(agents.len(), 2);
        
        let agent_ids: Vec<Uuid> = agents.iter().map(|a| a.id).collect();
        assert!(agent_ids.contains(&id1));
        assert!(agent_ids.contains(&id2));
    }

    #[tokio::test]
    async fn test_agent_store_update_config() {
        let store = AgentStore::new();
        let id = store.add_agent("test-agent".to_string(), "https://example.com".to_string()).await;
        
        let config = AgentConfig {
            version: "1.0.0".to_string(),
            capabilities: vec!["capability1".to_string()],
            settings: HashMap::new(),
            last_updated: Utc::now(),
        };
        
        store.update_config(&id, config.clone()).await;
        
        let agent = store.get(&id).await.unwrap();
        assert!(agent.config.is_some());
        assert_eq!(agent.config.unwrap().version, "1.0.0");
    }

    #[tokio::test]
    async fn test_agent_store_update_health() {
        let store = AgentStore::new();
        let id = store.add_agent("test-agent".to_string(), "https://example.com".to_string()).await;
        
        let health = HealthStatus {
            healthy: true,
            last_check: Utc::now(),
            message: None,
        };
        
        store.update_health(&id, health).await;
        
        let agent = store.get(&id).await.unwrap();
        assert!(agent.health.is_some());
        assert!(agent.health.unwrap().healthy);
        assert!(matches!(agent.status, AgentStatus::Active));
    }

    #[tokio::test]
    async fn test_agent_store_update_health_unhealthy() {
        let store = AgentStore::new();
        let id = store.add_agent("test-agent".to_string(), "https://example.com".to_string()).await;
        
        let health = HealthStatus {
            healthy: false,
            last_check: Utc::now(),
            message: Some("Service unavailable".to_string()),
        };
        
        store.update_health(&id, health).await;
        
        let agent = store.get(&id).await.unwrap();
        assert!(agent.health.is_some());
        assert!(!agent.health.unwrap().healthy);
        assert!(matches!(agent.status, AgentStatus::Inactive));
    }

    #[tokio::test]
    async fn test_agent_store_remove_agent() {
        let store = AgentStore::new();
        let id = store.add_agent("test-agent".to_string(), "https://example.com".to_string()).await;
        
        let removed = store.remove_agent(&id).await;
        assert!(removed);
        
        let agent = store.get(&id).await;
        assert!(agent.is_none());
    }

    #[tokio::test]
    async fn test_agent_store_get_nonexistent() {
        let store = AgentStore::new();
        let fake_id = Uuid::new_v4();
        
        let agent = store.get(&fake_id).await;
        assert!(agent.is_none());
    }

    #[tokio::test]
    async fn test_agent_store_remove_nonexistent() {
        let store = AgentStore::new();
        let fake_id = Uuid::new_v4();
        
        let removed = store.remove_agent(&fake_id).await;
        assert!(!removed);
    }
}