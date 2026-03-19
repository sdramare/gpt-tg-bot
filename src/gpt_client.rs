use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
use base64::{Engine as _, engine::general_purpose};
use dashmap::DashMap;
use derive_more::{Constructor, From};
#[cfg(test)]
use mockall::automock;
use serde::{Deserialize, Serialize};
use tracing::info;

type AStr = Arc<str>;

const MAX_TOOL_CALL_ROUNDS: usize = 5;

#[derive(Debug, Serialize, Clone)]
struct FunctionDef {
    name: &'static str,
    description: &'static str,
    parameters: serde_json::Value,
}

#[derive(Debug, Serialize, Clone)]
struct Tool {
    #[serde(rename = "type")]
    tool_type: &'static str,
    function: FunctionDef,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ToolCallFunction {
    name: AStr,
    arguments: AStr,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct ToolCall {
    id: AStr,
    #[serde(rename = "type", default = "default_tool_type")]
    tool_type: AStr,
    function: ToolCallFunction,
}

#[derive(Debug, Serialize, Constructor)]
struct Request<'a> {
    model: &'a str,
    messages: &'a Vec<Message>,
    temperature: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<&'a Vec<Tool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<&'static str>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "role", rename_all = "snake_case")]
enum Message {
    User {
        content: Value,
    },
    System {
        content: Value,
    },
    #[serde(rename = "assistant")]
    Assistant {
        content: Value,
    },
    #[serde(rename = "assistant")]
    AssistantToolCall {
        tool_calls: Vec<ToolCall>,
    },
    Tool {
        tool_call_id: AStr,
        content: Value,
    },
}

#[derive(Debug, Serialize, Deserialize, Constructor, From, Clone)]
struct Url {
    url: AStr,
}

#[derive(Debug, Serialize, Deserialize, Constructor, From, Clone)]
#[serde(rename_all = "snake_case")]
struct Base64Image {
    b64_json: AStr,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Content {
    Text { text: AStr },
    ImageUrl { image_url: Url },
}

#[derive(Debug, Serialize, Clone)]
#[serde(untagged)]
enum Value {
    Plain(AStr),
    Complex(Vec<Content>),
}

#[derive(Debug, Deserialize)]
struct Response {
    choices: Vec<Choice>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "finish_reason", rename_all = "snake_case")]
enum Choice {
    #[serde(rename = "stop")]
    Content {
        message: ContentResponse,
    },
    ToolCalls {
        message: ToolCallsResponse,
    },
    #[serde(other)]
    Other,
}
#[derive(Debug, Deserialize)]
struct ContentResponse {
    content: AStr,
}

#[derive(Debug, Deserialize)]
struct ToolCallsResponse {
    tool_calls: Vec<ToolCall>,
}

pub enum CompletionResult {
    Text(AStr),
    Image(Vec<u8>),
}

#[derive(Debug)]
pub struct GtpClient {
    token: &'static str,
    model: &'static str,
    voice: &'static str,
    smart_model: &'static str,
    image_model: &'static str,
    image_size: Option<&'static str>,
    image_moderation: Option<&'static str>,
    http_client: reqwest::Client,
    chat_url: String,
    dalle_url: String,
    messages: DashMap<i64, Vec<Message>>,
    base_rules: Vec<Message>,
}

#[derive(Debug, Serialize, Constructor)]
struct ImageGenerationRequest<'a> {
    model: &'static str,
    prompt: &'a str,
    n: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<&'static str>,
    quality: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    moderation: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<&'static str>,
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

#[derive(Clone, Copy)]
enum ModelMode {
    Fast,
    Smart,
}

fn default_tool_type() -> AStr {
    "function".into()
}

fn make_generate_image_tool() -> Tool {
    Tool {
        tool_type: "function",
        function: FunctionDef {
            name: "generate_image",
            description: "Generate an image based on a text prompt. \
                Use this when the user asks to draw, create, or generate \
                an image.",
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "prompt": {
                        "type": "string",
                        "description": "The image generation prompt"
                    }
                },
                "required": ["prompt"]
            }),
        },
    }
}

