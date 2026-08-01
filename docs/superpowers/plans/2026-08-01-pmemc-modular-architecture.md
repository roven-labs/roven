# PMEMC Modular Architecture Refactor Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Split the `lib.rs` orchestration hotspot into responsibility-based modules without changing Version 1 behavior.

**Architecture:** Keep the existing concrete adapters. Move provider submission into an application module, baseline policy into a pure-ish policy module, and CLI workflows into command modules. Keep `lib.rs` as a small compatibility facade.

**Tech Stack:** Stable Rust, existing PMEMC dependencies only.

## Global constraints

- Preserve the Version 1 specification and current CLI output.
- Preserve `pmemc::submit_approved_bundle` for existing integration tests.
- Do not add speculative interfaces or dependencies.
- Do not change SQLite schema or migration behavior.
- Apply test-first extraction: move code mechanically, then prove behavior with the existing suite.

## Tasks

### Task 1: Extract baseline policy

- [x] Add `src/baseline.rs` with baseline status serialization, fingerprint collection, baseline comparison, status filtering, and path helpers.
- [x] Make the functions `pub(crate)` only where command modules require them.
- [x] Remove duplicate implementations from `lib.rs`.
- [x] Run baseline and status tests.

### Task 2: Extract provider application workflow

- [x] Add `src/application.rs` with `submit_approved_bundle`, `submit_staged_bundle`, and provider failure handling.
- [x] Keep the public function signature unchanged through `pub use application::submit_approved_bundle`.
- [x] Run provider and inspection tests.

### Task 3: Extract CLI command modules

- [x] Add `src/commands/mod.rs` for dispatch and shared project-ID parsing.
- [x] Move inspection flow to `src/commands/inspect.rs`.
- [x] Move review flow to `src/commands/review.rs`.
- [x] Move status rendering to `src/commands/status.rs`.
- [x] Move project/history commands to `src/commands/project.rs`.
- [x] Keep command output and error behavior unchanged.

### Task 4: Reduce and review the facade

- [x] Reduce `src/lib.rs` to module declarations, `run`, and compatibility exports.
- [x] Check module dependency direction and remove unused imports.
- [x] Run Ponytail review against the diff for unnecessary abstractions or duplication.

### Task 5: Verify

- [x] Run `cargo fmt --all --check`.
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --all-targets --all-features`.
- [x] Run `git diff --check` and inspect scope.
