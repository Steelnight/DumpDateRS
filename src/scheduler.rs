use crate::store;
use crate::waste::parse_ical;
use anyhow::Result;
use chrono::{Datelike, Duration, Local, Timelike};
use futures::stream::StreamExt;
use log::{error, info};
use sqlx::SqlitePool;
use std::sync::Arc;
use teloxide::prelude::*;
use tokio_cron_scheduler::{Job, JobScheduler};

// Constants
// const ICAL_UPDATE_INTERVAL_DAYS: i64 = 28; // Every 4 weeks

pub async fn run_scheduler(bot: Bot, pool: SqlitePool) {
    let pool = Arc::new(pool);
    // Handle error instead of unwrap
    let sched = match JobScheduler::new().await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to create JobScheduler: {:?}", e);
            return;
        }
    };

    // Spawn Notification Task
    // Schedule: Every hour at minute 0: "0 0 * * * *"
    // This cron expression might depend on the crate's parser.
    // tokio-cron-scheduler uses `cron` crate.
    // sec, min, hour, day of month, month, day of week, year (optional)
    let bot_clone = bot.clone();
    let pool_clone = pool.clone();

    // Notifications run every hour
    let notification_job = Job::new_async("0 0 * * * *", move |_uuid, _l| {
        let bot = bot_clone.clone();
        let pool = pool_clone.clone();
        Box::pin(async move {
            let now = Local::now();
            let hour = now.hour();
            let time_str = format!("{:02}:00", hour);
            if let Err(e) = dispatch_notifications(&bot, &pool, &time_str).await {
                error!("Error dispatching {} notifications: {:?}", time_str, e);
            }
        })
    }).expect("Failed to create notification job");

    sched.add(notification_job).await.expect("Failed to add notification job");

    // Spawn iCal Update Task
    // Run once a month on the first Saturday at 4 AM.
    // Cron: "0 0 4 * * Sat" (Every Saturday at 4 AM)
    // Check inside: if day of month <= 7.
    let pool_clone_ical = pool.clone();
    let ical_job = Job::new_async("0 0 4 * * Sat", move |_uuid, _l| {
        let pool = pool_clone_ical.clone();
        Box::pin(async move {
            let now = Local::now();
            if now.day() > 7 {
                return;
            }
            if let Err(e) = update_all_icals(&pool).await {
                error!("Error updating iCals: {:?}", e);
            }
        })
    }).expect("Failed to create iCal job");

    sched.add(ical_job).await.expect("Failed to add iCal job");

    // Run iCal update immediately on startup (asynchronously)
    let pool_clone_startup = pool.clone();
    tokio::spawn(async move {
         if let Err(e) = update_all_icals(&pool_clone_startup).await {
            error!("Error performing startup iCal update: {:?}", e);
        }
    });

    if let Err(e) = sched.start().await {
        error!("Error starting scheduler: {:?}", e);
    }

    // Keep the scheduler running. The main loop in main.rs keeps the process alive,
    // but run_scheduler was previously spawned and expected to run forever.
    // Since sched.start() runs in background, we need to wait here or let the task finish
    // but the scheduler lives in `sched`.
    // Actually, `sched` will be dropped if we exit this function unless we keep it alive.
    // But `JobScheduler` spawns tasks.
    // However, the `sched` struct itself might need to be held?
    // Looking at docs: "The scheduler must be kept alive".

    // So we will just park here.
    if let Err(e) = tokio::signal::ctrl_c().await {
        error!("Error waiting for ctrl_c: {:?}", e);
    }
    info!("Scheduler stopping...");
}

