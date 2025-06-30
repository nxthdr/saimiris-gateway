-- Migration to create user limits table
-- This table stores per-user probe limits with hashed user identifiers

CREATE TABLE IF NOT EXISTS user_limits (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_hash VARCHAR(64) NOT NULL UNIQUE,
    probe_limit INTEGER NOT NULL DEFAULT 10000, -- Default probe limit
    created_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMP WITH TIME ZONE NOT NULL DEFAULT NOW()
);

-- Create an index on user_hash for efficient lookups
CREATE INDEX IF NOT EXISTS idx_user_limits_user_hash
ON user_limits (user_hash);

-- Create a trigger to update the updated_at column
CREATE OR REPLACE FUNCTION update_updated_at_column()
RETURNS TRIGGER AS $$
BEGIN
    NEW.updated_at = NOW();
    RETURN NEW;
END;
$$ language 'plpgsql';

CREATE TRIGGER update_user_limits_updated_at
    BEFORE UPDATE ON user_limits
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at_column();
