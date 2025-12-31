use teloxide::{
    dispatching::dialogue::InMemStorage,
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup},
    utils::command::BotCommands,
};
use sqlx::SqlitePool;
use std::sync::Arc;
use crate::store;

type MyDialogue = Dialogue<State, InMemStorage<State>>;
type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

#[derive(Clone, Default)]
pub enum State {
    #[default]
    Start,
    AwaitingLocation,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Supported commands:")]
pub enum Command {
    #[command(description = "Start the bot and setup location.")]
    Start,
    #[command(description = "Setup your location ID.")]
    Setup,
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
                .endpoint(command_handler)
        )
        .branch(
            dptree::case![State::AwaitingLocation]
                .endpoint(receive_location_handler)
        )
        .branch(
            dptree::case![State::Start]
                .endpoint(invalid_state_handler)
        );

    let callback_handler = Update::filter_callback_query()
        .endpoint(callback_query_handler);

    Dispatcher::builder(bot, dptree::entry().branch(handler).branch(callback_handler))
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
        Command::Start | Command::Setup => {
            bot.send_message(msg.chat.id, "Please enter your Location ID (Standort-ID). You can find it on the Dresden waste management website.")
                .await?;
            dialogue.update(State::AwaitingLocation).await?;
        }
        Command::Settings => {
             settings_handler(bot, &msg.chat.id, &pool).await?;
        }
        Command::Stop => {
            store::delete_user(&pool, msg.chat.id.0).await?;
            bot.send_message(msg.chat.id, "You have been unsubscribed and your data deleted.").await?;
        }
    }
    Ok(())
}

async fn receive_location_handler(
    bot: Bot,
    dialogue: MyDialogue,
    msg: Message,
    pool: Arc<SqlitePool>,
) -> HandlerResult {
    if let Some(text) = msg.text() {
        let location_id = text.trim();
        if location_id.is_empty() {
             bot.send_message(msg.chat.id, "Please enter a valid Location ID.").await?;
             return Ok(());
        }

        // Save user
        store::create_user(&pool, msg.chat.id.0, location_id).await?;

        // Add default subscriptions
        store::add_subscription(&pool, msg.chat.id.0, "Bio").await?;
        store::add_subscription(&pool, msg.chat.id.0, "Rest").await?;
        store::add_subscription(&pool, msg.chat.id.0, "Papier").await?;
        store::add_subscription(&pool, msg.chat.id.0, "Gelb").await?;

        bot.send_message(msg.chat.id, format!("Location set to '{}'. Default subscriptions added.", location_id)).await?;

        // Show settings
        settings_handler(bot, &msg.chat.id, &pool).await?;

        dialogue.exit().await?;
    }
    Ok(())
}

async fn invalid_state_handler(
    bot: Bot,
    msg: Message,
) -> HandlerResult {
    bot.send_message(msg.chat.id, "Please use /start or /setup to begin.").await?;
    Ok(())
}

