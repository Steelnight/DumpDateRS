use anyhow::{Context, Result};
use sqlx::migrate::MigrateDatabase;
use sqlx::sqlite::SqlitePool;
use std::env;
use std::str::FromStr;

pub type DbPool = SqlitePool;

pub async fn create_schema(pool: &DbPool) -> Result<()> {
    // Since we are not migrating data, we drop tables to ensure schema matches.
    // Order matters for foreign keys.
    sqlx::query("DROP TABLE IF EXISTS subscriptions")
        .execute(pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS user_locations")
        .execute(pool)
        .await?;
    sqlx::query("DROP TABLE IF EXISTS users")
        .execute(pool)
        .await?;
    // pickup_events does not depend on users, so we can keep it,
    // but users might refer to it? No, users refer to location_id which is a string.
    // However, if we want a clean slate for everything, maybe drop pickup_events too?
    // The requirement didn't say drop event data, but it might be safer.
    // Let's keep pickup_events for now as it's just a cache of external data.

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

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_user_locations_user_id ON user_locations(user_id);",
    )
    .execute(pool)
    .await
    .context("Failed to create index on user_locations(user_id)")?;

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
