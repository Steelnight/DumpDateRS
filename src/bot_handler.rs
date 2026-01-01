use crate::store;
use crate::waste::WasteType;
use sqlx::SqlitePool;
use std::sync::Arc;
use teloxide::{
    dispatching::dialogue::InMemStorage,
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup},
    utils::command::BotCommands,
};

type MyDialogue = Dialogue<State, InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    AwaitingLocationId,
    AwaitingLocationAlias(String), // Stores location_id while waiting for alias
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
pub enum Command {
    #[command(description = "Start the bot and setup location.")]
    Start,
    #[command(description = "Add a new location.")]
    AddLocation,
    #[command(description = "List your locations.")]
    Locations,
    #[command(description = "Manage your subscriptions.")]
    Settings,
    #[command(description = "Unsubscribe from all notifications and delete data.")]
    Stop,
}

pub async fn run_bot(bot: Bot, pool: SqlitePool) {
    let pool = Arc::new(pool);

    let handler = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .branch(
            dptree::entry()
                .filter_command::<Command>()
                .endpoint(command_handler),
        )
        .branch(dptree::case![State::AwaitingLocationId].endpoint(receive_location_id_handler))
        .branch(
            dptree::case![State::AwaitingLocationAlias(location_id)]
                .endpoint(receive_alias_handler),
        )
        .branch(dptree::case![State::Start].endpoint(invalid_state_handler));

    let callback_handler = Update::filter_callback_query().endpoint(callback_query_handler);

    Dispatcher::builder(
        bot,
        dptree::entry().branch(handler).branch(callback_handler),
    )
    .dependencies(dptree::deps![InMemStorage::<State>::new(), pool])
    .enable_ctrlc_handler()
    .build()
    .dispatch()
    .await;
}

async fn command_handler(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    cmd: Command,
    pool: Arc<SqlitePool>,
) -> HandlerResult {
    match cmd {
        Command::Start | Command::AddLocation => {
            bot.send_message(msg.chat.id, "Please enter your Location ID (Standort-ID). You can find it on the Dresden waste management website.")
                .await?;
            dialogue.update(State::AwaitingLocationId).await?;
        }
        Command::Locations => {
            list_locations_handler(bot, &msg.chat.id, &pool).await?;
        }
        Command::Settings => {
            list_locations_handler(bot, &msg.chat.id, &pool).await?;
        }
        Command::Stop => {
            store::delete_user(&pool, msg.chat.id.0).await?;
            bot.send_message(
                msg.chat.id,
                "You have been unsubscribed and your data deleted.",
            )
            .await?;
        }
    }
    Ok(())
}

async fn receive_location_id_handler(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
) -> HandlerResult {
    if let Some(text) = msg.text() {
        let location_id = text.trim().to_string();
        if !crate::waste::is_valid_location_id(&location_id) {
            bot.send_message(
                msg.chat.id,
                "Invalid Location ID. It must be alphanumeric and max 20 characters.",
            )
            .await?;
            return Ok(());
        }

        bot.send_message(
            msg.chat.id,
            "Please give this location a short alias (e.g., 'Home', 'Office').",
        )
        .await?;

        dialogue
            .update(State::AwaitingLocationAlias(location_id))
            .await?;
    }
    Ok(())
}

async fn receive_alias_handler(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    pool: Arc<SqlitePool>,
    location_id: String,
) -> HandlerResult {
    if let Some(alias) = msg.text() {
        let alias = alias.trim();

        match store::add_user_location(&pool, msg.chat.id.0, &location_id, Some(alias)).await {
            Ok(user_loc_id) => {
                for waste in WasteType::default_subscriptions() {
                    store::add_subscription(&pool, user_loc_id, waste.as_str()).await?;
                }

                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Location '{}' ({}) added with default subscriptions.",
                        alias, location_id
                    ),
                )
                .await?;

                list_locations_handler(bot, &msg.chat.id, &pool).await?;
                dialogue.exit().await?;
            }
            Err(e) => {
                bot.send_message(msg.chat.id, format!("Error adding location: {}", e))
                    .await?;
                dialogue.exit().await?;
            }
        }
    }
    Ok(())
}

