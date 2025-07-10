// Quick test to verify the async user ID assignment works correctly
use saimiris_gateway::{database::Database, get_or_create_user_id};

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
