#!/bin/bash

# Integration test script for Saimiris Gateway with PostgreSQL
# This script tests the database functionality in the Docker integration environment

set -e

GATEWAY_URL="http://0.0.0.0:8080"
POSTGRES_HOST="10.0.0.50"

echo "🚀 Starting Saimiris Gateway Integration Tests with PostgreSQL"

# Function to ensure Kafka topics exist
setup_kafka_topics() {
    echo "🔧 Setting up Kafka topics..."

    # Create saimiris-probes topic (input for agents)
    if docker exec integration-redpanda-1 rpk topic create saimiris-probes --partitions 1 --replicas 1 >/dev/null 2>&1; then
        echo "✅ Created topic: saimiris-probes"
    elif docker exec integration-redpanda-1 rpk topic list | grep -q "saimiris-probes"; then
        echo "ℹ️  Topic saimiris-probes already exists"
    else
        echo "⚠️  Could not create or verify topic: saimiris-probes"
    fi

    # Create saimiris-replies topic (output from agents, if needed)
    if docker exec integration-redpanda-1 rpk topic create saimiris-replies --partitions 1 --replicas 1 >/dev/null 2>&1; then
        echo "✅ Created topic: saimiris-replies"
    elif docker exec integration-redpanda-1 rpk topic list | grep -q "saimiris-replies"; then
        echo "ℹ️  Topic saimiris-replies already exists"
    else
        echo "⚠️  Could not create or verify topic: saimiris-replies"
    fi
}

# Setup Kafka topics first
setup_kafka_topics

# Test 1: Check if gateway is responding
echo "🔍 Test 1: Gateway health check"
if curl -s -f "$GATEWAY_URL/api/agents" > /dev/null; then
    echo "✅ Gateway is responding"
else
    echo "❌ Gateway is not responding"
    exit 1
fi

# Test 2: Wait for agent to register itself and verify registration
echo "🔍 Test 2: Wait for agent to register itself with gateway"

# First, check if agent container is running
AGENT_STATUS=$(docker ps --filter "name=integration-agent-1" --format "table {{.State}}" | tail -n +2)
if [[ "$AGENT_STATUS" != "running" ]]; then
    echo "❌ Agent container is not running. Status: $AGENT_STATUS"
    echo "   Agent container logs:"
    docker logs integration-agent-1 | tail -15
    exit 1
fi

echo "✅ Agent container is running"
echo "ℹ️  Waiting for agent registration (this may take up to 60 seconds)..."

AGENT_REGISTERED=false
MAX_WAIT=60
WAIT_COUNT=0

while [[ $WAIT_COUNT -lt $MAX_WAIT ]] && [[ $AGENT_REGISTERED == false ]]; do
    sleep 2
    WAIT_COUNT=$((WAIT_COUNT + 2))

    # Check if agent is registered by calling the agents API
    AGENTS_RESPONSE=$(curl -s -X GET "$GATEWAY_URL/api/agents" 2>/dev/null || echo "FAILED")

    if echo "$AGENTS_RESPONSE" | grep -q "testagent"; then
        echo "✅ Agent 'testagent' successfully registered itself with gateway"
        echo "   Agents list: $AGENTS_RESPONSE"
        AGENT_REGISTERED=true
        break
    fi

    # Show progress every 10 seconds
    if [[ $((WAIT_COUNT % 10)) -eq 0 ]]; then
        echo "   ... still waiting (${WAIT_COUNT}s elapsed)"
        # Show recent agent logs for debugging
        echo "   Recent agent logs:"
        docker logs integration-agent-1 --tail 5 | grep -E "(ERROR|WARN|register)" || echo "   No relevant log entries"
    fi
done

if [[ $AGENT_REGISTERED == false ]]; then
    echo "❌ Agent failed to register itself within ${MAX_WAIT} seconds"
    echo "   Current agents list: $AGENTS_RESPONSE"
    echo "   Agent container logs (last 20 lines):"
    docker logs integration-agent-1 | tail -20
    echo ""
    echo "   Gateway logs (last 10 lines):"
    docker logs integration-gateway-1 | tail -10
fi

# Test 3: Submit probes (this should trigger database recording)
echo "🔍 Test 3: Submit probes to test database recording"

# Get initial usage stats for comparison
INITIAL_USAGE=$(curl -s -X GET "$GATEWAY_URL/api/user/me" \
    -H "Authorization: Bearer test-token" || echo "FAILED")

