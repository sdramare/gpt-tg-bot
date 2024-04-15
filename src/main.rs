#![cfg_attr(not(debug_assertions), deny(warnings))]

use std::path::Path;

use anyhow::{anyhow, bail, Result};
use chrono::{Duration, Utc};
use derive_more::Constructor;
use dyn_fmt::AsStrFormatExt;
use lambda_http::ext::PayloadError;
use lambda_http::Body::Empty;
use lambda_http::{
    http, run, service_fn, Body, Error, Request, RequestPayloadExt, Response,
};
use rand::Rng;
use thiserror::Error;
use tracing::{error, info, instrument, span, warn, Instrument, Span};

use crate::gpt_client::GtpClient;
use crate::tg_client::{Chat, Message, TgClient, Update, PRIVATE_CHAT};

mod gpt_client;
mod tg_client;

#[derive(Constructor)]
struct TgBot {
    gtp_client: GtpClient,
    private_gtp_client: GtpClient,
    tg_client: TgClient,
    tg_bot_names: Vec<&'static str>,
    tg_bot_allow_chats: Vec<i64>,
    preamble: String,
}

impl TgBot {
    fn gtp_client(&self, chat: &Chat) -> &GtpClient {
        if chat.is_private() {
            &self.private_gtp_client
        } else {
            &self.gtp_client
        }
    }
}

#[derive(Error, Debug)]
enum RequestError {
    #[error("Bad request body")]
    BadBody(String),
}

async fn function_handler(
    event: Request,
    tg_bot: &TgBot,
) -> Result<Response<Body>> {
    if let Err(error) = process_event(&event, tg_bot).await {
        if let Some(request_error) = error.downcast_ref::<RequestError>() {
            match request_error {
                RequestError::BadBody(body) => {
                    let msg = request_error.to_string();
                    error!({ ?body, ?msg, ?error }, "Error on request")
                }
            }
        } else if let Some(payload_error) = error.downcast_ref::<PayloadError>()
        {
            let msg = payload_error.to_string();
            let body = error
                .downcast_ref::<String>()
                .map_or(Default::default(), |s| s.as_str());
            let backtrace = error.backtrace();

            error!({ ?body, ?msg, ?backtrace }, "Error on payload")
        } else {
            error!(?error, "Error on process")
        }
    };

    let resp = Response::builder()
        .status(http::StatusCode::OK)
        .body(Empty)?;
    Ok(resp)
}

async fn process_event(event: &Request, tg_bot: &TgBot) -> Result<()> {
    let update = get_update(event)?;

    match update.and_then(|x| x.message) {
        None => {
            let body = get_request_body(event.body());
            bail!(RequestError::BadBody(body.to_string()));
        }
        Some(message) => {
            let utc = Utc::now().naive_utc();
            if message.date < (utc - Duration::minutes(10)) {
                warn!(date = ?message.date, "Too old message");
                return Ok(());
            }

            process_message(tg_bot, message).await?;
        }
    };

    Ok(())
}

async fn process_photo(tg_bot: &TgBot, message: Message) -> Result<()> {
    let text = message.caption.unwrap_or("Что на картинке?".to_string());

    let used_name = tg_bot
        .tg_bot_names
        .iter()
        .copied()
        .find(|&name| text.starts_with(name));

    if should_answer(
        message.reply_to_message,
        &message.chat,
        used_name,
        &tg_bot.tg_bot_allow_chats,
    ) {
        let Some(photos) = message.photo else {
            return Ok(());
        };
        let Some(photo) = photos.iter().max_by_key(|x| x.file_size) else {
            return Ok(());
        };
        info!("Photo request");
        let photo_url = tg_bot.tg_client.get_file_url(&photo.file_id).await?;

        let result = tg_bot
            .gtp_client(&message.chat)
            .get_image_completion(text, photo_url)
            .instrument(Span::current())
            .await?;

        info!("Sending answer to TG");

        tg_bot
            .tg_client
            .send_message(message.chat.id, result.as_str(), "MarkdownV2".into())
            .instrument(Span::current())
            .await?;

        info!("Complete");
    }

    Ok(())
}

