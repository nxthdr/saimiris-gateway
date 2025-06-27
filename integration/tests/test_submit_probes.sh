#!/usr/bin/env bash

# This script tests the probe submission endpoint,
# by sending a batch of probes.

# Setup
API_URL=${API_URL:-"http://0.0.0.0:8080/api"}

# Generate test payload
cat > test_payload.json <<EOL
{
  "metadata": [
    {
      "id": "testagent"
    }
  ],
  "probes": [
    ["1.1.1.1", 12345, 53, 30, "icmp"],
    ["8.8.8.8", 12345, 53, 30, "udp"],
    ["2001:4860:4860::8888", 12345, 53, 30, "icmp"]
  ]
}
EOL

echo "Sending test probe batch to $API_URL/probes..."

# Send the request
curl -X POST "$API_URL/probes" \
  -H "Content-Type: application/json" \
  -d @test_payload.json

# Clean up
rm test_payload.json
