#!/bin/bash
set -e

GATEWAY_URL="http://127.0.0.1:8080"
AGENT_ID="testagent"
AGENT_IP="2001:db8::c95b:3f80:0:1"

echo "🚀 Gateway Probe Pipeline Test (Agent in Dry-Run Mode)"
echo "========================================================"
echo ""
echo "Note: Agent runs in dry-run mode (no actual probe transmission)."
echo "This test verifies the gateway's probe handling pipeline."
echo ""

# Test 1: Gateway health
echo "[1/6] Gateway health check..."
if ! curl -4 -sf "$GATEWAY_URL/api/agents" > /dev/null; then
    echo "❌ Gateway not responding"
    exit 1
fi
echo "✅ Gateway is healthy"
echo ""

# Test 2: Agent registration
echo "[2/6] Verifying agent registration..."
AGENTS=$(curl -4 -s "$GATEWAY_URL/api/agents")
if ! echo "$AGENTS" | grep -q "$AGENT_ID"; then
    echo "❌ Agent '$AGENT_ID' not registered"
    echo "Available agents: $AGENTS"
    exit 1
fi
echo "✅ Agent '$AGENT_ID' is registered"
echo ""

# Test 3: Get initial usage stats
echo "[3/6] Recording initial usage..."
INITIAL_USAGE=$(curl -4 -s "$GATEWAY_URL/api/user/me")
INITIAL_SUBMISSIONS=$(echo "$INITIAL_USAGE" | grep -o '"submission_count":[0-9]*' | cut -d':' -f2)
INITIAL_PROBES=$(echo "$INITIAL_USAGE" | grep -o '"used":[0-9]*' | cut -d':' -f2)
echo "   Submissions: $INITIAL_SUBMISSIONS, Probes: $INITIAL_PROBES"
echo ""

# Test 4: Submit probes
echo "[4/6] Submitting probes..."
PROBE_PAYLOAD='{
  "metadata": [{
    "id": "'"$AGENT_ID"'",
    "ip_address": "'"$AGENT_IP"'"
  }],
  "probes": [
    ["2001:db8::1", 12345, 33434, 64, "icmpv6"],
    ["2001:db8::2", 12345, 33434, 64, "icmpv6"]
  ]
}'

RESPONSE=$(curl -4 -s -X POST "$GATEWAY_URL/api/probes" \
    -H "Content-Type: application/json" \
    -d "$PROBE_PAYLOAD")

MEASUREMENT_ID=$(echo "$RESPONSE" | grep -o '"id":"[^"]*' | cut -d'"' -f4)
if [[ -z "$MEASUREMENT_ID" ]]; then
    echo "❌ Probe submission failed"
    echo "Response: $RESPONSE"
    exit 1
fi
echo "✅ Probes submitted (measurement: $MEASUREMENT_ID)"
echo ""

# Test 5: Verify probe processing
echo "[5/6] Verifying probe processing (20 seconds)..."
sleep 20

echo "   Checking agent logs..."
AGENT_LOGS=$(docker logs integration-agent-1 2>&1 | tail -100)

PIPELINE_OK=true

if echo "$AGENT_LOGS" | grep -qi "received.*probe\|message.*received\|deserialized"; then
    echo "   ✅ Agent received probes from Kafka"
else
    echo "   ❌ No evidence of Kafka message reception"
    PIPELINE_OK=false
fi

if echo "$AGENT_LOGS" | grep -qi "distributing.*probes"; then
    echo "   ✅ Agent processed and queued probes"
else
    echo "   ❌ No evidence of probe processing"
    PIPELINE_OK=false
fi
echo ""

# Test 6: Verify usage stats updated
echo "[6/6] Verifying usage statistics..."
FINAL_USAGE=$(curl -4 -s "$GATEWAY_URL/api/user/me")
FINAL_SUBMISSIONS=$(echo "$FINAL_USAGE" | grep -o '"submission_count":[0-9]*' | cut -d':' -f2)
FINAL_PROBES=$(echo "$FINAL_USAGE" | grep -o '"used":[0-9]*' | cut -d':' -f2)

if [[ $FINAL_SUBMISSIONS -gt $INITIAL_SUBMISSIONS ]] && [[ $FINAL_PROBES -gt $INITIAL_PROBES ]]; then
    echo "✅ Usage updated: submissions +$((FINAL_SUBMISSIONS - INITIAL_SUBMISSIONS)), probes +$((FINAL_PROBES - INITIAL_PROBES))"
else
    echo "❌ Usage not updated correctly"
    PIPELINE_OK=false
fi
echo ""

# Summary
echo "========================================================"
if [[ "$PIPELINE_OK" = true ]]; then
    echo "🎉 SUCCESS: Gateway pipeline working!"
    echo ""
    echo "Verified:"
    echo "  ✅ Gateway API"
    echo "  ✅ Agent registration"
    echo "  ✅ Probe submission & validation"
    echo "  ✅ Kafka message delivery"
    echo "  ✅ Agent probe processing"
    echo "  ✅ Usage tracking & limits"
    echo ""
    echo "ℹ️  Agent runs in dry-run mode (no actual network probes sent)"
    exit 0
else
    echo "❌ FAILURE: Pipeline has issues"
    echo ""
    echo "Recent agent logs:"
    echo "$AGENT_LOGS" | tail -20
    exit 1
fi
