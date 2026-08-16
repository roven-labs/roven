# Task 3 Report

Date: 2026-08-16

## Scope completed

Added a focused startup provider-status helper and wired it into the existing trust/startup UI. The implementation reuses:

- `ProviderKind`
- `resolve_api_key`
- `ProviderProfiles`
- the current `AppState` and ratatui view flow

It does not change provider transports or `/model` switching behavior.

## What changed

### Startup provider-status helper

Added `src/ui/startup.rs` with:

- `StartupProviderStatus`
- `detect_provider_status(...)`
- `banner_lines(...)`

The helper classifies provider access into exactly four states:

- no provider access
- OpenRouter-only
- Ollama-only
- both configured

Access is counted per provider kind, not per profile count.

### Environment keys count as configured

Startup detection uses the existing `resolve_api_key` path, so:

- `OPENROUTER_API_KEY` counts as configured OpenRouter access
- `OLLAMA_API_KEY` counts as configured Ollama access

This works even when no keyring secret exists for the matching profile.

### Startup/trust UI wiring

Added one `AppState` field:

- `startup_provider_status: Option<StartupProviderStatus>`

The status is refreshed during terminal startup and rendered on the trust screen under a `PROVIDER ACCESS` section.

Banner output reports OpenRouter and Ollama Cloud independently as `configured` or `missing`.

When no provider access exists, the banner shows the exact next step:

```text
roven auth set
```

No secrets are shown.

## TDD notes

Started with failing tests for:

- the four trust-screen banner states
- missing Ollama environment-key coverage

Then implemented the smallest UI helper and startup refresh path needed to make them pass.

## Tests added

### Startup banner states

- `ui::view::tests::trust_screen_shows_no_provider_access_and_setup_step`
- `ui::view::tests::trust_screen_shows_openrouter_only_access`
- `ui::view::tests::trust_screen_shows_ollama_only_access`
- `ui::view::tests::trust_screen_shows_both_provider_access_states`

### Credential environment coverage

- `credentials::tests::ollama_environment_key_takes_precedence_over_the_stored_profile_key`

Existing OpenRouter env coverage was preserved:

- `credentials::tests::environment_key_takes_precedence_over_the_stored_profile_key`

## Verification

Focused tests:

```powershell
cargo test trust_screen_shows -- --nocapture
cargo test environment_key_takes_precedence_over_the_stored_profile_key -- --nocapture
cargo test ollama_environment_key_takes_precedence_over_the_stored_profile_key -- --nocapture
```

Full suite:

```powershell
cargo test -- --nocapture
```

Result: all tests passed.

## Files changed for Task 3

- `src/credentials.rs`
- `src/ui/mod.rs`
- `src/ui/startup.rs`
- `src/ui/state.rs`
- `src/ui/terminal.rs`
- `src/ui/view.rs`

## Notes / concerns

- The repository still contains unrelated dirty changes outside Task 3, including `src/tools.rs`. They were left untouched.
- The startup banner intentionally reflects only the trust/startup screen; it does not alter the existing trusted chat footer or `/model` flow.

## Fix round 1 — 2026-08-16

Reviewer findings addressed:

1. Startup status now recognizes provider-level environment access even when there is no saved profile for that provider.
2. Added real startup-status tests that exercise the refresh/detection path against an empty profile store with provider env vars set and restored under an env lock.

### What changed

- Added provider-level environment helpers in `src/credentials.rs`:
  - `provider_env_api_key(...)`
  - `has_provider_env_api_key(...)`
- Kept saved-profile runtime resolution behavior intact by reusing the same env lookup inside `resolve_api_key(...)`.
- Extended `src/ui/startup.rs::detect_provider_status(...)` to seed status from provider-level env access before merging saved-profile access.
- Added `refresh_startup_provider_status_from(...)` in `src/ui/terminal.rs` so tests can exercise the real refresh path with a temporary empty profile store.
- When the current-user profile store is unavailable, startup status still reflects env-only provider access instead of collapsing to `NoProviderAccess`.

### Tests added

- `ui::terminal::tests::startup_status_detects_openrouter_env_without_saved_profile`
- `ui::terminal::tests::startup_status_detects_ollama_env_without_saved_profile`
- `ui::terminal::tests::startup_status_detects_both_envs_without_saved_profiles`
- `ui::startup::tests::detects_provider_level_access_without_saved_profiles`

### Verification

Focused tests:

```powershell
cargo test startup_status_detects_openrouter_env_without_saved_profile -- --nocapture
cargo test detects_provider_level_access_without_saved_profiles -- --nocapture
cargo test startup_status_detects -- --nocapture
```

Full suite:

```powershell
cargo test -- --nocapture
```

Result: all tests passed.
