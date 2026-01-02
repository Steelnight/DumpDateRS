use crate::waste::PickupEvent;
use anyhow::Result;
use sqlx::{sqlite::Sqlite, QueryBuilder, SqlitePool};

// User Operations
pub async fn create_user(pool: &SqlitePool, chat_id: i64) -> Result<()> {
    sqlx::query!(
        "INSERT INTO users (id) VALUES (?) ON CONFLICT(id) DO NOTHING",
        chat_id
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete_user(pool: &SqlitePool, chat_id: i64) -> Result<()> {
    sqlx::query!("DELETE FROM users WHERE id = ?", chat_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_user_location(
    pool: &SqlitePool,
    chat_id: i64,
    location_id: &str,
    alias: Option<&str>,
) -> Result<i64> {
    // Ensure user exists first
    create_user(pool, chat_id).await?;

    // notify_offset default to 1 (Day Before) as per schema, but here we can be explicit or rely on default.
    // relying on DB default.
    let id = sqlx::query!(
        "INSERT INTO user_locations (user_id, location_id, alias) VALUES (?, ?, ?)
         ON CONFLICT(user_id, location_id) DO UPDATE SET alias = excluded.alias
         RETURNING id",
        chat_id,
        location_id,
        alias
    )
    .fetch_one(pool)
    .await?
    .id;

    Ok(id)
}

pub struct UserLocation {
    pub id: i64,
    pub location_id: String,
    pub notify_time: String,
    pub notify_offset: i64,
    pub alias: Option<String>,
}

pub async fn get_user_locations(pool: &SqlitePool, chat_id: i64) -> Result<Vec<UserLocation>> {
    let rows = sqlx::query!(
        "SELECT id, location_id, notify_time, notify_offset, alias FROM user_locations WHERE user_id = ?",
        chat_id
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| UserLocation {
            id: r.id.expect("id should be present"),
            location_id: r.location_id,
            notify_time: r.notify_time,
            notify_offset: r.notify_offset,
            alias: r.alias,
        })
        .collect())
}

pub async fn delete_user_location(
    pool: &SqlitePool,
    chat_id: i64,
    alias_or_id: &str,
) -> Result<bool> {
    // Try to delete by alias or exact location_id
    let result = sqlx::query!(
        "DELETE FROM user_locations WHERE user_id = ? AND (alias = ? OR location_id = ?)",
        chat_id,
        alias_or_id,
        alias_or_id
    )
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn update_notify_time(
    pool: &SqlitePool,
    chat_id: i64,
    location_alias_or_id: &str,
    time: &str,
) -> Result<bool> {
    let result = sqlx::query!(
        "UPDATE user_locations SET notify_time = ? WHERE user_id = ? AND (alias = ? OR location_id = ?)",
        time,
        chat_id,
        location_alias_or_id,
        location_alias_or_id
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_notify_offset(
    pool: &SqlitePool,
    chat_id: i64,
    location_alias_or_id: &str,
    offset: i64,
) -> Result<bool> {
    let result = sqlx::query!(
        "UPDATE user_locations SET notify_offset = ? WHERE user_id = ? AND (alias = ? OR location_id = ?)",
        offset,
        chat_id,
        location_alias_or_id,
        location_alias_or_id
    )
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

// Subscription Operations
pub async fn add_subscription(
    pool: &SqlitePool,
    user_location_id: i64,
    waste_type: &str,
) -> Result<()> {
    sqlx::query!(
        "INSERT INTO subscriptions (user_location_id, waste_type) VALUES (?, ?) ON CONFLICT DO NOTHING",
        user_location_id,
        waste_type
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove_subscription(
    pool: &SqlitePool,
    user_location_id: i64,
    waste_type: &str,
) -> Result<()> {
    sqlx::query!(
        "DELETE FROM subscriptions WHERE user_location_id = ? AND waste_type = ?",
        user_location_id,
        waste_type
    )
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_subscriptions(pool: &SqlitePool, user_location_id: i64) -> Result<Vec<String>> {
    let recs = sqlx::query!(
        "SELECT waste_type FROM subscriptions WHERE user_location_id = ?",
        user_location_id
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
    pub location_alias: Option<String>,
    pub location_id: String,
    pub notify_offset: i64,
}

pub async fn get_users_to_notify(
    pool: &SqlitePool,
    check_time: &str,
    current_date: &str,
    next_date: &str,
) -> Result<Vec<NotificationTask>> {
    // Logic:
    // Query users with matching notify_time.
    // AND check events:
    // (notify_offset = 0 AND date = current_date) OR (notify_offset = 1 AND date = next_date)

    let rows = sqlx::query!(
        r#"
        SELECT u.id as chat_id, s.waste_type, ul.alias, ul.location_id, ul.notify_offset
        FROM users u
        JOIN user_locations ul ON u.id = ul.user_id
        JOIN subscriptions s ON ul.id = s.user_location_id
        JOIN pickup_events e ON ul.location_id = e.location_id AND s.waste_type = e.waste_type
        WHERE ul.notify_time = ?
          AND (
               (ul.notify_offset = 0 AND e.date = ?)
            OR (ul.notify_offset = 1 AND e.date = ?)
          )
        "#,
        check_time,
        current_date,
        next_date
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| NotificationTask {
            chat_id: r.chat_id,
            waste_type: r.waste_type,
            location_alias: r.alias,
            location_id: r.location_id,
            notify_offset: r.notify_offset,
        })
        .collect())
}