async fn invalid_state_handler(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(msg.chat.id, "Please use /start or /addlocation to begin.")
        .await?;
    Ok(())
}

async fn list_locations_handler(bot: Bot, chat_id: &ChatId, pool: &SqlitePool) -> HandlerResult {
    let locations = store::get_user_locations(pool, chat_id.0).await?;
    if locations.is_empty() {
        bot.send_message(*chat_id, "You have no locations set up. Use /addlocation.")
            .await?;
        return Ok(());
    }

    bot.send_message(*chat_id, "Your Locations:")
        .reply_markup(build_locations_keyboard(&locations))
        .await?;

    Ok(())
}

async fn show_location_settings(
    bot: &Bot,
    chat_id: ChatId,
    message_id: Option<teloxide::types::MessageId>,
    pool: &SqlitePool,
    loc_id: i64,
) -> HandlerResult {
    let locations = store::get_user_locations(pool, chat_id.0).await?;
    let loc = locations.iter().find(|l| l.id == loc_id);

    if let Some(loc) = loc {
        let subs = store::get_subscriptions(pool, loc_id).await?;
        let keyboard = build_settings_keyboard(loc_id, &subs, &loc.notify_time);

        let text = format!(
            "Settings for {}:",
            loc.alias.as_deref().unwrap_or(&loc.location_id)
        );

        if let Some(mid) = message_id {
            bot.edit_message_text(chat_id, mid, text)
                .reply_markup(keyboard)
                .await?;
        } else {
            bot.send_message(chat_id, text)
                .reply_markup(keyboard)
                .await?;
        }
    } else {
        if let Some(mid) = message_id {
            bot.edit_message_text(chat_id, mid, "Location not found.")
                .await?;
        }
    }
    Ok(())
}

