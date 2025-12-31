use sqlx::sqlite::SqlitePool;
use sqlx::migrate::MigrateDatabase;
use anyhow::{Result, Context};
use std::env;

pub type DbPool = SqlitePool;

pub async fn init_db() -> Result<DbPool> {
    let database_url = env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:waste_bot.db".to_string());

    if !sqlx::Sqlite::database_exists(&database_url).await.unwrap_or(false) {
        println!("Creating database {}", database_url);
        sqlx::Sqlite::create_database(&database_url).await.context("Failed to create database")?;
    } else {
        println!("Database {} already exists", database_url);
    }

    let pool = SqlitePool::connect(&database_url).await.context("Failed to connect to database")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("Failed to run migrations")?;

    Ok(pool)
}
