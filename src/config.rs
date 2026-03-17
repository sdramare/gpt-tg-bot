use std::collections::HashMap;
use std::time::Duration;

use anyhow::{Context, Result};
use rand::rngs::ThreadRng;
use tracing::{info, warn};

use crate::gpt_client::GtpClient;
use crate::message_processor::{Config, TgBot};
use crate::s3_client::fetch_rules_from_s3;
use crate::tg_client::{ConsoleClient, TgClient};

macro_rules! context_env {
    ($name: literal) => {
        std::env::var($name).context($name)?
    };
}

const DEFAULT_API_URL: &str = "https://api.openai.com/v1/chat/completions";
const DEFAULT_VOICE: &str = "onyx";
const DEFAULT_IMAGE_MODEL: &str = "gpt-image-1";

pub struct AppConfig {
    pub tg_token: String,
    pub gpt_token: &'static str,
    pub gpt_model: &'static str,
    pub gpt_smart_model: &'static str,
    pub gpt_image_model: &'static str,
    pub gpt_image_size: Option<&'static str>,
    pub gpt_image_moderation: Option<&'static str>,
    pub base_rules: String,
    pub private_base_rules: String,
    pub gtp_preamble: String,
    pub voice: &'static str,
    pub tg_bot_names: Vec<&'static str>,
    pub dummy_answers: Vec<&'static str>,
    pub tg_bot_allow_chats: Vec<i64>,
    pub api_url: &'static str,
    pub private_api_url: &'static str,
    pub private_model: &'static str,
    pub private_token: &'static str,
    pub names_map: HashMap<String, String>,
    pub heartbeat_interval: Option<Duration>,
}

