# Provider-Neutral OpenAI-Compatible Diagnostics Design

## Goal

Keep Roven's provider boundary generic for any configured OpenAI-compatible Chat Completions endpoint and expose safe, useful errors when a provider rejects or interrupts a streamed response.

## Design

`OpenAiCompatibleProvider` remains the single transport adapter. The profile supplies the complete HTTPS endpoint, model ID, and credential; the adapter adds no vendor path, vendor request field, vendor retry policy, or vendor fallback. The wire request remains the existing OpenAI-compatible Chat Completions shape with streaming and tools.

Stream errors are normalized at the adapter boundary. The parser accepts a numeric or string `error.code`, reads the optional `error.message` and `error.metadata.error_type`, and produces one sanitized, bounded diagnostic. It never exposes authorization headers, API keys, prompts, request bodies, or raw provider response bodies. HTTP 429 continues to use the existing typed rate-limit error; in-stream errors remain generic because a provider may report rate limiting after HTTP 200.

Remaining OpenRouter-only wording is removed from source comments and user-facing documentation where it describes the generic provider mechanism. OpenRouter remains valid only as an example profile.

## Data flow

```text
profile endpoint/model/key
          |
          v
OpenAiCompatibleProvider -- standard Chat Completions request --> configured provider
          ^                                                      |
          |                                                      v
          +-- safe ProviderError <--- HTTP status or streamed error event
```

## Verification

- Unit-test standard request serialization and endpoint preservation.
- Unit-test streamed errors with numeric codes, string codes, metadata categories, and sensitive messages.
- Run provider tests, the full Rust test suite, formatting, and clippy.
- Build and install the tested Windows executable only after verification.

## Out of scope

- OpenAI Responses API support.
- Provider-specific adapters, retries, fallback routing, or model capability inference.
- Changes to the agent loop, tool execution, credentials storage format, or UI layout.
