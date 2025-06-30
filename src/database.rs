use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row};
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

// Mock storage for testing
#[derive(Debug, Clone)]
pub(crate) struct MockStorage {
    records: Arc<Mutex<Vec<ProbeUsageRecord>>>,
    user_limits: Arc<Mutex<Vec<UserLimit>>>,
}

const DEFAULT_PROBE_LIMIT: u32 = 10_000; // Default probe limit for users

impl MockStorage {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
            user_limits: Arc::new(Mutex::new(Vec::new())),
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
        let records = self.records.lock().unwrap();
        let filtered_records: Vec<_> = records
            .iter()
            .filter(|r| {
                r.user_hash == user_hash && r.timestamp >= start_time && r.timestamp <= end_time
            })
            .collect();

        let last_submitted = filtered_records.iter().map(|r| r.timestamp).max();

        // Get user limit from mock storage
        let user_limits = self.user_limits.lock().unwrap();
        let limit = user_limits
            .iter()
            .find(|ul| ul.user_hash == user_hash)
            .map(|ul| ul.probe_limit)
            .unwrap_or(DEFAULT_PROBE_LIMIT); // Default limit if not found

        UserUsageStats {
            user_hash: user_hash.to_string(),
            submission_count: filtered_records.len() as u32,
            total_probes: filtered_records.iter().map(|r| r.probe_count as u32).sum(),
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
        let mut records = self.records.lock().unwrap().clone();
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

    pub async fn record_probe_usage(
        &self,
        user_id: &str,
        probe_count: i32,
    ) -> Result<ProbeUsageRecord, sqlx::Error> {
        let user_hash = hash_user_id(user_id);
        let timestamp = Utc::now();
        let id = Uuid::new_v4();

        let record = ProbeUsageRecord {
            id,
            user_hash: user_hash.clone(),
            probe_count,
            timestamp,
        };

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO probe_usage (id, user_hash, probe_count, timestamp)
                    VALUES ($1, $2, $3, $4)
                    "#,
                )
                .bind(&id)
                .bind(&user_hash)
                .bind(probe_count)
                .bind(timestamp)
                .execute(pool)
                .await?;

                debug!(
                    "Recorded probe usage: user_hash={}, probe_count={}, timestamp={}",
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
        let user_hash = hash_user_id(user_id);

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

                // Then get the usage stats
                let row = sqlx::query(
                    r#"
                    SELECT
                        COUNT(*) as submission_count,
                        COALESCE(SUM(probe_count), 0) as total_probes,
                        MAX(timestamp) as last_submitted
                    FROM probe_usage
                    WHERE user_hash = $1
                    AND timestamp >= $2
                    AND timestamp <= $3
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
                    SELECT id, user_hash, probe_count, timestamp
                    FROM probe_usage
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
                        probe_count: row.get("probe_count"),
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
        let user_hash = hash_user_id(user_id);

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
        let user_hash = hash_user_id(user_id);

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

    pub fn get_pool(&self) -> Option<&PgPool> {
        match &self.impl_ {
            DatabaseImpl::Real(pool) => Some(pool),
            DatabaseImpl::Mock(_) => None,
        }
    }
}

/// Hash a user identifier (client_id or sub) to protect privacy while maintaining uniqueness
fn hash_user_id(user_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_user_id() {
        let user_id = "test-user-123";
        let hash1 = hash_user_id(user_id);
        let hash2 = hash_user_id(user_id);

        // Same input should produce same hash
        assert_eq!(hash1, hash2);

        // Hash should be 64 characters (32 bytes * 2 hex chars)
        assert_eq!(hash1.len(), 64);

        // Different inputs should produce different hashes
        let different_hash = hash_user_id("different-user");
        assert_ne!(hash1, different_hash);
    }

    #[tokio::test]
    async fn test_mock_database() {
        let db = Database::new_mock();

        // Initialize mock database
        db.initialize().await.unwrap();

        // Test recording probe usage
        let user_id = "test-user";
        let probe_count = 5;
        let record = db.record_probe_usage(user_id, probe_count).await.unwrap();

        assert_eq!(record.probe_count, probe_count);
        assert_eq!(record.user_hash, hash_user_id(user_id));

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
        assert_eq!(new_limit.user_hash, hash_user_id(user_id));

        // Test getting the user limit
        let retrieved_limit = db.get_user_limit(user_id).await.unwrap();
        assert!(retrieved_limit.is_some());
        let retrieved_limit = retrieved_limit.unwrap();
        assert_eq!(retrieved_limit.probe_limit, 5000);
        assert_eq!(retrieved_limit.user_hash, hash_user_id(user_id));

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

        // Record some usage
        db.record_probe_usage(user_id, 60).await.unwrap();

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

        // Record some usage today
        db.record_probe_usage(user_id, 50).await.unwrap();

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
}
