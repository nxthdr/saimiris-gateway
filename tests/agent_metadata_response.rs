use saimiris_gateway::probe::{AgentMetadata, SubmitProbesResponse};
use std::net::IpAddr;

#[test]
fn test_agent_metadata_in_response() {
    // Test that AgentMetadata can be serialized and includes IP address
    let agent_meta = AgentMetadata {
        id: "test-agent".to_string(),
        ip_address: Some("2001:db8::1".parse::<IpAddr>().unwrap()),
    };

    let response = SubmitProbesResponse {
        id: "test-measurement-id".to_string(),
        probes: 5,
        agents: vec![agent_meta],
    };

    // Serialize to JSON to verify the structure
    let json = serde_json::to_string(&response).unwrap();
    println!("Response JSON: {}", json);

    // Verify the JSON contains the IP address
    assert!(json.contains("2001:db8::1"));
    assert!(json.contains("test-agent"));
    assert!(json.contains("test-measurement-id"));

    // Deserialize back to verify round-trip
    let deserialized: SubmitProbesResponse = serde_json::from_str(&json).unwrap();
    assert_eq!(deserialized.id, "test-measurement-id");
    assert_eq!(deserialized.probes, 5);
    assert_eq!(deserialized.agents.len(), 1);
    assert_eq!(deserialized.agents[0].id, "test-agent");
    assert_eq!(
        deserialized.agents[0].ip_address,
        Some("2001:db8::1".parse::<IpAddr>().unwrap())
    );
}
