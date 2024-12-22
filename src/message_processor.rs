use std::collections::HashMap;
use std::time::Duration;

use anyhow::bail;
use chrono::Utc;
use derive_more::Constructor;
use derive_new::new;
use dyn_fmt::AsStrFormatExt;
use lambda_http::{Request, RequestPayloadExt};
use rand::seq::SliceRandom;
use rand::Rng;
use thiserror::Error;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tracing::{error, info, span, warn, Instrument, Span};

use crate::event_handler::EventHandler;
use crate::gpt_client::GtpInteractor;
use crate::tg_client::{
    Chat, Message, TelegramInteractor, Update, PRIVATE_CHAT,
};

const DRAW_COMMAND: &str = "нарисуй";

#[derive(new)]
pub struct Config {
    name_map: HashMap<String, String>,
    preamble: String,
    dummy_answers: Vec<&'static str>,
    tg_bot_allow_chats: Vec<i64>,
    tg_bot_names: Vec<&'static str>,
    #[new(value = "std::time::Duration::from_secs(20)")]
    pub message_delay: Duration,
}

#[derive(Constructor)]
pub struct TgBot<TgClient: TelegramInteractor, GtpClient: GtpInteractor, R: Rng>
{
    gtp_client: GtpClient,
    private_gtp_client: GtpClient,
    tg_client: TgClient,
    config: Config,
    rng: fn() -> R,
}

