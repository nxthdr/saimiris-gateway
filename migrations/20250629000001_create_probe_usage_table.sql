-- Migration to create probe usage tracking table
-- This table stores probe submission statistics with hashed user identifiers (client_id)

CREATE TABLE IF NOT EXISTS probe_usage (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_hash VARCHAR(64) NOT NULL,
    probe_count INTEGER NOT NULL,
    timestamp TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Create an index on user_hash and timestamp for efficient queries
CREATE INDEX IF NOT EXISTS idx_probe_usage_user_timestamp
ON probe_usage (user_hash, timestamp);

-- Create an index on timestamp for recent queries
CREATE INDEX IF NOT EXISTS idx_probe_usage_timestamp
ON probe_usage (timestamp DESC);

-- Optional: Create a view for usage statistics
CREATE OR REPLACE VIEW user_usage_stats AS
SELECT
    user_hash,
    COUNT(*) as submission_count,
    SUM(probe_count) as total_probes,
    MIN(timestamp) as first_submission,
    MAX(timestamp) as last_submission,
    DATE_TRUNC('day', timestamp) as submission_date,
    COUNT(*) FILTER (WHERE timestamp >= NOW() - INTERVAL '24 hours') as submissions_last_24h,
    SUM(probe_count) FILTER (WHERE timestamp >= NOW() - INTERVAL '24 hours') as probes_last_24h,
    COUNT(*) FILTER (WHERE timestamp >= NOW() - INTERVAL '7 days') as submissions_last_7d,
    SUM(probe_count) FILTER (WHERE timestamp >= NOW() - INTERVAL '7 days') as probes_last_7d,
    COUNT(*) FILTER (WHERE timestamp >= NOW() - INTERVAL '30 days') as submissions_last_30d,
    SUM(probe_count) FILTER (WHERE timestamp >= NOW() - INTERVAL '30 days') as probes_last_30d
FROM probe_usage
GROUP BY user_hash, DATE_TRUNC('day', timestamp);