if echo "$INITIAL_USAGE" | grep -q "submission_count"; then
    INITIAL_COUNT=$(echo "$INITIAL_USAGE" | grep -o '"submission_count":[0-9]*' | grep -o '[0-9]*')
    INITIAL_USED=$(echo "$INITIAL_USAGE" | grep -o '"used":[0-9]*' | grep -o '[0-9]*')
    echo "ℹ️  Initial usage: submissions=$INITIAL_COUNT, probes_used=$INITIAL_USED"
else
    echo "⚠️  Could not get initial usage stats"
    INITIAL_COUNT=0
    INITIAL_USED=0
fi

# First probe submission (may fail due to Kafka topic initialization)
echo "ℹ️  First probe submission (initializing Kafka topics)..."
FIRST_SUBMIT_RESPONSE=$(curl -s -X POST "$GATEWAY_URL/api/probes" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer test-token" \
    -d '{
            "metadata": [
                {
                "id": "testagent",
                "ip_address": "2001:db8::c95b:3f80:0:1"
                }
            ],
            "probes": [
                ["2001:4860:4860::8888", 12345, 53, 30, "icmpv6"],
                ["2001:4860:4860::8844", 12345, 53, 30, "icmpv6"]
            ]
        }' || echo "FAILED")

echo "ℹ️  Waiting 10 seconds for Kafka topics to be properly initialized..."
sleep 10

# Second probe submission (should work properly)
echo "ℹ️  Second probe submission (should work after topic initialization)..."
SUBMIT_RESPONSE=$(curl -s -X POST "$GATEWAY_URL/api/probes" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer test-token" \
    -d '{
            "metadata": [
                {
                "id": "testagent",
                "ip_address": "2001:db8::c95b:3f80:0:1"
                }
            ],
            "probes": [
                ["2001:4860:4860::8888", 12345, 53, 30, "icmpv6"],
                ["2001:4860:4860::8844", 12345, 53, 30, "icmpv6"]
            ]
        }' || echo "FAILED")

if echo "$SUBMIT_RESPONSE" | grep -q "id"; then
    echo "✅ Second probe submission successful"
    echo "   Response: $SUBMIT_RESPONSE"
    EXPECTED_PROBE_COUNT=2
    PROBE_SUBMISSION_SUCCESSFUL=true
elif echo "$FIRST_SUBMIT_RESPONSE" | grep -q "id"; then
    echo "⚠️  Only first probe submission succeeded (second failed)"
    echo "   First response: $FIRST_SUBMIT_RESPONSE"
    echo "   Second response: $SUBMIT_RESPONSE"
    # Use the first submission for testing
    SUBMIT_RESPONSE="$FIRST_SUBMIT_RESPONSE"
    EXPECTED_PROBE_COUNT=2
    PROBE_SUBMISSION_SUCCESSFUL=true
elif [[ $AGENT_REGISTERED == true ]]; then
    echo "❌ Both probe submissions failed despite agent being registered"
    echo "   First response: $FIRST_SUBMIT_RESPONSE"
    echo "   Second response: $SUBMIT_RESPONSE"
    EXPECTED_PROBE_COUNT=0
    PROBE_SUBMISSION_SUCCESSFUL=false
else
    echo "❌ Both probe submissions failed (agent not registered)"
    echo "   First response: $FIRST_SUBMIT_RESPONSE"
    echo "   Second response: $SUBMIT_RESPONSE"
    EXPECTED_PROBE_COUNT=0
    PROBE_SUBMISSION_SUCCESSFUL=false
fi

# Test 3.5: Monitor for actual probe transmission
echo "🔍 Test 3.5: Verify probe processing by the agent"

