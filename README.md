# GPT Telegram Bot

Rust Telegram bot for AWS Lambda with OpenAI chat/vision/image/voice support.

## Features

- Text chat with per-chat conversation history
- Group and private chat handling with allow-list checks
- Smart-model routing via the "подумай" keyword
- Image generation through model tool calls (`generate_image`)
- Vision support for photo messages
- Text-to-speech voice responses
- Optional heartbeat "still thinking" messages in private chats
- Local debug console mode (`--console`)

## Requirements

- Rust 1.70+
- Telegram bot token
- OpenAI API token

## Environment Variables

Required:

```env
TG_TOKEN=your-telegram-bot-token
GPT_TOKEN=your-openai-api-key
GPT_MODEL=gpt-4o-mini
GPT_SMART_MODEL=gpt-4o
BOT_ALIAS=botname1,botname2
TG_ALLOW_CHATS=123456789,987654321
GPT_RULES=base-rules-for-gpt
GPT_PREAMBLE=interaction-preamble-template
DUMMY_ANSWERS=answer1,answer2
NAMES_MAP={"john":"John"}
```

Optional:

```env
# OpenAI API base URL override
GPT_CHAT_URL=https://api.openai.com/v1

# OpenAI image generation model override (default: gpt-image-1)
GPT_IMAGE_MODEL=gpt-image-1

# Optional image size for image generation requests.
# Empty value means the size field is omitted from the request.
GPT_IMAGE_SIZE=1024x1024

# Optional image moderation for image generation requests.
# Empty value means the moderation field is omitted from the request.
GPT_IMAGE_MODERATION=low

# Private chat model API base URL/profile
GPT_PRIVATE_CHAT_URL=https://api.openai.com/v1
GPT_PRIVATE_MODEL=gpt-4o-mini
GPT_PRIVATE_TOKEN=private-openai-token
PRIVATE_GPT_RULES=private-chat-rules

# Startup rule enrichment from S3
S3_RULES_URI=s3://bucket/path/to/rules.txt

# Runtime behavior
HEARTBEAT_INTERVAL_SECONDS=30
VOICE=onyx
# Enable probabilistic voice replies in public chats (default: false)
VOICE_ENABLED=true
```

## Run

```bash
# build
cargo build

# run in debug mode using message.json fixture
cargo run

# run in interactive console mode (debug builds)
cargo run -- --console
```

In console mode, type a prompt and press Enter. Use `quit` or `exit` to stop.

## Test

```bash
cargo test
```

## Local Debug Modes

- `cargo run`: reads `message.json` and processes it once.
- `cargo run -- --console`: interactive stdin loop that simulates group-chat routing and prints outgoing bot messages to stdout.

Console image outputs are printed as `data:image/png;base64,...` URLs.

## Architecture

- `src/main.rs`: Lambda entrypoint + debug modes
- `src/config.rs`: env loading and bot wiring factory
- `src/message_processor.rs`: core routing and response flow
- `src/gpt_client.rs`: OpenAI client and conversation history
- `src/tg_client.rs`: Telegram HTTP client + console transport
- `src/event_handler.rs`: event handling trait abstraction
- `src/s3_client.rs`: optional S3 rules loader

## Deployment

Production target is AWS Lambda via webhook events from Telegram.
