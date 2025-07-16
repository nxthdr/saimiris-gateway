use std::sync::Arc;

// Quick test to verify the async user ID assignment works correctly
use saimiris_gateway::{database::Database, get_or_create_user_id, hash_user_identifier};
use tokio::sync::Barrier;

#[tokio::test]
async fn test_async_user_id_assignment() {
    // Create a mock database
    let db = Database::new_mock();

    let user1 = "user1@example.com";
    let user2 = "user2@example.com";

    // Get user IDs for both users
    let id1_first = get_or_create_user_id(&db, user1).await.unwrap();
    let id2_first = get_or_create_user_id(&db, user2).await.unwrap();

    // Different users should get different IDs
    assert_ne!(id1_first, id2_first);

    // Same user should get the same ID on subsequent calls
    let id1_second = get_or_create_user_id(&db, user1).await.unwrap();
    let id2_second = get_or_create_user_id(&db, user2).await.unwrap();

    assert_eq!(id1_first, id1_second);
    assert_eq!(id2_first, id2_second);

    println!("✅ Async user ID assignment test passed:");
    println!("   User 1: {} -> ID: {}", user1, id1_first);
    println!("   User 2: {} -> ID: {}", user2, id2_first);
}

#[tokio::test]
async fn test_concurrent_user_creation() {
    // Create a mock database
    let database = Database::new_mock();
    let user_identifier = "test-user-concurrent";

    // Number of concurrent requests to simulate
    const CONCURRENCY: usize = 10;

    // Create a barrier to synchronize all tasks
    let barrier = Arc::new(Barrier::new(CONCURRENCY));

    // Spawn concurrent tasks that all try to create the same user
    let mut handles = Vec::new();
    for i in 0..CONCURRENCY {
        let db = database.clone();
        let user_id = user_identifier.to_string();
        let barrier_clone = barrier.clone();

        let handle = tokio::spawn(async move {
            // Wait for all tasks to be ready
            barrier_clone.wait().await;

            // All tasks try to get/create user ID simultaneously
            let result = get_or_create_user_id(&db, &user_id).await;
            (i, result)
        });
        handles.push(handle);
    }

    // Wait for all tasks to complete
    let mut results = Vec::new();
    for handle in handles {
        let result = handle.await.unwrap();
        results.push(result);
    }

    // All requests should succeed
    let mut user_ids = Vec::new();
    for (task_id, result) in results {
        match result {
            Ok(user_id) => {
                user_ids.push(user_id);
                println!("Task {} got user ID: {}", task_id, user_id);
            }
            Err(e) => panic!("Task {} failed: {}", task_id, e),
        }
    }

    // All tasks should get the same user ID
    assert!(!user_ids.is_empty(), "No user IDs were generated");
    let first_user_id = user_ids[0];
    for user_id in &user_ids {
        assert_eq!(
            *user_id, first_user_id,
            "All concurrent requests should get the same user ID"
        );
    }

    println!(
        "All {} concurrent requests successfully got user ID: {}",
        CONCURRENCY, first_user_id
    );

    // Verify the user was actually created in the database
    let user_hash = hash_user_identifier(user_identifier);
    let stored_user_id = database.get_user_id_by_hash(&user_hash).await.unwrap();
    assert_eq!(
        stored_user_id,
        Some(first_user_id),
        "User ID should be stored in database"
    );
}

#[tokio::test]
async fn test_single_user_creation() {
    let database = Database::new_mock();
    let user_identifier = "test-user-single";

    // First call should create the user
    let user_id1 = get_or_create_user_id(&database, user_identifier)
        .await
        .unwrap();

    // Second call should return the same user ID
    let user_id2 = get_or_create_user_id(&database, user_identifier)
        .await
        .unwrap();

    assert_eq!(
        user_id1, user_id2,
        "Same user should get same ID on subsequent calls"
    );

    // Verify the user was stored correctly
    let user_hash = hash_user_identifier(user_identifier);
    let stored_user_id = database.get_user_id_by_hash(&user_hash).await.unwrap();
    assert_eq!(
        stored_user_id,
        Some(user_id1),
        "User ID should be stored in database"
    );
}
