# GPT Telegram Bot — Project Architecture

## Overview

An AWS Lambda-deployed Telegram bot that forwards user messages to OpenAI's GPT API and returns responses. Supports text completions, image generation, vision (photo analysis), and text-to-speech. The system is designed around trait abstractions to enable full unit-test coverage via mocking.

---

## System Architecture

```
AWS Lambda HTTP Event (Telegram webhook)
        │
        ▼
  function_handler  (main.rs)
        │
        ▼
  EventHandler::process_event  (TgBot in message_processor.rs)
        │
        ├── reject if message > 10 min old
        ├── reject if chat not in allow-list
        │
        ▼
  process_message  (concurrent via tokio::try_join!)
        ├── wait_loop: heartbeat "still thinking" messages
        └── process_message_internal
                ├── photo → GtpInteractor::get_image_completion
                └── text
                        ├── "подумай" keyword → GtpInteractor::get_smart_completion
                        └── default → GtpInteractor::get_completion
                           (model may issue `generate_image` tool call)
        │
        ▼
  TelegramInteractor  (tg_client.rs)
        ├── send_message  (MarkdownV2, auto-chunked)
        ├── send_image
        ├── send_voice
        └── leave_chat
```

---

## Components

### `main.rs` — Entry Point

- In **release** mode: registers `function_handler` as an AWS Lambda HTTP handler via `lambda_http::run`.
- In **debug** mode: loads `message.json` and calls `process_message` directly for local testing.
- Initializes structured logging: pretty (dev) or JSON (Lambda / CloudWatch).
- Builds `AppConfig` and delegates to `TgBot`.

---

### `config.rs` — Configuration & Factory

**`AppConfig`** loads all runtime configuration from environment variables via the `context_env!` macro (fails fast with the variable name as context if missing).

Key responsibilities:
- Reads tokens, models, allowed chat IDs, bot aliases, preamble templates, dummy answers, voice settings, and heartbeat intervals.
- Optionally fetches additional GPT base rules from S3 (`S3_RULES_URI`) and appends them to `GPT_RULES`.
- Constructs the fully-wired `TgBot` instance: two `GtpClient`s (public/private), one `TgClient`, and `Config`.

---

### `event_handler.rs` — Trait Definition

```rust
trait EventHandler {
    async fn process_event(&self, event: &Request) -> anyhow::Result<()>;
}
```

- A single-method trait with `#[cfg_attr(test, automock)]` generating `MockEventHandler` for unit tests.
- Implemented by `TgBot` in `message_processor.rs`.

---

### `message_processor.rs` — Core Bot Logic

**`Config`** — runtime behavior settings:
- Bot aliases (name prefixes that trigger a response in group chats)
- Per-user display name mappings
- Preamble template (rendered with display name via `dyn-fmt`)
- Dummy answers list
- Allowed chat IDs
- Heartbeat message interval

**`TgBot<TgClient, GtpClient, R: Rng>`** — generic over all I/O dependencies for testability:

| Method | Behavior |
|---|---|
| `process_event` | Parses Telegram `Update` JSON, rejects stale/unauthorized messages, delegates to `process_message` |
| `process_message` | Runs `wait_loop` and `process_message_internal` concurrently |
| `wait_loop` | Sends "still thinking" heartbeat in private chats; cancels when main processing completes |
| `process_message_internal` | Routes to photo or text handler |
| `process_text_update` | Ignores URL-only messages; checks `should_answer`; strips bot name prefix |
| `process_and_answer` | Routes text requests to GPT; selects fast vs. smart model |
| `process_text_message` | Handles `CompletionResult`: sends text or image to Telegram |
| `send_text_response` | In groups: randomly may leave (`num < 20`) or reply with voice (`num > 100`) |
| `gtp_client()` | Returns private `GtpClient` for DMs, public one for groups |
| `should_answer()` | Allow-list check AND (private chat OR name prefix OR bot reply) |

`contains_case_insensitive()` implements a KMP-based Unicode-aware case-insensitive substring search used for bot alias detection.

**`RequestError`** — `thiserror`-derived typed error for missing Telegram message fields.

---

### `gpt_client.rs` — OpenAI API Client

**`GtpInteractor` trait** (mockable via `mockall`):

