use crate::hash_user_identifier;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tracing::debug;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DatabaseConfig {
    pub database_url: String,
}

impl DatabaseConfig {
    pub fn new(database_url: String) -> Self {
        Self { database_url }
    }
}

#[derive(Debug, Clone)]
pub struct ProbeUsageRecord {
    pub id: Uuid,
    pub user_hash: String,
    pub probe_count: i32,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UserUsageStats {
    pub user_hash: String,
    pub submission_count: u32,
    pub total_probes: u32,
    pub limit: u32,
    pub last_submitted: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct UserLimit {
    pub id: Uuid,
    pub user_hash: String,
    pub probe_limit: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MeasurementTracking {
    pub id: Uuid,
    pub user_hash: String,
    pub measurement_id: Uuid,
    pub agent_id: String,
    pub expected_probes: i32,
    pub sent_probes: i32,
    pub is_complete: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct MeasurementStatus {
    pub measurement_id: Uuid,
    pub user_hash: String,
    pub total_agents: i64,
    pub total_expected_probes: i64,
    pub total_sent_probes: i64,
    pub completed_agents: i64,
    pub measurement_complete: bool,
    pub started_at: DateTime<Utc>,
    pub last_updated: DateTime<Utc>,
}

// Mock storage for testing
#[derive(Debug, Clone)]
pub(crate) struct MockStorage {
    records: Arc<Mutex<Vec<ProbeUsageRecord>>>,
    user_limits: Arc<Mutex<Vec<UserLimit>>>,
    user_id_mappings: Arc<Mutex<HashMap<String, u32>>>,
    measurement_tracking: Arc<Mutex<Vec<MeasurementTracking>>>,
}

const DEFAULT_PROBE_LIMIT: u32 = 10_000; // Default probe limit for users

impl MockStorage {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            user_limits: Arc::new(Mutex::new(Vec::new())),
            user_id_mappings: Arc::new(Mutex::new(HashMap::new())),
            measurement_tracking: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn insert(&self, record: ProbeUsageRecord) {
        let mut records = self.records.lock().unwrap();
        records.push(record);
    }

    fn get_user_stats(
        &self,
        user_hash: &str,
        start_time: DateTime<Utc>,
        end_time: DateTime<Utc>,
    ) -> UserUsageStats {
        // Get stats from measurement tracking instead of probe records
        let tracking = self.measurement_tracking.lock().unwrap();
        let filtered_tracking: Vec<_> = tracking
            .iter()
            .filter(|t| {
                t.user_hash == user_hash && t.created_at >= start_time && t.created_at <= end_time
            })
            .collect();

        // Group by measurement_id to count unique measurements
        let mut measurements = std::collections::HashSet::new();
        let mut total_probes = 0;
        let mut last_submitted: Option<DateTime<Utc>> = None;

        for tracking_entry in &filtered_tracking {
            measurements.insert(&tracking_entry.measurement_id);
            total_probes += tracking_entry.expected_probes as u32;
            let updated_at = tracking_entry.updated_at;
            last_submitted = Some(match last_submitted {
                Some(existing) => existing.max(updated_at),
                None => updated_at,
            });
        }

        // Get user limit from mock storage
        let user_limits = self.user_limits.lock().unwrap();
        let limit = user_limits
            .iter()
            .find(|ul| ul.user_hash == user_hash)
            .map(|ul| ul.probe_limit)
            .unwrap_or(DEFAULT_PROBE_LIMIT);

        UserUsageStats {
            user_hash: user_hash.to_string(),
            submission_count: measurements.len() as u32,
            total_probes,
            limit,
            last_submitted,
        }
    }

    fn set_user_limit(&self, user_hash: &str, probe_limit: u32) -> UserLimit {
        let mut user_limits = self.user_limits.lock().unwrap();
        let now = Utc::now();

        // Check if user limit already exists
        if let Some(existing) = user_limits.iter_mut().find(|ul| ul.user_hash == user_hash) {
            existing.probe_limit = probe_limit;
            existing.updated_at = now;
            existing.clone()
        } else {
            // Create new user limit
            let new_limit = UserLimit {
                id: Uuid::new_v4(),
                user_hash: user_hash.to_string(),
                probe_limit,
                created_at: now,
                updated_at: now,
            };
            user_limits.push(new_limit.clone());
            new_limit
        }
    }

    fn get_user_limit(&self, user_hash: &str) -> Option<UserLimit> {
        let user_limits = self.user_limits.lock().unwrap();
        user_limits
            .iter()
            .find(|ul| ul.user_hash == user_hash)
            .cloned()
    }

    fn get_recent(&self, limit: i32) -> Vec<ProbeUsageRecord> {
        // Get recent measurements from tracking data
        let tracking = self.measurement_tracking.lock().unwrap();

        // Group by measurement_id and sum probe counts
        let mut measurement_map: std::collections::HashMap<(Uuid, String), (i32, DateTime<Utc>)> =
            std::collections::HashMap::new();
        for entry in tracking.iter() {
            let key = (entry.measurement_id, entry.user_hash.clone());
            measurement_map
                .entry(key)
                .and_modify(|(probe_count, timestamp)| {
                    *probe_count += entry.sent_probes;
                    *timestamp = (*timestamp).max(entry.updated_at);
                })
                .or_insert((entry.sent_probes, entry.updated_at));
        }

        // Convert to ProbeUsageRecord format
        let mut records: Vec<_> = measurement_map
            .into_iter()
            .map(
                |((measurement_id, user_hash), (probe_count, timestamp))| ProbeUsageRecord {
                    id: measurement_id,
                    user_hash,
                    probe_count,
                    timestamp,
                },
            )
            .collect();

        records.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
        records.into_iter().take(limit as usize).collect()
    }
}

#[derive(Clone)]
#[allow(private_interfaces)]
pub enum DatabaseImpl {
    Real(PgPool),
    Mock(MockStorage),
}

#[derive(Clone)]
pub struct Database {
    impl_: DatabaseImpl,
}

impl Database {
    pub async fn new(config: &DatabaseConfig) -> Result<Self, sqlx::Error> {
        let pool = PgPool::connect(&config.database_url).await?;
        Ok(Self {
            impl_: DatabaseImpl::Real(pool),
        })
    }

    /// Create a mock database for testing (doesn't require PostgreSQL)
    pub fn new_mock() -> Self {
        Self {
            impl_: DatabaseImpl::Mock(MockStorage::new()),
        }
    }

    pub async fn initialize(&self) -> Result<(), sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                debug!("Running database migrations...");

                // Run sqlx migrations
                sqlx::migrate!("./migrations").run(pool).await?;

                debug!("Database migrations completed successfully");
                Ok(())
            }
            DatabaseImpl::Mock(_) => {
                debug!("Mock database initialized (no-op)");
                Ok(())
            }
        }
    }

    /// Record probe usage by updating measurement tracking (replaces old probe_usage table)
    /// This should be called when probes are submitted to initialize the measurement tracking
    pub async fn record_probe_usage(
        &self,
        user_id: &str,
        measurement_id: Uuid,
        probe_count: i32,
    ) -> Result<ProbeUsageRecord, sqlx::Error> {
        let user_hash = hash_user_identifier(user_id);
        let timestamp = Utc::now();

        // This method is now deprecated in favor of measurement tracking
        // But we keep it for compatibility. It returns a synthetic record.
        let record = ProbeUsageRecord {
            id: measurement_id,
            user_hash: user_hash.clone(),
            probe_count,
            timestamp,
        };

        match &self.impl_ {
            DatabaseImpl::Real(_pool) => {
                // In the real implementation, this is handled by measurement tracking
                // This method is kept for backward compatibility but doesn't do actual DB operations
                debug!(
                    "Probe usage recorded via measurement tracking: user_hash={}, probe_count={}, timestamp={}",
                    user_hash, probe_count, timestamp
                );
            }
            DatabaseImpl::Mock(storage) => {
                storage.insert(record.clone());
                debug!(
                    "Mock recorded probe usage: user_hash={}, probe_count={}, timestamp={}",
                    user_hash, probe_count, timestamp
                );
            }
        }

        Ok(record)
    }

    pub async fn get_user_usage_stats(
        &self,
        user_id: &str,
        start_time: Option<DateTime<Utc>>,
        end_time: Option<DateTime<Utc>>,
    ) -> Result<UserUsageStats, sqlx::Error> {
        let user_hash = hash_user_identifier(user_id);

        // Default to today's usage (from start of day to now)
        let now = Utc::now();
        let today_start = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
        let start_time = start_time.unwrap_or(today_start);
        let end_time = end_time.unwrap_or(now);

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                // First get the user's limit
                let limit_row = sqlx::query(
                    r#"
                    SELECT probe_limit FROM user_limits WHERE user_hash = $1
                    "#,
                )
                .bind(&user_hash)
                .fetch_optional(pool)
                .await?;

                let limit = limit_row
                    .map(|row| row.get::<i32, _>("probe_limit") as u32)
                    .unwrap_or(DEFAULT_PROBE_LIMIT); // Default limit if not found

                // Get usage stats from measurement tracking table
                let row = sqlx::query(
                    r#"
                    SELECT
                        COUNT(DISTINCT measurement_id) as submission_count,
                        COALESCE(SUM(expected_probes), 0) as total_probes,
                        MAX(updated_at) as last_submitted
                    FROM measurement_tracking
                    WHERE user_hash = $1
                    AND created_at >= $2
                    AND created_at <= $3
                    "#,
                )
                .bind(&user_hash)
                .bind(start_time)
                .bind(end_time)
                .fetch_one(pool)
                .await?;

                Ok(UserUsageStats {
                    user_hash,
                    submission_count: row.get::<i64, _>("submission_count") as u32,
                    total_probes: row.get::<i64, _>("total_probes") as u32,
                    limit,
                    last_submitted: row.get::<Option<DateTime<Utc>>, _>("last_submitted"),
                })
            }
            DatabaseImpl::Mock(storage) => {
                let stats = storage.get_user_stats(&user_hash, start_time, end_time);
                Ok(stats)
            }
        }
    }

    pub async fn get_recent_usage(
        &self,
        limit: Option<i32>,
    ) -> Result<Vec<ProbeUsageRecord>, sqlx::Error> {
        let limit = limit.unwrap_or(100);

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let rows = sqlx::query(
                    r#"
                    SELECT
                        measurement_id as id,
                        user_hash,
                        SUM(sent_probes) as probe_count,
                        MAX(updated_at) as timestamp
                    FROM measurement_tracking
                    GROUP BY measurement_id, user_hash
                    ORDER BY timestamp DESC
                    LIMIT $1
                    "#,
                )
                .bind(limit)
                .fetch_all(pool)
                .await?;

                let mut records = Vec::new();
                for row in rows {
                    records.push(ProbeUsageRecord {
                        id: row.get("id"),
                        user_hash: row.get("user_hash"),
                        probe_count: row.get::<i64, _>("probe_count") as i32,
                        timestamp: row.get("timestamp"),
                    });
                }

                Ok(records)
            }
            DatabaseImpl::Mock(storage) => {
                let records = storage.get_recent(limit);
                Ok(records)
            }
        }
    }

    /// Set a user's probe limit
    pub async fn set_user_limit(
        &self,
        user_id: &str,
        probe_limit: u32,
    ) -> Result<UserLimit, sqlx::Error> {
        let user_hash = hash_user_identifier(user_id);

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let row = sqlx::query(
                    r#"
                    INSERT INTO user_limits (user_hash, probe_limit)
                    VALUES ($1, $2)
                    ON CONFLICT (user_hash)
                    DO UPDATE SET
                        probe_limit = EXCLUDED.probe_limit,
                        updated_at = NOW()
                    RETURNING id, user_hash, probe_limit, created_at, updated_at
                    "#,
                )
                .bind(&user_hash)
                .bind(probe_limit as i32)
                .fetch_one(pool)
                .await?;

                Ok(UserLimit {
                    id: row.get("id"),
                    user_hash: row.get("user_hash"),
                    probe_limit: row.get::<i32, _>("probe_limit") as u32,
                    created_at: row.get("created_at"),
                    updated_at: row.get("updated_at"),
                })
            }
            DatabaseImpl::Mock(storage) => {
                let limit = storage.set_user_limit(&user_hash, probe_limit);
                Ok(limit)
            }
        }
    }

    /// Get a user's probe limit
    pub async fn get_user_limit(&self, user_id: &str) -> Result<Option<UserLimit>, sqlx::Error> {
        let user_hash = hash_user_identifier(user_id);

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let row = sqlx::query(
                    r#"
                    SELECT id, user_hash, probe_limit, created_at, updated_at
                    FROM user_limits
                    WHERE user_hash = $1
                    "#,
                )
                .bind(&user_hash)
                .fetch_optional(pool)
                .await?;

                Ok(row.map(|r| UserLimit {
                    id: r.get("id"),
                    user_hash: r.get("user_hash"),
                    probe_limit: r.get::<i32, _>("probe_limit") as u32,
                    created_at: r.get("created_at"),
                    updated_at: r.get("updated_at"),
                }))
            }
            DatabaseImpl::Mock(storage) => {
                let limit = storage.get_user_limit(&user_hash);
                Ok(limit)
            }
        }
    }

    /// Check if a user can submit additional probes (daily rate limiting)
    pub async fn can_user_submit_probes(
        &self,
        user_id: &str,
        additional_probes: u32,
        days_back: Option<i64>,
    ) -> Result<bool, sqlx::Error> {
        let now = Utc::now();
        let start_time = if let Some(days) = days_back {
            now - chrono::Duration::days(days)
        } else {
            // Default to today's usage (from start of day)
            now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc()
        };

        let stats = self
            .get_user_usage_stats(user_id, Some(start_time), Some(now))
            .await?;

        Ok(stats.total_probes + additional_probes <= stats.limit)
    }

    /// Get user ID by user hash from the database
    pub async fn get_user_id_by_hash(&self, user_hash: &str) -> Result<Option<u32>, sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let row = sqlx::query("SELECT user_id FROM user_id_mappings WHERE user_hash = $1")
                    .bind(user_hash)
                    .fetch_optional(pool)
                    .await?;

                Ok(row.map(|row| row.get::<i32, _>("user_id") as u32))
            }
            DatabaseImpl::Mock(storage) => {
                let mappings = storage.user_id_mappings.lock().unwrap();
                Ok(mappings.get(user_hash).copied())
            }
        }
    }

    /// Create a new user ID mapping in the database
    pub async fn create_user_id_mapping(
        &self,
        user_hash: &str,
        user_id: u32,
    ) -> Result<(), sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                match sqlx::query("INSERT INTO user_id_mappings (user_hash, user_id, created_at) VALUES ($1, $2, $3)")
                    .bind(user_hash)
                    .bind(user_id as i32)
                    .bind(chrono::Utc::now())
                    .execute(pool)
                    .await
                {
                    Ok(_) => Ok(()),
                    Err(sqlx::Error::Database(db_err)) => {
                        // Check if this is a constraint violation and provide better context
                        let error_message = db_err.message();
                        if error_message.contains("user_id_mappings_pkey") || error_message.contains("user_hash") {
                            // Primary key violation on user_hash - another request created this user
                            return Err(sqlx::Error::Io(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "UNIQUE constraint failed: user_hash already exists",
                            )));
                        } else if error_message.contains("user_id") {
                            // Unique constraint violation on user_id - need to try different ID
                            return Err(sqlx::Error::Io(std::io::Error::new(
                                std::io::ErrorKind::AlreadyExists,
                                "UNIQUE constraint failed: user_id already exists",
                            )));
                        }
                        // Re-throw the original error for other database issues
                        Err(sqlx::Error::Database(db_err))
                    }
                    Err(e) => Err(e),
                }
            }
            DatabaseImpl::Mock(storage) => {
                let mut mappings = storage.user_id_mappings.lock().unwrap();
                if mappings.contains_key(user_hash) {
                    // Simulate user_hash primary key violation
                    return Err(sqlx::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "UNIQUE constraint failed: user_hash already exists",
                    )));
                }
                if mappings.values().any(|&v| v == user_id) {
                    // Simulate user_id unique constraint violation
                    return Err(sqlx::Error::Io(std::io::Error::new(
                        std::io::ErrorKind::AlreadyExists,
                        "UNIQUE constraint failed: user_id already exists",
                    )));
                }
                mappings.insert(user_hash.to_string(), user_id);
                Ok(())
            }
        }
    }

    /// Create a new measurement tracking entry
    pub async fn create_measurement_tracking(
        &self,
        user_hash: &str,
        measurement_id: Uuid,
        agent_id: &str,
        expected_probes: i32,
    ) -> Result<MeasurementTracking, sqlx::Error> {
        let now = Utc::now();

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let record = sqlx::query_as::<_, MeasurementTracking>(
                    r#"INSERT INTO measurement_tracking
                       (user_hash, measurement_id, agent_id, expected_probes, sent_probes, is_complete, created_at, updated_at)
                       VALUES ($1, $2, $3, $4, 0, false, $5, $5)
                       RETURNING id, user_hash, measurement_id, agent_id, expected_probes, sent_probes, is_complete, created_at, updated_at"#
                )
                .bind(user_hash)
                .bind(measurement_id)
                .bind(agent_id)
                .bind(expected_probes)
                .bind(now)
                .fetch_one(pool)
                .await?;

                Ok(record)
            }
            DatabaseImpl::Mock(storage) => {
                let record = MeasurementTracking {
                    id: Uuid::new_v4(),
                    user_hash: user_hash.to_string(),
                    measurement_id,
                    agent_id: agent_id.to_string(),
                    expected_probes,
                    sent_probes: 0,
                    is_complete: false,
                    created_at: now,
                    updated_at: now,
                };

                let mut tracking = storage.measurement_tracking.lock().unwrap();
                tracking.push(record.clone());
                Ok(record)
            }
        }
    }

    /// Update probe count for a measurement tracking entry
    pub async fn update_measurement_probe_count(
        &self,
        measurement_id: Uuid,
        user_hash: &str,
        agent_id: &str,
        sent_probes: i32,
        is_complete: bool,
    ) -> Result<MeasurementTracking, sqlx::Error> {
        let now = Utc::now();

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let record = sqlx::query_as::<_, MeasurementTracking>(
                    r#"UPDATE measurement_tracking
                       SET sent_probes = $4, is_complete = $5, updated_at = $6
                       WHERE measurement_id = $1 AND user_hash = $2 AND agent_id = $3
                       RETURNING id, user_hash, measurement_id, agent_id, expected_probes, sent_probes, is_complete, created_at, updated_at"#
                )
                .bind(measurement_id)
                .bind(user_hash)
                .bind(agent_id)
                .bind(sent_probes)
                .bind(is_complete)
                .bind(now)
                .fetch_one(pool)
                .await?;

                Ok(record)
            }
            DatabaseImpl::Mock(storage) => {
                let mut tracking = storage.measurement_tracking.lock().unwrap();

                if let Some(record) = tracking.iter_mut().find(|t| {
                    t.measurement_id == measurement_id
                        && t.user_hash == user_hash
                        && t.agent_id == agent_id
                }) {
                    record.sent_probes = sent_probes;
                    record.is_complete = is_complete;
                    record.updated_at = now;
                    Ok(record.clone())
                } else {
                    Err(sqlx::Error::RowNotFound)
                }
            }
        }
    }

    /// Get measurement status for a specific measurement
    pub async fn get_measurement_status(
        &self,
        measurement_id: Uuid,
        user_hash: &str,
    ) -> Result<Option<MeasurementStatus>, sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let status = sqlx::query_as::<_, MeasurementStatus>(
                    r#"SELECT
                       measurement_id,
                       user_hash,
                       total_agents,
                       total_expected_probes,
                       total_sent_probes,
                       completed_agents,
                       measurement_complete,
                       started_at,
                       last_updated
                       FROM measurement_status
                       WHERE measurement_id = $1 AND user_hash = $2"#,
                )
                .bind(measurement_id)
                .bind(user_hash)
                .fetch_optional(pool)
                .await?;

                Ok(status)
            }
            DatabaseImpl::Mock(storage) => {
                let tracking = storage.measurement_tracking.lock().unwrap();
                let records: Vec<_> = tracking
                    .iter()
                    .filter(|t| t.measurement_id == measurement_id && t.user_hash == user_hash)
                    .collect();

                if records.is_empty() {
                    return Ok(None);
                }

                let total_agents = records.len() as i64;
                let total_expected_probes: i64 =
                    records.iter().map(|r| r.expected_probes as i64).sum();
                let total_sent_probes: i64 = records.iter().map(|r| r.sent_probes as i64).sum();
                let completed_agents = records.iter().filter(|r| r.is_complete).count() as i64;
                let measurement_complete = completed_agents == total_agents;
                let started_at = records.iter().map(|r| r.created_at).min().unwrap();
                let last_updated = records.iter().map(|r| r.updated_at).max().unwrap();

                Ok(Some(MeasurementStatus {
                    measurement_id,
                    user_hash: user_hash.to_string(),
                    total_agents,
                    total_expected_probes,
                    total_sent_probes,
                    completed_agents,
                    measurement_complete,
                    started_at,
                    last_updated,
                }))
            }
        }
    }

    /// Get all measurement tracking entries for a measurement
    pub async fn get_measurement_tracking(
        &self,
        measurement_id: Uuid,
        user_hash: &str,
    ) -> Result<Vec<MeasurementTracking>, sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let records = sqlx::query_as::<_, MeasurementTracking>(
                    r#"SELECT id, user_hash, measurement_id, agent_id, expected_probes, sent_probes, is_complete, created_at, updated_at
                       FROM measurement_tracking
                       WHERE measurement_id = $1 AND user_hash = $2
                       ORDER BY created_at"#
                )
                .bind(measurement_id)
                .bind(user_hash)
                .fetch_all(pool)
                .await?;

                Ok(records)
            }
            DatabaseImpl::Mock(storage) => {
                let tracking = storage.measurement_tracking.lock().unwrap();
                let mut records: Vec<_> = tracking
                    .iter()
                    .filter(|t| t.measurement_id == measurement_id && t.user_hash == user_hash)
                    .cloned()
                    .collect();

                records.sort_by(|a, b| a.created_at.cmp(&b.created_at));
                Ok(records)
            }
        }
    }

    /// Get measurement tracking entry by measurement ID and agent ID (for agent updates)
    pub async fn get_measurement_tracking_by_agent(
        &self,
        measurement_id: Uuid,
        agent_id: &str,
    ) -> Result<Option<MeasurementTracking>, sqlx::Error> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                let record = sqlx::query_as::<_, MeasurementTracking>(
                    r#"SELECT id, user_hash, measurement_id, agent_id, expected_probes, sent_probes, is_complete, created_at, updated_at
                       FROM measurement_tracking
                       WHERE measurement_id = $1 AND agent_id = $2"#
                )
                .bind(measurement_id)
                .bind(agent_id)
                .fetch_optional(pool)
                .await?;

                Ok(record)
            }
            DatabaseImpl::Mock(storage) => {
                let tracking = storage.measurement_tracking.lock().unwrap();
                let record = tracking
                    .iter()
                    .find(|t| t.measurement_id == measurement_id && t.agent_id == agent_id)
                    .cloned();

                Ok(record)
            }
        }
    }

    pub fn get_pool(&self) -> Option<&PgPool> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => Some(pool),
            DatabaseImpl::Mock(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_user_id() {
        let user_id = "test-user-123";
        let hash1 = hash_user_identifier(user_id);
        let hash2 = hash_user_identifier(user_id);

        // Same input should produce same hash
        assert_eq!(hash1, hash2);

        // Hash should be 64 characters (32 bytes * 2 hex chars)
        assert_eq!(hash1.len(), 64);

        // Different inputs should produce different hashes
        let different_hash = hash_user_identifier("different-user");
        assert_ne!(hash1, different_hash);
    }

    #[tokio::test]
    async fn test_mock_database() {
        let db = Database::new_mock();

        // Initialize mock database
        db.initialize().await.unwrap();

        // Test recording probe usage via measurement tracking
        let user_id = "test-user";
        let probe_count = 5;
        let measurement_id = Uuid::new_v4();
        let agent_id = "test-agent";

        // Create measurement tracking entry
        let _tracking = db
            .create_measurement_tracking(
                &hash_user_identifier(user_id),
                measurement_id,
                agent_id,
                5,
            )
            .await
            .unwrap();

        // Update measurement tracking with probe count
        let _updated = db
            .update_measurement_probe_count(
                measurement_id,
                &hash_user_identifier(user_id),
                agent_id,
                probe_count,
                true,
            )
            .await
            .unwrap();

        // Test getting user stats
        let stats = db.get_user_usage_stats(user_id, None, None).await.unwrap();
        assert_eq!(stats.submission_count, 1);
        assert_eq!(stats.total_probes, 5);
        assert!(
            stats.last_submitted.is_some(),
            "last_submitted should be set"
        );

        // Test getting recent usage
        let recent = db.get_recent_usage(Some(10)).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].probe_count, probe_count);
    }

    #[tokio::test]
    async fn test_user_limits() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let user_id = "test-user";

        // Test default limit (no limit set)
        let limit = db.get_user_limit(user_id).await.unwrap();
        assert!(limit.is_none());

        // Test setting a user limit
        let new_limit = db.set_user_limit(user_id, 5000).await.unwrap();
        assert_eq!(new_limit.probe_limit, 5000);
        assert_eq!(new_limit.user_hash, hash_user_identifier(user_id));

        // Test getting the user limit
        let retrieved_limit = db.get_user_limit(user_id).await.unwrap();
        assert!(retrieved_limit.is_some());
        let retrieved_limit = retrieved_limit.unwrap();
        assert_eq!(retrieved_limit.probe_limit, 5000);
        assert_eq!(retrieved_limit.user_hash, hash_user_identifier(user_id));

        // Test updating the user limit
        let updated_limit = db.set_user_limit(user_id, 8000).await.unwrap();
        assert_eq!(updated_limit.probe_limit, 8000);
        assert!(updated_limit.updated_at >= updated_limit.created_at);

        // Test that stats include the custom limit
        let stats = db.get_user_usage_stats(user_id, None, None).await.unwrap();
        assert_eq!(stats.limit, 8000);
    }

    #[tokio::test]
    async fn test_rate_limiting() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let user_id = "test-user";

        // Set a low limit for testing
        db.set_user_limit(user_id, 100).await.unwrap();

        // Test that user can submit probes within limit
        let can_submit = db.can_user_submit_probes(user_id, 50, None).await.unwrap();
        assert!(can_submit);

        // Record some usage via measurement tracking
        let measurement_id = Uuid::new_v4();
        let agent_id = "test-agent";
        let _tracking = db
            .create_measurement_tracking(
                &hash_user_identifier(user_id),
                measurement_id,
                agent_id,
                60,
            )
            .await
            .unwrap();
        let _updated = db
            .update_measurement_probe_count(
                measurement_id,
                &hash_user_identifier(user_id),
                agent_id,
                60,
                true,
            )
            .await
            .unwrap();

        // Test that user can still submit more probes within limit
        let can_submit = db.can_user_submit_probes(user_id, 30, None).await.unwrap();
        assert!(can_submit);

        // Test that user cannot exceed limit
        let can_submit = db.can_user_submit_probes(user_id, 50, None).await.unwrap();
        assert!(!can_submit);

        // Test with exactly at limit
        let can_submit = db.can_user_submit_probes(user_id, 40, None).await.unwrap();
        assert!(can_submit);

        // Test with one over limit
        let can_submit = db.can_user_submit_probes(user_id, 41, None).await.unwrap();
        assert!(!can_submit);
    }

    #[tokio::test]
    async fn test_default_limit_behavior() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let user_id = "test-user";

        // Test that default limit is used when no limit is set
        let stats = db.get_user_usage_stats(user_id, None, None).await.unwrap();
        assert_eq!(stats.limit, DEFAULT_PROBE_LIMIT); // Default limit

        // Test rate limiting with default limit
        let can_submit = db
            .can_user_submit_probes(user_id, 5000, None)
            .await
            .unwrap();
        assert!(can_submit);

        let can_submit = db
            .can_user_submit_probes(user_id, 15000, None)
            .await
            .unwrap();
        assert!(!can_submit);
    }

    #[tokio::test]
    async fn test_daily_rate_limiting() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let user_id = "test-user";

        // Set a limit for testing
        db.set_user_limit(user_id, 100).await.unwrap();

        // Test daily usage calculation
        let today_start = Utc::now()
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc();

        // Get today's stats (should be zero initially)
        let stats = db
            .get_user_usage_stats(user_id, Some(today_start), None)
            .await
            .unwrap();
        assert_eq!(stats.total_probes, 0);
        assert_eq!(stats.limit, 100);

        // Record some usage today via measurement tracking
        let measurement_id = Uuid::new_v4();
        let agent_id = "test-agent";
        let _tracking = db
            .create_measurement_tracking(
                &hash_user_identifier(user_id),
                measurement_id,
                agent_id,
                50,
            )
            .await
            .unwrap();
        let _updated = db
            .update_measurement_probe_count(
                measurement_id,
                &hash_user_identifier(user_id),
                agent_id,
                50,
                true,
            )
            .await
            .unwrap();

        // Test that today's usage is calculated correctly (use None for end_time to get current time)
        let stats = db
            .get_user_usage_stats(user_id, Some(today_start), None)
            .await
            .unwrap();
        assert_eq!(stats.total_probes, 50);

        // Test daily rate limiting
        let can_submit = db.can_user_submit_probes(user_id, 30, None).await.unwrap();
        assert!(can_submit); // 50 + 30 = 80 <= 100

        let can_submit = db.can_user_submit_probes(user_id, 60, None).await.unwrap();
        assert!(!can_submit); // 50 + 60 = 110 > 100

        // Test default behavior (today's usage)
        let stats = db.get_user_usage_stats(user_id, None, None).await.unwrap();
        assert_eq!(stats.total_probes, 50); // Should be today's usage
    }

    #[tokio::test]
    async fn test_measurement_tracking() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let measurement_id = Uuid::new_v4();
        let user_hash = "test_user_hash";
        let agent1_id = "agent1";
        let agent2_id = "agent2";

        // Test creating measurement tracking for multiple agents
        let tracking1 = db
            .create_measurement_tracking(user_hash, measurement_id, agent1_id, 100)
            .await
            .unwrap();

        let _tracking2 = db
            .create_measurement_tracking(user_hash, measurement_id, agent2_id, 100)
            .await
            .unwrap();

        assert_eq!(tracking1.measurement_id, measurement_id);
        assert_eq!(tracking1.agent_id, agent1_id);
        assert_eq!(tracking1.expected_probes, 100);
        assert_eq!(tracking1.sent_probes, 0);
        assert!(!tracking1.is_complete);

        // Test updating probe count for agent1 (partial completion)
        let updated_tracking1 = db
            .update_measurement_probe_count(measurement_id, user_hash, agent1_id, 50, false)
            .await
            .unwrap();

        assert_eq!(updated_tracking1.sent_probes, 50);
        assert!(!updated_tracking1.is_complete);

        // Test completing agent1
        let completed_tracking1 = db
            .update_measurement_probe_count(measurement_id, user_hash, agent1_id, 100, true)
            .await
            .unwrap();

        assert_eq!(completed_tracking1.sent_probes, 100);
        assert!(completed_tracking1.is_complete);

        // Test getting measurement status (should show 1 of 2 agents complete)
        let status = db
            .get_measurement_status(measurement_id, user_hash)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(status.total_agents, 2);
        assert_eq!(status.completed_agents, 1);
        assert_eq!(status.total_expected_probes, 200); // 2 agents * 100 each
        assert_eq!(status.total_sent_probes, 100); // Only agent1 has sent probes
        assert!(!status.measurement_complete); // Not all agents complete

        // Complete agent2
        db.update_measurement_probe_count(measurement_id, user_hash, agent2_id, 100, true)
            .await
            .unwrap();

        // Test measurement is now complete
        let final_status = db
            .get_measurement_status(measurement_id, user_hash)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(final_status.completed_agents, 2);
        assert_eq!(final_status.total_sent_probes, 200);
        assert!(final_status.measurement_complete);

        // Test getting detailed tracking
        let tracking_details = db
            .get_measurement_tracking(measurement_id, user_hash)
            .await
            .unwrap();

        assert_eq!(tracking_details.len(), 2);
        for tracking in &tracking_details {
            assert_eq!(tracking.sent_probes, 100);
            assert!(tracking.is_complete);
        }
    }

    #[tokio::test]
    async fn test_per_agent_completion_tracking() {
        let db = Database::new_mock();
        db.initialize().await.unwrap();

        let measurement_id = Uuid::new_v4();
        let user_hash = "test_user_hash";
        let agent1_id = "fast_agent";
        let agent2_id = "slow_agent";
        let agent3_id = "failing_agent";

        // Create tracking for 3 agents
        for agent_id in [agent1_id, agent2_id, agent3_id] {
            db.create_measurement_tracking(user_hash, measurement_id, agent_id, 50)
                .await
                .unwrap();
        }

        // Agent1 completes quickly
        db.update_measurement_probe_count(measurement_id, user_hash, agent1_id, 50, true)
            .await
            .unwrap();

        // Agent2 makes partial progress
        db.update_measurement_probe_count(measurement_id, user_hash, agent2_id, 25, false)
            .await
            .unwrap();

        // Agent3 fails (0 probes sent)
        db.update_measurement_probe_count(measurement_id, user_hash, agent3_id, 0, true)
            .await
            .unwrap();

        // Check status shows per-agent completion
        let status = db
            .get_measurement_status(measurement_id, user_hash)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(status.total_agents, 3);
        assert_eq!(status.completed_agents, 2); // agent1 and agent3 marked complete
        assert_eq!(status.total_expected_probes, 150); // 3 agents * 50 each
        assert_eq!(status.total_sent_probes, 75); // 50 + 25 + 0
        assert!(!status.measurement_complete); // agent2 not complete yet

        // Get detailed tracking to verify per-agent status
        let tracking_details = db
            .get_measurement_tracking(measurement_id, user_hash)
            .await
            .unwrap();

        let agent1_tracking = tracking_details
            .iter()
            .find(|t| t.agent_id == agent1_id)
            .unwrap();
        let agent2_tracking = tracking_details
            .iter()
            .find(|t| t.agent_id == agent2_id)
            .unwrap();
        let agent3_tracking = tracking_details
            .iter()
            .find(|t| t.agent_id == agent3_id)
            .unwrap();

        assert_eq!(agent1_tracking.sent_probes, 50);
        assert!(agent1_tracking.is_complete);

        assert_eq!(agent2_tracking.sent_probes, 25);
        assert!(!agent2_tracking.is_complete);

        assert_eq!(agent3_tracking.sent_probes, 0);
        assert!(agent3_tracking.is_complete); // Marked complete even with 0 probes (failure case)

        // Agent2 finishes
        db.update_measurement_probe_count(measurement_id, user_hash, agent2_id, 50, true)
            .await
            .unwrap();

        // Verify measurement is now complete
        let final_status = db
            .get_measurement_status(measurement_id, user_hash)
            .await
            .unwrap()
            .unwrap();

        assert!(final_status.measurement_complete);
        assert_eq!(final_status.completed_agents, 3);
        assert_eq!(final_status.total_sent_probes, 100); // 50 + 50 + 0
    }

    #[tokio::test]
    async fn test_database_integration() {
        // Use mock database instead of real database to avoid connection issues
        let db = Database::new_mock();

        // Initialize the database (run migrations)
        db.initialize().await.unwrap();

        let user_id = "integrate_test_user";
        let measurement_id = Uuid::new_v4();

        // Test setting and getting user limit
        let limit = db.set_user_limit(user_id, 500).await.unwrap();
        assert_eq!(limit.probe_limit, 500);

        let retrieved_limit = db.get_user_limit(user_id).await.unwrap().unwrap();
        assert_eq!(retrieved_limit.probe_limit, 500);

        // Test measurement tracking
        let agent_id = "test_agent";
        let user_hash = &hash_user_identifier(user_id);
        let tracking = db
            .create_measurement_tracking(user_hash, measurement_id, agent_id, 50)
            .await
            .unwrap();
        assert_eq!(tracking.expected_probes, 50);
        assert_eq!(tracking.sent_probes, 0);
        assert!(!tracking.is_complete);

        // Update and complete the measurement tracking
        let updated_tracking = db
            .update_measurement_probe_count(measurement_id, user_hash, agent_id, 50, true)
            .await
            .unwrap();
        assert_eq!(updated_tracking.sent_probes, 50);
        assert!(updated_tracking.is_complete);

        // Check measurement status
        let status = db
            .get_measurement_status(measurement_id, user_hash)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(status.total_agents, 1);
        assert_eq!(status.completed_agents, 1);
        assert_eq!(status.total_expected_probes, 50);
        assert_eq!(status.total_sent_probes, 50);
        assert!(status.measurement_complete);

        // Test user usage stats
        let stats = db.get_user_usage_stats(user_id, None, None).await.unwrap();
        assert_eq!(stats.total_probes, 50);
        assert_eq!(stats.limit, 500);
        assert_eq!(stats.submission_count, 1);
    }
}
