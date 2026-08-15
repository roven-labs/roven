# Provider-Neutral OpenAI-Compatible Diagnostics Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the existing OpenAI-compatible provider boundary vendor-neutral and preserve the real safe reason for streamed provider failures.

**Architecture:** Keep `OpenAiCompatibleProvider` as the only transport adapter. It sends the existing standard Chat Completions request to the exact configured endpoint and normalizes HTTP/SSE failures into the existing `ProviderError` boundary without adding vendor-specific behavior.

**Tech Stack:** Rust 2024, serde/serde_json, thiserror, ureq, Cargo tests, Windows release build.

## Global Constraints

- The provider must use the profile-provided endpoint, model ID, and API key without OpenRouter-specific request branches.
- The adapter must not append a vendor path, add vendor-only request fields, retry automatically, or fall back to another provider.
- Diagnostics must not expose authorization headers, API keys, prompts, request bodies, or raw provider response bodies.
- Existing UI, agent orchestration, tool execution, credential storage format, and profile format remain unchanged.
- Do not modify unrelated dirty UI files: `PRODUCT.md`, `docs/TOOLS.md`, `src/agent.rs`, `src/tools.rs`, `src/ui/state.rs`, `src/ui/terminal.rs`, `src/ui/transcript.rs`, or `src/ui/view.rs`.

---

### Task 1: Normalize streamed OpenAI-compatible errors

**Files:**
- Modify: `src/provider.rs`
- Test: `src/provider.rs`

**Interfaces:**
- Keep `OpenAiCompatibleProvider`, `ProviderError`, and `parse_sse_line` signatures unchanged.
- Change only the diagnostic detail produced for a streamed JSON error event.

- [ ] **Step 1: Add failing tests for the standard streamed error shape**

Add tests beside the existing streamed-error test:

```rust
#[test]
fn provider_stream_errors_show_numeric_code_and_safe_category() {
    let error = parse_sse_line(
        r#"data: {"error":{"code":429,"message":"temporary failure","metadata":{"error_type":"rate_limit_exceeded"}}}"#,
    )
    .unwrap_err()
    .to_string();

    assert_eq!(
        error,
        "Provider stream failed (remote_error): provider reported stream error code=429 type=rate_limit_exceeded"
    );
}

#[test]
fn provider_stream_errors_never_include_sensitive_messages() {
    let error = parse_sse_line(
        r#"data: {"error":{"code":"invalid_request","message":"Bearer secret-value in prompt"}}"#,
    )
    .unwrap_err()
    .to_string();

    assert!(!error.contains("secret-value"));
    assert!(error.contains("code=invalid_request"));
}
```

- [ ] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test provider::tests::provider_stream_errors --lib`

Expected: the new numeric-code assertion fails because the current parser reports `code=unspecified` and omits the metadata category.

- [ ] **Step 3: Implement the smallest provider-neutral parser change**

Read `error.code` as a displayable JSON scalar, use `error.metadata.error_type` as an optional safe category, and include only those bounded fields in the diagnostic. Keep the existing `sanitize_diagnostic` path and do not include `error.message` in the displayed detail.

- [ ] **Step 4: Run the focused tests and verify they pass**

Run: `cargo test provider::tests::provider_stream_errors --lib`

Expected: PASS, including the existing string-code test and both new tests.

### Task 2: Remove misleading vendor-only wording

**Files:**
- Modify: `src/credentials.rs`
- Modify: `docs/SETUP.md`

**Interfaces:**
- No Rust API or storage behavior changes.
- Keep OpenRouter only as an explicit example endpoint, not as the provider mechanism name.

- [ ] **Step 1: Update the credential module documentation**

Change the module comment from OpenRouter-specific wording to an OS credential-store description for named provider profiles. Leave credential account names and storage behavior untouched.

- [ ] **Step 2: Update setup wording**

Keep the OpenRouter endpoint example, but label it as one OpenAI-compatible endpoint and describe HTTP 429 as a provider response. Do not add another provider-specific example or change setup commands.

- [ ] **Step 3: Run documentation-sensitive tests and inspect the diff**

Run: `cargo test provider::tests profiles::tests commands::auth::tests --lib`

Expected: PASS, with no changes outside `src/provider.rs`, `src/credentials.rs`, and `docs/SETUP.md` for implementation files.

### Task 3: Verify the provider-neutral boundary and installed executable

**Files:**
- Modify: only files required by compiler or formatter diagnostics from Tasks 1–2.

- [ ] **Step 1: Run formatting, tests, and lint**

Run: `cargo fmt --check; cargo test; cargo clippy -- -D warnings`

Expected: all commands exit 0.

- [ ] **Step 2: Build the Windows release executable**

Run: `cargo build --release`

Expected: `target\\release\\roven.exe` is produced.

- [ ] **Step 3: Install the verified executable**

Run: `Copy-Item -LiteralPath 'target\\release\\roven.exe' -Destination 'C:\\Users\\visha\\AppData\\Local\\Programs\\Roven\\roven.exe' -Force`

Expected: the installed executable is the verified release build.

## Self-review

- The plan changes one shared parser rather than adding provider-specific branches.
- Numeric and string streamed error codes are covered without storing raw provider messages.
- The current Chat Completions protocol remains the only supported protocol.
- UI, agent, tools, profiles, and credentials behavior remain out of scope.
