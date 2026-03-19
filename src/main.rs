#![cfg_attr(not(debug_assertions), deny(warnings))]

use std::backtrace::Backtrace;
use std::io::{self, BufRead};
use std::path::Path;

use dotenvy::dotenv;
use lambda_http::Body::Empty;
use lambda_http::{Body, Error, Request, Response, http, run, service_fn};
use tracing::error;

use crate::config::AppConfig;
use crate::event_handler::EventHandler;
use crate::gpt_client::GtpClient;
use crate::message_processor::TgBot;
use crate::tg_client::{Chat, ConsoleClient, Message, User};

mod config;
mod event_handler;
mod gpt_client;
mod message_processor;
mod s3_client;
mod tg_client;

async fn function_handler(
    event: Request,
    tg_bot: &impl EventHandler,
) -> Result<Response<Body>, Box<dyn std::error::Error>> {
    if let Err(error) = tg_bot.process_event(&event).await {
        let body = get_request_body(event.body());
        let backtrace = Backtrace::force_capture();
        error!({ ?body, ?backtrace, error = format!("{:?}", error) }, "error in request handler");
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

fn init_tracing() {
    if cfg!(debug_assertions) {
        color_eyre::install().expect("failed to install color_eyre");
        tracing_subscriber::fmt().pretty().init();
    } else {
        tracing_subscriber::fmt()
            .json()
            .with_max_level(tracing::Level::INFO)
            // disable printing the name of the module in
            // every log line.
            .with_target(false)
            // disabling time is handy because CloudWatch
            // will add the ingestion time.
            .without_time()
            .init();
    }
}

fn has_console_flag() -> bool {
    std::env::args().any(|arg| arg == "--console")
}

async fn run_console_mode(
    tg_bot: &TgBot<ConsoleClient, GtpClient, rand::rngs::ThreadRng>,
    chat_id: i64,
) -> Result<(), Error> {
    println!("Console debug mode is enabled.");
    println!(
        "Using group-chat simulation to apply preamble and regular \
         message-routing logic."
    );
    println!("Type a prompt and press Enter. Type 'quit' or 'exit' to stop.");

    let stdin = io::stdin();
    let mut message_id = 1_i32;

    for line in stdin.lock().lines() {
        let input = line?;
        let prompt = input.trim();

        if prompt.is_empty() {
            continue;
        }

        if prompt.eq_ignore_ascii_case("quit")
            || prompt.eq_ignore_ascii_case("exit")
        {
            println!("Stopping console debug mode.");
            break;
        }

        let message = Message {
            message_id,
            from: User {
                id: chat_id,
                is_bot: false,
                first_name: "Console".to_string(),
                last_name: None,
                username: Some("console_user".to_string()),
                language_code: Some("en".to_string()),
            },
            chat: Chat {
                id: chat_id,
                first_name: Some("Console".to_string()),
                last_name: None,
                username: Some("console_user".to_string()),
                chat_type: "private".to_string(),
            },
            date: chrono::Utc::now().naive_utc(),
            text: Some(prompt.to_string()),
            caption: None,
            photo: None,
            reply_to_message: Some(Box::new(Message {
                message_id: 0,
                from: User {
                    id: -1,
                    is_bot: true,
                    first_name: "Bot".to_string(),
                    last_name: None,
                    username: Some("console_bot".to_string()),
                    language_code: None,
                },
                chat: Chat {
                    id: chat_id,
                    first_name: Some("Console".to_string()),
                    last_name: None,
                    username: Some("console_user".to_string()),
                    chat_type: "private".to_string(),
                },
                date: chrono::Utc::now().naive_utc(),
                text: Some("console".to_string()),
                caption: None,
                photo: None,
                reply_to_message: None,
            })),
        };

        if let Err(err) = tg_bot.process_message(message).await {
            eprintln!("[console-error] {err:#}");
        }

        message_id = message_id.saturating_add(1);
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    if cfg!(debug_assertions) {
        dotenv()?;
    }

    init_tracing();

    let console_mode = cfg!(debug_assertions) && has_console_flag();
    let app_config = AppConfig::from_env().await?;

    if console_mode {
        let chat_id = app_config.first_allowed_chat_id()?;
        let tg_bot = app_config.build_console_tg_bot();
        run_console_mode(&tg_bot, chat_id).await?;
        return Ok(());
    }

    let tg_bot = app_config.build_tg_bot();

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