async fn dispatch_notifications(bot: &Bot, pool: &SqlitePool, time: &str) -> Result<()> {
    info!("Dispatching notifications for time: {}", time);
    let today = Local::now().date_naive();
    let tomorrow = today + Duration::days(1);

    let today_str = today.format("%Y-%m-%d").to_string();
    let tomorrow_str = tomorrow.format("%Y-%m-%d").to_string();

    let tasks = store::get_users_to_notify(pool, time, &today_str, &tomorrow_str).await?;

    // Optimization: Send notifications in parallel with a concurrency limit.
    // This prevents one slow request from blocking others and speeds up the overall process.
    // Telegram broadcasting limit is ~30 messages/second.
    // A concurrency of 15 is a safe heuristic: even with fast network (200ms RTT),
    // 15 req / 0.2s = 75 req/s (burst). But sustained average with processing overhead should be safer.
    // To be strictly safe without a complex rate limiter, we keep this conservative.
    futures::stream::iter(tasks)
        .for_each_concurrent(15, |task| async move {
            let chat_id = ChatId(task.chat_id);

            // Determine prefix based on notify_offset
            // offset 1 = Day Before ("Tomorrow")
            // offset 0 = Same Day ("Today")
            let prefix = if task.notify_offset == 1 {
                "Tomorrow"
            } else {
                "Today"
            };

            let loc_label = task
                .location_alias
                .as_deref()
                .unwrap_or(&task.location_id);

            let message = format!(
                "📅 {} at {}: {} collection.",
                prefix, loc_label, task.waste_type
            );

            if let Err(e) = bot.send_message(chat_id, message).await {
                error!("Failed to send notification to {}: {:?}", task.chat_id, e);
                // Handle block/deactivated
                if let teloxide::RequestError::Api(
                    teloxide::ApiError::BotBlocked | teloxide::ApiError::UserDeactivated,
                ) = &e
                {
                    info!(
                        "User {} blocked bot or is deactivated. Removing...",
                        task.chat_id
                    );
                    // We should delete all user data? Or just the specific subscription?
                    // Probably delete user entirely if they blocked the bot.
                    let _ = store::delete_user(pool, task.chat_id).await;
                }
            }
        })
        .await;

    Ok(())
}

async fn update_all_icals(pool: &SqlitePool) -> Result<()> {
    info!("Starting iCal update...");

    // Get all unique location_ids from user_locations
    // We need to join with user_locations now because location_id is there
    let locations: Vec<String> =
        sqlx::query_scalar!("SELECT DISTINCT location_id FROM user_locations")
            .fetch_all(pool)
            .await?;

    // Sentinel: Added timeout to prevent hanging if the external API is unresponsive.
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let now = Local::now().date_naive();
    // Start date: today
    // End date: today + 3 months
    let start_date = now.format("%d.%m.%Y").to_string(); // Check API format!
    let end_date = (now + Duration::days(90)).format("%d.%m.%Y").to_string();

    for loc_id in locations {
        info!("Updating iCal for location: {}", loc_id);

        let params = [
            ("STANDORT", loc_id.as_str()),
            ("DATUM_VON", start_date.as_str()),
            ("DATUM_BIS", end_date.as_str()),
        ];

        let url =
            "https://stadtplan.dresden.de/project/cardo3Apps/IDU_DDStadtplan/abfall/ical.ashx";

        match client.get(url).query(&params).send().await {
            Ok(resp) => {
                if resp.status().is_success() {
                    match resp.text().await {
                        Ok(text) => {
                            // Validate content type or content
                            if !text.contains("BEGIN:VCALENDAR") {
                                error!("Invalid iCal response for location {}", loc_id);
                                continue;
                            }

                            match parse_ical(&text) {
                                Ok(events) => {
                                    if let Err(e) =
                                        store::upsert_events(pool, &loc_id, &events).await
                                    {
                                        error!("Failed to upsert events for {}: {:?}", loc_id, e);
                                    }
                                }
                                Err(e) => {
                                    error!("Failed to parse iCal for {}: {:?}", loc_id, e);
                                }
                            }
                        }
                        Err(e) => error!("Failed to read response body for {}: {:?}", loc_id, e),
                    }
                } else {
                    error!(
                        "Failed to fetch iCal for {}: Status {}",
                        loc_id,
                        resp.status()
                    );
                }
            }
            Err(e) => error!("Failed to connect for {}: {:?}", loc_id, e),
        }

        // Sleep a bit to be nice to the API
        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
    }

    info!("iCal update finished.");
    Ok(())
}
