use std::time::Duration;

use anyhow::{bail, Result};
use chrono::naive::serde::ts_seconds::deserialize as from_ts;
use chrono::NaiveDateTime;
use derive_more::Constructor;
#[cfg(test)]
use mockall::automock;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware};
use reqwest_retry::policies::ExponentialBackoff;
use reqwest_retry::RetryTransientMiddleware;
use serde::{Deserialize, Serialize};
use tracing::error;

pub const PRIVATE_CHAT: &str = "private";

const MAX_MSG_SIZE: usize = 4096;

static ESCAPE_UNARY_SYMBOLS: phf::Set<char> = phf::phf_set! {
    '_', '[', ']', '(', ')', '~', '>', '#', '+', '-', '=', '|','\\',
    '{', '}', '.', '!',
};

static ESCAPE_PAIR_SYMBOLS: phf::Set<char> = phf::phf_set! {
   '*',
};

#[derive(Debug, Serialize, Deserialize)]
pub struct Update {
    pub update_id: i64,
    pub message: Option<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PhotoSize {
    pub file_id: String,
    pub file_size: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub message_id: i32,
    pub from: User,
    pub chat: Chat,
    #[serde(deserialize_with = "from_ts")]
    pub date: NaiveDateTime,
    pub text: Option<String>,
    pub caption: Option<String>,
    pub photo: Option<Vec<PhotoSize>>,
    pub reply_to_message: Option<Box<Message>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub id: i64,
    pub is_bot: bool,
    pub first_name: String,
    // Because some users might not have a last name
    pub last_name: Option<String>,
    // Username is also not always present
    pub username: Option<String>,
    pub language_code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Chat {
    pub id: i64,
    pub first_name: Option<String>,
    // Because some chats might not have a last name
    pub last_name: Option<String>,
    // Username is also not always present
    pub username: Option<String>,
    #[serde(rename = "type")]
    pub chat_type: String,
}

impl Chat {
    pub fn is_private(&self) -> bool {
        self.chat_type == PRIVATE_CHAT
    }
}

#[derive(Debug)]
pub struct TgClient {
    http_client: ClientWithMiddleware,
    send_message_url: String,
    send_image_url: String,
    get_file_url: String,
    download_file_url: String,
}

#[derive(Debug, Default, Constructor, Serialize)]
struct TgMessageRequest<'a> {
    chat_id: i64,
    text: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'static str>,
}

#[derive(Debug, Constructor, Serialize)]
struct TgMessageImageRequest<'a> {
    chat_id: i64,
    photo: &'a str,
}

#[derive(Debug, Deserialize)]
struct TgResponse<T> {
    ok: bool,
    result: Option<T>,
}

#[derive(Debug, Deserialize)]
struct FileMetadata {
    file_path: String,
}

impl TgClient {
    pub fn new(token: String) -> Self {
        let url = format!("https://api.telegram.org/bot{token}");
        let retry_policy = ExponentialBackoff::builder()
            .retry_bounds(Duration::from_secs(2), Duration::from_secs(10))
            .build_with_max_retries(3);
        let http_client = ClientBuilder::new(reqwest::Client::new())
            // Retry failed requests.
            .with(RetryTransientMiddleware::new_with_policy(retry_policy))
            .build();

        TgClient {
            http_client,
            send_message_url: format!("{url}/sendMessage"),
            send_image_url: format!("{url}/sendPhoto"),
            get_file_url: format!("{url}/getFile"),
            download_file_url: format!(
                "https://api.telegram.org/file/bot{token}"
            ),
        }
    }

    async fn get_file_path(&self, file_id: &str) -> Result<String> {
        let response = self
            .http_client
            .get(&self.get_file_url)
            .query(&[("file_id", file_id)])
            .send()
            .await?;

        if response.status().is_success() {
            let tg_response =
                response.json::<TgResponse<FileMetadata>>().await?;
            if tg_response.ok {
                if let Some(file) = tg_response.result {
                    Ok(file.file_path)
                } else {
                    bail!("Bad file id")
                }
            } else {
                bail!("Bad file id")
            }
        } else {
            bail!(response.text().await?)
        }
    }

    async fn send_text(
        &self,
        chat_id: i64,
        result_text: &str,
        parse_mode: Option<&'static str>,
    ) -> Result<()> {
        let request_data =
            TgMessageRequest::new(chat_id, result_text, parse_mode);

        let response = self
            .http_client
            .post(&self.send_message_url)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            let tg_error = response.text().await?;
            error!(
                "Telegram send error. Error: {}. Request {}",
                tg_error, request_data.text
            );
            let error = format!("Telegram send error. Error: {}", tg_error);
            bail!(error);
        }
        Ok(())
    }

    async fn send_message_by_chunks(
        &self,
        chat_id: i64,
        parse_mode: Option<&'static str>,
        result_text: &str,
    ) -> Result<()> {
        let mut i = 0;
        let mut j = 0;
        let len = result_text.len();

        while i < len {
            j += MAX_MSG_SIZE;
            if j > len {
                j = len;
            };

            let chunk = &result_text[i..j];
            self.send_text(chat_id, chunk, parse_mode).await?;
            i += MAX_MSG_SIZE;
        }
        Ok(())
    }
}

impl TelegramInteractor for TgClient {
    async fn get_file_url(&self, file_id: &str) -> Result<String> {
        let file_path = self.get_file_path(file_id).await?;
        let base_url = self.download_file_url.as_str();
        Ok(format!("{base_url}/{file_path}"))
    }
    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&'static str>,
    ) -> Result<()> {
        let result_text = escape_text(text);

        if result_text.chars().count() < MAX_MSG_SIZE {
            self.send_text(chat_id, &result_text, parse_mode).await?;
            return Ok(());
        }

        self.send_message_by_chunks(chat_id, parse_mode, &result_text)
            .await?;

        Ok(())
    }

    async fn send_image(&self, chat_id: i64, url: &str) -> Result<()> {
        let request_data = TgMessageImageRequest::new(chat_id, url);

        let response = self
            .http_client
            .post(&self.send_image_url)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = format!(
                "Telegram send error. Error: {}. Request {}",
                response.text().await?,
                request_data.photo
            );
            bail!(error);
        }

        Ok(())
    }
}

fn escape_text(text: &str) -> String {
    let mut result_text = String::with_capacity(text.len());

    let mut peekable = text.chars().peekable();
    let mut prev = '\0';

    while let Some(ch) = peekable.next() {
        if ESCAPE_UNARY_SYMBOLS.contains(&ch)
            || (ESCAPE_PAIR_SYMBOLS.contains(&ch)
                && (prev != ch
                    && peekable.peek().is_some_and(|n_ch| *n_ch != ch)))
        {
            result_text.push('\\');
        }

        result_text.push(ch);
        prev = ch
    }
    result_text
}

#[cfg_attr(test, automock)]
pub trait TelegramInteractor: Send + Sync {
    async fn get_file_url(&self, file_id: &str) -> Result<String>;
    async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&'static str>,
    ) -> Result<()>;
    async fn send_image(&self, chat_id: i64, url: &str) -> Result<()>;
}
