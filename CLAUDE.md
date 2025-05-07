# GPT Telegram Bot - Development Guidelines

## Build & Test Commands
- `cargo build` - Build the project
- `cargo run` - Run locally with debug features
- `cargo test` - Run all tests
- `cargo test <test_name>` - Run a specific test
- `cargo test <module>::tests::` - Run all tests in a specific module
- `cargo clippy` - Run linter
- `cargo fmt` - Format code

## Code Style Guidelines
- **Formatting**: 80-column max line width (defined in rustfmt.toml)
- **Error Handling**: Use `anyhow` for general errors, `thiserror` for library errors
- **Logging**: Use `tracing` macros (`error!`, `info!`, etc.) for structured logging
- **Testing**: Write unit tests in a `mod tests` block with appropriate mocks
- **Macros**: Use `context_env!` for environment variable loading
- **Async**: Use `tokio` runtime with `#[tokio::main]` and `async/await` patterns
- **Types**: Prefer strong typing with `derive_more` when appropriate
- **Abstractions**: Use traits (like `EventHandler`) for testable component interfaces
- **Naming**: Use snake_case for variables/functions, CamelCase for types/traits
- **Documentation**: Document public interfaces with appropriate comments

## Environment Setup
Required environment variables are loaded from `.env` file in debug mode:
- `TG_TOKEN` - Telegram Bot API token
- `GPT_TOKEN` - OpenAI API token
- `GPT_MODEL` - Default GPT model to use
- `GPT_SMART_MODEL` - Smart GPT model
- `BOT_ALIAS` - Comma-separated bot name aliases
- `TG_ALLOW_CHATS` - Comma-separated list of allowed chat IDs
- `GPT_RULES` - Base rules for GPT interactions
- `GPT_PREAMBLE` - Preamble for GPT requests
- `DUMMY_ANSWERS` - Comma-separated list of dummy answers