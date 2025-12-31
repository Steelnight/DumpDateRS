#[cfg(test)]
mod tests {
    use super::super::store::*;
    use super::super::waste::{PickupEvent, WasteType};
    use sqlx::sqlite::SqlitePoolOptions;
    use sqlx::migrate::MigrateDatabase;
    use chrono::NaiveDate;

    async fn setup_db() -> sqlx::SqlitePool {
        let db_url = "sqlite::memory:";
        let pool = SqlitePoolOptions::new()
            .connect(db_url)
            .await
            .unwrap();

        sqlx::migrate!("./migrations")
            .run(&pool)
            .await
            .unwrap();

        pool
    }

    #[tokio::test]
    async fn test_user_crud() {
        let pool = setup_db().await;

        // Create
        create_user(&pool, 12345, "LOC1").await.unwrap();

        // Read
        let user = get_user(&pool, 12345).await.unwrap().unwrap();
        assert_eq!(user.0, "LOC1");
        assert_eq!(user.1, "18:00");

        // Update Time
        update_notify_time(&pool, 12345, "06:00").await.unwrap();
        let user = get_user(&pool, 12345).await.unwrap().unwrap();
        assert_eq!(user.1, "06:00");

        // Delete
        delete_user(&pool, 12345).await.unwrap();
        let user = get_user(&pool, 12345).await.unwrap();
        assert!(user.is_none());
    }

    #[tokio::test]
    async fn test_subscriptions() {
        let pool = setup_db().await;
        create_user(&pool, 12345, "LOC1").await.unwrap();

        // Add subs
        add_subscription(&pool, 12345, "Bio").await.unwrap();
        add_subscription(&pool, 12345, "Rest").await.unwrap();

        let subs = get_subscriptions(&pool, 12345).await.unwrap();
        assert_eq!(subs.len(), 2);
        assert!(subs.contains(&"Bio".to_string()));
        assert!(subs.contains(&"Rest".to_string()));

        // Remove sub
        remove_subscription(&pool, 12345, "Bio").await.unwrap();
        let subs = get_subscriptions(&pool, 12345).await.unwrap();
        assert_eq!(subs.len(), 1);
        assert!(subs.contains(&"Rest".to_string()));

        // Cascading delete
        delete_user(&pool, 12345).await.unwrap();
        // Verify subs are gone (raw query or ensure no constraint error on re-insert if we were checking that)
        // Check manually:
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM subscriptions WHERE user_id = 12345")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_pickup_events() {
        let pool = setup_db().await;

        // Use dates far in the future to pass the ">= today" check in upsert_events
        let events = vec![
            PickupEvent {
                date: NaiveDate::from_ymd_opt(2099, 10, 27).unwrap(),
                waste_types: vec![WasteType::Bio, WasteType::Rest],
            },
            PickupEvent {
                date: NaiveDate::from_ymd_opt(2099, 10, 28).unwrap(),
                waste_types: vec![WasteType::Yellow],
            }
        ];

        upsert_events(&pool, "LOC1", &events).await.unwrap();

        // Query to verify
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM pickup_events")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 3); // Bio, Rest, Yellow
    }

    #[tokio::test]
    async fn test_notification_query() {
        let pool = setup_db().await;
        create_user(&pool, 1, "LOC1").await.unwrap();
        add_subscription(&pool, 1, "Bio").await.unwrap();
        update_notify_time(&pool, 1, "18:00").await.unwrap();

        create_user(&pool, 2, "LOC1").await.unwrap();
        add_subscription(&pool, 2, "Rest").await.unwrap();
        update_notify_time(&pool, 2, "06:00").await.unwrap();

        // Use future dates
        let events = vec![
            PickupEvent {
                date: NaiveDate::from_ymd_opt(2099, 10, 28).unwrap(),
                waste_types: vec![WasteType::Bio],
            }
        ];
        upsert_events(&pool, "LOC1", &events).await.unwrap();

        // Case 1: 18:00 check for tomorrow (2099-10-28)
        // User 1 should get notified (subscribed to Bio, notifies at 18:00)
        let tasks = get_users_to_notify(&pool, "18:00", "2099-10-27", "2099-10-28").await.unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].chat_id, 1);
        assert_eq!(tasks[0].waste_type, "Bio");

        // Case 2: 06:00 check for today (2099-10-28)
        // User 2 should get notified if they were subscribed to Bio, but they are subscribed to Rest.
        // User 1 is 18:00, so filtered out.
        let tasks = get_users_to_notify(&pool, "06:00", "2099-10-28", "2099-10-29").await.unwrap();
        assert_eq!(tasks.len(), 0);
    }
}
