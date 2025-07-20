-- Migration to create measurement tracking table (replacing probe_usage)
-- This table stores measurement status and probe counts per agent
-- It consolidates both measurement tracking and probe usage into one table

CREATE TABLE IF NOT EXISTS measurement_tracking (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_hash VARCHAR(64) NOT NULL,
    measurement_id UUID NOT NULL,
    agent_id VARCHAR(255) NOT NULL,
    expected_probes INTEGER NOT NULL DEFAULT 0,
    sent_probes INTEGER NOT NULL DEFAULT 0,
    is_complete BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),

    -- Ensure unique combination of measurement_id, user_hash, and agent_id
    UNIQUE(measurement_id, user_hash, agent_id)
);

-- Create indexes for efficient queries
CREATE INDEX IF NOT EXISTS idx_measurement_tracking_measurement_id
ON measurement_tracking (measurement_id);

CREATE INDEX IF NOT EXISTS idx_measurement_tracking_user_hash
ON measurement_tracking (user_hash);

CREATE INDEX IF NOT EXISTS idx_measurement_tracking_agent_id
ON measurement_tracking (agent_id);

-- Index for checking incomplete measurements
CREATE INDEX IF NOT EXISTS idx_measurement_tracking_incomplete
ON measurement_tracking (measurement_id, is_complete)
WHERE is_complete = FALSE;

-- Index for user usage statistics (replaces probe_usage table)
CREATE INDEX IF NOT EXISTS idx_measurement_tracking_user_timestamp
ON measurement_tracking (user_hash, created_at);

-- Create a view for measurement status aggregation
CREATE OR REPLACE VIEW measurement_status AS
SELECT
    measurement_id,
    user_hash,
    COUNT(*) as total_agents,
    SUM(expected_probes) as total_expected_probes,
    SUM(sent_probes) as total_sent_probes,
    COUNT(*) FILTER (WHERE is_complete = TRUE) as completed_agents,
    CASE
        WHEN COUNT(*) FILTER (WHERE is_complete = TRUE) = COUNT(*) THEN TRUE
        ELSE FALSE
    END as measurement_complete,
    MIN(created_at) as started_at,
    MAX(updated_at) as last_updated
FROM measurement_tracking
GROUP BY measurement_id, user_hash;

-- Drop the old user_usage_stats view before creating the new one (replaces probe_usage queries)
DROP VIEW IF EXISTS user_usage_stats;

-- Create a view for user usage statistics (replaces probe_usage queries)
CREATE OR REPLACE VIEW user_usage_stats AS
SELECT
    user_hash,
    COUNT(DISTINCT measurement_id) as submission_count,
    SUM(expected_probes) as total_expected_probes,
    SUM(sent_probes) as total_probes,
    MIN(created_at) as first_submission,
    MAX(updated_at) as last_submission,
    DATE_TRUNC('day', created_at) as submission_date,
    COUNT(DISTINCT measurement_id) FILTER (WHERE created_at >= NOW() - INTERVAL '24 hours') as submissions_last_24h,
    SUM(expected_probes) FILTER (WHERE created_at >= NOW() - INTERVAL '24 hours') as expected_probes_last_24h,
    SUM(sent_probes) FILTER (WHERE updated_at >= NOW() - INTERVAL '24 hours') as probes_last_24h,
    COUNT(DISTINCT measurement_id) FILTER (WHERE created_at >= NOW() - INTERVAL '7 days') as submissions_last_7d,
    SUM(expected_probes) FILTER (WHERE created_at >= NOW() - INTERVAL '7 days') as expected_probes_last_7d,
    SUM(sent_probes) FILTER (WHERE updated_at >= NOW() - INTERVAL '7 days') as probes_last_7d,
    COUNT(DISTINCT measurement_id) FILTER (WHERE created_at >= NOW() - INTERVAL '30 days') as submissions_last_30d,
    SUM(expected_probes) FILTER (WHERE created_at >= NOW() - INTERVAL '30 days') as expected_probes_last_30d,
    SUM(sent_probes) FILTER (WHERE updated_at >= NOW() - INTERVAL '30 days') as probes_last_30d
FROM measurement_tracking
GROUP BY user_hash, DATE_TRUNC('day', created_at);
