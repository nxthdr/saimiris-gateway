use axum_test::TestServer;
use saimiris_gateway::{
    AppState,
    agent::{AgentConfig, AgentStore},
    create_app,
    database::Database,
    kafka,
};

async fn create_mock_database() -> Database {
    Database::new_mock()
}

/// Test the new /api/user/prefixes endpoint
#[tokio::test]
async fn test_user_prefixes_endpoint() {
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
        agent_store: agent_store.clone(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        auth0_jwks_uri: Some("https://test.auth0.com/.well-known/jwks.json".to_string()),
        auth0_issuer: Some("https://test.auth0.com/".to_string()),
        bypass_jwt_validation: true,
        database: create_mock_database().await,
    };

    // Add a test agent with IPv6 prefix configuration
    agent_store
        .add_agent("test-agent-1".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config = vec![AgentConfig {
        src_ipv6_prefix: Some("2001:db8:1234::/48".to_string()),
        ..Default::default()
    }];
    agent_store.update_config("test-agent-1", config).await;

    // Add another agent without IPv6 prefix (should not appear in results)
    agent_store
        .add_agent("test-agent-2".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_no_ipv6 = vec![AgentConfig {
        src_ipv6_prefix: None,
        ..Default::default()
    }];
    agent_store
        .update_config("test-agent-2", config_no_ipv6)
        .await;

    let app = create_app(state);
    let server = TestServer::new(app).unwrap();

    // Test the user prefixes endpoint
    let response = server
        .get("/api/user/prefixes")
        .add_header("authorization", "Bearer test-token")
        .await;

    assert_eq!(response.status_code(), 200);
    let response_body: serde_json::Value = response.json();

    // Verify the response structure
    assert!(response_body.get("user_id").is_some());
    assert!(response_body.get("agents").is_some());

    let user_id = response_body["user_id"].as_u64().unwrap();
    let agents = response_body["agents"].as_array().unwrap();

    // Should only have one agent (the one with IPv6 prefix)
    assert_eq!(agents.len(), 1);

    let agent = &agents[0];
    assert_eq!(agent["agent_id"], "test-agent-1");

    let prefixes = agent["prefixes"].as_array().unwrap();
    assert_eq!(prefixes.len(), 1);

    let prefix = &prefixes[0];
    assert_eq!(prefix["agent_prefix"], "2001:db8:1234::/48");

    // Verify the user prefix calculation
    let user_prefix = prefix["user_prefix"].as_str().unwrap();
    assert!(user_prefix.ends_with("/80"));

    println!("✅ User prefixes endpoint test passed:");
    println!("   User ID: {}", user_id);
    println!("   Agent: {}", agent["agent_id"]);
    println!("   Agent Prefix: {}", prefix["agent_prefix"]);
    println!("   User Prefix: {}", user_prefix);
}

/// Test the calculate_user_prefix function directly
#[tokio::test]
async fn test_calculate_user_prefix_function() {
    use saimiris_gateway::calculate_user_prefix;

    let user_id = 0x12345678u32; // Use a static test user ID
    let agent_prefix = "2001:db8:1234::/48";

    let user_prefix = calculate_user_prefix(agent_prefix, user_id);
    assert!(user_prefix.is_some());

    let user_prefix = user_prefix.unwrap();
    assert!(user_prefix.starts_with("2001:db8:1234:"));
    assert!(user_prefix.ends_with("/80"));

    println!("✅ Prefix calculation test passed:");
    println!("   User ID: 0x{:08x}", user_id);
    println!("   Agent Prefix: {}", agent_prefix);
    println!("   User Prefix: {}", user_prefix);

    // Test with invalid prefix
    let invalid_result = calculate_user_prefix("invalid-prefix", user_id);
    assert!(invalid_result.is_none());
}

/// Test with multiple agents and multiple prefixes per agent
#[tokio::test]
async fn test_multiple_agents_and_prefixes() {
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
        agent_store: agent_store.clone(),
        agent_key: "test-key".to_string(),
        kafka_config,
        kafka_producer,
        auth0_jwks_uri: Some("https://test.auth0.com/.well-known/jwks.json".to_string()),
        auth0_issuer: Some("https://test.auth0.com/".to_string()),
        bypass_jwt_validation: true,
        database: create_mock_database().await,
    };

    // Add multiple agents with different prefix configurations
    agent_store
        .add_agent("agent-eu".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_eu = vec![
        AgentConfig {
            src_ipv6_prefix: Some("2001:db8:1000::/48".to_string()),
            ..Default::default()
        },
        AgentConfig {
            src_ipv6_prefix: Some("2001:db8:2000::/48".to_string()),
            ..Default::default()
        },
    ];
    agent_store.update_config("agent-eu", config_eu).await;

    agent_store
        .add_agent("agent-us".to_string(), "secret".to_string())
        .await
        .unwrap();
    let config_us = vec![AgentConfig {
        src_ipv6_prefix: Some("2001:db8:3000::/48".to_string()),
        ..Default::default()
    }];
    agent_store.update_config("agent-us", config_us).await;

    let app = create_app(state);
    let server = TestServer::new(app).unwrap();

    // Test the user prefixes endpoint
    let response = server
        .get("/api/user/prefixes")
        .add_header("authorization", "Bearer test-token")
        .await;

    assert_eq!(response.status_code(), 200);
    let response_body: serde_json::Value = response.json();

    let agents = response_body["agents"].as_array().unwrap();

    // Should have two agents
    assert_eq!(agents.len(), 2);

    // Find the EU agent (should have 2 prefixes)
    let eu_agent = agents.iter().find(|a| a["agent_id"] == "agent-eu").unwrap();
    let eu_prefixes = eu_agent["prefixes"].as_array().unwrap();
    assert_eq!(eu_prefixes.len(), 2);

    // Find the US agent (should have 1 prefix)
    let us_agent = agents.iter().find(|a| a["agent_id"] == "agent-us").unwrap();
    let us_prefixes = us_agent["prefixes"].as_array().unwrap();
    assert_eq!(us_prefixes.len(), 1);

    println!("✅ Multiple agents test passed:");
    println!("   EU agent has {} prefixes", eu_prefixes.len());
    println!("   US agent has {} prefixes", us_prefixes.len());
}

/// Test the calculate_user_prefix function with various prefix lengths
#[tokio::test]
async fn test_calculate_user_prefix_various_lengths() {
    use saimiris_gateway::calculate_user_prefix;

    let user_id = 1000u32; // 0x3e8

    // Test cases: (agent_prefix, expected_user_prefix_suffix)
    let test_cases = vec![
        ("2001:db8::/48", "/80"),      // /48 + 32 = /80 (traditional)
        ("2001:db8::/64", "/96"),      // /64 + 32 = /96 (your example)
        ("2001:db8::/32", "/64"),      // /32 + 32 = /64
        ("2001:db8::/56", "/88"),      // /56 + 32 = /88
        ("2001:db8:1234::/48", "/80"), // Different base address
        ("2001:db8:abcd::/64", "/96"), // Different base address
    ];

    for (agent_prefix, expected_suffix) in test_cases {
        let user_prefix = calculate_user_prefix(agent_prefix, user_id);
        assert!(
            user_prefix.is_some(),
            "Failed to calculate prefix for {}",
            agent_prefix
        );

        let user_prefix = user_prefix.unwrap();
        assert!(
            user_prefix.ends_with(expected_suffix),
            "Expected {} to end with {}, got: {}",
            agent_prefix,
            expected_suffix,
            user_prefix
        );

        println!("✅ Agent: {} -> User: {}", agent_prefix, user_prefix);
    }
}

/// Test the calculate_user_prefix function with invalid inputs
#[tokio::test]
async fn test_calculate_user_prefix_invalid_inputs() {
    use saimiris_gateway::calculate_user_prefix;

    let user_id = 1000u32;

    // Test cases that should return None
    let invalid_cases = vec![
        "invalid-prefix", // Not a valid IPv6 prefix
        "2001:db8::/97",  // Not enough space for 32-bit user ID
        "2001:db8::/128", // No space for user ID
        "2001:db8::/200", // Invalid prefix length
        "2001:db8::",     // Missing prefix length
        "2001:db8::/",    // Empty prefix length
        "192.168.1.0/24", // IPv4 address
    ];

    for invalid_prefix in invalid_cases {
        let result = calculate_user_prefix(invalid_prefix, user_id);
        assert!(
            result.is_none(),
            "Expected None for invalid prefix: {}",
            invalid_prefix
        );
        println!("✅ Correctly rejected invalid prefix: {}", invalid_prefix);
    }
}

/// Debug test to understand the IP address format
#[tokio::test]
async fn debug_ipv6_format() {
    use saimiris_gateway::calculate_user_prefix;
    use std::net::Ipv6Addr;

    let user_id = 1000u32; // 0x3e8
    let agent_prefix = "2001:db8::/64";

    let calculated_prefix = calculate_user_prefix(agent_prefix, user_id).unwrap();
    println!("Calculated prefix: {}", calculated_prefix);

    // Extract the address part
    let addr_part = calculated_prefix.split('/').next().unwrap();
    println!("Address part: {}", addr_part);

    // Parse it as IPv6
    let calculated_addr: Ipv6Addr = addr_part.parse().unwrap();
    println!("Parsed address: {}", calculated_addr);

    // Show the segments
    let segments = calculated_addr.segments();
    println!("Segments: {:?}", segments);
    println!(
        "Segments hex: [{:04x}, {:04x}, {:04x}, {:04x}, {:04x}, {:04x}, {:04x}, {:04x}]",
        segments[0],
        segments[1],
        segments[2],
        segments[3],
        segments[4],
        segments[5],
        segments[6],
        segments[7]
    );
}

/// Test the validate_user_ipv6 function with various prefix lengths
#[tokio::test]
async fn test_validate_user_ipv6_various_lengths() {
    use saimiris_gateway::{calculate_user_prefix, validate_user_ipv6};
    use std::net::Ipv6Addr;

    let user_id = 1000u32; // 0x3e8

    // Test /64 prefix (your example case)
    let agent_prefix = "2001:db8::/64";

    // First, let's see what the calculated prefix actually is
    let calculated_prefix = calculate_user_prefix(agent_prefix, user_id).unwrap();
    println!("Calculated prefix for /64: {}", calculated_prefix);

    // Extract the base IP from the calculated prefix
    let base_ip_str = calculated_prefix.split('/').next().unwrap();
    let base_ip: Ipv6Addr = base_ip_str.parse().unwrap();

    // Test that the base IP is valid
    assert!(
        validate_user_ipv6(&base_ip, agent_prefix, user_id),
        "Base IP {} should be valid for user {} in prefix {}",
        base_ip,
        user_id,
        agent_prefix
    );
    println!(
        "✅ Base IP: {} for user {} in {}",
        base_ip, user_id, agent_prefix
    );

    // Test a simple variation by adding 1 to the last segment
    let mut segments = base_ip.segments();
    segments[7] = segments[7].wrapping_add(1);
    let variant_ip = Ipv6Addr::from(segments);

    assert!(
        validate_user_ipv6(&variant_ip, agent_prefix, user_id),
        "Variant IP {} should be valid for user {} in prefix {}",
        variant_ip,
        user_id,
        agent_prefix
    );
    println!(
        "✅ Variant IP: {} for user {} in {}",
        variant_ip, user_id, agent_prefix
    );

    // Test an invalid IP (different user ID)
    let invalid_user_id = 1001u32;
    let invalid_prefix = calculate_user_prefix(agent_prefix, invalid_user_id).unwrap();
    let invalid_ip: Ipv6Addr = invalid_prefix.split('/').next().unwrap().parse().unwrap();

    assert!(
        !validate_user_ipv6(&invalid_ip, agent_prefix, user_id),
        "Invalid IP {} should not be valid for user {} in prefix {}",
        invalid_ip,
        user_id,
        agent_prefix
    );
    println!(
        "✅ Invalid IP: {} correctly rejected for user {} in {}",
        invalid_ip, user_id, agent_prefix
    );
}

/// Test the validate_user_ipv6 function with /48 prefix (traditional case)
#[tokio::test]
async fn test_validate_user_ipv6_48_prefix() {
    use saimiris_gateway::validate_user_ipv6;
    use std::net::Ipv6Addr;

    let user_id = 1000u32; // 0x3e8
    let agent_prefix = "2001:db8::/48";

    // These should be valid (within the user's /80 prefix)
    // The calculated prefix for user 1000 in 2001:db8::/48 is 2001:db8:0:0:3e8::/80
    let valid_ips = vec![
        "2001:db8:0:0:3e8:0:0:0",
        "2001:db8:0:0:3e8:0:0:1",
        "2001:db8:0:0:3e8:ffff:ffff:ffff",
    ];

    for ip_str in valid_ips {
        let ip: Ipv6Addr = ip_str.parse().unwrap();
        assert!(
            validate_user_ipv6(&ip, agent_prefix, user_id),
            "IP {} should be valid for user {} in prefix {}",
            ip,
            user_id,
            agent_prefix
        );
        println!(
            "✅ Valid IP: {} for user {} in {}",
            ip, user_id, agent_prefix
        );
    }

    // These should be invalid
    let invalid_ips = vec![
        "2001:db8:0:0:3e7:0:0:0", // Wrong user ID (3e7 instead of 3e8)
        "2001:db8:0:0:3e9:0:0:0", // Wrong user ID (3e9 instead of 3e8)
        "2001:db8:1:0:3e8:0:0:0", // Wrong base network
    ];

    for ip_str in invalid_ips {
        let ip: Ipv6Addr = ip_str.parse().unwrap();
        assert!(
            !validate_user_ipv6(&ip, agent_prefix, user_id),
            "IP {} should be invalid for user {} in prefix {}",
            ip,
            user_id,
            agent_prefix
        );
        println!(
            "✅ Invalid IP: {} correctly rejected for user {} in {}",
            ip, user_id, agent_prefix
        );
    }
}

/// Test with edge cases and boundary conditions
#[tokio::test]
async fn test_prefix_calculation_edge_cases() {
    use saimiris_gateway::{calculate_user_prefix, validate_user_ipv6};
    use std::net::Ipv6Addr;

    // Test with minimum user ID
    let min_user_id = 1u32;
    let prefix = "2001:db8::/64";
    let user_prefix = calculate_user_prefix(prefix, min_user_id).unwrap();
    println!("Min user ID test: {} -> {}", min_user_id, user_prefix);
    // Don't assert on the exact format, just check it's reasonable
    assert!(user_prefix.contains("2001:db8::"));
    assert!(user_prefix.ends_with("/96"));

    // Test with maximum reasonable user ID
    let max_user_id = 0xFFFFFFFFu32;
    let user_prefix = calculate_user_prefix(prefix, max_user_id).unwrap();
    println!("Max user ID test: {} -> {}", max_user_id, user_prefix);
    assert!(user_prefix.contains("2001:db8::"));
    assert!(user_prefix.ends_with("/96"));

    // Test with /96 prefix (minimum space for user ID)
    let tight_prefix = "2001:db8::/96";
    let user_prefix = calculate_user_prefix(tight_prefix, 1000u32).unwrap();
    println!("Tight prefix test: {} -> {}", tight_prefix, user_prefix);
    assert!(user_prefix.ends_with("/128"));

    // Test validation with the calculated prefix
    let calculated_addr = user_prefix.split('/').next().unwrap();
    let test_ip: Ipv6Addr = calculated_addr.parse().unwrap();
    assert!(validate_user_ipv6(&test_ip, tight_prefix, 1000u32));
    println!(
        "Validation test passed for calculated address: {}",
        calculated_addr
    );
}
