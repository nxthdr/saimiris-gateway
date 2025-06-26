use serde_json::json;
use std::fs::File;
use std::io::Write;

fn main() -> std::io::Result<()> {
    // Number of probes to generate in the file
    let num_probes = 10_000; // Change this number as needed

    // Generate sample metadata
    let metadata = json!([{
        "id": "test-agent-1",
        "ip_address": "192.168.1.1"
    }]);

    // Generate probe data
    let mut probes = Vec::with_capacity(num_probes);
    for i in 0..num_probes {
        // Generate some variety in the probes
        let ip = match i % 5 {
            0 => "1.1.1.1",
            1 => "8.8.8.8",
            2 => "9.9.9.9",
            3 => "2001:db8::1",
            _ => "192.168.1.1",
        };
        let src_port = 10000 + (i % 20000) as u16;
        let dst_port = match i % 4 {
            0 => 53,  // DNS
            1 => 80,  // HTTP
            2 => 443, // HTTPS
            _ => 22,  // SSH
        };
        let ttl = 20 + (i % 64) as u8;
        let protocol = match i % 3 {
            0 => "icmp",
            1 => "udp",
            _ => "tcp",
        };

        // Add the probe in array format
        probes.push(json!([ip, src_port, dst_port, ttl, protocol]));
    }

    // Create the full request payload
    let payload = json!({
        "metadata": metadata,
        "probes": probes
    });

    let file_name = format!("test_probes_{}.json", num_probes);
    let mut file = File::create(&file_name)?;
    let json = serde_json::to_string(&payload)?;
    file.write_all(json.as_bytes())?;

    println!("Generated '{}' with {} probes", file_name, num_probes);
    println!("Compact JSON size: {} bytes", json.len());

    Ok(())
}
