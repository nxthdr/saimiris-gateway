use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::str::FromStr;

/// Represents the protocol type for a probe
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub enum Protocol {
    ICMP,
    UDP,
    TCP,
}

impl FromStr for Protocol {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "icmp" => Ok(Protocol::ICMP),
            "udp" => Ok(Protocol::UDP),
            "tcp" => Ok(Protocol::TCP),
            _ => Err(format!("Invalid protocol: {}", s)),
        }
    }
}

impl std::fmt::Display for Protocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Protocol::ICMP => write!(f, "icmp"),
            Protocol::UDP => write!(f, "udp"),
            Protocol::TCP => write!(f, "tcp"),
        }
    }
}

/// Raw probe representation as an array for efficient deserialization
#[derive(Debug, Deserialize)]
#[serde(transparent)]
struct RawProbe(pub [serde_json::Value; 5]);

/// A probe represents a packet sent to a destination address
/// using a specific protocol at a given TTL
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(from = "RawProbe")]
pub struct Probe {
    pub dst_addr: IpAddr,
    pub src_port: u16,
    pub dst_port: u16,
    pub ttl: u8,
    pub protocol: Protocol,
}

// Efficient conversion from array representation
impl From<RawProbe> for Probe {
    fn from(raw: RawProbe) -> Self {
        // Extract and convert values from the raw array
        // Note: This will panic if conversion fails, but since this
        // is an internal conversion that's an acceptable tradeoff for performance

        // Parse IP address
        let dst_addr = match &raw.0[0] {
            serde_json::Value::String(s) => {
                // Handle string representation of IP
                IpAddr::from_str(s).unwrap_or_else(|_| panic!("Invalid IP address: {}", s))
            }
            // If deserialized as something else (unlikely), try to convert anyway
            v => serde_json::from_value(v.clone())
                .unwrap_or_else(|_| panic!("Invalid IP address format")),
        };

        // Extract ports and ttl
        let src_port: u16 = serde_json::from_value(raw.0[1].clone())
            .unwrap_or_else(|_| panic!("Invalid source port"));

        let dst_port: u16 = serde_json::from_value(raw.0[2].clone())
            .unwrap_or_else(|_| panic!("Invalid destination port"));

        let ttl: u8 =
            serde_json::from_value(raw.0[3].clone()).unwrap_or_else(|_| panic!("Invalid TTL"));

        // Parse protocol
        let protocol = match &raw.0[4] {
            serde_json::Value::String(s) => {
                Protocol::from_str(s).unwrap_or_else(|e| panic!("{}", e))
            }
            _ => panic!("Protocol must be a string"),
        };

        Probe {
            dst_addr,
            src_port,
            dst_port,
            ttl,
            protocol,
        }
    }
}

impl std::fmt::Display for Probe {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{},{},{},{},{}",
            self.dst_addr, self.src_port, self.dst_port, self.ttl, self.protocol
        )
    }
}

/// Agent metadata for a measurement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMetadata {
    pub id: String,
    pub ip_address: IpAddr,
    // Additional fields can be added as needed
}

/// Request structure for submitting probes
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProbesRequest {
    pub metadata: Vec<AgentMetadata>,
    pub probes: Vec<Probe>,
}

/// Response structure for submitted probes
#[derive(Debug, Serialize, Deserialize)]
pub struct SubmitProbesResponse {
    pub measurement_id: String,
    pub accepted_probes: usize,
    pub assigned_agents: Vec<String>,
}

/// Safer validation function for probes - returns errors instead of panicking
pub fn validate_probes(probes: &Vec<Probe>) -> Result<(), String> {
    // Perform additional validation beyond what happens during deserialization
    for (i, probe) in probes.iter().enumerate() {
        // Validate TTL
        if probe.ttl == 0 {
            return Err(format!("Probe {} has invalid TTL: 0", i));
        }

        // Validate port ranges
        if probe.src_port == 0 || probe.dst_port == 0 {
            return Err(format!("Probe {} has invalid port number", i));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{from_value, json};

    #[test]
    fn test_protocol_from_str() {
        assert_eq!(Protocol::from_str("icmp").unwrap(), Protocol::ICMP);
        assert_eq!(Protocol::from_str("udp").unwrap(), Protocol::UDP);
        assert_eq!(Protocol::from_str("tcp").unwrap(), Protocol::TCP);
        assert!(Protocol::from_str("invalid").is_err());
    }

    #[test]
    fn test_probe_display() {
        let probe = Probe {
            dst_addr: "192.168.1.1".parse().unwrap(),
            src_port: 12345,
            dst_port: 80,
            ttl: 64,
            protocol: Protocol::TCP,
        };
        assert_eq!(format!("{}", probe), "192.168.1.1,12345,80,64,tcp");
    }

    #[test]
    fn test_validate_probes() {
        let valid_probe = Probe {
            dst_addr: "192.168.1.1".parse().unwrap(),
            src_port: 12345,
            dst_port: 80,
            ttl: 64,
            protocol: Protocol::TCP,
        };
        let invalid_probe = Probe {
            dst_addr: "192.168.1.2".parse().unwrap(),
            src_port: 0, // Invalid port
            dst_port: 80,
            ttl: 64,
            protocol: Protocol::TCP,
        };
        let probes = vec![valid_probe.clone(), invalid_probe.clone()];

        assert!(validate_probes(&probes).is_err());
        assert!(validate_probes(&vec![valid_probe]).is_ok());
    }

    #[test]
    fn test_probe_deserialization() {
        // Test compact array format
        let probe_json = json!(["192.168.1.1", 12345, 53, 64, "tcp"]);
        let probe: Probe = from_value(probe_json).expect("Failed to deserialize probe");

        assert_eq!(probe.dst_addr, IpAddr::from_str("192.168.1.1").unwrap());
        assert_eq!(probe.src_port, 12345);
        assert_eq!(probe.dst_port, 53);
        assert_eq!(probe.ttl, 64);
        assert_eq!(probe.protocol.to_string(), "tcp");
    }

    #[test]
    fn test_request_deserialization() {
        let json_data = json!({
            "metadata": [{
                "id": "test-agent",
                "ip_address": "1.1.1.1"
            }],
            "probes": [
                ["192.168.1.1", 12345, 53, 64, "tcp"],
                ["8.8.8.8", 53, 80, 30, "udp"]
            ]
        });

        let request: SubmitProbesRequest =
            from_value(json_data).expect("Failed to deserialize request");

        assert_eq!(request.metadata.len(), 1);
        assert_eq!(request.metadata[0].id, "test-agent");
        assert_eq!(request.probes.len(), 2);
        assert_eq!(
            request.probes[0].dst_addr,
            IpAddr::from_str("192.168.1.1").unwrap()
        );
        assert_eq!(
            request.probes[1].dst_addr,
            IpAddr::from_str("8.8.8.8").unwrap()
        );
    }

    #[test]
    #[should_panic(expected = "Invalid IP address")]
    fn test_invalid_ip() {
        let probe_json = json!(["invalid-ip", 12345, 53, 64, "tcp"]);
        let _probe: Probe = from_value(probe_json).unwrap();
    }

    #[test]
    #[should_panic(expected = "Invalid protocol")]
    fn test_invalid_protocol() {
        let probe_json = json!(["1.1.1.1", 12345, 53, 64, "invalid"]);
        let _probe: Probe = from_value(probe_json).unwrap();
    }
}