impl GtpClient {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        api_url: &'static str,
        model: &'static str,
        smart_model: &'static str,
        image_model: &'static str,
        image_size: Option<&'static str>,
        image_moderation: Option<&'static str>,
        voice: &'static str,
        token: &'static str,
        base_rules: String,
    ) -> Self {
        let http_client = reqwest::Client::new();

        let base_rules = if base_rules.is_empty() {
            Vec::new()
        } else {
            vec![Message::System {
                content: Value::Plain(base_rules.into()),
            }]
        };

        GtpClient {
            token,
            model,
            voice,
            smart_model,
            image_model,
            image_size,
            image_moderation,
            http_client,
            chat_url: format!("{}/chat/completions", &api_url),
            dalle_url: format!("{}/images/generations", &api_url),
            messages: DashMap::new(),
            base_rules,
        }
    }

    async fn get_value_completion(
        &self,
        user_id: i64,
        value: Value,
        mode: ModelMode,
    ) -> Result<CompletionResult> {
        let tools = vec![make_generate_image_tool()];
        let mut messages = self.user_messages(user_id);
        messages.push(Message::User { content: value });

        let model = self.model_for(mode);

        for _ in 0..MAX_TOOL_CALL_ROUNDS {
            let choice = self
                .request_chat_completion(model, &messages, &tools)
                .await?;

            match choice {
                Choice::Content {
                    message: content_response,
                } => {
                    return Ok(self.finalize_text_response(
                        user_id,
                        &mut messages,
                        content_response.content,
                    ));
                }
                Choice::ToolCalls {
                    message: tool_calls_response,
                } => {
                    if let Some(result) = self
                        .handle_tool_calls(
                            user_id,
                            &mut messages,
                            tool_calls_response.tool_calls,
                        )
                        .await?
                    {
                        return Ok(result);
                    }
                    continue;
                }
                Choice::Other => bail!("unexpected finish reason in choice"),
            }
        }
        bail!(
            "failed to get a valid completion after {} attempts",
            MAX_TOOL_CALL_ROUNDS
        );
    }

    fn user_messages(&self, user_id: i64) -> Vec<Message> {
        self.messages
            .get(&user_id)
            .map_or_else(|| self.base_rules.clone(), |chat| chat.clone())
    }

    fn model_for(&self, mode: ModelMode) -> &'static str {
        match mode {
            ModelMode::Fast => self.model,
            ModelMode::Smart => self.smart_model,
        }
    }

    async fn request_chat_completion(
        &self,
        model: &'static str,
        messages: &Vec<Message>,
        tools: &Vec<Tool>,
    ) -> Result<Choice> {
        let request_data =
            Request::new(model, messages, 1.0, Some(tools), Some("auto"));
        let response = self
            .http_client
            .post(&self.chat_url)
            .bearer_auth(self.token)
            .json(&request_data)
            .send()
            .await?;

        if !response.status().is_success() {
            bail!(response.text().await?)
        }

        let mut completion = response.json::<Response>().await?;
        let result = completion
            .choices
            .pop()
            .ok_or_else(|| anyhow!("no choices in response"))?;
        Ok(result)
    }

    async fn handle_tool_calls(
        &self,
        user_id: i64,
        messages: &mut Vec<Message>,
        tool_calls: Vec<ToolCall>,
    ) -> Result<Option<CompletionResult>> {
        messages.push(Message::AssistantToolCall {
            tool_calls: tool_calls.clone(),
        });

        for tool_call in tool_calls {
            if tool_call.function.name.as_ref() != "generate_image" {
                continue;
            }

            let image_bytes = self
                .execute_generate_image_call(messages, &tool_call)
                .await?;
            self.messages.insert(user_id, messages.clone());
            return Ok(Some(CompletionResult::Image(image_bytes)));
        }

        Ok(None)
    }

    async fn execute_generate_image_call(
        &self,
        messages: &mut Vec<Message>,
        tool_call: &ToolCall,
    ) -> Result<Vec<u8>> {
        let args: serde_json::Value =
            serde_json::from_str(&tool_call.function.arguments)
                .context("parse generate_image arguments")?;
        let prompt = args["prompt"]
            .as_str()
            .context("missing prompt in generate_image args")?;

        let image_bytes = self.get_image_internal(prompt).await?;
        let data_url = format!(
            "data:image/png;base64,{}",
            general_purpose::STANDARD.encode(&image_bytes)
        );

        messages.push(Message::Tool {
            tool_call_id: tool_call.id.clone(),
            content: Value::Plain(
                format!("Image generated for prompt: '{prompt}'").into(),
            ),
        });
        messages.push(Message::Assistant {
            content: Value::Complex(vec![Content::ImageUrl {
                image_url: Url::new(data_url.into()),
            }]),
        });

        Ok(image_bytes)
    }

    fn finalize_text_response(
        &self,
        user_id: i64,
        messages: &mut Vec<Message>,
        result: AStr,
    ) -> CompletionResult {
        messages.push(Message::Assistant {
            content: Value::Plain(result.clone()),
        });
        self.messages.insert(user_id, messages.clone());
        CompletionResult::Text(result)
    }

    async fn get_image_value(
        &self,
        text: String,
        image_url: String,
    ) -> Result<Value> {
        let image_bytes = self
            .http_client
            .get(&image_url)
            .send()
            .await
            .with_context(|| format!("download image {image_url}"))?
            .bytes()
            .await
            .with_context(|| format!("get image bytes {image_url}"))?;

        let base64_image = general_purpose::STANDARD.encode(&image_bytes);

        let format = if image_url.ends_with(".png") {
            "png"
        } else {
            "jpeg"
        };

        let data_url = format!("data:image/{format};base64,{base64_image}");

        Ok(Value::Complex(vec![
            Content::Text { text: text.into() },
            Content::ImageUrl {
                image_url: Url::new(data_url.into()),
            },
        ]))
    }

    #[tracing::instrument(skip(self))]
    async fn get_image_internal(&self, prompt: &str) -> Result<Vec<u8>> {
        let reponse_format = if self.image_moderation.is_some() {
            None
        } else {
            Some("b64_json")
        };
        let dalle_request = ImageGenerationRequest::new(
            self.image_model,
            prompt,
            1,
            self.image_size,
            "high",
            self.image_moderation,
            reponse_format,
        );

        info!(model = self.image_model, "requesting image generation");

        let response = self
            .http_client
            .post(&self.dalle_url)
            .bearer_auth(self.token)
            .json(&dalle_request)
            .send()
            .await?;

        if !response.status().is_success() {
            bail!(response.text().await?)
        }

        let mut completion = response.json::<GptImageResponse>().await?;
        let img_resp = completion
            .data
            .pop()
            .ok_or_else(|| anyhow!("no image data found in response"))?;

        general_purpose::STANDARD
            .decode(img_resp.b64_json.as_bytes())
            .with_context(|| format!("decode image {}", img_resp.b64_json))
    }
}

