#![cfg_attr(not(debug_assertions), deny(warnings))]

use std::backtrace::Backtrace;
use std::path::Path;

use dotenvy::dotenv;
use lambda_http::Body::Empty;
use lambda_http::{Body, Error, Request, Response, http, run, service_fn};
use tracing::error;

use crate::config::AppConfig;
use crate::event_handler::EventHandler;
use crate::tg_client::Message;

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

fn init_tracing() {
    if cfg!(debug_assertions) {
        color_eyre::install().expect("Failed to install color_eyre");
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

#[tokio::main]
async fn main() -> Result<(), Error> {
    if cfg!(debug_assertions) {
        dotenv()?;
    }

    init_tracing();

    let app_config = AppConfig::from_env().await?;
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