impl<TgClient: TelegramInteractor, GtpClient: GtpInteractor, R: Rng>
    TgBot<TgClient, GtpClient, R>
{
    pub async fn process_message(
        &self,
        message: Message,
    ) -> anyhow::Result<()> {
        let chat_id = message.chat.id;

        let (tx, mut rx) = oneshot::channel::<usize>();
        let duration = self.config.message_delay;

        let wait_loop = self.wait_loop(chat_id, duration, tx);

        let process_task = async {
            let result = self.process_message_internal(message).await;

            rx.close();

            result
        };

        let (_, result) = tokio::join!(wait_loop, process_task);

        result
    }

    async fn wait_loop(
        &self,
        chat_id: i64,
        duration: Duration,
        mut tx: oneshot::Sender<usize>,
    ) {
        let start = Instant::now() + duration;

        let timeout = tokio::time::sleep(duration * 10);

        tokio::pin!(timeout);

        let mut interval = tokio::time::interval_at(start, duration);

        loop {
            tokio::select! {
                _ = &mut timeout => {

                    let _ = self.tg_client
                    .send_message(chat_id, "Я не знаю что на это ответить", None)
                    .await;

                    break;
                },
                _ = tx.closed() => {
                    break;
                },
                _ = interval.tick() => {

                    let result = self.tg_client
                    .send_message(chat_id, "Погоди, надо еще подумать", None)
                    .await;

                    match result {
                        Ok(_) => {
                            break;
                        }
                        Err(e) => {
                            error!(?e);
                        }
                    }
                }
            }
        }
    }

    async fn process_message_internal(
        &self,
        message: Message,
    ) -> anyhow::Result<()> {
        if message.photo.is_some() {
            return self.process_photo(message).await;
        }

        if let Some(text) = message.text {
            if text.contains("https://") {
                self.dummy_reaction(message.chat.id).await?;

                return Ok(());
            }

            let used_name = self
                .config
                .tg_bot_names
                .iter()
                .copied()
                .find(|&name| text.starts_with(name));

            if should_answer(
                message.reply_to_message.as_deref(),
                &message.chat,
                used_name,
                &self.config.tg_bot_allow_chats,
            ) {
                let text = used_name
                    .map(|name| text.replace(name, ""))
                    .unwrap_or(text);

                let mut first_name = message.from.first_name;

                for (name, replacement) in &self.config.name_map {
                    first_name = first_name.replace(name, replacement);
                }

                let span = span!(
                    tracing::Level::INFO,
                    "response",
                    user_name = first_name
                );

                let _enter = span.enter();

                let result = self
                    .process_and_answer(&message.chat, &text, &first_name)
                    .await;

                if let Err(error) = result {
                    if message.chat.is_private() {
                        let error_message = format!("```\n{}\n```", &error);
                        self.tg_client
                            .send_message(
                                message.chat.id,
                                &error_message,
                                "MarkdownV2".into(),
                            )
                            .await?;
                        return Err(error);
                    }
                }

                info!("Complete");

                drop(_enter)
            }
        }

        Ok(())
    }

    async fn process_and_answer(
        &self,
        chat: &Chat,
        text: &str,
        first_name: &str,
    ) -> anyhow::Result<()> {
        if let Some(index) = text.to_lowercase().find(DRAW_COMMAND) {
            self.process_image_request(text, &index, chat).await?;

            return Ok(());
        }

        self.process_text_message(text, first_name, chat).await?;

        Ok(())
    }

    async fn process_text_message(
        &self,
        text: &str,
        first_name: &str,
        chat: &Chat,
    ) -> anyhow::Result<()> {
        let text = if chat.is_private() {
            text.to_owned()
        } else {
            let mut prepend = self.config.preamble.format(&[first_name]);
            prepend.push_str(text);
            prepend
        };

        info!("Ask GPT");

        let result = if chat.is_private()
            && contains_case_insensitive(&text, "подумай")
        {
            info!("Smart completion");
            self.gtp_client(chat)
                .get_smart_completion(text)
                .instrument(Span::current())
                .await?
        } else {
            self.gtp_client(chat)
                .get_completion(text)
                .instrument(Span::current())
                .await?
        };

        info!("Sending answer to TG");

        self.tg_client
            .send_message(chat.id, result.as_str(), "MarkdownV2".into())
            .instrument(Span::current())
            .await?;
        Ok(())
    }

    async fn process_image_request(
        &self,
        text: &str,
        index: &usize,
        chat: &Chat,
    ) -> anyhow::Result<()> {
        let text = &text[index + DRAW_COMMAND.len()..];

        info!("Image request");

        let url = self.gtp_client(chat).get_image(text).await;

        match url {
            Ok(url) => {
                self.tg_client.send_image(chat.id, &url).await?;
            }
            Err(error) => {
                self.tg_client
                    .send_message(
                        chat.id,
                        "Сейчас я такое не могу нарисовать",
                        None,
                    )
                    .await?;
                return Err(error);
            }
        }
        Ok(())
    }

    fn gtp_client(&self, chat: &Chat) -> &GtpClient {
        if chat.is_private() {
            &self.private_gtp_client
        } else {
            &self.gtp_client
        }
    }

    async fn process_photo(&self, message: Message) -> anyhow::Result<()> {
        let text = message.caption.unwrap_or("Что на картинке?".to_string());

        let used_name = self
            .config
            .tg_bot_names
            .iter()
            .copied()
            .find(|&name| text.starts_with(name));

        if should_answer(
            message.reply_to_message.as_deref(),
            &message.chat,
            used_name,
            &self.config.tg_bot_allow_chats,
        ) {
            let Some(photos) = message.photo else {
                return Ok(());
            };
            let Some(photo) = photos.iter().max_by_key(|x| x.file_size) else {
                return Ok(());
            };
            info!("Photo request");
            let photo_url = self.tg_client.get_file_url(&photo.file_id).await?;

            let result = self
                .gtp_client(&message.chat)
                .get_image_completion(text, photo_url)
                .instrument(Span::current())
                .await;

            info!("Sending answer to TG");

            match result {
                Ok(result) => {
                    self.tg_client
                        .send_message(
                            message.chat.id,
                            result.as_str(),
                            "MarkdownV2".into(),
                        )
                        .instrument(Span::current())
                        .await?;
                }
                Err(error) => {
                    self.tg_client
                        .send_message(
                            message.chat.id,
                            "Прости, я задумался. Можешь повторить?",
                            "MarkdownV2".into(),
                        )
                        .instrument(Span::current())
                        .await?;

                    bail!(error)
                }
            };

            info!("Complete");
        }

        Ok(())
    }

    fn get_random_answer(&self) -> Option<&str> {
        let mut rng = (self.rng)();
        let num = rng.gen_range(0..100);
        if num < 30 {
            self.config.dummy_answers.choose(&mut rng).copied()
        } else {
            None
        }
    }

    async fn dummy_reaction(&self, chat_id: i64) -> anyhow::Result<()> {
        let Some(answer) = self.get_random_answer() else {
            return Ok(());
        };

        self.tg_client
            .send_message(chat_id, answer, "MarkdownV2".into())
            .await?;

        Ok(())
    }
}