impl GtpInteractor for GtpClient {
    async fn get_completion(
        &self,
        user_id: i64,
        prompt: String,
    ) -> Result<CompletionResult> {
        self.get_value_completion(
            user_id,
            Value::Plain(prompt.into()),
            ModelMode::Fast,
        )
        .await
    }

    async fn get_smart_completion(
        &self,
        user_id: i64,
        prompt: String,
    ) -> Result<CompletionResult> {
        self.get_value_completion(
            user_id,
            Value::Plain(prompt.into()),
            ModelMode::Smart,
        )
        .await
    }

    async fn get_image_completion(
        &self,
        user_id: i64,
        text: String,
        image_url: String,
    ) -> Result<AStr> {
        let value = self.get_image_value(text, image_url).await?;
        match self
            .get_value_completion(user_id, value, ModelMode::Fast)
            .await?
        {
            CompletionResult::Text(t) => Ok(t),
            CompletionResult::Image(_) => {
                bail!("unexpected image result from image completion")
            }
        }
    }

    async fn get_image_smart_completion(
        &self,
        user_id: i64,
        text: String,
        image_url: String,
    ) -> Result<AStr> {
        let value = self.get_image_value(text, image_url).await?;
        match self
            .get_value_completion(user_id, value, ModelMode::Smart)
            .await?
        {
            CompletionResult::Text(t) => Ok(t),
            CompletionResult::Image(_) => {
                bail!("unexpected image result from image completion")
            }
        }
    }

