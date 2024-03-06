use derive_more::Constructor;
use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use anyhow::{bail, Result};

#[derive(Debug, Serialize, Constructor)]
struct Request<'a> {
    model: &'a str,
    messages: &'a Vec<Message>,
    temperature: f64,
}

#[derive(Debug, Serialize, Deserialize, Constructor, Clone)]
struct Message {
    role: &'static str,
    content: Arc<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Serialize, Deserialize)]
struct Choice {
    index: i32,
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ResponseMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Usage {
    prompt_tokens: i32,
    completion_tokens: i32,
    total_tokens: i32,
}

#[derive(Debug)]
pub struct GtpClient {
    token: String,
    model: String,
    http_client: reqwest::Client,
    chat_url: &'static str,
    dalle_url: &'static str,
    messages: Mutex<Vec<Message>>,
}

#[derive(Debug, Serialize, Constructor)]
struct DalleRequest<'a> {
    model: &'a str,
    prompt: &'a str,
    n: i32,
    size: &'static str,
}

#[derive(Debug, Deserialize, Constructor)]
struct DalleResponse {
    data: Vec<DalleResponseData>,
}

#[derive(Debug, Deserialize, Constructor)]
struct DalleResponseData {
    url: String,
}

impl GtpClient {
    pub fn new(model: String, token: String, base_rules: String) -> Self {
        let url = "https://api.openai.com/v1/chat/completions";
        let http_client = reqwest::Client::new();

        GtpClient {
            token,
            model,
            http_client,
            chat_url: url,
            dalle_url: "https://api.openai.com/v1/images/generations",
            messages: Mutex::new(vec![Message::new("user", base_rules.into())]),
        }
    }

    pub async fn get_completion(&self, prompt: String) -> Result<Arc<String>> {
        let message = Message::new("user", prompt.into());
        let messages = {
            let mut messages = self.messages.lock().await;

            messages.push(message);
            messages.clone()
        };

        let request_data = Request::new(self.model.as_str(), &messages, 0.7);
        let token = &self.token;
        let response = self
            .http_client
            .post(self.chat_url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&request_data)
            .send()
            .await?;

        if response.status().is_success() {
            let mut completion = response.json::<Response>().await?;
            let choice = completion.choices.remove(0);
            let result = Arc::new(choice.message.content);
            let message = Message::new("system", result.clone());

            {
                let mut messages = self.messages.lock().await;
                messages.push(message);
            }

            Ok(result)
        } else {
            bail!(response.text().await?)
        }
    }

    pub async fn get_image(&self, prompt: &str) -> Result<String> {
        let dalle_request =
            DalleRequest::new("dall-e-3", prompt, 1, "1024x1024");

        let token = &self.token;
        let response = self
            .http_client
            .post(self.dalle_url)
            .header("Authorization", format!("Bearer {token}"))
            .json(&dalle_request)
            .send()
            .await?;

        if response.status().is_success() {
            let mut completion = response.json::<DalleResponse>().await?;
            let response = completion.data.remove(0);

            Ok(response.url)
        } else {
            bail!(response.text().await?)
        }
    }
}