async fn settings_handler(bot: Bot, chat_id: &ChatId, pool: &SqlitePool) -> HandlerResult {
    let user = store::get_user(pool, chat_id.0).await?;
    if user.is_none() {
        bot.send_message(*chat_id, "Please run /setup first.").await?;
        return Ok(());
    }

    let (_, notify_time) = user.unwrap();
    let subs = store::get_subscriptions(pool, chat_id.0).await?;

    // Build keyboard
    let mut keyboard = Vec::new();

    // Toggle buttons for Waste Types
    let all_types = vec!["Bio", "Rest", "Papier", "Gelb", "Weihnachtsbaum"];
    for w_type in all_types {
        let is_subbed = subs.contains(&w_type.to_string());
        let label = format!("{} {}", if is_subbed { "✅" } else { "❌" }, w_type);
        let action = if is_subbed { "unsub" } else { "sub" };
        let data = format!("{}:{}", action, w_type);
        keyboard.push(vec![InlineKeyboardButton::callback(label, data)]);
    }

    // Time toggle
    let time_label = format!("Notify Time: {}", notify_time);
    let next_time = if notify_time == "06:00" { "18:00" } else { "06:00" };
    let time_data = format!("time:{}", next_time);
    keyboard.push(vec![InlineKeyboardButton::callback(time_label, time_data)]);

    // Stop button
    keyboard.push(vec![InlineKeyboardButton::callback("🛑 Unsubscribe All", "stop")]);

    bot.send_message(*chat_id, "Your Settings:")
        .reply_markup(InlineKeyboardMarkup::new(keyboard))
        .await?;

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
        let chat_id = q.message.as_ref().map(|m| m.chat().id).unwrap_or(ChatId(0)); // Should exist

        if chat_id.0 == 0 {
             return Ok(());
        }

        match action {
            "sub" => {
                if parts.len() > 1 {
                    store::add_subscription(&pool, chat_id.0, parts[1]).await?;
                    answer_and_refresh(&bot, &q, chat_id, &pool, "Subscribed!").await?;
                }
            }
            "unsub" => {
                if parts.len() > 1 {
                    store::remove_subscription(&pool, chat_id.0, parts[1]).await?;
                    answer_and_refresh(&bot, &q, chat_id, &pool, "Unsubscribed!").await?;
                }
            }
            "time" => {
                 if parts.len() > 1 {
                    store::update_notify_time(&pool, chat_id.0, parts[1]).await?;
                    answer_and_refresh(&bot, &q, chat_id, &pool, "Time updated!").await?;
                 }
            }
            "stop" => {
                store::delete_user(&pool, chat_id.0).await?;
                bot.answer_callback_query(q.id).text("Unsubscribed from everything.").await?;
                if let Some(msg) = q.message {
                    bot.edit_message_text(chat_id, msg.id(), "You have been unsubscribed and your data deleted.").await?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn answer_and_refresh(bot: &Bot, q: &CallbackQuery, chat_id: ChatId, pool: &SqlitePool, text: &str) -> HandlerResult {
    bot.answer_callback_query(&q.id).text(text).await?;

    // Refresh settings message
    // We need to call settings logic but edit message instead of sending new
    // Refactoring settings_handler to return Markup or Text would be better, but we can just copy logic for now
    // or call settings_handler which sends a NEW message? The prompt said "Present the user with an interactive menu...".
    // Usually inline keyboards update in-place.

    // Let's implement edit_settings_message

    let user = store::get_user(pool, chat_id.0).await?;
    if user.is_none() {
        return Ok(());
    }

    let (_, notify_time) = user.unwrap();
    let subs = store::get_subscriptions(pool, chat_id.0).await?;

    let mut keyboard = Vec::new();
    let all_types = vec!["Bio", "Rest", "Papier", "Gelb", "Weihnachtsbaum"];
    for w_type in all_types {
        let is_subbed = subs.contains(&w_type.to_string());
        let label = format!("{} {}", if is_subbed { "✅" } else { "❌" }, w_type);
        let action = if is_subbed { "unsub" } else { "sub" };
        let data = format!("{}:{}", action, w_type);
        keyboard.push(vec![InlineKeyboardButton::callback(label, data)]);
    }

    let time_label = format!("Notify Time: {}", notify_time);
    let next_time = if notify_time == "06:00" { "18:00" } else { "06:00" };
    let time_data = format!("time:{}", next_time);
    keyboard.push(vec![InlineKeyboardButton::callback(time_label, time_data)]);
    keyboard.push(vec![InlineKeyboardButton::callback("🛑 Unsubscribe All", "stop")]);

    if let Some(msg) = &q.message {
        bot.edit_message_reply_markup(chat_id, msg.id())
            .reply_markup(InlineKeyboardMarkup::new(keyboard))
            .await?;
    }

    Ok(())
}
