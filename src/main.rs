#![cfg_attr(not(debug_assertions), deny(warnings))]

use std::path::Path;

use anyhow::Result;
use lambda_http::ext::PayloadError;
use lambda_http::Body::Empty;
use lambda_http::{http, run, service_fn, Body, Error, Request, Response};
use tracing::error;

use crate::event_handler::EventHandler;
use crate::gpt_client::GtpClient;
use crate::message_processor::{Config, RequestError, TgBot};
use crate::tg_client::{Message, TgClient};

mod event_handler;
mod gpt_client;
mod message_processor;
mod tg_client;

async fn function_handler<TEventHandler: EventHandler>(
    event: Request,
    tg_bot: &TEventHandler,
) -> Result<Response<Body>> {
    if let Err(error) = tg_bot.process_event(&event).await {
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
    let dummy_answers =
        std::env::var("DUMMY_ANSWERS")?.leak().split(',').collect();

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
    let gtp_client = GtpClient::new(gpt_model, gpt_token, base_rules);
    let private_gtp_client =
        GtpClient::new(gpt_model, gpt_token, String::default());
    let names_map = std::env::var("NAMES_MAP")?;
    let names_map = serde_json::from_str(&names_map)?;

    let tg_bot = TgBot::new(
        gtp_client,
        private_gtp_client,
        tg_client,
        Config::new(names_map, gtp_preamble, dummy_answers, tg_bot_allow_chats, tg_bot_names),
        rand::thread_rng,
    );

    if cfg!(debug_assertions) {
        let message_path = Path::new(env!("CARGO_MANIFEST_DIR"));

        let message_json =
            std::fs::read_to_string(message_path.join("message.json"))?;

        let message: Message = serde_json::from_str(message_json.as_str())?;

        tg_bot.process_message(message).await?;
    } else {
        run(service_fn(|event| function_handler(event, &tg_bot))).await?;
    }

    Ok(())
}
