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
    pub last_submitted: Option<DateTime<Utc>>,
}

// Mock storage for testing
#[derive(Debug, Clone)]
pub(crate) struct MockStorage {
    records: Arc<Mutex<Vec<ProbeUsageRecord>>>,
}

impl MockStorage {
    fn new() -> Self {
        Self {
            records: Arc::new(Mutex::new(Vec::new())),
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

        UserUsageStats {
            user_hash: user_hash.to_string(),
            submission_count: filtered_records.len() as u32,
            total_probes: filtered_records.iter().map(|r| r.probe_count as u32).sum(),
            last_submitted,
        }
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
        let start_time = start_time.unwrap_or_else(|| Utc::now() - chrono::Duration::days(30));
        let end_time = end_time.unwrap_or_else(Utc::now);

        match &self.impl_ {
            DatabaseImpl::Real(pool) => {
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
}