async fn callback_query_handler(
    bot: Bot,
    q: CallbackQuery,
    pool: Arc<SqlitePool>,
) -> HandlerResult {
    if let Some(data) = q.data.clone() {
        let parts: Vec<&str> = data.split(':').collect();
        let action = parts[0];
        let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0));

        if chat_id.0 == 0 {
            return Ok(());
        }

        match action {
            "edit" => {
                if let Ok(loc_id) = parts[1].parse::<i64>() {
                    show_location_settings(
                        &bot,
                        chat_id,
                        q.message.as_ref().map(|m| m.id()),
                        &pool,
                        loc_id,
                    )
                    .await?;
                    bot.answer_callback_query(q.id).await?;
                }
            }
            "back" => {
                let locations = store::get_user_locations(&pool, chat_id.0).await?;
                if let Some(message) = q.message {
                    bot.edit_message_text(chat_id, message.id(), "Your Locations:")
                        .reply_markup(build_locations_keyboard(&locations))
                        .await?;
                }
                bot.answer_callback_query(q.id).await?;
            }
            "sub" => {
                if parts.len() > 2 {
                    let loc_id = parts[1].parse::<i64>()?;
                    store::add_subscription(&pool, loc_id, parts[2]).await?;
                    refresh_settings(&bot, &q, chat_id, &pool, loc_id, "Subscribed!").await?;
                }
            }
            "unsub" => {
                if parts.len() > 2 {
                    let loc_id = parts[1].parse::<i64>()?;
                    store::remove_subscription(&pool, loc_id, parts[2]).await?;
                    refresh_settings(&bot, &q, chat_id, &pool, loc_id, "Unsubscribed!").await?;
                }
            }
            "time" => {
                if parts.len() > 2 {
                    let loc_id = parts[1].parse::<i64>()?;
                    let current_time = parts[2];
                    let next_time = increment_time(current_time);

                    let locations = store::get_user_locations(&pool, chat_id.0).await?;
                    if let Some(loc) = locations.iter().find(|l| l.id == loc_id) {
                        store::update_notify_time(&pool, chat_id.0, &loc.location_id, &next_time)
                            .await?;
                        refresh_settings(&bot, &q, chat_id, &pool, loc_id, "Time updated!").await?;
                    }
                }
            }
            "delloc" => {
                if let Ok(loc_id) = parts[1].parse::<i64>() {
                    let locations = store::get_user_locations(&pool, chat_id.0).await?;
                    if let Some(loc) = locations.iter().find(|l| l.id == loc_id) {
                        store::delete_user_location(&pool, chat_id.0, &loc.location_id).await?;

                        let locations = store::get_user_locations(&pool, chat_id.0).await?;
                        if let Some(message) = q.message {
                            if locations.is_empty() {
                                bot.edit_message_text(
                                    chat_id,
                                    message.id(),
                                    "No locations left.",
                                )
                                .reply_markup(InlineKeyboardMarkup::default())
                                .await?;
                            } else {
                                bot.edit_message_text(
                                    chat_id,
                                    message.id(),
                                    "Your Locations:",
                                )
                                .reply_markup(build_locations_keyboard(&locations))
                                .await?;
                            }
                        }
                        bot.answer_callback_query(q.id)
                            .text("Location deleted.")
                            .await?;
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

fn increment_time(time: &str) -> String {
    let parts: Vec<&str> = time.split(':').collect();
    if parts.len() != 2 {
        return "18:00".to_string();
    }
    let mut hour: u8 = parts[0].parse().unwrap_or(18);
    hour += 1;
    if hour >= 24 {
        hour = 0;
    }
    format!("{:02}:00", hour)
}

async fn refresh_settings(
    bot: &Bot,
    q: &CallbackQuery,
    chat_id: ChatId,
    pool: &SqlitePool,
    loc_id: i64,
    text: &str,
) -> HandlerResult {
    bot.answer_callback_query(&q.id).text(text).await?;

    let locations = store::get_user_locations(pool, chat_id.0).await?;
    if let Some(loc) = locations.iter().find(|l| l.id == loc_id) {
        let subs = store::get_subscriptions(pool, loc_id).await?;
        let keyboard = build_settings_keyboard(loc_id, &subs, &loc.notify_time);

        if let Some(msg) = &q.message {
            bot.edit_message_reply_markup(chat_id, msg.id())
                .reply_markup(keyboard)
                .await?;
        }
    }
    Ok(())
}

fn build_locations_keyboard(locations: &[store::UserLocation]) -> InlineKeyboardMarkup {
    let mut keyboard = Vec::new();
    for loc in locations {
        let label = loc.alias.as_deref().unwrap_or(&loc.location_id);
        keyboard.push(vec![InlineKeyboardButton::callback(
            label.to_string(),
            format!("edit:{}", loc.id),
        )]);
    }
    InlineKeyboardMarkup::new(keyboard)
}

fn build_settings_keyboard(
    loc_id: i64,
    subs: &[String],
    notify_time: &str,
) -> InlineKeyboardMarkup {
    let mut keyboard = Vec::new();

    // Toggle buttons for Waste Types
    for w_type in WasteType::supported_types() {
        let w_str = w_type.as_str();
        let is_subbed = subs.contains(&w_str.to_string());
        let label = format!("{} {}", if is_subbed { "✅" } else { "❌" }, w_str);
        let action = if is_subbed { "unsub" } else { "sub" };
        let data = format!("{}:{}:{}", action, loc_id, w_str);
        keyboard.push(vec![InlineKeyboardButton::callback(label, data)]);
    }

    // Time toggle
    let time_label = format!("Notify Time: {}", notify_time);
    let time_data = format!("time:{}:{}", loc_id, notify_time);
    keyboard.push(vec![InlineKeyboardButton::callback(time_label, time_data)]);

    // Delete Location
    keyboard.push(vec![InlineKeyboardButton::callback(
        "🗑️ Delete Location",
        format!("delloc:{}", loc_id),
    )]);

    // Back button
    keyboard.push(vec![InlineKeyboardButton::callback(
        "🔙 Back to Locations",
        "back",
    )]);

    InlineKeyboardMarkup::new(keyboard)
}
