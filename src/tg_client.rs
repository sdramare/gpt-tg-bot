use anyhow::{bail, Result};
use chrono::naive::serde::ts_seconds::deserialize as from_ts;
use chrono::NaiveDateTime;
use derive_more::Constructor;
use serde::{Deserialize, Serialize};

pub const PRIVATE_CHAT: &str = "private";

static ESCAPE_SYMBOLS: phf::Set<char> = phf::phf_set! {
    '_', '*', '[', ']', '(', ')', '~', '>', '#', '+', '-', '=', '|','\\',
    '{', '}', '.', '!',
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
    http_client: reqwest::Client,
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
        let http_client = reqwest::Client::new();

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

    pub async fn get_file_url(&self, file_id: &str) -> Result<String> {
        let file_path = self.get_file_path(file_id).await?;
        let base_url = self.download_file_url.as_str();
        Ok(format!("{base_url}/{file_path}"))
    }

    pub async fn send_message(
        &self,
        chat_id: i64,
        text: &str,
        parse_mode: Option<&'static str>,
    ) -> Result<()> {
        let mut result_text = String::with_capacity(text.len());

        for ch in text.chars() {
            if ESCAPE_SYMBOLS.contains(&ch) {
                result_text.push('\\');
            }

            result_text.push(ch);
        }

        let request_data =
            TgMessageRequest::new(chat_id, &result_text, parse_mode);

        let response = self
            .http_client
            .post(&self.send_message_url)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            let error = format!(
                "Telegram send error. Error: {}. Request {}",
                response.text().await?,
                request_data.text
            );
            bail!(error);
        }

        Ok(())
    }

    pub async fn send_image(&self, chat_id: i64, url: &str) -> Result<()> {
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
