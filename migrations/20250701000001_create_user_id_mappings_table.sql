-- Add migration script here
CREATE TABLE user_id_mappings (
    user_hash VARCHAR(64) PRIMARY KEY,
    user_id INTEGER UNIQUE NOT NULL,
    created_at TIMESTAMP WITH TIME ZONE DEFAULT CURRENT_TIMESTAMP NOT NULL
);

-- Create index on user_id for faster lookups by user_id (if needed)
CREATE INDEX idx_user_id_mappings_user_id ON user_id_mappings(user_id);