impl AppConfig {
    pub async fn from_env() -> Result<Self> {
        let tg_bot_names =
            context_env!("BOT_ALIAS").leak().split(',').collect();
        let dummy_answers =
            context_env!("DUMMY_ANSWERS").leak().split(',').collect();

        let tg_token = context_env!("TG_TOKEN");
        let gpt_token = context_env!("GPT_TOKEN").leak();
        let gpt_model = context_env!("GPT_MODEL").leak();
        let gpt_smart_model = context_env!("GPT_SMART_MODEL").leak();
        let gpt_image_model = std::env::var("GPT_IMAGE_MODEL")
            .map(|s| s.leak() as &'static str)
            .unwrap_or(DEFAULT_IMAGE_MODEL);
        let gpt_image_size =
            std::env::var("GPT_IMAGE_SIZE").ok().and_then(|size| {
                let trimmed = size.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string().leak() as &'static str)
                }
            });
        let gpt_image_moderation = std::env::var("GPT_IMAGE_MODERATION")
            .ok()
            .and_then(|moderation| {
                let trimmed = moderation.trim();
                if trimmed.is_empty() {
                    None
                } else {
                    Some(trimmed.to_string().leak() as &'static str)
                }
            });
        let mut base_rules = context_env!("GPT_RULES");

        if let Ok(s3_uri) = std::env::var("S3_RULES_URI")
            && !s3_uri.is_empty()
        {
            match fetch_rules_from_s3(&s3_uri).await {
                Ok(s3_rules) => {
                    info!("Loaded additional rules from S3: {s3_uri}");
                    base_rules.push('\n');
                    base_rules.push_str(&s3_rules);
                }
                Err(err) => {
                    warn!(
                        error = format!("{err:#}"),
                        "Failed to load rules from S3: {s3_uri}, \
                         falling back to GPT_RULES env var only"
                    );
                }
            }
        }

        let private_base_rules =
            std::env::var("PRIVATE_GPT_RULES").unwrap_or_default();

        let gtp_preamble = context_env!("GPT_PREAMBLE");

        let heartbeat_interval = if let Ok(secs) =
            std::env::var("HEARTBEAT_INTERVAL_SECONDS")
        {
            Some(Duration::from_secs(
                secs.parse()
                    .context("HEARTBEAT_INTERVAL_SECONDS must be a number")?,
            ))
        } else {
            None
        };

        let voice = std::env::var("VOICE")
            .unwrap_or(DEFAULT_VOICE.to_string())
            .leak();

        let mut tg_bot_allow_chats = Vec::new();
        for chat_id in context_env!("TG_ALLOW_CHATS").split(',') {
            tg_bot_allow_chats.push(chat_id.parse::<i64>()?);
        }

        let api_url = std::env::var("GPT_CHAT_URL")
            .map(|s| s.leak() as &'static str)
            .unwrap_or_else(|_| DEFAULT_API_URL);

        let private_api_url = std::env::var("GPT_PRIVATE_CHAT_URL")
            .map(|s| s.leak() as &'static str)
            .unwrap_or_else(|_| DEFAULT_API_URL);

        let private_model = std::env::var("GPT_PRIVATE_MODEL")
            .map(|s| s.leak() as &'static str)
            .unwrap_or(gpt_model);

        let private_token = std::env::var("GPT_PRIVATE_TOKEN")
            .map(|s| s.leak() as &'static str)
            .unwrap_or(gpt_token);

        let names_map = context_env!("NAMES_MAP");
        let names_map = serde_json::from_str(&names_map)?;

        Ok(AppConfig {
            tg_token,
            gpt_token,
            gpt_model,
            gpt_smart_model,
            gpt_image_model,
            gpt_image_size,
            gpt_image_moderation,
            base_rules,
            private_base_rules,
            gtp_preamble,
            voice,
            tg_bot_names,
            dummy_answers,
            tg_bot_allow_chats,
            api_url,
            private_api_url,
            private_model,
            private_token,
            names_map,
            heartbeat_interval,
        })
    }

    pub fn build_tg_bot(self) -> TgBot<TgClient, GtpClient, ThreadRng> {
        let tg_client = TgClient::new(self.tg_token);

        let gtp_client = GtpClient::new(
            self.api_url,
            self.gpt_model,
            self.gpt_smart_model,
            self.gpt_image_model,
            self.gpt_image_size,
            self.gpt_image_moderation,
            self.voice,
            self.gpt_token,
            self.base_rules,
        );

        let private_gtp_client = GtpClient::new(
            self.private_api_url,
            self.private_model,
            self.gpt_smart_model,
            self.gpt_image_model,
            self.gpt_image_size,
            self.gpt_image_moderation,
            self.voice,
            self.private_token,
            self.private_base_rules,
        );

        let mut config = Config::new(
            self.names_map,
            self.gtp_preamble,
            self.dummy_answers,
            self.tg_bot_allow_chats,
            self.tg_bot_names,
        );

        if let Some(heartbeat_interval) = self.heartbeat_interval {
            config.message_delay = heartbeat_interval;
        }

        TgBot::new(
            gtp_client,
            private_gtp_client,
            tg_client,
            config,
            rand::thread_rng,
        )
    }

    pub fn first_allowed_chat_id(&self) -> Result<i64> {
        self.tg_bot_allow_chats
            .first()
            .copied()
            .context("TG_ALLOW_CHATS must contain at least one chat id")
    }

    pub fn build_console_tg_bot(
        self,
    ) -> TgBot<ConsoleClient, GtpClient, ThreadRng> {
        let gtp_client = GtpClient::new(
            self.api_url,
            self.gpt_model,
            self.gpt_smart_model,
            self.gpt_image_model,
            self.gpt_image_size,
            self.gpt_image_moderation,
            self.voice,
            self.gpt_token,
            self.base_rules,
        );

        let private_gtp_client = GtpClient::new(
            self.private_api_url,
            self.private_model,
            self.gpt_smart_model,
            self.gpt_image_model,
            self.gpt_image_size,
            self.gpt_image_moderation,
            self.voice,
            self.private_token,
            self.private_base_rules,
        );

        let mut config = Config::new(
            self.names_map,
            self.gtp_preamble,
            self.dummy_answers,
            self.tg_bot_allow_chats,
            self.tg_bot_names,
        );

        if let Some(heartbeat_interval) = self.heartbeat_interval {
            config.message_delay = heartbeat_interval;
        }

        TgBot::new(
            gtp_client,
            private_gtp_client,
            ConsoleClient,
            config,
            rand::thread_rng,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_config() -> AppConfig {
        AppConfig {
            tg_token: "test-tg-token".to_string(),
            gpt_token: "test-gpt-token",
            gpt_model: "gpt-4",
            gpt_smart_model: "gpt-4-turbo",
            gpt_image_model: "gpt-image-1",
            gpt_image_size: None,
            gpt_image_moderation: None,
            base_rules: "Be helpful.".to_string(),
            private_base_rules: String::new(),
            gtp_preamble: "Hello".to_string(),
            voice: "onyx",
            tg_bot_names: vec!["bot"],
            dummy_answers: vec!["idk"],
            tg_bot_allow_chats: vec![1, 2, 3],
            api_url: "https://api.openai.com/v1",
            private_api_url: "https://api.openai.com/v1",
            private_model: "gpt-4",
            private_token: "test-gpt-token",
            names_map: HashMap::from([(
                "alice".to_string(),
                "Alice".to_string(),
            )]),
            heartbeat_interval: None,
        }
    }

    #[test]
    fn build_tg_bot_constructs_successfully() {
        let config = make_config();
        let _bot = config.build_tg_bot();
    }

    #[test]
    fn build_tg_bot_with_heartbeat_interval() {
        let mut config = make_config();
        config.heartbeat_interval = Some(Duration::from_secs(30));
        let _bot = config.build_tg_bot();
    }

    #[test]
    fn build_tg_bot_with_empty_base_rules() {
        let mut config = make_config();
        config.base_rules = String::new();
        let _bot = config.build_tg_bot();
    }

    #[test]
    fn build_tg_bot_with_private_base_rules() {
        let mut config = make_config();
        config.private_base_rules = "Private rules.".to_string();
        let _bot = config.build_tg_bot();
    }

    #[test]
    fn parse_allow_chats() {
        let input = "123,-456,789";
        let result: Result<Vec<i64>, _> =
            input.split(',').map(|s| s.parse::<i64>()).collect();
        let chats = result.unwrap();
        assert_eq!(chats, vec![123, -456, 789]);
    }

    #[test]
    fn parse_allow_chats_invalid() {
        let input = "123,abc,789";
        let result: Result<Vec<i64>, _> =
            input.split(',').map(|s| s.parse::<i64>()).collect();
        assert!(result.is_err());
    }

    #[test]
    fn default_voice_value() {
        assert_eq!(DEFAULT_VOICE, "onyx");
    }

    #[test]
    fn default_api_url_value() {
        assert_eq!(
            DEFAULT_API_URL,
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn names_map_json_parsing() {
        let json = r#"{"alice": "Alice", "bob": "Bob"}"#;
        let map: HashMap<String, String> = serde_json::from_str(json).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(map["alice"], "Alice");
        assert_eq!(map["bob"], "Bob");
    }

    #[test]
    fn names_map_invalid_json() {
        let json = "not json";
        let result: Result<HashMap<String, String>, _> =
            serde_json::from_str(json);
        assert!(result.is_err());
    }
}
