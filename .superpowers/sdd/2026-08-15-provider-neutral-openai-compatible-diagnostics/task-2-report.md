# Task 2 Report

Date: 2026-08-15

## Scope

Implemented only Task 2 from `task-2-brief.md`:

- Removed misleading OpenRouter-only wording from `src/credentials.rs`.
- Updated `docs/SETUP.md` only as needed to frame OpenRouter as one OpenAI-compatible endpoint example.
- Did not touch `src/provider.rs`.
- Preserved credential account names, storage behavior, commands, and unrelated dirty files.

## Changed Files

- `src/credentials.rs`
- `docs/SETUP.md`

## Diff Summary

### `src/credentials.rs`

- Changed the module comment from:
  - `OpenRouter API-key storage in Windows Credential Manager.`
- To:
  - `Operating-system credential-store support for named provider profiles.`

This keeps behavior unchanged and matches the existing provider-neutral account naming and OS keyring usage.

### `docs/SETUP.md`

- Changed the examples label from:
  - `Examples:`
- To:
  - `Examples of complete OpenAI-compatible endpoints (including OpenRouter):`

This keeps the existing commands and example endpoints intact while making OpenRouter explicitly one example, not the mechanism name.

## Commands Run And Output

### 1. Repo state

Command:

```powershell
git status --short
```

Relevant output before Task 2 changes:

```text
 M PRODUCT.md
 M docs/TOOLS.md
 M src/agent.rs
 M src/tools.rs
 M src/ui/state.rs
 M src/ui/terminal.rs
 M src/ui/transcript.rs
 M src/ui/view.rs
```

### 2. Targeted diff inspection

Command:

```powershell
git diff -- src/credentials.rs docs/SETUP.md
```

Output after edits:

```diff
diff --git a/docs/SETUP.md b/docs/SETUP.md
index 8a477e8..3150ba3 100644
--- a/docs/SETUP.md
+++ b/docs/SETUP.md
@@ -31,7 +31,7 @@ The endpoint must be HTTPS, must not contain credentials, query parameters, or
 fragments, and must be the complete endpoint. Roven does not append
 `/chat/completions` automatically.
 
-Examples:
+Examples of complete OpenAI-compatible endpoints (including OpenRouter):
 
 ```text
 https://api.groq.com/openai/v1/chat/completions
diff --git a/src/credentials.rs b/src/credentials.rs
index 33ae1e2..8080cd1 100644
--- a/src/credentials.rs
+++ b/src/credentials.rs
@@ -1,4 +1,4 @@
-//! OpenRouter API-key storage in Windows Credential Manager.
+//! Operating-system credential-store support for named provider profiles.
 
 use thiserror::Error;
```

### 3. Focused tests requested by the brief

Command from brief:

```powershell
cargo test provider::tests profiles::tests commands::auth::tests --lib
```

Output:

```text
error: unexpected argument 'profiles::tests' found

Usage: cargo.exe test [OPTIONS] [TESTNAME] [-- [ARGS]...]
```

Because `cargo test` accepts one filter at a time, I ran the three requested filters separately.

#### `provider::tests`

Command:

```powershell
cargo test provider::tests --lib
```

Output:

```text
running 13 tests
test provider::tests::provider_uses_the_configured_endpoint_without_appending_a_path ... ok
test provider::tests::rate_limits_are_actionable_and_include_the_server_retry_delay ... ok
test provider::tests::parser_returns_text_and_completion_events ... ok
test provider::tests::request_error_keeps_the_http_status_without_exposing_credentials ... ok
test provider::tests::provider_stream_errors_show_numeric_code_and_safe_category ... ok
test provider::tests::provider_stream_errors_expose_a_code_without_logging_the_response_message ... ok
test provider::tests::diagnostic_detail_removes_bearer_tokens_and_line_breaks ... ok
test provider::tests::provider_stream_errors_never_include_sensitive_messages ... ok
test provider::tests::streamed_data_errors_report_the_stage_that_failed ... ok
test provider::tests::request_errors_preserve_the_transport_failure_category ... ok
test provider::tests::request_uses_profile_model_and_standard_openai_fields ... ok
test provider::tests::tool_call_chunks_become_roven_tool_calls ... ok
test provider::tests::stream_reports_real_http_rate_limits_and_unexpected_eof ... ok

test result: ok. 13 passed; 0 failed; 0 ignored; 0 measured; 91 filtered out
```

#### `profiles::tests`

Command:

```powershell
cargo test profiles::tests --lib
```

Output:

```text
running 3 tests
test profiles::tests::rejects_unsafe_or_incomplete_endpoints ... ok
test profiles::tests::creates_a_named_profile_with_a_normalized_endpoint ... ok
test profiles::tests::profile_storage_rejects_invalid_input_and_tracks_default_removal ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out
```

#### `commands::auth::tests`

Command:

```powershell
cargo test commands::auth::tests --lib
```

Output:

```text
running 2 tests
test commands::auth::tests::list_shows_the_user_chosen_name_without_a_secret ... ok
test commands::auth::tests::selection_requires_an_existing_number ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 102 filtered out
```

## Self-Review

- Confirmed no behavior changes in credential storage, account naming, commands, or provider implementation.
- Confirmed `src/provider.rs` was not modified.
- Kept the doc change minimal and provider-neutral.
- Avoided touching unrelated dirty UI files already present in the worktree.

## Concerns

- The test command written in the brief is not valid `cargo test` syntax and had to be split into three separate invocations.
