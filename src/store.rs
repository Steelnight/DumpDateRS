use crate::waste::PickupEvent;
use anyhow::Result;
use sqlx::{sqlite::Sqlite, QueryBuilder, SqlitePool};

// User Operations
pub async fn create_user(pool: &SqlitePool, chat_id: i64, location_id: &str) -> Result<()> {
    sqlx::query!(
        "INSERT INTO users (id, location_id) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET location_id = excluded.location_id",
        chat_id,
        location_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user(pool: &SqlitePool, chat_id: i64) -> Result<Option<(String, String)>> {
    let rec = sqlx::query!(
        "SELECT location_id, notify_time FROM users WHERE id = ?",
        chat_id
    )
    .fetch_optional(pool)
    .await?;

    Ok(rec.map(|r| (r.location_id, r.notify_time)))
}

pub async fn delete_user(pool: &SqlitePool, chat_id: i64) -> Result<()> {
    sqlx::query!("DELETE FROM users WHERE id = ?", chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_notify_time(pool: &SqlitePool, chat_id: i64, time: &str) -> Result<()> {
    sqlx::query!(
        "UPDATE users SET notify_time = ? WHERE id = ?",
        time,
        chat_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

// Subscription Operations
pub async fn add_subscription(pool: &SqlitePool, chat_id: i64, waste_type: &str) -> Result<()> {
    sqlx::query!(
        "INSERT INTO subscriptions (user_id, waste_type) VALUES (?, ?) ON CONFLICT DO NOTHING",
        chat_id,
        waste_type
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_subscription(pool: &SqlitePool, chat_id: i64, waste_type: &str) -> Result<()> {
    sqlx::query!(
        "DELETE FROM subscriptions WHERE user_id = ? AND waste_type = ?",
        chat_id,
        waste_type
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_subscriptions(pool: &SqlitePool, chat_id: i64) -> Result<Vec<String>> {
    let recs = sqlx::query!(
        "SELECT waste_type FROM subscriptions WHERE user_id = ?",
        chat_id
    )
    .fetch_all(pool)
    .await?;

    Ok(recs.into_iter().map(|r| r.waste_type).collect())
}

// Event Operations
pub async fn upsert_events(
    pool: &SqlitePool,
    location_id: &str,
    events: &[PickupEvent],
) -> Result<()> {
    let mut tx = pool.begin().await?;

    // Strategy: Delete all FUTURE events for this location, then insert the new ones.
    let today = chrono::Local::now()
        .date_naive()
        .format("%Y-%m-%d")
        .to_string();

    sqlx::query!(
        "DELETE FROM pickup_events WHERE location_id = ? AND date >= ?",
        location_id,
        today
    )
    .execute(&mut *tx)
    .await?;

    // Prepare data for batch insert with chunking to avoid large allocations
    // Optimization: Reduces database round-trips while staying within SQLite variable limits.
    // Chunk size of 250 means 750 variables per query (3 cols * 250 rows), well under the strict 999 limit.
    // Using a fixed-size buffer reduces memory pressure compared to collecting all items first.
    let mut buffer: Vec<(&str, String, &str)> = Vec::with_capacity(250);

    for event in events {
        let date_str = event.date.format("%Y-%m-%d").to_string();
        if date_str < today {
            continue;
        }

        for waste in &event.waste_types {
            buffer.push((location_id, date_str.clone(), waste.as_str()));

            if buffer.len() >= 250 {
                let mut query_builder: QueryBuilder<Sqlite> =
                    QueryBuilder::new("INSERT INTO pickup_events (location_id, date, waste_type) ");

                query_builder.push_values(&buffer, |mut b, (loc, date, waste)| {
                    b.push_bind(loc).push_bind(date).push_bind(waste);
                });

                query_builder.build().execute(&mut *tx).await?;
                buffer.clear();
            }
        }
    }

    if !buffer.is_empty() {
        let mut query_builder: QueryBuilder<Sqlite> =
            QueryBuilder::new("INSERT INTO pickup_events (location_id, date, waste_type) ");

        query_builder.push_values(&buffer, |mut b, (loc, date, waste)| {
            b.push_bind(loc).push_bind(date).push_bind(waste);
        });

        query_builder.build().execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(())
}

// Query for notifications
pub struct NotificationTask {
    pub chat_id: i64,
    pub waste_type: String,
}

pub async fn get_users_to_notify(
    pool: &SqlitePool,
    check_time: &str,
    current_date: &str,
    next_date: &str,
) -> Result<Vec<NotificationTask>> {
    // check_time is '06:00' or '18:00'
    // If '06:00', we notify for events TODAY (current_date)
    // If '18:00', we notify for events TOMORROW (next_date)

    // Logic:
    // Select users where notify_time = check_time
    // Join subscriptions
    // Join pickup_events matching location_id and waste_type and date

    let target_date = if check_time == "06:00" {
        current_date
    } else {
        next_date
    };

    let rows = sqlx::query!(
        r#"
        SELECT u.id as chat_id, s.waste_type
        FROM users u
        JOIN subscriptions s ON u.id = s.user_id
        JOIN pickup_events e ON u.location_id = e.location_id AND s.waste_type = e.waste_type
        WHERE u.notify_time = ? AND e.date = ?
        "#,
        check_time,
        target_date
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| NotificationTask {
            chat_id: r.chat_id.unwrap_or(0),
            waste_type: r.waste_type,
        })
        .collect())
}