impl<TgClient: TelegramInteractor, GtpClient: GtpInteractor, R: Rng>
    EventHandler for TgBot<TgClient, GtpClient, R>
{
    async fn process_event(&self, event: &Request) -> anyhow::Result<()> {
        let update: Option<Update> = event.payload()?;

        match update.and_then(|x| x.message) {
            None => bail!(RequestError::new("Message field is missing")),
            Some(message) => {
                let utc = Utc::now().naive_utc();
                if message.date < (utc - chrono::Duration::minutes(10)) {
                    warn!(date = ?message.date, "Too old message");
                    return Ok(());
                }

                self.process_message(message).await?;
            }
        };

        Ok(())
    }
}

fn should_answer(
    reply_to_message: Option<&Message>,
    chat: &Chat,
    used_name: Option<&str>,
    tg_bot_allow_chats: &[i64],
) -> bool {
    (tg_bot_allow_chats.contains(&chat.id))
        && (chat.chat_type == PRIVATE_CHAT
            || used_name.is_some()
            || reply_to_message.is_some_and(|reply| reply.from.is_bot))
}

fn contains_case_insensitive(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }

    let haystack_chars = haystack.chars();
    let needle_chars: Vec<char> = needle.chars().collect();

    let m = needle_chars.len();

    let mut pi = vec![0; m];
    let mut k = 0;
    for q in 1..m {
        while k > 0 && !eq_case_insensitive(needle_chars[k], needle_chars[q]) {
            k = pi[k - 1];
        }
        if eq_case_insensitive(needle_chars[k], needle_chars[q]) {
            k += 1;
        }
        pi[q] = k;
    }

    let mut q = 0;
    for ch in haystack_chars {
        while q > 0 && !eq_case_insensitive(needle_chars[q], ch) {
            q = pi[q - 1];
        }
        if eq_case_insensitive(needle_chars[q], ch) {
            q += 1;
        }
        if q == m {
            return true;
        }
    }

    false
}

fn eq_case_insensitive(a: char, b: char) -> bool {
    let mut a_lower = a.to_lowercase();
    let mut b_lower = b.to_lowercase();
    loop {
        match (a_lower.next(), b_lower.next()) {
            (Some(a_c), Some(b_c)) if a_c == b_c => continue,
            (None, None) => return true,
            _ => return false,
        }
    }
}

#[derive(Error, Debug, Constructor)]
#[error("{msg:?}")]
pub struct RequestError {
    pub msg: &'static str,
}

