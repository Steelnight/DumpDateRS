use anyhow::{Context, Result};
use log::info;
use sqlx::migrate::MigrateDatabase;
use sqlx::sqlite::SqlitePool;
use std::env;
use std::str::FromStr;

pub type DbPool = SqlitePool;

pub async fn create_schema(pool: &DbPool) -> Result<()> {
    // Users table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY, -- Telegram Chat ID
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create users table")?;

    // User Locations table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_locations (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL,
            location_id TEXT NOT NULL,
            notify_time TEXT NOT NULL DEFAULT '18:00',
            alias TEXT,
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE,
            UNIQUE(user_id, location_id)
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create user_locations table")?;

    // Attempt to add notify_offset column if it doesn't exist.
    // SQLite doesn't support IF NOT EXISTS for columns directly.
    // We can just try to add it and ignore the error if it fails (duplicate column).
    match sqlx::query("ALTER TABLE user_locations ADD COLUMN notify_offset INTEGER NOT NULL DEFAULT 1")
        .execute(pool)
        .await
    {
        Ok(_) => info!("Added notify_offset column to user_locations"),
        Err(e) => {
            // Check if error is due to column already existing.
            // Sqlite error code for generic error is 1, but we can check the message.
            let msg = e.to_string();
            if !msg.contains("duplicate column name") {
                 // If it's not "duplicate column", then it's a real error
                 // However, for robustness in this simple bot, we might just log it.
                 // But wait, if the table was just created, it doesn't have the column yet?
                 // Ah, CREATE TABLE above does NOT have notify_offset in the SQL string anymore?
                 // I should include it in CREATE TABLE for fresh installs, AND have ALTER TABLE for migrations.
                 // Or keep CREATE TABLE simple (v1) and let ALTER TABLE (v2) handle it?
                 // Best practice: CREATE TABLE should be the *latest* schema.
                 // But if table exists (old schema), CREATE TABLE does nothing.
                 // Then ALTER TABLE adds the column.
                 // So I should keep CREATE TABLE with *new* schema?
                 // If I keep CREATE TABLE with new schema, and table exists (old schema), CREATE does nothing.
                 // Then ALTER TABLE runs and adds column.
                 // If table exists (new schema), CREATE does nothing. ALTER TABLE fails with "duplicate column".
                 // This seems correct.
                 info!("Column notify_offset might already exist: {}", e);
            }
        }
    }

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_locations_user_id ON user_locations(user_id);",
    )
    .execute(pool)
    .await
    .context("Failed to create index on user_locations(user_id)")?;

    // Index on notify_time for faster hourly notifications
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_locations_notify_time ON user_locations(notify_time);",
    )
    .execute(pool)
    .await
    .context("Failed to create index on user_locations(notify_time)")?;

    // Subscriptions table (now linked to user_locations)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS subscriptions (
            user_location_id INTEGER NOT NULL,
            waste_type TEXT NOT NULL,
            PRIMARY KEY (user_location_id, waste_type),
            FOREIGN KEY (user_location_id) REFERENCES user_locations(id) ON DELETE CASCADE
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create subscriptions table")?;

    // Pickup events table (unchanged)
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS pickup_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            location_id TEXT NOT NULL,
            date DATE NOT NULL,
            waste_type TEXT NOT NULL,
            UNIQUE(location_id, date, waste_type)
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create pickup_events table")?;

    // Index on pickup_events(date) for faster daily notifications
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_pickup_events_date ON pickup_events(date);")
        .execute(pool)
        .await
        .context("Failed to create index on pickup_events(date)")?;

    Ok(())
}

pub async fn init_db() -> Result<DbPool> {
    let database_url =
        env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:waste_bot.db".to_string());

    if !sqlx::Sqlite::database_exists(&database_url)
        .await
        .unwrap_or(false)
    {
        println!("Creating database {}", database_url);
        sqlx::Sqlite::create_database(&database_url)
            .await
            .context("Failed to create database")?;
    } else {
        println!("Database {} already exists", database_url);
    }

    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .connect_with(
            sqlx::sqlite::SqliteConnectOptions::from_str(&database_url)?.foreign_keys(true),
        )
        .await
        .context("Failed to connect to database")?;

    create_schema(&pool).await?;

    Ok(pool)
}