#[instrument(skip_all)]
async fn process_message(tg_bot: &TgBot, message: Message) -> Result<()> {
    if message.photo.is_some() {
        return process_photo(tg_bot, message).await;
    }

    if let Some(text) = message.text {
        if text.contains("https://") {
            dummy_reaction(&tg_bot.tg_client, message.chat.id).await?;

            return Ok(());
        }

        let used_name = tg_bot
            .tg_bot_names
            .iter()
            .copied()
            .find(|&name| text.starts_with(name));

        if should_answer(
            message.reply_to_message,
            &message.chat,
            used_name,
            &tg_bot.tg_bot_allow_chats,
        ) {
            let mut text =
                used_name.map(|name| text.replace(name, "")).unwrap_or(text);

            let first_name = message
                .from
                .first_name
                .replace("Yury", "Юра")
                .replace("Frol", "Фрол");

            let span =
                span!(tracing::Level::INFO, "response", user_name = first_name);

            let _enter = span.enter();

            const DRAW_COMMAND: &str = "нарисуй";

            if let Some(index) = text.to_lowercase().find(DRAW_COMMAND) {
                let text = &text[index + DRAW_COMMAND.len()..];

                info!("Image request");

                let url =
                    tg_bot.gtp_client(&message.chat).get_image(text).await;

                match url {
                    Ok(url) => {
                        tg_bot
                            .tg_client
                            .send_image(message.chat.id, url.as_str())
                            .await?;
                    }
                    Err(error) => {
                        error!(?error);
                        tg_bot
                            .tg_client
                            .send_message(
                                message.chat.id,
                                "Сейчас я такое не могу нарисовать",
                                None,
                            )
                            .await?;
                    }
                }

                return Ok(());
            }

            let mut prepend = tg_bot.preamble.format(&[first_name]);
            prepend.push_str(&text);
            text = prepend;

            info!("Ask GPT");

            let result = tg_bot
                .gtp_client(&message.chat)
                .get_completion(text)
                .instrument(Span::current())
                .await?;

            info!("Sending answer to TG");

            tg_bot
                .tg_client
                .send_message(
                    message.chat.id,
                    result.as_str(),
                    "MarkdownV2".into(),
                )
                .instrument(Span::current())
                .await?;

            info!("Complete");

            drop(_enter)
        }
    }
    Ok(())
}

async fn dummy_reaction(tg_client: &TgClient, chat_id: i64) -> Result<()> {
    let num = rand::thread_rng().gen_range(0..100);
    if num < 30 {
        let num = rand::thread_rng().gen_range(0..6);
        let answer = match num {
            0 => "боян",
            1 => "прикол",
            2 => "ну такое",
            3 => "было уже",
            _ => "хуйня какая-то",
        };
        tg_client
            .send_message(chat_id, answer, "MarkdownV2".into())
            .await?;
    }
    Ok(())
}

fn should_answer(
    reply_to_message: Option<Box<Message>>,
    chat: &Chat,
    used_name: Option<&str>,
    tg_bot_allow_chats: &[i64],
) -> bool {
    (tg_bot_allow_chats.contains(&chat.id))
        && (chat.chat_type == PRIVATE_CHAT
            || used_name.is_some()
            || reply_to_message.is_some_and(|reply| reply.from.is_bot))
}

fn get_update(event: &Request) -> Result<Option<Update>> {
    let update: Option<Update> = event.payload().map_err(|error| {
        let body = get_request_body(event.body());
        anyhow!(error).context(body.to_string())
    })?;

    Ok(update)
}

#[inline]
fn get_request_body(body: &Body) -> &str {
    match body {
        Body::Text(text) => text,
        _ => Default::default(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if cfg!(debug_assertions) {
        dotenv::dotenv()?;
    }

    tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    let tg_bot_names = std::env::var("BOT_ALIAS")?.leak().split(',').collect();

    let tg_token = std::env::var("TG_TOKEN")?;
    let gpt_token = std::env::var("GPT_TOKEN")?.leak();
    let gpt_model = std::env::var("GPT_MODEL")?.leak();    
    let base_rules = std::env::var("GPT_RULES")?;
    let gtp_preamble = std::env::var("GPT_PREAMBLE")?;
    let mut tg_bot_allow_chats = Vec::new();

    for chat_id in std::env::var("TG_ALLOW_CHATS")?.split(',') {
        tg_bot_allow_chats.push(chat_id.parse::<i64>()?);
    }

    let tg_client = TgClient::new(tg_token);
    let gtp_client =
        GtpClient::new(gpt_model, gpt_token, base_rules);
    let private_gtp_client = GtpClient::new(gpt_model, gpt_token, String::default());

    let tg_bot = TgBot::new(
        gtp_client,
        private_gtp_client,
        tg_client,
        tg_bot_names,
        tg_bot_allow_chats,
        gtp_preamble,
    );

    if cfg!(debug_assertions) {
        let message_path = Path::new(env!("CARGO_MANIFEST_DIR"));

        let message_json =
            std::fs::read_to_string(message_path.join("message.json"))?;

        let message: Message = serde_json::from_str(message_json.as_str())?;

        process_message(&tg_bot, message).await?;
    } else {
        run(service_fn(|event| function_handler(event, &tg_bot))).await?;
    }

    Ok(())
}
