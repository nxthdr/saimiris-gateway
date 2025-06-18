use chrono::Utc;
use saimiris_gateway::agent::{AgentConfig, HealthStatus};
use std::collections::HashMap;

#[test]
fn test_agent_config_serialization() {
    let config = AgentConfig {
        version: "1.0.0".to_string(),
        capabilities: vec!["test".to_string()],
        settings: HashMap::new(),
        last_updated: Utc::now(),
    };
    
    let serialized = serde_json::to_string(&config).unwrap();
    let deserialized: AgentConfig = serde_json::from_str(&serialized).unwrap();
    
    assert_eq!(config.version, deserialized.version);
    assert_eq!(config.capabilities, deserialized.capabilities);
}

#[test]
fn test_health_status_serialization() {
    let health = HealthStatus {
        healthy: true,
        last_check: Utc::now(),
        message: Some("All systems operational".to_string()),
    };
    
    let serialized = serde_json::to_string(&health).unwrap();
    let deserialized: HealthStatus = serde_json::from_str(&serialized).unwrap();
    
    assert_eq!(health.healthy, deserialized.healthy);
    assert_eq!(health.message, deserialized.message);
}