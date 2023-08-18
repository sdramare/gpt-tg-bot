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
) -> Result<Response<Body>, Error> {
    let update: Option<Update> = event.payload()?;

    let status_code = match update {
        None => reqwest::StatusCode::BAD_REQUEST,
        Some(update) => {
            let result = gtp_client.get_completion(update.message.text).await?;

            tg_client
                .send_message_async(
                    update.message.chat.id,
                    result,
                    "MarkdownV2".into(),
                )
                .await?;

            reqwest::StatusCode::OK
        }
    };

    let resp = Response::builder().status(status_code).body(Empty)?;
    Ok(resp)
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

    let tg_token = std::env::var("TOKEN")?;
    let gpt_token = std::env::var("GPT_TOKEN")?;
    let gpt_model = std::env::var("GPT_MODEL")?;

    let tg_client = TgClient::new(tg_token);
    let gtp_client = GtpClient::new(gpt_model, gpt_token);

    run(service_fn(|event| {
        function_handler(event, &gtp_client, &tg_client)
    }))
    .await
}
