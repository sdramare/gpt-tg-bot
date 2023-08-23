mod gpt_client;
mod tg_client;

use crate::gpt_client::GtpClient;
use crate::tg_client::{TgClient, Update};
use lambda_http::Body::Empty;
use lambda_http::{
    run, service_fn, Body, Error, Request, RequestPayloadExt, Response,
};

async fn function_handler(
    event: Request,
    gtp_client: &GtpClient,
    tg_client: &TgClient,
    tg_bot_names: &Vec<&str>,
) -> Result<Response<Body>, Error> {
    let update: Option<Update> = event.payload().map_err(|error| {
        let body = get_response_body(event.body());
        Error::from(format!("Bad payload. Error {error}. Body {body}"))
    })?;

    let status_code = match update.and_then(|x| x.message) {
        None => {
            let body = get_response_body(event.body());
            eprint!("Bad payload. Body {body}");
            reqwest::StatusCode::OK
        }
        Some(message) => {
            if let Some(text) = message.text {
                let used_name =
                    tg_bot_names.iter().find(|&&name| text.starts_with(name));

                if used_name.is_some()
                    || message
                        .reply_to_message
                        .is_some_and(|reply| reply.from.is_bot)
                {
                    let text = used_name
                        .map(|name| text.replace(name, ""))
                        .unwrap_or(text);

                    let result = gtp_client.get_completion(text).await?;

                    tg_client
                        .send_message_async(
                            message.chat.id,
                            result,
                            "MarkdownV2".into(),
                        )
                        .await?;
                }
            }

            reqwest::StatusCode::OK
        }
    };

    let resp = Response::builder().status(status_code).body(Empty)?;
    Ok(resp)
}

#[inline]
fn get_response_body(body: &Body) -> &str {
    const EMPTY: &str = "";
    match body {
        Empty => EMPTY,
        Body::Text(text) => text,
        Body::Binary(_) => EMPTY,
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        // disable printing the name of the module in every log line.
        .with_target(false)
        // disabling time is handy because CloudWatch will add the ingestion time.
        .without_time()
        .init();

    let tg_bot_names = std::env::var("BOT_ALIAS")?;
    let tg_bot_names = tg_bot_names.split(',').collect();

    let tg_token = std::env::var("TOKEN")?;
    let gpt_token = std::env::var("GPT_TOKEN")?;
    let gpt_model = std::env::var("GPT_MODEL")?;

    let tg_client = TgClient::new(tg_token);
    let gtp_client = GtpClient::new(gpt_model, gpt_token);

    run(service_fn(|event| {
        function_handler(event, &gtp_client, &tg_client, &tg_bot_names)
    }))
    .await
}
