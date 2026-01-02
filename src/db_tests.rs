use crate::store::{
    add_subscription, add_user_location, create_user, delete_user, delete_user_location,
    get_subscriptions, get_user_locations, update_notify_time, upsert_events,
};
use crate::waste::{PickupEvent, WasteType};
use sqlx::sqlite::SqlitePoolOptions;
use std::env;
use std::str::FromStr;

#[tokio::test]
async fn test_db_operations() {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());

    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::from_str(&database_url)
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();

    crate::db::create_schema(&pool).await.unwrap();

    // Test User Creation and Location
    create_user(&pool, 12345).await.unwrap();

    // Use ignore variable to silence warning, or use it
    let _loc_id_initial = add_user_location(&pool, 12345, "LOC1", Some("Home"))
        .await
        .unwrap();

    let locations = get_user_locations(&pool, 12345).await.unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].location_id, "LOC1");
    assert_eq!(locations[0].alias.as_deref(), Some("Home"));

    // Test Notification Time Update
    update_notify_time(&pool, 12345, "LOC1", "06:00")
        .await
        .unwrap();
    let locations = get_user_locations(&pool, 12345).await.unwrap();
    assert_eq!(locations[0].notify_time, "06:00");

    // Test Delete User
    delete_user(&pool, 12345).await.unwrap();
    let locations = get_user_locations(&pool, 12345).await.unwrap();
    assert!(locations.is_empty());

    // Test Subscriptions
    // Re-add user and location
    // Note: create_user is called inside add_user_location, but let's be explicit if needed.
    // add_user_location calls create_user.

    let loc_id = add_user_location(&pool, 12345, "LOC1", Some("Home"))
        .await
        .unwrap();

    // Ensure the location exists before adding subscription
    let check = get_user_locations(&pool, 12345).await.unwrap();
    assert!(
        !check.is_empty(),
        "User location should exist after re-adding"
    );

    add_subscription(&pool, loc_id, "Bio").await.unwrap();
    let subs = get_subscriptions(&pool, loc_id).await.unwrap();
    assert_eq!(subs, vec!["Bio"]);

    // Test Events
    // Use dynamic date to ensure it's not filtered out by "today" check in upsert_events
    let today = chrono::Local::now().date_naive();
    let today_str = today.format("%Y-%m-%d").to_string();
    let tomorrow_str = (today + chrono::Duration::days(1))
        .format("%Y-%m-%d")
        .to_string();

    let event = PickupEvent {
        date: today,
        waste_types: vec![WasteType::Bio],
    };
    upsert_events(&pool, "LOC1", &[event]).await.unwrap();

    // Test Notification Query
    // We need to set notify time to match
    update_notify_time(&pool, 12345, "LOC1", "06:00")
        .await
        .unwrap();
    // Also update offset to 0 (Same Day) since we are testing "today"
    crate::store::update_notify_offset(&pool, 12345, "LOC1", 0)
        .await
        .unwrap();

    let tasks = crate::store::get_users_to_notify(
        &pool,
        "06:00",
        &today_str,    // today
        &tomorrow_str, // tomorrow
    )
    .await
    .unwrap();

    assert_eq!(tasks.len(), 1);
    assert_eq!(tasks[0].chat_id, 12345);
    assert_eq!(tasks[0].waste_type, "Bio");

    // Clean up
    delete_user(&pool, 12345).await.unwrap();
}

#[tokio::test]
async fn test_batch_insert() {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());

    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::from_str(&database_url)
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();

    crate::db::create_schema(&pool).await.unwrap();

    // Simulate 1000 events
    let mut events = Vec::new();
    let today = chrono::Local::now().date_naive();

    for i in 0..1000 {
        events.push(PickupEvent {
            date: today + chrono::Duration::days(i),
            waste_types: vec![WasteType::Bio],
        });
    }

    upsert_events(&pool, "LOC_BATCH", &events).await.unwrap();

    // Verify count
    let count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM pickup_events WHERE location_id = 'LOC_BATCH'")
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(count, 1000);
}

#[tokio::test]
async fn test_multiple_locations() {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite::memory:".to_string());

    let pool = SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::from_str(&database_url)
                .unwrap()
                .foreign_keys(true),
        )
        .await
        .unwrap();

    crate::db::create_schema(&pool).await.unwrap();

    let chat_id = 999;
    create_user(&pool, chat_id).await.unwrap();

    let _loc1_id = add_user_location(&pool, chat_id, "LOC1", Some("Home"))
        .await
        .unwrap();
    let _loc2_id = add_user_location(&pool, chat_id, "LOC2", Some("Office"))
        .await
        .unwrap();

    update_notify_time(&pool, chat_id, "LOC1", "18:00")
        .await
        .unwrap();
    update_notify_time(&pool, chat_id, "LOC2", "08:00")
        .await
        .unwrap();

    let locations = get_user_locations(&pool, chat_id).await.unwrap();
    assert_eq!(locations.len(), 2);

    let l1 = locations.iter().find(|l| l.location_id == "LOC1").unwrap();
    assert_eq!(l1.notify_time, "18:00");

    let l2 = locations.iter().find(|l| l.location_id == "LOC2").unwrap();
    assert_eq!(l2.notify_time, "08:00");

    // Test delete location by alias
    delete_user_location(&pool, chat_id, "Home").await.unwrap();
    let locations = get_user_locations(&pool, chat_id).await.unwrap();
    assert_eq!(locations.len(), 1);
    assert_eq!(locations[0].alias.as_deref(), Some("Office"));
}
