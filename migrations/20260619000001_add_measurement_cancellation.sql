-- Add measurement cancellation support.
-- A user can cancel a stuck/in-progress measurement; its not-yet-complete agent
-- rows are marked `cancelled`, which makes the measurement terminal so it stops
-- showing as "in progress" forever (e.g. when an agent dies mid-run).

ALTER TABLE measurement_tracking
    ADD COLUMN IF NOT EXISTS cancelled BOOLEAN NOT NULL DEFAULT FALSE;

-- Recreate the status view: a measurement is terminal ("complete") when every
-- agent is either complete or cancelled, and expose whether it was cancelled.
-- DROP + CREATE (not CREATE OR REPLACE): Postgres forbids reordering/renaming an
-- existing view's columns via CREATE OR REPLACE, and we add a column mid-list.
DROP VIEW IF EXISTS measurement_status;
CREATE VIEW measurement_status AS
SELECT
    measurement_id,
    user_hash,
    COUNT(*) as total_agents,
    SUM(expected_probes) as total_expected_probes,
    SUM(sent_probes) as total_sent_probes,
    COUNT(*) FILTER (WHERE is_complete = TRUE) as completed_agents,
    CASE
        WHEN COUNT(*) FILTER (WHERE is_complete = TRUE OR cancelled = TRUE) = COUNT(*) THEN TRUE
        ELSE FALSE
    END as measurement_complete,
    bool_or(cancelled) as measurement_cancelled,
    MIN(created_at) as started_at,
    MAX(updated_at) as last_updated
FROM measurement_tracking
GROUP BY measurement_id, user_hash;