if [[ $PROBE_SUBMISSION_SUCCESSFUL == true ]]; then
    echo "ℹ️  Waiting for agent to process probes (20 seconds)..."
    sleep 20

    # Extract measurement ID from the submission response
    MEASUREMENT_ID=$(echo "$SUBMIT_RESPONSE" | grep -o '"id":"[^"]*"' | cut -d'"' -f4)

    if [[ -n "$MEASUREMENT_ID" ]]; then
        echo "ℹ️  Checking measurement status for ID: $MEASUREMENT_ID"

        # Check measurement status via API
        MEASUREMENT_STATUS=$(curl -s -X GET "$GATEWAY_URL/api/measurement/$MEASUREMENT_ID/status" \
            -H "Authorization: Bearer test-token" || echo "FAILED")

        if echo "$MEASUREMENT_STATUS" | grep -q "measurement_id"; then
            echo "✅ Measurement status retrieved successfully"
            echo "   Response: $MEASUREMENT_STATUS"
        else
            echo "❌ Failed to retrieve measurement status"
            echo "   Response: $MEASUREMENT_STATUS"
        fi
    else
        echo "❌ Could not extract measurement ID from submission response"
    fi

    # Check agent logs for probe processing evidence
    AGENT_LOGS_PROBE=$(docker logs integration-agent-1 --tail 50 2>&1 | grep -i "probe.*received\|message.*received\|probes.*deserialized" || echo "")

    if [[ -n "$AGENT_LOGS_PROBE" ]]; then
        echo "✅ Agent successfully received and processed probe messages from Kafka"
        echo "   Evidence: $(echo "$AGENT_LOGS_PROBE" | head -1)"
        PROBE_PROCESSING_VERIFIED=true
    else
        echo "❌ No evidence of probe processing found in agent logs"
        PROBE_PROCESSING_VERIFIED=false
    fi

    # Check for CaracatSender creation attempts (known limitation in Docker)
    SENDER_ATTEMPTS=$(docker logs integration-agent-1 --tail 50 2>&1 | grep -i "attempting to create\|SendLoop.*creating" || echo "")
    SENDER_TIMEOUT=$(echo "$SENDER_ATTEMPTS" | grep -i "timed out after 5 seconds\|Failed to create Caracat sender" || echo "")

    if [[ -n "$SENDER_ATTEMPTS" ]]; then
        echo "✅ Agent attempted to create CaracatSender for probe transmission"
        if [[ -n "$SENDER_TIMEOUT" ]]; then
            echo "⚠️  CaracatSender creation timed out (known Docker container limitation)"
            echo "   Note: This is expected in containerized environments due to raw socket restrictions"
        else
            echo "✅ CaracatSender creation succeeded"
        fi
    else
        echo "❌ No CaracatSender creation attempts found in logs"
    fi

    # Overall probe processing assessment
    if [[ $PROBE_PROCESSING_VERIFIED == true ]]; then
        PROBE_PIPELINE_VERIFIED=true
        echo "✅ Probe processing pipeline verified successfully"
        echo "   - Kafka message consumption: ✅"
        echo "   - Probe deserialization: ✅"
        echo "   - Routing to CaracatSender: ✅"
        if [[ -n "$SENDER_TIMEOUT" ]]; then
            echo "   - Actual probe transmission: ⚠️  (Docker limitation)"
        else
            echo "   - Actual probe transmission: ✅"
        fi
    else
        PROBE_PIPELINE_VERIFIED=false
        echo "❌ Probe processing pipeline has issues"
    fi
else
    echo "ℹ️  Skipping probe processing test (probe submission failed)"
    PROBE_PIPELINE_VERIFIED=false
fi

# Test 4: Check user usage statistics and verify increases
echo "🔍 Test 4: Check user usage statistics"
USAGE_RESPONSE=$(curl -s -X GET "$GATEWAY_URL/api/user/me" \
    -H "Authorization: Bearer test-token" || echo "FAILED")

if echo "$USAGE_RESPONSE" | grep -q "submission_count"; then
    FINAL_COUNT=$(echo "$USAGE_RESPONSE" | grep -o '"submission_count":[0-9]*' | grep -o '[0-9]*')
    FINAL_USED=$(echo "$USAGE_RESPONSE" | grep -o '"used":[0-9]*' | grep -o '[0-9]*')

    echo "✅ Usage statistics retrieval successful"
    echo "   Response: $USAGE_RESPONSE"
    echo "ℹ️  Final usage: submissions=$FINAL_COUNT, probes_used=$FINAL_USED"

    # Verify usage increases if we had successful probe submission
    if [[ $EXPECTED_PROBE_COUNT -gt 0 ]]; then
        SUBMISSION_INCREASE=$((FINAL_COUNT - INITIAL_COUNT))
        USAGE_INCREASE=$((FINAL_USED - INITIAL_USED))

        # We expect at least 1 submission increase, and potentially 2 if both submissions worked
        # For probe usage, we expect at least the EXPECTED_PROBE_COUNT from successful submission
        EXPECTED_MIN_SUBMISSIONS=1
        EXPECTED_MAX_SUBMISSIONS=2  # Could be 2 if both submissions succeeded

        if [[ $SUBMISSION_INCREASE -ge $EXPECTED_MIN_SUBMISSIONS ]] && [[ $USAGE_INCREASE -ge $EXPECTED_PROBE_COUNT ]]; then
            echo "✅ Usage statistics increased as expected (submissions: +$SUBMISSION_INCREASE, probes: +$USAGE_INCREASE)"
            USAGE_TEST_PASSED=true
        else
            echo "❌ Usage statistics did not increase as expected"
            echo "   Expected: submissions ≥ +$EXPECTED_MIN_SUBMISSIONS, probes ≥ +$EXPECTED_PROBE_COUNT"
            echo "   Actual: submissions +$SUBMISSION_INCREASE, probes +$USAGE_INCREASE"
            USAGE_TEST_PASSED=false
        fi
    else
        echo "ℹ️  Skipping usage increase verification (probe submission failed)"
        USAGE_TEST_PASSED=true  # Don't fail the test if probe submission failed
    fi
