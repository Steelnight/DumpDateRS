use sqlx::SqlitePool;
use anyhow::Result;
use crate::waste::PickupEvent;

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
    sqlx::query!("UPDATE users SET notify_time = ? WHERE id = ?", time, chat_id)
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
pub async fn upsert_events(pool: &SqlitePool, location_id: &str, events: &[PickupEvent]) -> Result<()> {
    let mut tx = pool.begin().await?;

    // Strategy: Delete all FUTURE events for this location, then insert the new ones.
    // This handles moved dates correctly. Past events should probably be left alone or cleaned up separately.
    // However, if the feed contains past events, we might duplicate them if we don't delete them.
    // Usually iCal feeds contain a window.
    // Let's delete ALL events for this location to be safe and ensure sync,
    // BUT only if we successfully insert the new ones (which the transaction ensures).
    // Wait, if we delete past events, we lose history? Not critical for this bot.
    // Let's safe-guard: Delete events >= today.

    let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();

    sqlx::query!(
        "DELETE FROM pickup_events WHERE location_id = ? AND date >= ?",
        location_id,
        today
    )
    .execute(&mut *tx)
    .await?;

    for event in events {
        // Only insert events that are >= today (or just insert everything from the feed?
        // If the feed has old events and we didn't delete them, we might get duplicates if we didn't use ON CONFLICT)
        // Since we only deleted >= today, we should probably only insert >= today to avoid conflict on old events
        // OR we use INSERT OR REPLACE/IGNORE for the rest.

        let date_str = event.date.format("%Y-%m-%d").to_string();
        if date_str < today {
            continue;
        }

        for waste in &event.waste_types {
            let waste_str = waste.as_str();
            sqlx::query!(
                "INSERT INTO pickup_events (location_id, date, waste_type) VALUES (?, ?, ?)",
                location_id,
                date_str,
                waste_str
            )
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(())
}

// Query for notifications
#[allow(dead_code)]
pub struct NotificationTask {
    pub chat_id: i64,
    pub waste_type: String,
    pub event_date: String,
}

pub async fn get_users_to_notify(pool: &SqlitePool, check_time: &str, current_date: &str, next_date: &str) -> Result<Vec<NotificationTask>> {
    // check_time is '06:00' or '18:00'
    // If '06:00', we notify for events TODAY (current_date)
    // If '18:00', we notify for events TOMORROW (next_date)

    // Logic:
    // Select users where notify_time = check_time
    // Join subscriptions
    // Join pickup_events matching location_id and waste_type and date

    let target_date = if check_time == "06:00" { current_date } else { next_date };

    let rows = sqlx::query!(
        r#"
        SELECT u.id as chat_id, s.waste_type, e.date as event_date
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

    Ok(rows.into_iter().map(|r| NotificationTask {
        chat_id: r.chat_id.unwrap_or(0),
        waste_type: r.waste_type,
        event_date: r.event_date.to_string(), // chrono::NaiveDate via sqlx might need conversion if mapped
    }).collect())
}
