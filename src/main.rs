mod db;
mod waste;
mod store;
mod bot_handler;
mod scheduler;
#[cfg(test)]
mod db_tests;

use db::init_db;
use teloxide::prelude::*;
use dotenvy::dotenv;
use log::info;
use std::error::Error;
use bot_handler::run_bot;
use scheduler::run_scheduler;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    dotenv().ok();
    env_logger::init();

    info!("Starting Dresden Waste Bot...");

    let pool = init_db().await?;
    info!("Database initialized and migrations run.");

    let bot = Bot::from_env();

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
