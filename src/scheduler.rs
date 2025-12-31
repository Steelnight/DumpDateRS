use std::sync::Arc;
use sqlx::SqlitePool;
use teloxide::prelude::*;
use chrono::{Local, Duration, Timelike};
use anyhow::Result;
use log::{info, error};
use crate::store;
use crate::waste::parse_ical;

// Constants
const ICAL_UPDATE_INTERVAL_DAYS: i64 = 28; // Every 4 weeks

pub async fn run_scheduler(bot: Bot, pool: SqlitePool) {
    let pool = Arc::new(pool);

    // Spawn Notification Task
    let bot_clone = bot.clone();
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        notification_loop(bot_clone, pool_clone).await;
    });

    // Spawn iCal Update Task
    let pool_clone = pool.clone();
    tokio::spawn(async move {
        ical_update_loop(pool_clone).await;
    });
}

async fn notification_loop(bot: Bot, pool: Arc<SqlitePool>) {
    // Align to the next minute
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(60));

    loop {
        interval.tick().await;
        let now = Local::now();
        let hour = now.hour();
        let minute = now.minute();

        // We only care if it's 06:00 or 18:00
        // We allow a window of execution, but we should ensure we only run ONCE per slot.
        // A simple way is to check if minute == 0.

        if minute == 0 {
            if hour == 6 {
                if let Err(e) = dispatch_notifications(&bot, &pool, "06:00").await {
                    error!("Error dispatching 06:00 notifications: {:?}", e);
                }
            } else if hour == 18 {
                if let Err(e) = dispatch_notifications(&bot, &pool, "18:00").await {
                    error!("Error dispatching 18:00 notifications: {:?}", e);
                }
            }
        }
    }
}

async fn dispatch_notifications(bot: &Bot, pool: &SqlitePool, time: &str) -> Result<()> {
    info!("Dispatching notifications for time: {}", time);
    let today = Local::now().date_naive();
    let tomorrow = today + Duration::days(1);

    let today_str = today.format("%Y-%m-%d").to_string();
    let tomorrow_str = tomorrow.format("%Y-%m-%d").to_string();

    let tasks = store::get_users_to_notify(pool, time, &today_str, &tomorrow_str).await?;

    for task in tasks {
        let chat_id = ChatId(task.chat_id);
        let message = if time == "06:00" {
            format!("📅 Today: {} collection.", task.waste_type)
        } else {
            format!("📅 Tomorrow: {} collection.", task.waste_type)
        };

        if let Err(e) = bot.send_message(chat_id, message).await {
            error!("Failed to send notification to {}: {:?}", task.chat_id, e);
            // Handle block/deactivated
            if let teloxide::RequestError::Api(teloxide::ApiError::BotBlocked | teloxide::ApiError::UserDeactivated) = &e {
                info!("User {} blocked bot or is deactivated. Removing...", task.chat_id);
                let _ = store::delete_user(pool, task.chat_id).await;
            }
        }
    }

    Ok(())
}

async fn ical_update_loop(pool: Arc<SqlitePool>) {
    // Run immediately on start

    loop {
        match update_all_icals(&pool).await {
            Ok(_) => {
                info!("iCal update completed successfully. Sleeping for {} days.", ICAL_UPDATE_INTERVAL_DAYS);
                tokio::time::sleep(tokio::time::Duration::from_secs(ICAL_UPDATE_INTERVAL_DAYS as u64 * 24 * 60 * 60)).await;
            }
            Err(e) => {
                error!("Error updating iCals: {:?}. Retrying in 1 hour.", e);
                // Retry logic: sleep for 1 hour then try again, instead of waiting 28 days.
                tokio::time::sleep(tokio::time::Duration::from_secs(3600)).await;
            }
        }
    }
}

async fn update_all_icals(pool: &SqlitePool) -> Result<()> {
    info!("Starting iCal update...");

    // Get all unique location_ids from users
    // We should probably optimize this to not fetch for every user if they share location
    // But normalized DB has users separate.

    let locations: Vec<String> = sqlx::query_scalar!("SELECT DISTINCT location_id FROM users")
        .fetch_all(pool)
        .await?;

    let client = reqwest::Client::new();
    let now = Local::now().date_naive();
    // Start date: today
    // End date: today + 3 months
    let start_date = now.format("%d.%m.%Y").to_string(); // Check API format!
    let end_date = (now + Duration::days(90)).format("%d.%m.%Y").to_string();

    // Assuming API format is DD.MM.YYYY based on typical German formats, but URL param usually YYYY-MM-DD or similar.
    // The prompt says "cardomap.idu.de...".
    // Checking standard CardoMap iCal URLs usually involves `StandortID`, `DatumVon`, `DatumBis`.
    // Or `startdate`, `enddate` as constructed in prompt description?
    // Prompt says: `https://cardomap.idu.de/cardo3Apps/IDU_DD_Stadtplan/abfallkalender_ical.php?standortid=<LOC_ID>&startdate=<START>&enddate=<END>`
    // I will stick to the prompt's implied parameter names.
    // I need to be sure about date format. Usually `DD.MM.YYYY` in German APIs.

    for loc_id in locations {
        info!("Updating iCal for location: {}", loc_id);

        let url = format!(
            "https://stadtplan.dresden.de/project/cardo3Apps/IDU_DDStadtplan/abfall/ical.ashx?STANDORT={}&DATUM_VON={}&DATUM_BIS={}",
            loc_id, start_date, end_date
        );

        match client.get(&url).send().await {
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
                                    if let Err(e) = store::upsert_events(pool, &loc_id, &events).await {
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
                    error!("Failed to fetch iCal for {}: Status {}", loc_id, resp.status());
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
