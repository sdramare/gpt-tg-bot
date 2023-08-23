use derive_more::Constructor;
use futures::lock::Mutex;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    id: String,
    object: String,
    created: i64,
    model: String,
    choices: Vec<Choice>,
    usage: Usage,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Choice {
    index: i32,
    message: ResponseMessage,
    finish_reason: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResponseMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Usage {
    prompt_tokens: i32,
    completion_tokens: i32,
    total_tokens: i32,
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
    ) -> Result<Arc<String>, Box<dyn std::error::Error + Send + Sync>> {
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
            Err(response.text().await?.into())
        }
    }
}
