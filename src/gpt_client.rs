use derive_more::{Constructor, Display, Error};
use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::sync::Arc;

#[derive(Error, Display, Debug, Constructor)]
pub struct GtpError {
    response: String,
}

#[derive(Debug, Serialize, Constructor)]
pub struct Request<'a> {
    model: &'a str,
    messages: &'a Vec<Message>,
    temperature: f64,
}

#[derive(Debug, Serialize, Deserialize, Constructor, Clone)]
pub struct Message {
    role: &'static str,
    content: Arc<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub id: String,
    pub object: String,
    pub created: i64,
    pub model: String,
    pub choices: Vec<Choice>,
    pub usage: Usage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    pub index: i32,
    pub message: ResponseMessage,
    pub finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: i32,
    pub completion_tokens: i32,
    pub total_tokens: i32,
}

#[derive(Debug)]
pub struct GtpClient {
    token: String,
    model: String,
    http_client: reqwest::Client,
    url: &'static str,
    messages: Mutex<Vec<Message>>,
}

impl GtpClient {
    pub fn new(model: String, token: String) -> Self {
        let url = "https://api.openai.com/v1/chat/completions";
        let http_client = reqwest::Client::new();

        GtpClient {
            token,
            model,
            http_client,
            url,
            messages: Mutex::new(Vec::new()),
        }
    }

    pub async fn get_completion(
        &self,
        prompt: String,
    ) -> Result<Arc<String>, Box<dyn Error + Send + Sync>> {
        let message = Message::new("user", prompt.into());
        let messages = {
            let mut messages = self.messages.lock().await;

            messages.push(message);
            messages.clone()
        };

        let request_data = Request::new(&*self.model, &messages, 0.7);
        let token = &self.token;
        let response = self
            .http_client
            .post(self.url)
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
            let error = response.text().await?;
            Err(GtpError::new(error).into())
        }
    }
}
