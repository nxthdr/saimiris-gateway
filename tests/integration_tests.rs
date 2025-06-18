use chrono::Utc;
use saimiris_gateway::agent::{AgentConfig, HealthStatus, RateLimitingMethod};
use std::net::{Ipv4Addr, Ipv6Addr};

#[test]
fn test_agent_config_serialization() {
    let config = AgentConfig {
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
        rate_limiting_method: RateLimitingMethod::None,
    };

    let serialized = serde_json::to_string(&config).unwrap();
    let deserialized: AgentConfig = serde_json::from_str(&serialized).unwrap();

    assert_eq!(config.batch_size, deserialized.batch_size);
    assert_eq!(config.instance_id, deserialized.instance_id);
    assert_eq!(config.dry_run, deserialized.dry_run);
    assert_eq!(config.min_ttl, deserialized.min_ttl);
    assert_eq!(config.max_ttl, deserialized.max_ttl);
    assert_eq!(config.integrity_check, deserialized.integrity_check);
    assert_eq!(config.interface, deserialized.interface);
    assert_eq!(config.src_ipv4_addr, deserialized.src_ipv4_addr);
    assert_eq!(config.src_ipv6_addr, deserialized.src_ipv6_addr);
    assert_eq!(config.packets, deserialized.packets);
    assert_eq!(config.probing_rate, deserialized.probing_rate);
    assert_eq!(
        config.rate_limiting_method,
        deserialized.rate_limiting_method
    );
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
