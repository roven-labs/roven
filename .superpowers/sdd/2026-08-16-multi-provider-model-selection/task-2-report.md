# Task 2 Report

Date: 2026-08-16

## Scope completed

Implemented the provider-first `/model` workflow inside the existing ratatui/crossterm UI. The change stays inside the current profile storage and terminal state machine and reuses:

- `crate::model_catalog::{ProviderKind, validate_model}`
- `crate::credentials::resolve_api_key`

No second CLI flow or separate preference store was added.

## What changed

### Existing UI flow

- Added a new in-UI model switch state to `AppState`.
- `/model` now opens a provider selection screen first.
- Provider selection shows:
  - provider name
  - current model
  - access state from `resolve_api_key`

### Model entry flow

- Enter on a provider opens model entry for that provider.
- The current model is prefilled.
- `Enter` on blank input cancels without changing anything.
- `Esc` cancels from provider selection or model entry without changing anything.
- Unsupported model IDs keep the dialog open and show a friendly provider-specific error.

### Persistence behavior

- Added `ProviderProfiles::update_model(id, model)` to update only the selected profile.
- Successful `/model` save:
  - validates the entered model with `validate_model`
  - updates only the selected profile’s model
  - sets that profile as default
  - refreshes the footer summary from the stored default profile

### Footer behavior

- The footer now shows `provider name · model id · context status`.
- It does not show raw token totals or secrets.
- It preserves the current Task 1 percentage-based context display.

### Ollama allowlist

- Expanded the explicit Ollama Cloud allowlist in `src/model_catalog.rs`.
- Unknown model IDs are still rejected.
- Added coverage for accepted current cloud IDs and rejection of unknown IDs.

## TDD notes

Started with failing Task 2 tests that referenced the missing `/model` selection state, profile update path, footer shape, and expanded Ollama allowlist. After that:

1. Added the minimal profile update API.
2. Added the minimal UI state machine for provider selection and model entry.
3. Wired `/model` into the existing terminal loop.
4. Updated the footer summary.
5. Expanded the Ollama allowlist.

## Tests added

- `profiles::tests::update_model_only_changes_the_selected_profile`
- `model_catalog::tests::ollama_catalog_accepts_only_allowlisted_models`
- `ui::view::tests::model_picker_lists_provider_access_without_showing_secrets`
- `ui::view::tests::footer_uses_only_the_configured_model_and_context_percent`
- `ui::terminal::tests::model_switch_changes_the_selected_provider_and_model`
- `ui::terminal::tests::blank_model_entry_cancels_without_changing_the_current_selection`
- `ui::terminal::tests::escape_cancels_provider_selection_without_changing_the_current_selection`
- `ui::terminal::tests::unsupported_model_keeps_the_previous_selection_and_shows_a_friendly_error`

## Verification

Ran:

```powershell
cargo test -- --nocapture
```

Result: all tests passed.

## Files changed for Task 2

- `src/model_catalog.rs`
- `src/profiles.rs`
- `src/ui/state.rs`
- `src/ui/terminal.rs`
- `src/ui/view.rs`

## Notes / concerns

- The repository already had unrelated dirty changes outside Task 2, including `src/tools.rs` and documentation/context files. They were left untouched.
- `src/ui/state.rs` and `src/ui/view.rs` already contained uncommitted Task 1-era context footer changes in the worktree; Task 2 builds on those current on-disk files rather than reverting or splitting them.

## Fix round 1

Date: 2026-08-16

Reviewer note addressed: the `/model` save path no longer performs separate `update_model` and `set_default` writes.

### What changed

- Replaced the split profile writes with `ProviderProfiles::switch_model_and_default(id, model)`.
- The new operation:
  - reads one profile document
  - updates the selected profile model
  - updates `default_profile_id`
  - commits once through the existing atomic file writer
- Updated the `/model` save path in `src/ui/terminal.rs` to use that single operation.

### Regression coverage

- Replaced the old profile update test with:
  - `profiles::tests::switch_model_and_default_updates_both_fields_in_one_operation`
- Re-ran the existing `/model` behavior tests to preserve:
  - successful switching
  - blank cancel
  - Escape cancel
  - unsupported-model errors preserving prior selection

### Verification

Focused tests:

```powershell
cargo test switch_model_and_default_updates_both_fields_in_one_operation -- --nocapture
cargo test ui::terminal::tests:: -- --nocapture
```

Full suite:

```powershell
cargo test -- --nocapture
```

Result: all tests passed.
