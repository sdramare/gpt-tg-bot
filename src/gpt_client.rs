use std::sync::Arc;

use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose};
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

#[derive(Debug, Serialize, Deserialize, Constructor, From, Clone)]
struct Base64Image {
    b64_json: Arc<String>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: Arc<String> },
    ImageUrl { image_url: Url },
}

#[derive(Debug, Serialize, Deserialize, Constructor, From, Clone)]
struct Image {
    url: String,
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
    chat_url: String,
    dalle_url: String,
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

#[derive(Debug, Deserialize, Constructor)]
struct GptImageResponse {
    data: Vec<Base64Image>,
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
            chat_url: format!("{}/chat/completions", &api_url),
            dalle_url: format!("{}/images/generations", &api_url),
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
        let token = self.token;
        let response = self
            .http_client
            .post(&self.chat_url)
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

    async fn get_image_value(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Value> {
        // Download the image from URL
        let image_bytes = self
            .http_client
            .get(&image_url)
            .send()
            .await
            .with_context(|| format!("download image {image_url}"))?
            .bytes()
            .await
            .with_context(|| format!("get image bytes {image_url}"))?;

        // Convert to base64
        let base64_image = general_purpose::STANDARD.encode(&image_bytes);

        // Determine image format from URL or content
        let format = if image_url.ends_with(".png") {
            "png"
        } else {
            "jpeg" // Default to jpeg
        };

        let data_url = format!("data:image/{};base64,{}", format, base64_image);

        let value = Value::Complex(vec![
            Content::Text { text: text.into() },
            Content::ImageUrl {
                image_url: Url::new(data_url.into()),
            },
        ]);
        Ok(value)
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
        let value = self.get_image_value(text, image_url).await?;
        self.get_value_completion(value, ModelMode::Fast).await
    }

    async fn get_image_smart_completion(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Arc<String>> {
        let value = self.get_image_value(text, image_url).await?;
        self.get_value_completion(value, ModelMode::Smart).await
    }

    async fn get_image(&self, prompt: &str) -> Result<Arc<String>> {
        let dalle_request =
            DalleRequest::new("gpt-image-1", prompt, 1, "1024x1024", "low");

        let token = self.token;
        let response = self
            .http_client
            .post(&self.dalle_url)
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

    async fn get_image_smart_completion(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Arc<String>>;

    async fn get_image(&self, prompt: &str) -> Result<Arc<String>>;

    async fn get_audio(&self, prompt: &str) -> Result<Vec<u8>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_get_completion() {
        // Setup mock server
        let mock_server = MockServer::start().await;

        // Create a response body that matches the expected structure
        let response_body = r#"{
            "id": "test-id",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "This is a test response"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 20,
                "total_tokens": 30
            }
        }"#;

        // Configure the mock to return our response for chat completions
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body),
            )
            .mount(&mock_server)
            .await;

        // Create a client with a custom URL pointing to our mock server
        let http_client = reqwest::Client::new();

        // Format the URLs and convert them to 'static lifetimes
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            http_client,
            chat_url,
            dalle_url,
            messages: Mutex::new(Vec::new()),
        };

        // Test the get_completion method
        let result = client.get_completion("Test prompt".to_string()).await;

        // Assert the result is as expected
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_ref(), "This is a test response");
    }

    #[tokio::test]
    async fn test_get_image_completion() {
        // Setup mock servers
        let mock_server = MockServer::start().await;
        let image_server = MockServer::start().await;

        // Create a test image response
        let image_data = vec![0, 1, 2, 3, 4]; // Simple mock image data

        // Expected base64 encoded data
        let expected_base64 = general_purpose::STANDARD.encode(&image_data);

        // Create a custom request matcher for checking body contents
        struct BodyContainsMatcher {
            expected_content: String,
        }

        impl wiremock::Match for BodyContainsMatcher {
            fn matches(&self, request: &wiremock::Request) -> bool {
                // Convert the body to a string and check if it contains our expected base64
                let body_str = String::from_utf8_lossy(&request.body);
                body_str.contains(&self.expected_content)
            }
        }

        // Mock image server to return our test image
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_bytes(image_data.clone()),
            )
            .mount(&image_server)
            .await;

        // Chat API response
        let response_body = r#"{
            "id": "test-id",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "This is a response about an image"
                    },
                    "finish_reason": "stop"
                }
            ],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 20,
                "total_tokens": 120
            }
        }"#;

        // Mock GPT API with request validation
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(BodyContainsMatcher {
                expected_content: expected_base64.clone(),
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body),
            )
            .mount(&mock_server)
            .await;

        // Create a client with custom URLs
        let http_client = reqwest::Client::new();

        // Format the URLs and convert them to 'static lifetimes
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            http_client,
            chat_url,
            dalle_url,
            messages: Mutex::new(Vec::new()),
        };

        // Test the image completion with our image URL
        let image_url = format!("{}/test.jpg", image_server.uri());
        let result = client
            .get_image_completion("Describe this image".to_string(), image_url)
            .await;

        // Assert the results
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_ref(),
            "This is a response about an image"
        );
    }

    #[tokio::test]
    async fn test_get_image() {
        // Setup mock server
        let mock_server = MockServer::start().await;

        // Create DALL-E API response
        let response_body = r#"{
            "data": [
                {"url": "https://example.com/generated-image.jpg"}
            ]
        }"#;

        // Mock DALL-E API
        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body),
            )
            .mount(&mock_server)
            .await;

        // Create client
        let http_client = reqwest::Client::new();

        // Format the URLs and convert them to 'static lifetimes
        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            http_client,
            chat_url,
            dalle_url,
            messages: Mutex::new(Vec::new()),
        };

        // Test getting an image
        let result = client.get_image("Draw a test image").await;

        // Assert results
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_ref(),
            "https://example.com/generated-image.jpg"
        );
    }
}
