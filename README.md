# GPT Telegram Bot

A Rust-based Telegram bot that integrates with OpenAI's GPT models to provide AI-powered conversations, image generation, and audio responses.

## Features

- Text completion using GPT models
- Handles both private chats and group mentions
- Image generation via DALL-E 3
- Voice message support with text-to-speech
- Smart context handling for conversations
- Support for different GPT models (regular and "smart" models)
- Filtering for allowed chat IDs
- Image understanding with GPT Vision

## Requirements

- Rust 1.70+ 
- Telegram Bot API token
- OpenAI API key

## Environment Variables

The following environment variables are required:

```
TG_TOKEN=your-telegram-bot-token
GPT_TOKEN=your-openai-api-key
GPT_MODEL=gpt-3.5-turbo
GPT_SMART_MODEL=gpt-4
BOT_ALIAS=your-bot-name
TG_ALLOW_CHATS=123456789,987654321
GPT_RULES=base-rules-for-gpt
GPT_PREAMBLE=interaction-preamble
DUMMY_ANSWERS=answer1,answer2
NAMES_MAP={"John":"Jane"}
```

Optional environment variables:
```
GPT_CHAT_URL=custom-openai-api-url
GPT_PRIVATE_CHAT_URL=custom-private-api-url
GPT_PRIVATE_MODEL=custom-private-model
GPT_PRIVATE_TOKEN=custom-private-token
HEARTBEAT_INTERVAL_SECONDS=30
VOICE=onyx
```

## Development

### Build and Run

```bash
# Build the project
cargo build

# Run in development mode
cargo run
```

### Testing

```bash
# Run all tests
cargo test

# Run specific test
cargo test gpt_client::tests::test_get_image_completion
```

## Deployment

This bot is designed to run as an AWS Lambda function. Deploy it using your preferred AWS deployment method.

For local development, a `message.json` file in the project root can be used to simulate Telegram updates.

## Architecture

The bot consists of several main components:

- `main.rs`: Entry point and Lambda handler
- `gpt_client.rs`: Handles communication with OpenAI API
- `tg_client.rs`: Manages Telegram API integration
- `message_processor.rs`: Core logic for processing messages
- `event_handler.rs`: Processes incoming webhook events

## Features in Detail

### Text Conversations

The bot processes text messages and responds using GPT models. It can:
- Respond to direct messages in private chats
- Respond when mentioned in group chats
- Continue conversations with context awareness

### Image Generation

Use the command "нарисуй" followed by a description to generate images with DALL-E.

### Image Understanding

Send an image with a question, and the bot will analyze the image and respond.

### Voice Responses

The bot can convert text responses to voice messages using OpenAI's text-to-speech API.

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the LICENSE file for details.