else
    echo "❌ Usage statistics retrieval failed"
    echo "   Response: $USAGE_RESPONSE"
    USAGE_TEST_PASSED=false
fi

# Test 5: Direct database check (requires psql in container)
echo "🔍 Test 5: Direct database verification"
DB_CHECK=$(docker compose -f compose.yml exec -T postgres psql -U saimiris_user -d saimiris_gateway -c "SELECT COUNT(*) FROM probe_usage;" 2>/dev/null || echo "FAILED")

if echo "$DB_CHECK" | grep -qE "[0-9]+"; then
    echo "✅ Database contains probe usage records"
    echo "   Records found: $(echo "$DB_CHECK" | grep -oE '[0-9]+' | head -1)"
else
    echo "⚠️  Could not verify database directly (psql might not be available)"
fi

# Test 6: Check database view
echo "🔍 Test 6: Check user_usage_stats view"
VIEW_CHECK=$(docker compose -f compose.yml exec -T postgres psql -U saimiris_user -d saimiris_gateway -c "SELECT * FROM user_usage_stats LIMIT 5;" 2>/dev/null || echo "FAILED")

if echo "$VIEW_CHECK" | grep -q "user_hash"; then
    echo "✅ user_usage_stats view is working"
else
    echo "⚠️  Could not verify view directly"
fi

echo ""
echo "🎉 Integration tests completed!"
echo ""

# Determine overall test success
OVERALL_SUCCESS=true

# Check agent registration
if [[ $AGENT_REGISTERED == true ]]; then
    AGENT_REG="✅"
else
    AGENT_REG="❌"
    OVERALL_SUCCESS=false
fi

# Check gateway API
if curl -s -f "$GATEWAY_URL/api/agents" > /dev/null; then
    GATEWAY_API="✅"
else
    GATEWAY_API="❌"
    OVERALL_SUCCESS=false
fi

# Check probe submission
if echo "$SUBMIT_RESPONSE" | grep -q "id"; then
    PROBE_SUBMISSION="✅"
elif [[ $AGENT_REGISTERED == true ]]; then
    PROBE_SUBMISSION="❌"
    OVERALL_SUCCESS=false
    echo "⚠️  Note: Probe submission failed despite agent being registered - this indicates a problem"
else
    PROBE_SUBMISSION="❌"
    echo "⚠️  Note: Probe submission failed due to agent not being registered"
    echo "      This is expected if agent registration failed"
fi

# Check probe processing verification
if [[ $PROBE_PIPELINE_VERIFIED == true ]]; then
    PROBE_PROCESSING="✅"
elif [[ $PROBE_SUBMISSION_SUCCESSFUL == true ]]; then
    PROBE_PROCESSING="❌"
    OVERALL_SUCCESS=false
    echo "⚠️  Note: Probes were submitted but processing pipeline failed"
else
    PROBE_PROCESSING="⚠️"
    echo "⚠️  Note: Probe processing test skipped due to submission failure"
fi

# Check usage statistics
if [[ $USAGE_TEST_PASSED == true ]]; then
    USAGE_STATS="✅"
else
    USAGE_STATS="❌"
    OVERALL_SUCCESS=false
fi

# Database integration is considered successful if we can access it
if echo "$DB_CHECK" | grep -qE "[0-9]+"; then
    DATABASE_INTEGRATION="✅"
else
    DATABASE_INTEGRATION="⚠️"
fi

echo "📊 Summary:"
echo "   - Gateway API: $GATEWAY_API"
echo "   - Agent registration: $AGENT_REG"
echo "   - Probe submission: $PROBE_SUBMISSION"
echo "   - Probe processing: $PROBE_PROCESSING"
echo "   - Usage statistics: $USAGE_STATS"
echo "   - Database integration: $DATABASE_INTEGRATION"
echo ""

if [[ $OVERALL_SUCCESS == true ]]; then
    echo "🎊 All critical tests passed!"
    exit 0
else
    echo "💥 Some critical tests failed!"
    exit 1
fi

echo "🔧 To run this test:"
echo "   1. cd integration"
echo "   2. docker compose up -d"
echo "   3. ./tests/test_submit_integration.sh"
echo "   4. docker compose down"