//unit tests
#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::Utc;
    use mockall::predicate::eq;
    use rand::rngs::mock::StepRng;

    use crate::gpt_client::MockGtpInteractor;
    use crate::message_processor::contains_case_insensitive;
    use crate::tg_client::{
        Chat, Message, MockTelegramInteractor, PhotoSize, User, PRIVATE_CHAT,
    };

    use super::{should_answer, Config, TgBot};

    #[test]
    fn test_contains_case_insensitive() {
        assert!(contains_case_insensitive("Hello", "hello"));
        assert!(contains_case_insensitive("Придумай", "придумай"));
    }

    // test for should_answer function
    #[test]
    fn test_should_answer() {
        let reply_to_message = build_private_message();
        let chat = Chat {
            id: 123,
            first_name: None,
            last_name: None,
            username: None,
            chat_type: "private".to_string(),
        };
        let used_name = Some("Hello");
        let tg_bot_allow_chats = vec![123];
        assert!(should_answer(
            reply_to_message.as_deref(),
            &chat,
            used_name,
            &tg_bot_allow_chats
        ));
    }

    // test for should_answer function for negative case
    #[test]
    fn test_should_answer_negative() {
        let reply_to_message = build_private_message();
        let chat = Chat {
            id: 123,
            first_name: None,
            last_name: None,
            username: None,
            chat_type: "private".to_string(),
        };
        let used_name = Some("Hello");
        let tg_bot_allow_chats = vec![124];
        assert!(!should_answer(
            reply_to_message.as_deref(),
            &chat,
            used_name,
            &tg_bot_allow_chats
        ));
    }

    //test for process_message function
    #[tokio::test]
    async fn test_process_message() {
        let message = build_public_message().unwrap();
        let mut tg_client = MockTelegramInteractor::new();
        let private_gtp_client = MockGtpInteractor::new();
        let mut public_gtp_client = MockGtpInteractor::new();

        public_gtp_client
            .expect_get_completion()
            .times(1)
            .with(eq("Call me Bob.  Hello".to_string()))
            .returning(|_| Ok("How are you?".to_string().into()));

        tg_client
            .expect_send_message()
            .times(1)
            .with(eq(0), eq("How are you?"), eq(Some("MarkdownV2")))
            .returning(|_, _, _| Ok(()));

        let bot = TgBot::new(
            public_gtp_client,
            private_gtp_client,
            tg_client,
            build_test_config(),
            || StepRng::new(0, 0),
        );
        let result = bot.process_message(*message).await;
        assert!(result.is_ok());
    }

    fn build_test_config() -> Config {
        Config::new(
            HashMap::from_iter(vec![("Sam".to_string(), "Bob".to_string())]),
            "Call me {}. ".to_string(),
            vec![
                "Dummy answer",
                "Another dummy answer",
                "Yet another dummy answer",
            ],
            vec![0],
            vec!["simple bot"],
        )
    }

    // Test when the message contains a photo
    #[tokio::test]
    async fn test_process_message_with_photo() {
        let mut tg_client = MockTelegramInteractor::new();
        let mut gtp_client = MockGtpInteractor::new();
        let public_gtp_client = MockGtpInteractor::new();

        tg_client
            .expect_get_file_url()
            .times(1)
            .with(eq("file_id"))
            .returning(|_| Ok("url".to_string()));

        gtp_client
            .expect_get_image_completion()
            .times(1)
            .with(eq("Что на картинке?".to_string()), eq("url".to_string()))
            .returning(|_, _| Ok("Red image".to_string().into()));

        tg_client
            .expect_send_message()
            .times(1)
            .with(eq(123), eq("Red image"), eq(Some("MarkdownV2")))
            .returning(|_, _, _| Ok(()));

        let bot = create_bot(tg_client, gtp_client, public_gtp_client);
        let message = create_private_message(
            None,
            Some(vec![PhotoSize {
                file_id: "file_id".to_string(),
                file_size: 1,
            }]),
        );
        let result = bot.process_message(message).await;
        assert!(result.is_ok());
    }

    // Test when the message contains a text with a URL
    #[tokio::test]
    async fn test_process_message_with_url() {
        let mut tg_client = MockTelegramInteractor::new();
        let gtp_client = MockGtpInteractor::new();
        let public_gtp_client = MockGtpInteractor::new();

        tg_client
            .expect_send_message()
            .with(eq(123), eq("Another dummy answer"), eq(Some("MarkdownV2")))
            .times(1)
            .returning(|_, _, _| Ok(()));

        let bot = create_bot(tg_client, gtp_client, public_gtp_client);
        let message = create_public_message(
            Some("https://example.com".to_string()),
            None,
        );
        let result = bot.process_message(message).await;
        assert!(result.is_ok());
    }

    // Test when the message contains a text with a bot name
    #[tokio::test]
    async fn test_process_message_with_bot_name() {
        let mut tg_client = MockTelegramInteractor::new();
        let gtp_client = MockGtpInteractor::new();
        let mut public_gtp_client = MockGtpInteractor::new();

        public_gtp_client
            .expect_get_completion()
            .with(eq("preamble Hello".to_string()))
            .times(1)
            .returning(|_| Ok("Hello Sir".to_string().into()));

        tg_client
            .expect_send_message()
            .with(eq(123), eq("Hello Sir"), eq(Some("MarkdownV2")))
            .times(1)
            .returning(|_, _, _| Ok(()));

        let bot = create_bot(tg_client, gtp_client, public_gtp_client);
        let message =
            create_public_message(Some("bot_name Hello".to_string()), None);
        let result = bot.process_message(message).await;
        assert!(result.is_ok());
    }

    // Test when the message contains a text with a draw command
    #[tokio::test]
    async fn test_process_message_with_draw_command() {
        let mut tg_client = MockTelegramInteractor::new();
        let mut gtp_client = MockGtpInteractor::new();
        let public_gtp_client = MockGtpInteractor::new();

        gtp_client
            .expect_get_image()
            .with(eq(" cat"))
            .times(1)
            .returning(|_| Ok("url".to_string().into()));

        tg_client
            .expect_send_image()
            .with(eq(123), eq("url"))
            .times(1)
            .returning(|_, _| Ok(()));

        let bot = create_bot(tg_client, gtp_client, public_gtp_client);
        let message =
            create_private_message(Some("нарисуй cat".to_string()), None);
        let result = bot.process_message(message).await;
        assert!(result.is_ok());
    }

    // Test when the message contains a text without a bot name or draw command
    #[tokio::test]
    async fn test_process_message_without_bot_name_or_draw_command() {
        let mut tg_client = MockTelegramInteractor::new();
        let gtp_client = MockGtpInteractor::new();
        let mut public_gtp_client = MockGtpInteractor::new();

        public_gtp_client
            .expect_get_completion()
            .with(eq("preamble Hello".to_string()))
            .times(1)
            .returning(|_| Ok("Hello Sir".to_string().into()));

        tg_client
            .expect_send_message()
            .with(eq(123), eq("Hello Sir"), eq(Some("MarkdownV2")))
            .times(1)
            .returning(|_, _, _| Ok(()));

        let bot = create_bot(tg_client, gtp_client, public_gtp_client);
        let message =
            create_public_message(Some("bot_name Hello".to_string()), None);
        let result = bot.process_message(message).await;
        assert!(result.is_ok());
    }

    fn build_private_message() -> Option<Box<Message>> {
        Some(Box::new(Message {
            message_id: 0,
            from: User {
                id: 0,
                is_bot: false,
                first_name: "Sam".to_string(),
                last_name: None,
                username: None,
                language_code: None,
            },
            chat: Chat {
                id: 0,
                first_name: None,
                last_name: None,
                username: None,
                chat_type: PRIVATE_CHAT.to_string(),
            },
            date: Default::default(),
            text: Some("Hello".to_string()),
            caption: None,
            photo: None,
            reply_to_message: None,
        }))
    }

    fn build_public_message() -> Option<Box<Message>> {
        Some(Box::new(Message {
            message_id: 0,
            from: User {
                id: 0,
                is_bot: false,
                first_name: "Sam".to_string(),
                last_name: None,
                username: None,
                language_code: None,
            },
            chat: Chat {
                id: 0,
                first_name: None,
                last_name: None,
                username: None,
                chat_type: "PUBLIC".to_string(),
            },
            date: Default::default(),
            text: Some("simple bot Hello".to_string()),
            caption: None,
            photo: None,
            reply_to_message: None,
        }))
    }

    fn create_bot(
        tg_client: MockTelegramInteractor,
        gtp_client: MockGtpInteractor,
        public_gtp_client: MockGtpInteractor,
    ) -> TgBot<MockTelegramInteractor, MockGtpInteractor, StepRng> {
        TgBot::new(
            public_gtp_client,
            gtp_client,
            tg_client,
            Config::new(
                HashMap::default(),
                "preamble".to_string(),
                vec![
                    "Dummy answer",
                    "Another dummy answer",
                    "Yet another dummy answer",
                ],
                vec![123],
                vec!["bot_name"],
            ),
            || StepRng::new(1000000000, 100000000),
        )
    }

    // Helper function to create a Message instance
    fn create_public_message(
        text: Option<String>,
        photo: Option<Vec<PhotoSize>>,
    ) -> Message {
        Message {
            message_id: 1,
            from: User {
                id: 1,
                is_bot: false,
                first_name: "Yury".to_string(),
                last_name: None,
                username: None,
                language_code: None,
            },
            chat: Chat {
                id: 123,
                first_name: None,
                last_name: None,
                username: None,
                chat_type: "public".to_string(),
            },
            date: Utc::now().naive_utc(),
            text,
            caption: None,
            photo,
            reply_to_message: None,
        }
    }

    fn create_private_message(
        text: Option<String>,
        photo: Option<Vec<PhotoSize>>,
    ) -> Message {
        Message {
            message_id: 1,
            from: User {
                id: 1,
                is_bot: false,
                first_name: "Yury".to_string(),
                last_name: None,
                username: None,
                language_code: None,
            },
            chat: Chat {
                id: 123,
                first_name: None,
                last_name: None,
                username: None,
                chat_type: PRIVATE_CHAT.to_string(),
            },
            date: Utc::now().naive_utc(),
            text,
            caption: None,
            photo,
            reply_to_message: None,
        }
    }
}
