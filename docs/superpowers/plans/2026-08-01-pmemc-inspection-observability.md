# PMEMC Inspection Observability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Expose the existing inspection workflow as safe, colored, structured terminal progress with actionable provider-validation diagnostics.

**Architecture:** Add a presentation-only `InspectionReporter` in `src/output.rs`. The existing inspection coordinator emits events at real workflow boundaries; it does not move Git, storage, or provider logic into the reporter. Refine provider errors to carry bounded diagnostic reasons while retaining the existing durable failure category.

**Tech Stack:** Stable Rust 2024, standard-library terminal detection and ANSI escapes, existing `thiserror`, `serde_json`, and integration-test subprocesses.

## Global Constraints

- Version 1 remains a local Windows 11 PowerShell CLI.
- Registered repositories are read-only inspection targets and are never executed.
- Provider keys, source excerpts, full prompts, raw responses, and suspected secrets never enter progress output.
- Model output remains untrusted and cannot directly mutate verified facts or baselines.
- Do not add a production dependency, database migration, persistent log file, daemon, GUI, TUI, or provider tool loop.
- Run `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, and `git diff --check` before completion.

---

### Task 1: Add the terminal reporter contract

**Files:**
- Create: `src/output.rs`
- Modify: `src/lib.rs`
- Test: `src/output.rs` unit tests

**Interfaces:**
- Produces `InspectionReporter::new()`, `stage`, `detail`, `success`, `warning`, and `failure` methods for `commands::inspect`.
- `InspectionReporter` owns only output destination/style state; it does not accept secrets or repository content.

- [x] **Step 1: Write the failing tests**

Add unit tests for the formatting helpers: plain output contains no `\x1b`, styled output contains a reset code, and `NO_COLOR` disables styling.

- [x] **Step 2: Run the focused test and verify it fails**

Run: `cargo test output::tests --lib`

Expected: FAIL because `src/output.rs` does not exist.

- [x] **Step 3: Implement the minimal reporter**

Use `std::io::IsTerminal` and `std::env::var_os("NO_COLOR")`. Emit seven numbered stages with fixed labels. Use ANSI `32`, `36`, `33`, and `31` only when stdout is a terminal and `NO_COLOR` is absent. Keep all output plain when captured or redirected.

- [x] **Step 4: Run the focused test and verify it passes**

Run: `cargo test output::tests --lib`

Expected: PASS.

### Task 2: Add safe provider validation diagnostics

**Files:**
- Modify: `src/provider.rs`
- Test: `tests/provider.rs`

**Interfaces:**
- `ProviderError::InvalidResponse` carries a bounded safe reason.
- `ProviderError::failure_category()` continues to return `InvalidResponse`, so storage schema and retry behavior remain unchanged.

- [x] **Step 1: Write failing provider tests**

Cover invalid outer JSON, missing `choices[0].message.content`, invalid model JSON, unsupported schema version, unknown fields, invalid fact kind, duplicate evidence, evidence outside the bundle, and committed proposals citing non-committed evidence. Assert the display contains the category reason but not the supplied raw body.

- [x] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test --test provider provider_rejects -- --nocapture`

Expected: FAIL because the current error has no diagnostic reason.

- [x] **Step 3: Implement typed bounded reasons**

Introduce a small internal `InvalidResponseReason` enum or bounded reason constructor. Map each parse/validation branch to a stable message. Do not include response bodies, file contents, or unbounded provider text. Preserve the existing error category display and make `parse_openrouter_response` distinguish transport JSON from model-content JSON.

- [x] **Step 4: Run the focused tests and verify they pass**

Run: `cargo test --test provider -- --nocapture`

Expected: PASS.

### Task 3: Emit inspection progress at real boundaries

**Files:**
- Modify: `src/commands/inspect.rs`
- Modify: `src/lib.rs`
- Test: `tests/inspection.rs`

**Interfaces:**
- `commands::inspect::run` creates one reporter and emits progress without changing workflow order.
- Existing `application::submit_staged_bundle` remains the persistence/provider boundary; no new orchestration layer is introduced.

- [x] **Step 1: Write failing CLI-output tests**

Extend the existing subprocess inspection tests to assert stage labels, retry/bundle reuse details, pending-review completion guidance, and absence of ANSI escapes in captured output.

- [x] **Step 2: Run the focused tests and verify they fail**

Run: `cargo test --test inspection denied_inspection -- --nocapture`

Expected: FAIL because stage labels are not emitted.

- [x] **Step 3: Implement stage events**

Emit repository metadata and changed-path counts after Git reads. After package creation or retry selection, report file count, serialized bundle byte count, and redaction count. Report staging attempt ID, provider/model configuration without credentials, the provider wait state, response counts, pending-review storage, and the next command. On errors, emit a red failure line with the safe provider reason before returning the existing error.

- [x] **Step 4: Run the focused tests and verify they pass**

Run: `cargo test --test inspection -- --nocapture`

Expected: PASS.

### Task 4: Document and verify the complete change

**Files:**
- Modify: `README.md`
- Inspect: `docs/superpowers/specs/2026-08-01-pmemc-inspection-observability-design.md`

- [x] **Step 1: Document the terminal-only progress behavior**

Add a concise example showing inspection phases, colors, safe diagnostics, and the fact that PMEMC never prints raw prompts, responses, keys, or source excerpts.

- [x] **Step 2: Run the complete verification suite**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

Expected: all commands exit successfully with zero warnings and zero test failures.

- [x] **Step 3: Review scope and security**

Inspect `git diff` for new dependencies, raw response logging, secret exposure, changed review authority, database mutations, or future-version scaffolding. Remove any such change before reporting completion.
