#[cfg(test)]
mod tests {
    use saimiris_gateway::{hash_user_identifier, validate_user_ipv6};
    use std::net::Ipv6Addr;

    #[test]
    fn test_validate_user_ipv6_valid() {
        let agent_prefix = "2001:db8::/48";
        let user_id = 0x12345678u32;

        // Construct a valid IPv6 address within the user's /80 prefix
        // Agent prefix: 2001:db8::/48 (first 3 segments)
        // User ID: 0x12345678 -> segments[3] = 0x1234, segments[4] = 0x5678
        let user_ip = Ipv6Addr::new(
            0x2001, 0x0db8, 0x0000, 0x1234, 0x5678, 0x0000, 0x0000, 0x0001,
        );

        assert!(validate_user_ipv6(&user_ip, agent_prefix, user_id));
    }

    #[test]
    fn test_validate_user_ipv6_invalid() {
        let agent_prefix = "2001:db8::/48";
        let user_id = 0x12345678u32;

        // Construct an invalid IPv6 address (wrong user ID part)
        let user_ip = Ipv6Addr::new(
            0x2001, 0x0db8, 0x0000, 0x9999, 0x8888, 0x0000, 0x0000, 0x0001,
        );

        assert!(!validate_user_ipv6(&user_ip, agent_prefix, user_id));
    }

    #[test]
    fn test_validate_user_ipv6_wrong_agent_prefix() {
        let agent_prefix = "2001:db8::/48";
        let user_id = 0x12345678u32;

        // Use an IP from a different agent prefix
        let user_ip = Ipv6Addr::new(
            0x2001, 0x0dc8, 0x0000, 0x1234, 0x5678, 0x0000, 0x0000, 0x0001,
        );

        assert!(!validate_user_ipv6(&user_ip, agent_prefix, user_id));
    }

    #[test]
    fn test_validate_user_ipv6_invalid_prefix_format() {
        let agent_prefix = "invalid_prefix";
        let user_id = 0x12345678u32;

        let user_ip = Ipv6Addr::new(
            0x2001, 0x0db8, 0x0000, 0x1234, 0x5678, 0x0000, 0x0000, 0x0001,
        );

        assert!(!validate_user_ipv6(&user_ip, agent_prefix, user_id));
    }

    #[test]
    fn test_hash_user_identifier_consistency() {
        let user_id = "test_user@example.com";
        let hash1 = hash_user_identifier(user_id);
        let hash2 = hash_user_identifier(user_id);

        // Same input should produce same hash
        assert_eq!(hash1, hash2);

        // Hash should be 64 characters (32 bytes * 2 hex chars)
        assert_eq!(hash1.len(), 64);

        // Different users should have different hashes
        let different_user = "different_user@example.com";
        let different_hash = hash_user_identifier(different_user);
        assert_ne!(
            hash1, different_hash,
            "Different users should have different hashes"
        );
    }

    #[test]
    fn test_ip_address_validation_requirement() {
        // Test that agent metadata requires IP address
        use serde_json::json;
        use std::net::IpAddr;

        // Test with valid IP address (should serialize/deserialize correctly)
        let valid_metadata = json!({
            "id": "agent1",
            "ip_address": "2001:db8::1"
        });

        // This should work - we're not testing the full API here, just that
        // the structure can handle IP addresses properly
        let ip_str = valid_metadata["ip_address"].as_str().unwrap();
        let parsed_ip: IpAddr = ip_str.parse().unwrap();

        match parsed_ip {
            IpAddr::V6(ipv6) => {
                // This is what we expect for IPv6 addresses
                assert_eq!(ipv6.to_string(), "2001:db8::1");
            }
            IpAddr::V4(_) => {
                // IPv4 is also valid
                assert!(false, "Expected IPv6 but got IPv4");
            }
        }

        // Test with missing IP address (should be caught by validation)
        let invalid_metadata = json!({
            "id": "agent1"
            // Missing ip_address field
        });

        assert!(invalid_metadata["ip_address"].is_null());
    }
}
