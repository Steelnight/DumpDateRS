use anyhow::{Context, Result};
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
            location_id TEXT NOT NULL,
            notify_time TEXT NOT NULL DEFAULT '18:00',
            created_at DATETIME DEFAULT CURRENT_TIMESTAMP
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create users table")?;

    // Index on users(location_id) for faster reverse lookups from events and distinct location queries
    sqlx::query("CREATE INDEX IF NOT EXISTS idx_users_location_id ON users(location_id);")
        .execute(pool)
        .await
        .context("Failed to create index on users(location_id)")?;

    // Subscriptions table
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS subscriptions (
            user_id INTEGER NOT NULL,
            waste_type TEXT NOT NULL,
            PRIMARY KEY (user_id, waste_type),
            FOREIGN KEY (user_id) REFERENCES users(id) ON DELETE CASCADE
        );",
    )
    .execute(pool)
    .await
    .context("Failed to create subscriptions table")?;

    // Pickup events table
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

    // Index on pickup_events(date) for efficient daily notification queries
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
