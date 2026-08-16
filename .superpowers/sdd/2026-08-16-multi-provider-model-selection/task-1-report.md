# Task 1 Report

## Scope completed

Implemented Task 1 in the requested focused modules:

- `src/model_catalog.rs`
- `src/lib.rs`
- `src/credentials.rs`
- `src/openrouter.rs`
- `src/provider.rs`
- `src/ollama.rs`

Preserved the native Ollama transport path and did not touch `src/tools.rs`.

## Changed files

- `src/model_catalog.rs`
  - Added `ProviderKind::{OpenRouter, OllamaCloud}` endpoint classification.
  - Added focused `ModelCatalog` trait plus provider catalogs.
  - Added explicit Ollama allowlist validation.
  - Added `validate_model(endpoint, model_id)` helper.
- `src/lib.rs`
  - Registered the new `model_catalog` module.
- `src/credentials.rs`
  - Added `resolve_api_key(profile, store)` with environment-first lookup and safe fallback to the keyring store.
  - Added focused env-precedence test coverage.
- `src/openrouter.rs`
  - Switched metadata URL construction to `/api/v1/models/{author}/{slug}/endpoints`.
  - Added focused parsing for positive `data.endpoints[].context_length` values.
  - Added unit coverage for URL construction and endpoint-record parsing.
- `src/provider.rs`
  - Collapsed the OpenRouter usage parse branch at the reported warning site without changing stream behavior.
- `src/ollama.rs`
  - Reused the catalog validation helper for the native Ollama metadata lookup path.

## TDD record

### Failing test run before implementation

Command:

```powershell
cargo test model_catalog -- --nocapture
```

Observed failure:

```text
error[E0432]: unresolved imports `super::ModelCatalog`, `super::ProviderKind`, `super::catalog_for`
error[E0432]: unresolved imports `super::metadata_url`, `super::parse_context_window`
error[E0425]: cannot find function `resolve_api_key` in this scope
```

This was the expected missing-behavior checkpoint before implementation.

## Verification

Commands run:

```powershell
cargo test model_catalog -- --nocapture
cargo test openrouter -- --nocapture
cargo test environment_key_takes_precedence -- --nocapture
cargo fmt
cargo test
```

Key results:

- `cargo test model_catalog -- --nocapture`
  - Passed `classifies_known_provider_endpoints`
  - Passed `ollama_catalog_accepts_only_allowlisted_models`
- `cargo test openrouter -- --nocapture`
  - Passed the new URL-construction and `data.endpoints[]` parsing tests
  - Preserved the existing OpenRouter usage parsing test in `src/provider.rs`
- `cargo test environment_key_takes_precedence -- --nocapture`
  - Passed the new environment-over-keyring precedence test
- `cargo test`
  - Passed all library tests: `124 passed, 0 failed`
  - Passed CLI tests: `3 passed, 0 failed`
  - Passed doctests: `0 failed`

## Concerns

- The explicit Ollama allowlist is intentionally small and currently includes the repo-known models `minimax-m3:cloud` and `gemma4:31b-cloud`. If PMEMC expects a broader supported Ollama Cloud catalog, Task 2 should expand it from a canonical product list before the UI starts enforcing model switches against it.
- `resolve_api_key(profile, store)` is implemented and tested, but the current UI worker still reads the keyring directly because `src/ui/terminal.rs` is outside the Task 1 file list. The environment-first behavior is therefore available as the new credential interface, but not yet wired into the runtime path in this task.

## Fix Round 1

### Scope

Addressed both review findings without expanding into `/model` or startup-banner work:

- Wired `credentials::resolve_api_key` into the real runtime path in `src/ui/terminal.rs` with environment-first lookup and keyring fallback.
- Made OpenAI-compatible tool-call accumulation accept compact `data:` SSE prefixes in `src/provider.rs`.

### Files changed

- `src/ui/terminal.rs`
  - Added `resolve_profile_api_key(profile, store)` and used it from `spawn_worker`.
  - Added a focused regression test proving runtime env-key precedence and keyring fallback.
- `src/provider.rs`
  - Normalized `append_tool_call_deltas` to accept both `data:` and `data: ` prefixes.
  - Added a regression test proving compact tool-call chunks survive accumulation.

### TDD checkpoint before production edits

Command:

```powershell
cargo test runtime_key_resolution_prefers_environment_and_falls_back_to_store -- --exact --nocapture
```

Observed failure:

```text
error[E0425]: cannot find function `resolve_profile_api_key` in module `super`
```

This was the expected missing-runtime-wiring failure before the production edit.

### Verification

Commands run:

```powershell
cargo test runtime_key_resolution_prefers_environment_and_falls_back_to_store -- --nocapture
cargo test compact_tool_call_chunks_become_roven_tool_calls -- --nocapture
cargo test
```

Observed output summary:

- `cargo test runtime_key_resolution_prefers_environment_and_falls_back_to_store -- --nocapture`
  - `1 passed; 0 failed`
- `cargo test compact_tool_call_chunks_become_roven_tool_calls -- --nocapture`
  - `1 passed; 0 failed`
- `cargo test`
  - Library tests: `126 passed; 0 failed`
  - CLI tests: `3 passed; 0 failed`
  - Doctests: `0 failed`

### Fix-round concerns

- The earlier Task 1 concern about runtime env-key wiring is resolved by the `src/ui/terminal.rs` change in this fix round.
- The Ollama allowlist concern remains unchanged: it still intentionally includes only the repo-known models `minimax-m3:cloud` and `gemma4:31b-cloud`.
