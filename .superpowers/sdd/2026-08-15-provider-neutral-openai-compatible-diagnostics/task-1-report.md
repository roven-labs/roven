# Task 1 Report

Date: 2026-08-15

## Scope

Implemented Task 1 from `task-1-brief.md` only.

## Changed Files

- `src/provider.rs`
- `.superpowers/sdd/2026-08-15-provider-neutral-openai-compatible-diagnostics/task-1-report.md`

## Changes Made

- Added the two brief-required streamed-error tests beside the existing streamed-error test.
- Kept `OpenAiCompatibleProvider`, `ProviderError`, and `parse_sse_line` signatures unchanged.
- Updated the streamed JSON error diagnostic path to:
  - accept displayable scalar `error.code` values, including numeric codes
  - optionally include `error.metadata.error_type` as `type=...`
  - continue excluding `error.message`
  - continue routing through `ProviderError::diagnostic`, which applies `sanitize_diagnostic`
- Updated the pre-existing streamed-error expectation to the normalized wording used by the new provider-neutral diagnostic.

## Commands Run

### 1. Focused tests before parser change

Command:

```powershell
cargo test provider::tests::provider_stream_errors --lib
```

Result: failed as expected

Output:

```text
running 3 tests
test provider::tests::provider_stream_errors_expose_a_code_without_logging_the_response_message ... ok
test provider::tests::provider_stream_errors_never_include_sensitive_messages ... ok
test provider::tests::provider_stream_errors_show_numeric_code_and_safe_category ... FAILED

failures:

---- provider::tests::provider_stream_errors_show_numeric_code_and_safe_category stdout ----

thread 'provider::tests::provider_stream_errors_show_numeric_code_and_safe_category' (22800) panicked at src\provider.rs:795:9:
assertion `left == right` failed
  left: "Provider stream failed (remote_error): provider reported a stream error code=unspecified"
 right: "Provider stream failed (remote_error): provider reported stream error code=429 type=rate_limit_exceeded"

failures:
    provider::tests::provider_stream_errors_show_numeric_code_and_safe_category

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 101 filtered out; finished in 0.00s
```

### 2. Focused tests after parser change

Command:

```powershell
cargo test provider::tests::provider_stream_errors --lib
```

Result: passed

Output:

```text
running 3 tests
test provider::tests::provider_stream_errors_show_numeric_code_and_safe_category ... ok
test provider::tests::provider_stream_errors_expose_a_code_without_logging_the_response_message ... ok
test provider::tests::provider_stream_errors_never_include_sensitive_messages ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 0.00s
```

### 3. Formatting

Command:

```powershell
cargo fmt --all -- src/provider.rs
```

Result: completed with no output

## Self-Review

- The diff stays inside the requested surface area.
- The parser change is provider-neutral because it only reads generic JSON scalar `error.code` values and an optional metadata category field without introducing vendor-specific branching.
- Sensitive `error.message` content is still not included in the final diagnostic.
- The new helper is intentionally narrow and only accepts scalar JSON values, leaving arrays/objects as `unspecified`.

## Concerns

- No functional concerns for Task 1.
- Only the focused streamed-error tests were run, per the brief. Broader regression coverage was not run in this task.
