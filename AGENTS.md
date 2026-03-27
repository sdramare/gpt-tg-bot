# Project Guidelines

## Scope

These instructions apply across the entire repository.
Use this file as the default behavior guide for coding tasks.

## Build And Test

- Build: `cargo build`
- Run tests: `cargo test`
- Run a single test: `cargo test <test_name>`
- Run module tests: `cargo test <module>::tests::`
- Lint: `cargo clippy`
- Format: `cargo fmt`
- Local debug run with fixture: `cargo run`
- Local interactive console: `cargo run -- --console`

If a change touches runtime behavior, run at least the closest module tests.
If a change touches shared logic (`src/message_processor.rs`, `src/gpt_client.rs`,
`src/tg_client.rs`), prefer running the full test suite.

## Architecture

The bot is an AWS Lambda Telegram webhook handler with trait-based dependency
injection for testability.

Core boundaries:

- `src/main.rs`: Lambda/debug entrypoints and logging setup
- `src/config.rs`: environment loading and runtime wiring factories
- `src/message_processor.rs`: routing, heartbeats, authorization, and response flow
- `src/gpt_client.rs`: OpenAI chat/vision/image/TTS interactions and history state
- `src/tg_client.rs`: Telegram API transport and MarkdownV2-safe output
- `src/event_handler.rs`: event handler trait abstraction
- `src/s3_client.rs`: optional S3 rules loader

Runtime notes that affect implementation choices:

- Two GPT clients are used at runtime: public (groups) and private (DMs).
- Message processing runs concurrently with a heartbeat loop.
- Image generation can be returned through completion tool calls.

## Conventions

- Keep lines within the rustfmt width limit configured in `rustfmt.toml`.
- Use `anyhow` for app-level errors and `thiserror` for typed domain errors.
- Use `tracing` for logs (`info!`, `warn!`, `error!`) instead of `println!`.
- Keep business logic behind traits where practical for mock-based tests.
- Prefer small, focused changes; avoid broad refactors unless required.

Testing conventions:

- Place unit tests in `mod tests` blocks in the same file.
- Use `#[tokio::test]` for async paths.
- Use `mockall` for trait mocks and `wiremock` for HTTP behavior tests.

## Project Gotchas

- Startup fails fast when required env vars are missing (`context_env!`).
- `TG_ALLOW_CHATS` must be a valid comma-separated list of i64 values.
- `S3_RULES_URI` is optional, and S3 fetch failures are non-fatal.
- Telegram messages use MarkdownV2 escaping and size-safe chunking.
- In production, avoid changing Lambda architecture assumptions without
  checking infrastructure settings.

## Link, Do Not Embed

For detailed architecture and environment examples, refer to:

- `README.md`
- `CLAUDE.md`
- `Taskfile.yml`
- `src/` module docs and tests

When adding new long-form guidance, prefer updating those docs and keeping this
file concise.
