use criterion::{BenchmarkId, Criterion, black_box, criterion_group, criterion_main};
use serde_json::{Value, json};

// Import the types from saimiris-gateway that we want to benchmark
use saimiris_gateway::probe::{SubmitProbesRequest, validate_probes};

/// Generate a test payload with the specified number of probes
fn generate_test_payload(num_probes: usize) -> Value {
    // Generate the metadata
    let metadata = json!([{
        "id": "test-agent",
        "ip_address": "1.1.1.1"
    }]);

    // Generate the probes
    let mut probes = Vec::with_capacity(num_probes);
    for i in 0..num_probes {
        // Alternate between different IPs and protocols for variety
        let ip = if i % 2 == 0 { "1.1.1.1" } else { "8.8.8.8" };
        let protocol = if i % 3 == 0 {
            "icmp"
        } else if i % 3 == 1 {
            "udp"
        } else {
            "tcp"
        };
        let ttl = 20 + (i % 64) as u8; // TTLs between 20 and 83

        probes.push(json!([ip, 12345, 53, ttl, protocol]));
    }

    json!({
        "metadata": metadata,
        "probes": probes
    })
}

fn bench_probe_deserialization(c: &mut Criterion) {
    // Create a benchmark group
    let mut group = c.benchmark_group("Probe Deserialization");

    // Test with different numbers of probes
    for size in [10, 100, 1000, 10000].iter() {
        group.bench_with_input(BenchmarkId::from_parameter(size), size, |b, &size| {
            // Generate a test payload with the specified number of probes
            let payload = generate_test_payload(size);
            let json_str = serde_json::to_string(&payload).unwrap();

            b.iter(|| {
                // This is the operation we're benchmarking
                let request: SubmitProbesRequest =
                    serde_json::from_str(&black_box(&json_str)).unwrap();

                // Return the length to ensure the compiler doesn't optimize away the deserialization
                black_box(request.probes.len())
            });
        });
    }

    group.finish();
}

fn compare_array_vs_object_format(c: &mut Criterion) {
    // Create a benchmark group for the comparison
    let mut group = c.benchmark_group("Array vs Object Format");

    // Probe count for comparison
    let count = 1000;

    // Generate array-format probes
    let array_format = generate_test_payload(count);
    let array_json = serde_json::to_string(&array_format).unwrap();

    // Generate the same payload in object format
    let mut object_probes = Vec::with_capacity(count);
    for i in 0..count {
        let ip = if i % 2 == 0 { "1.1.1.1" } else { "8.8.8.8" };
        let protocol = if i % 3 == 0 {
            "icmp"
        } else if i % 3 == 1 {
            "udp"
        } else {
            "tcp"
        };
        let ttl = 20 + (i % 64) as u8;

        object_probes.push(json!({
            "dst_addr": ip,
            "src_port": 12345,
            "dst_port": 53,
            "ttl": ttl,
            "protocol": protocol
        }));
    }

    let object_format = json!({
        "metadata": [{
            "id": "test-agent",
            "ip_address": "1.1.1.1"
        }],
        "probes": object_probes
    });

    let object_json = serde_json::to_string(&object_format).unwrap();

    // Compare the payload sizes
    println!("Array format size: {} bytes", array_json.len());
    println!("Object format size: {} bytes", object_json.len());
    println!(
        "Size reduction: {}%",
        ((object_json.len() - array_json.len()) as f64 / object_json.len() as f64 * 100.0).round()
    );

    // Benchmark array format deserialization
    group.bench_function("Array Format", |b| {
        b.iter(|| {
            // Parse using our custom array format deserializer
            let _: SubmitProbesRequest = serde_json::from_str(black_box(&array_json)).unwrap();
        });
    });

    group.finish();
}

fn benchmark_validation_overhead(c: &mut Criterion) {
    // Create a benchmark group for validation overhead
    let mut group = c.benchmark_group("Validation Overhead");

    // Test with different numbers of probes
    for size in [100, 1000].iter() {
        // Generate valid test data
        let payload = generate_test_payload(*size);
        let json_str = serde_json::to_string(&payload).unwrap();

        group.bench_with_input(
            BenchmarkId::new("Deserialization Only", size),
            size,
            |b, _| {
                b.iter(|| {
                    // Just deserialize without validation
                    let request: SubmitProbesRequest =
                        serde_json::from_str(black_box(&json_str)).unwrap();
                    black_box(request.probes.len())
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("With Explicit Validation", size),
            size,
            |b, _| {
                b.iter(|| {
                    // Deserialize and validate
                    let request: SubmitProbesRequest =
                        serde_json::from_str(black_box(&json_str)).unwrap();
                    validate_probes(&request.probes).unwrap();
                    black_box(request.probes.len())
                });
            },
        );
    }

    group.finish();
}

fn benchmark_probe_validation(c: &mut Criterion) {
    let mut group = c.benchmark_group("Probe Validation");

    // Generate a small payload with valid probes
    let valid_payload = generate_test_payload(10);
    let valid_json = serde_json::to_string(&valid_payload).unwrap();

    // Test validation time for a batch of valid probes
    group.bench_function("10 Valid Probes", |b| {
        b.iter(|| {
            let request: SubmitProbesRequest =
                serde_json::from_str(black_box(&valid_json)).unwrap();
            black_box(validate_probes(&request.probes))
        });
    });

    // Test validation time for probes with invalid TTL
    // Instead of invalid IP or protocol (which would panic), use invalid TTL
    // which our validate_probes function will catch
    let invalid_ttl_payload = json!({
        "metadata": [{
            "id": "test-agent",
            "ip_address": "1.1.1.1"
        }],
        "probes": [
            ["1.1.1.1", 12345, 53, 0, "icmp"], // TTL of 0 is invalid
            ["8.8.8.8", 12345, 53, 0, "udp"],  // TTL of 0 is invalid
            ["2001:db8::1", 12345, 53, 1, "tcp"]
        ]
    });
    let invalid_ttl_json = serde_json::to_string(&invalid_ttl_payload).unwrap();

    group.bench_function("Invalid TTL Validation", |b| {
        b.iter(|| {
            let request: SubmitProbesRequest =
                serde_json::from_str(black_box(&invalid_ttl_json)).unwrap();
            // This should return an error but not panic
            black_box(validate_probes(&request.probes).is_err())
        });
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_probe_deserialization,
    compare_array_vs_object_format,
    benchmark_validation_overhead,
    benchmark_probe_validation
);
criterion_main!(benches);