| Method | Description |
|---|---|
| `get_completion` | Completion with fast model; returns `CompletionResult` (`Text` or tool-generated `Image`) |
| `get_smart_completion` | Completion with smart model; returns `CompletionResult` (`Text` or tool-generated `Image`) |
| `get_image_completion` | Vision: analyze a photo with fast model |
| `get_image_smart_completion` | Vision: analyze a photo with smart model |
| `get_audio` | TTS via `tts-1`, returns raw audio bytes |

**`GtpClient`** maintains per-user conversation history in a `DashMap<i64, Vec<Message>>` (keyed by Telegram chat ID), enabling multi-turn conversations. Each call appends the user message and the assistant reply to that history.

Image generation is now modeled as an OpenAI tool call (`generate_image`) inside chat completions. When the model requests that tool, `GtpClient` executes image generation through the images endpoint, returns image bytes to `message_processor`, and stores the generated image as multimodal content in the same chat history.

Key types:
- `Message` — role-tagged messages including `system`, `user`, `assistant` (text/tool-call), and `tool`
- `Value` — `Plain(String)` or `Complex(Vec<Content>)` (for vision)
- `Content` — `Text(String)` or `ImageUrl` (base64 data URI)
- `ModelMode` — `Fast` vs `Smart`
- `CompletionResult` — `Text` or `Image`

---

### `tg_client.rs` — Telegram Bot API Client

**`TelegramInteractor` trait** (mockable):

| Method | Description |
|---|---|
| `get_file_url` | Resolves a Telegram file ID to a download URL |
| `send_message` | Sends MarkdownV2-formatted text, auto-splits at 4096 chars |
| `send_image` | Sends a PNG image |
| `send_voice` | Sends an audio file as voice message |
| `leave_chat` | Makes the bot leave a group chat |

**`TgClient`** uses `reqwest_middleware` with exponential backoff retry (2–10s, 3 retries). `send_message` handles MarkdownV2 escaping via `escape_text()` (using a compile-time `phf` set of special characters) and splits oversized messages into safe chunks.

Key Telegram data types: `Update`, `Message`, `User`, `Chat`, `PhotoSize`, `FileMetadata`.

---

### `s3_client.rs` — S3 Rules Loader

- `parse_s3_uri(uri)` — parses `s3://bucket/key` format.
- `fetch_rules_from_s3(uri)` — fetches a UTF-8 text object from S3 using `aws-sdk-s3`.
- Called once at startup in `AppConfig::from_env()` if `S3_RULES_URI` is set; failures are non-fatal (falls back to `GPT_RULES` only).

---

## Trait and Dependency Injection Map

```
TgBot<TC, GC, R>
  │
  ├── TC: TelegramInteractor  →  TgClient  (prod)  /  MockTelegramInteractor  (test)
  ├── GC: GtpInteractor       →  GtpClient (prod)  /  MockGtpInteractor       (test)
  └── R:  rand::Rng           →  ThreadRng (prod)  /  StepRng                 (test)
```

Two `GtpClient` instances exist at runtime:
- **public** — used for group chats
- **private** — used for direct messages (separate conversation history)

---

## External Dependencies

| Crate | Purpose |
|---|---|
| `lambda_http` / `lambda_runtime` | AWS Lambda HTTP handler and runtime |
| `tokio` | Async runtime, `try_join!`, timers, channels |
| `reqwest` / `reqwest-middleware` / `reqwest-retry` | HTTP client with automatic retry and backoff |
| `serde` / `serde_json` | JSON serialization for all API types |
| `tracing` / `tracing-subscriber` | Structured logging (pretty in dev, JSON in Lambda) |
| `anyhow` / `thiserror` | Error handling — general errors and typed library errors |
| `dashmap` | Lock-free concurrent `HashMap` for per-user GPT conversation history |
| `base64` | Encoding image bytes for OpenAI vision API |
| `mockall` | Auto-generated trait mocks for unit testing |
| `wiremock` | HTTP mock server for integration tests |
| `rand` | Random decisions (voice reply, leave chat, dummy answers) |
| `chrono` | Timestamp parsing and message age validation |
| `phf` | Compile-time perfect hash set for MarkdownV2 escape chars |
| `dyn-fmt` | Runtime string formatting for preamble templates |
| `dotenvy` | `.env` file loading in debug mode |
| `derive_more` / `derive-new` | Boilerplate reduction via derive macros |
| `color-eyre` | Rich error reporting in dev mode |
| `aws-config` / `aws-sdk-s3` | Optional S3-based GPT rules loading at startup |
| `futures` | Async stream utilities |
