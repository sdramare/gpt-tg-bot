use reqwest;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::error::Error;
use std::sync::Arc;

#[derive(Debug, Serialize, Deserialize)]
pub struct Update {
    pub(crate) update_id: i64,
    pub(crate) message: Option<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Message {
    pub(crate) message_id: i32,
    pub(crate) from: User,
    pub(crate) chat: Chat,
    pub(crate) date: i64,
    pub(crate) text: Option<String>,
    pub(crate) reply_to_message: Option<Box<Message>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct User {
    pub(crate) id: i64,
    pub(crate) is_bot: bool,
    pub(crate) first_name: String,
    pub(crate) last_name: Option<String>,
    // Because some users might not have a last name
    pub(crate) username: Option<String>,
    // Username is also not always present
    pub(crate) language_code: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Chat {
    pub(crate) id: i64,
    pub(crate) first_name: Option<String>,
    pub(crate) last_name: Option<String>,
    // Because some chats might not have a last name
    pub(crate) username: Option<String>,
    // Username is also not always present
    #[serde(rename = "type")]
    pub(crate) chat_type: String,
}

#[derive(Debug)]
pub struct TgClient {
    http_client: reqwest::Client,
    url: String,
    escape_symbols: HashSet<char>,
}

#[derive(Debug, Default, Serialize)]
struct TgMessageRequest {
    chat_id: i64,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    parse_mode: Option<&'static str>,
}

impl TgMessageRequest {
    pub fn new(
        chat_id: i64,
        text: String,
        parse_mode: Option<&'static str>,
    ) -> Self {
        TgMessageRequest {
            chat_id,
            text,
            parse_mode,
        }
    }
}

impl TgClient {
    pub fn new(token: String) -> Self {
        let escape_symbols = HashSet::from([
            '_', '*', '[', ']', '(', ')', '~', '>', '#', '+', '-', '=', '|',
            '{', '}', '.', '!',
        ]);
        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
        let http_client = reqwest::Client::new();

        TgClient {
            http_client,
            url,
            escape_symbols,
        }
    }

    pub async fn send_message_async(
        &self,
        chat_id: i64,
        text: Arc<String>,
        parse_mode: Option<&'static str>,
    ) -> Result<(), Box<dyn Error + Send + Sync>> {
        let mut result_text = String::with_capacity(text.capacity());

        for ch in text.chars() {
            if self.escape_symbols.contains(&ch) {
                result_text.push('\\');
            }

            result_text.push(ch);
        }

        let request_data =
            TgMessageRequest::new(chat_id, result_text, parse_mode);

        let response = self
            .http_client
            .post(&self.url)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            eprintln!("{:?}", response.text().await?);
        }

        Ok(())
    }
}
