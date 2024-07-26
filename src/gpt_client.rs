use std::sync::Arc;

use anyhow::{bail, Result};
use derive_more::{Constructor, From};
use futures::lock::Mutex;
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Constructor)]
struct Request<'a> {
    model: &'a str,
    messages: &'a Vec<Message>,
    temperature: f64,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "role", content = "content", rename_all = "snake_case")]
enum Message {
    User(Value),
    System(Value),
}

#[derive(Debug, Serialize, Deserialize, Constructor, From, Clone)]
struct Url {
    url: Arc<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: Arc<String> },
    ImageUrl { image_url: Url },
}

#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
enum Value {
    Plain(Arc<String>),
    Complex(Vec<Content>),
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
    token: &'static str,
    model: &'static str,
    http_client: reqwest::Client,
    chat_url: &'static str,
    dalle_url: &'static str,
    messages: Mutex<Vec<Message>>,
}

#[derive(Debug, Serialize, Constructor)]
struct DalleRequest<'a> {
    model: &'static str,
    prompt: &'a str,
    n: i32,
    size: &'static str,
}

#[derive(Debug, Deserialize, Constructor)]
struct DalleResponse {
    data: Vec<Url>,
}

impl GtpClient {
    pub fn new(
        model: &'static str,
        token: &'static str,
        base_rules: String,
    ) -> Self {
        let url = "https://api.openai.com/v1/chat/completions";
        let http_client = reqwest::Client::new();

        GtpClient {
            token,
            model,
            http_client,
            chat_url: url,
            dalle_url: "https://api.openai.com/v1/images/generations",
            messages: Mutex::new(vec![Message::System(Value::Plain(
                base_rules.into(),
            ))]),
        }
    }

    async fn get_value_completion(&self, value: Value) -> Result<Arc<String>> {
        let message = Message::User(value);
        let messages = {
            let mut messages = self.messages.lock().await;

            messages.push(message);

            messages.clone()
        };

        let request_data = Request::new(self.model, &messages, 0.7);
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
            let choice = completion.choices.swap_remove(0);
            let result = Arc::new(choice.message.content);
            let message = Message::System(Value::Plain(result.clone()));

            {
                let mut messages = self.messages.lock().await;
                messages.push(message);
            }

            Ok(result)
        } else {
            bail!(response.text().await?)
        }
    }
}

impl GtpInteractor for GtpClient {
    async fn get_completion(&self, prompt: String) -> Result<Arc<String>> {
        self.get_value_completion(Value::Plain(prompt.into())).await
    }
    async fn get_image_completion(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Arc<String>> {
        let value = Value::Complex(vec![
            Content::Text { text: text.into() },
            Content::ImageUrl {
                image_url: Arc::new(image_url).into(),
            },
        ]);
        self.get_value_completion(value).await
    }
    async fn get_image(&self, prompt: &str) -> Result<Arc<String>> {
        let dalle_request =
            DalleRequest::new("dall-e-3", prompt, 1, "1024x1024");

        let token = self.token;
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

#[cfg_attr(test, automock)]
pub trait GtpInteractor {
    async fn get_completion(&self, prompt: String) -> Result<Arc<String>>;
    async fn get_image_completion(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Arc<String>>;
    async fn get_image(&self, prompt: &str) -> Result<Arc<String>>;
}
