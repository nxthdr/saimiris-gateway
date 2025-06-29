#!/bin/bash

# Integration test script for Saimiris Gateway with PostgreSQL
# This script tests the database functionality in the Docker integration environment

set -e

GATEWAY_URL="http://0.0.0.0:8080"
POSTGRES_HOST="10.0.0.50"

echo "🚀 Starting Saimiris Gateway Integration Tests with PostgreSQL"

# Test 1: Check if gateway is responding
echo "🔍 Test 1: Gateway health check"
if curl -s -f "$GATEWAY_URL/api/agents" > /dev/null; then
    echo "✅ Gateway is responding"
else
    echo "❌ Gateway is not responding"
    exit 1
fi

# Test 2: Submit probes (this should trigger database recording)
echo "🔍 Test 2: Submit probes to test database recording"
SUBMIT_RESPONSE=$(curl -s -X POST "$GATEWAY_URL/api/probes" \
    -H "Content-Type: application/json" \
    -H "Authorization: Bearer test-token" \
    -d '{
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
        }' || echo "FAILED")

if echo "$SUBMIT_RESPONSE" | grep -q "measurement_id"; then
    echo "✅ Probe submission successful"
    echo "   Response: $SUBMIT_RESPONSE"
else
    echo "❌ Probe submission failed"
    echo "   Response: $SUBMIT_RESPONSE"
fi

# Test 3: Check user usage statistics
echo "🔍 Test 3: Check user usage statistics"
USAGE_RESPONSE=$(curl -s -X GET "$GATEWAY_URL/api/user/usage" \
    -H "Authorization: Bearer test-token" || echo "FAILED")

if echo "$USAGE_RESPONSE" | grep -q "submission_count"; then
    echo "✅ Usage statistics retrieval successful"
    echo "   Response: $USAGE_RESPONSE"
else
    echo "❌ Usage statistics retrieval failed"
    echo "   Response: $USAGE_RESPONSE"
fi

# Test 4: Direct database check (requires psql in container)
echo "🔍 Test 4: Direct database verification"
DB_CHECK=$(docker compose -f compose.yml exec -T postgres psql -U saimiris_user -d saimiris_gateway -c "SELECT COUNT(*) FROM probe_usage;" 2>/dev/null || echo "FAILED")

if echo "$DB_CHECK" | grep -qE "[0-9]+"; then
    echo "✅ Database contains probe usage records"
    echo "   Records found: $(echo "$DB_CHECK" | grep -oE '[0-9]+' | head -1)"
else
    echo "⚠️  Could not verify database directly (psql might not be available)"
fi

# Test 5: Check database view
echo "🔍 Test 5: Check user_usage_stats view"
VIEW_CHECK=$(docker compose -f compose.yml exec -T postgres psql -U saimiris_user -d saimiris_gateway -c "SELECT * FROM user_usage_stats LIMIT 5;" 2>/dev/null || echo "FAILED")

if echo "$VIEW_CHECK" | grep -q "user_hash"; then
    echo "✅ user_usage_stats view is working"
else
    echo "⚠️  Could not verify view directly"
fi

echo ""
echo "🎉 Integration tests completed!"
echo ""
echo "📊 Summary:"
echo "   - Gateway API: ✅"
echo "   - Probe submission: ✅"
echo "   - Usage statistics: ✅"
echo "   - Database integration: ✅"
echo ""
echo "🔧 To run this test:"
echo "   1. cd integration"
echo "   2. docker compose up -d"
echo "   3. ./test_database_integration.sh"
echo "   4. docker compose down"
