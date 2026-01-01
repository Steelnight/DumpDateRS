mod bot_handler;
mod db;
#[cfg(test)]
mod db_tests;
mod scheduler;
mod store;
mod waste;

use bot_handler::run_bot;
use db::init_db;
use dotenvy::dotenv;
use log::{error, info};
use scheduler::run_scheduler;
use std::env;
use std::error::Error;
use teloxide::prelude::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    env_logger::init();

    info!("Starting Dresden Waste Bot...");

    let pool = init_db().await?;
    info!("Database initialized and migrations run.");

    // Replace Bot::from_env() to avoid unwrap/panic
    let token = env::var("TELOXIDE_TOKEN").map_err(|_| {
        error!("TELOXIDE_TOKEN environment variable is not set");
        "TELOXIDE_TOKEN environment variable is not set"
    })?;

    let bot = Bot::new(token);

    // Start Scheduler
    let bot_clone = bot.clone();
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        run_scheduler(bot_clone, pool_clone).await;
    });

    // Run the bot
    run_bot(bot, pool).await;

    Ok(())
}
