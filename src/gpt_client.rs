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
    Assistant(Value),
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
    voice: &'static str,
    smart_model: &'static str,
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
    quality: &'static str,
}

#[derive(Debug, Deserialize, Constructor)]
struct DalleResponse {
    data: Vec<Url>,
}

#[derive(Serialize, Constructor)]
struct AudioSpeechRequest<'a> {
    model: &'a str,
    input: &'a str,
    voice: &'a str,
}

enum ModelMode {
    Fast,
    Smart,
}

impl GtpClient {
    pub fn new(
        api_url: &'static str,
        model: &'static str,
        smart_model: &'static str,
        voice: &'static str,
        token: &'static str,
        base_rules: String,
    ) -> Self {
        //let api_url = "https://api.openai.com/v1/chat/completions";
        let http_client = reqwest::Client::new();

        let messages = if base_rules.is_empty() {
            Vec::new()
        } else {
            vec![Message::System(Value::Plain(base_rules.into()))]
        };

        GtpClient {
            token,
            model,
            voice,
            smart_model,
            http_client,
            chat_url: format!("{}/chat/completions", &api_url).leak(),
            dalle_url: format!("{}/images/generations", &api_url).leak(),
            messages: Mutex::new(messages),
        }
    }

    async fn get_value_completion(
        &self,
        value: Value,
        mode: ModelMode,
    ) -> Result<Arc<String>> {
        let user_message = Message::User(value);
        let mut messages = {
            let messages = self.messages.lock().await;
            messages.clone()
        };

        messages.push(user_message.clone());

        let model = match mode {
            ModelMode::Fast => self.model,
            ModelMode::Smart => self.smart_model,
        };
        let request_data = Request::new(model, &messages, 1.0);
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
            let assist_message =
                Message::Assistant(Value::Plain(result.clone()));

            {
                let mut messages = self.messages.lock().await;
                messages.push(user_message);
                messages.push(assist_message);
            }

            Ok(result)
        } else {
            bail!(response.text().await?)
        }
    }
}

impl GtpInteractor for GtpClient {
    async fn get_completion(&self, prompt: String) -> Result<Arc<String>> {
        self.get_value_completion(Value::Plain(prompt.into()), ModelMode::Fast)
            .await
    }

    async fn get_smart_completion(
        &self,
        prompt: String,
    ) -> Result<Arc<String>> {
        self.get_value_completion(Value::Plain(prompt.into()), ModelMode::Smart)
            .await
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
        self.get_value_completion(value, ModelMode::Fast).await
    }
    async fn get_image(&self, prompt: &str) -> Result<Arc<String>> {
        let dalle_request =
            DalleRequest::new("dall-e-3", prompt, 1, "1024x1024", "hd");

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

            let anwer_message = Message::User(Value::Complex(vec![
                Content::Text {
                    text: format!("По запросу '{prompt}' ты нарисовал:").into(),
                },
                Content::ImageUrl {
                    image_url: response.clone(),
                },
            ]));

            {
                let mut messages = self.messages.lock().await;
                messages.push(anwer_message);
            }

            Ok(response.url)
        } else {
            bail!(response.text().await?)
        }
    }

    async fn get_audio(&self, prompt: &str) -> Result<Vec<u8>> {
        let request = AudioSpeechRequest::new("tts-1", prompt, self.voice);

        let token = self.token;
        let response = self
            .http_client
            .post("https://api.openai.com/v1/audio/speech")
            .header("Authorization", format!("Bearer {token}"))
            .json(&request)
            .send()
            .await?;

        if response.status().is_success() {
            let audio = response.bytes().await?;
            Ok(Vec::from(audio))
        } else {
            bail!(response.text().await?)
        }
    }
}

#[cfg_attr(test, automock)]
pub trait GtpInteractor {
    async fn get_completion(&self, prompt: String) -> Result<Arc<String>>;
    async fn get_smart_completion(&self, prompt: String)
        -> Result<Arc<String>>;
    async fn get_image_completion(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Arc<String>>;
    async fn get_image(&self, prompt: &str) -> Result<Arc<String>>;

    async fn get_audio(&self, prompt: &str) -> Result<Vec<u8>>;
}
