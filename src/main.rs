#![cfg_attr(not(debug_assertions), deny(warnings))]

use std::backtrace::Backtrace;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use dotenvy::dotenv;
use lambda_http::Body::Empty;
use lambda_http::{Body, Error, Request, Response, http, run, service_fn};
use tracing::error;

use crate::event_handler::EventHandler;
use crate::gpt_client::GtpClient;
use crate::message_processor::{Config, TgBot};
use crate::tg_client::{Message, TgClient};

mod event_handler;
mod gpt_client;
mod message_processor;
mod tg_client;

async fn function_handler(
    event: Request,
    tg_bot: &impl EventHandler,
) -> Result<Response<Body>, Box<dyn std::error::Error>> {
    if let Err(error) = tg_bot.process_event(&event).await {
        let body = get_request_body(event.body());
        let backtrace = Backtrace::force_capture();
        error!({ ?body, ?backtrace, error = format!("{:?}", error) }, "Error in request handler");
    };

    let resp = Response::builder()
        .status(http::StatusCode::OK)
        .body(Empty)?;

    Ok(resp)
}

#[inline]
fn get_request_body(body: &Body) -> &str {
    match body {
        Body::Text(text) => text,
        _ => Default::default(),
    }
}

macro_rules! context_env {
    ($name: literal) => {
        std::env::var($name).context($name)?
    };
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if cfg!(debug_assertions) {
        dotenv()?;
    }

    if cfg!(debug_assertions) {
        color_eyre::install()?;
        tracing_subscriber::fmt().pretty().init();
    } else {
        tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::INFO)
            // disable printing the name of the module in every log line.
            .with_target(false)
            // disabling time is handy because CloudWatch will add the ingestion time.
            .without_time()
            .init();
    }

    let tg_bot_names = context_env!("BOT_ALIAS").leak().split(',').collect();
    let dummy_answers =
        context_env!("DUMMY_ANSWERS").leak().split(',').collect();

    let tg_token = context_env!("TG_TOKEN");
    let gpt_token = context_env!("GPT_TOKEN").leak();
    let gpt_model = context_env!("GPT_MODEL").leak();
    let gpt_smart_model = context_env!("GPT_SMART_MODEL").leak();
    let base_rules = context_env!("GPT_RULES");
    let private_base_rules =
        std::env::var("PRIVATE_GPT_RULES").unwrap_or_default();

    let gtp_preamble = context_env!("GPT_PREAMBLE");

    let heartbeat_interval_seconds =
        std::env::var("HEARTBEAT_INTERVAL_SECONDS");

    let voice = std::env::var("VOICE").unwrap_or("onyx".to_string()).leak();

    let mut tg_bot_allow_chats = Vec::new();

    for chat_id in context_env!("TG_ALLOW_CHATS").split(',') {
        tg_bot_allow_chats.push(chat_id.parse::<i64>()?);
    }

    let api_url = std::env::var("GPT_CHAT_URL")
        .map(|s| s.leak() as &'static str)
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions");

    let private_api_url = std::env::var("GPT_PRIVATE_CHAT_URL")
        .map(|s| s.leak() as &'static str)
        .unwrap_or_else(|_| "https://api.openai.com/v1/chat/completions");

    let private_model = std::env::var("GPT_PRIVATE_MODEL")
        .map(|s| s.leak() as &'static str)
        .unwrap_or(gpt_model);

    let private_token = std::env::var("GPT_PRIVATE_TOKEN")
        .map(|s| s.leak() as &'static str)
        .unwrap_or(gpt_token);

    let tg_client = TgClient::new(tg_token);
    let gtp_client = GtpClient::new(
        api_url,
        gpt_model,
        gpt_smart_model,
        voice,
        gpt_token,
        base_rules,
    );
    let private_gtp_client = GtpClient::new(
        private_api_url,
        private_model,
        gpt_smart_model,
        voice,
        private_token,
        private_base_rules,
    );
    let names_map = context_env!("NAMES_MAP");
    let names_map = serde_json::from_str(&names_map)?;

    let mut config = Config::new(
        names_map,
        gtp_preamble,
        dummy_answers,
        tg_bot_allow_chats,
        tg_bot_names,
    );

    if let Ok(heartbeat_interval_seconds) = heartbeat_interval_seconds {
        config.message_delay =
            Duration::from_secs(heartbeat_interval_seconds.parse()?);
    }

    let tg_bot = TgBot::new(
        gtp_client,
        private_gtp_client,
        tg_client,
        config,
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