    async fn get_audio(&self, prompt: &str) -> Result<Vec<u8>> {
        let request = AudioSpeechRequest::new("tts-1", prompt, self.voice);

        let token = self.token;
        let response = self
            .http_client
            .post("https://api.openai.com/v1/audio/speech")
            .bearer_auth(token)
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
    async fn get_completion(
        &self,
        user_id: i64,
        prompt: String,
    ) -> Result<CompletionResult>;
    async fn get_smart_completion(
        &self,
        user_id: i64,
        prompt: String,
    ) -> Result<CompletionResult>;
    async fn get_image_completion(
        &self,
        user_id: i64,
        text: String,
        image_url: String,
    ) -> Result<AStr>;

    async fn get_image_smart_completion(
        &self,
        user_id: i64,
        text: String,
        image_url: String,
    ) -> Result<AStr>;

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
            image_model: "test-image-model",
            image_size: None,
            image_moderation: None,
            http_client,
            chat_url,
            dalle_url,
            messages: DashMap::new(),
            base_rules: Vec::new(),
        };

        // Test the get_completion method
        let result = client.get_completion(0, "Test prompt".to_string()).await;

        // Assert the result is as expected
        assert!(result.is_ok());
        let CompletionResult::Text(text) = result.unwrap() else {
            panic!("expected Text result");
        };
        assert_eq!(text.as_ref(), "This is a test response");
    }

    #[tokio::test]
    async fn test_get_completion_with_empty_choices_returns_error() {
        let mock_server = MockServer::start().await;

        let response_body = r#"{
            "id": "test-id",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "test-model",
            "choices": []
        }"#;

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body),
            )
            .mount(&mock_server)
            .await;

        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            image_model: "test-image-model",
            image_size: None,
            image_moderation: None,
            http_client: reqwest::Client::new(),
            chat_url,
            dalle_url,
            messages: DashMap::new(),
            base_rules: Vec::new(),
        };

        let result = client.get_completion(0, "Test prompt".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_completion_with_tool_call() {
        let mock_server = MockServer::start().await;

        struct BodyContainsMatcher {
            expected_content: String,
        }

        impl wiremock::Match for BodyContainsMatcher {
            fn matches(&self, request: &wiremock::Request) -> bool {
                let body_str = String::from_utf8_lossy(&request.body);
                body_str.contains(&self.expected_content)
            }
        }

        let tool_call_response = r#"{
            "id": "test-id",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_abc",
                                "type": "function",
                                "function": {
                                    "name": "generate_image",
                                    "arguments": "{\"prompt\":\"a red cat\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }"#;

        let image_b64 = general_purpose::STANDARD.encode(b"PNG_BYTES");
        let dalle_response =
            format!(r#"{{"data": [{{"b64_json": "{}"}}]}}"#, image_b64);

        // First call returns tool_calls, second call never happens
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(tool_call_response),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(BodyContainsMatcher {
                expected_content: "\"model\":\"test-image-model\"".to_string(),
            })
            .and(BodyContainsMatcher {
                expected_content: "\"size\":\"512x512\"".to_string(),
            })
            .and(BodyContainsMatcher {
                expected_content: "\"moderation\":\"low\"".to_string(),
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_string(dalle_response),
            )
            .mount(&mock_server)
            .await;

        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            image_model: "test-image-model",
            image_size: Some("512x512"),
            image_moderation: Some("low"),
            http_client: reqwest::Client::new(),
            chat_url,
            dalle_url,
            messages: DashMap::new(),
            base_rules: Vec::new(),
        };

        let result = client
            .get_completion(42, "draw a red cat".to_string())
            .await;
        assert!(result.is_ok(), "{:?}", result.err());
        let CompletionResult::Image(bytes) = result.unwrap() else {
            panic!("expected Image result");
        };
        assert_eq!(bytes, b"PNG_BYTES");
    }

    #[tokio::test]
    async fn test_get_completion_with_tool_call_without_size() {
        let mock_server = MockServer::start().await;

        struct BodyNotContainsMatcher {
            unexpected_content: String,
        }

        impl wiremock::Match for BodyNotContainsMatcher {
            fn matches(&self, request: &wiremock::Request) -> bool {
                let body_str = String::from_utf8_lossy(&request.body);
                !body_str.contains(&self.unexpected_content)
            }
        }

        let tool_call_response = r#"{
            "id": "test-id",
            "object": "chat.completion",
            "created": 1700000000,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "tool_calls": [
                            {
                                "id": "call_abc",
                                "type": "function",
                                "function": {
                                    "name": "generate_image",
                                    "arguments": "{\"prompt\":\"a red cat\"}"
                                }
                            }
                        ]
                    },
                    "finish_reason": "tool_calls"
                }
            ],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        }"#;

        let image_b64 = general_purpose::STANDARD.encode(b"PNG_BYTES");
        let dalle_response =
            format!(r#"{{"data": [{{"b64_json": "{}"}}]}}"#, image_b64);

        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(tool_call_response),
            )
            .mount(&mock_server)
            .await;

        Mock::given(method("POST"))
            .and(path("/v1/images/generations"))
            .and(BodyNotContainsMatcher {
                unexpected_content: "\"size\"".to_string(),
            })
            .and(BodyNotContainsMatcher {
                unexpected_content: "\"moderation\"".to_string(),
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_string(dalle_response),
            )
            .mount(&mock_server)
            .await;

        let chat_url = format!("{}/v1/chat/completions", mock_server.uri());
        let dalle_url = format!("{}/v1/images/generations", mock_server.uri());

        let client = GtpClient {
            token: "test-token",
            model: "test-model",
            voice: "test-voice",
            smart_model: "test-smart-model",
            image_model: "test-image-model",
            image_size: None,
            image_moderation: None,
            http_client: reqwest::Client::new(),
            chat_url,
            dalle_url,
            messages: DashMap::new(),
            base_rules: Vec::new(),
        };

        let result = client
            .get_completion(42, "draw a red cat".to_string())
            .await;
        assert!(result.is_ok(), "{:?}", result.err());
        let CompletionResult::Image(bytes) = result.unwrap() else {
            panic!("expected Image result");
        };
        assert_eq!(bytes, b"PNG_BYTES");
    }

    #[tokio::test]
    async fn test_get_completion_with_rules() {
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

        let rules = "base rule - be good".to_string();

        // Configure the mock to return our response for chat completions
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .and(BodyContainsMatcher {
                expected_content: rules.clone(),
            })
            .respond_with(
                ResponseTemplate::new(200).set_body_string(response_body),
            )
            .mount(&mock_server)
            .await;

        // Format the URLs and convert them to 'static lifetimes
        let api_url = format!("{}/v1", mock_server.uri());

        let client = GtpClient::new(
            api_url.leak(),
            "test-model",
            "test-smart-model",
            "test-image-model",
            None,
            None,
            "test-voice",
            "test-token",
            rules,
        );

        // Test the get_completion method
        let result = client.get_completion(0, "Test prompt".to_string()).await;
        assert!(result.is_ok());
        let CompletionResult::Text(text) = result.unwrap() else {
            panic!("expected Text result");
        };
        assert_eq!(text.as_ref(), "This is a test response");

        let result = client.get_completion(0, "Test prompt".to_string()).await;
        assert!(result.is_ok());
        let CompletionResult::Text(text) = result.unwrap() else {
            panic!("expected Text result");
        };
        assert_eq!(text.as_ref(), "This is a test response");
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
            image_model: "test-image-model",
            image_size: None,
            image_moderation: None,
            http_client,
            chat_url,
            dalle_url,
            messages: DashMap::new(),
            base_rules: Vec::new(),
        };

        // Test the image completion with our image URL
        let image_url = format!("{}/test.jpg", image_server.uri());
        let result = client
            .get_image_completion(
                0,
                "Describe this image".to_string(),
                image_url,
            )
            .await;

        // Assert the results
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().as_ref(),
            "This is a response about an image"
        );
    }
}
