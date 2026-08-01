# PMEMC Version 1 Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the existing PMEMC Version 1 trust, baseline, provenance, safety, and recovery defects without expanding product scope.

**Architecture:** Keep the current library crate and adapters, but centralize validation at the provider/storage boundary and add focused pure helpers for baseline comparison, conflict classification, and repository path containment. The CLI and SQLite adapter remain the integration shell; domain invariants are enforced before persistence and during finalization.

**Tech Stack:** Stable Rust, rusqlite with bundled SQLite, serde/serde_json, thiserror/anyhow, installed Git executable, Tree-sitter, ureq/OpenRouter.

## Global Constraints

- Work only on Version 1 requirements.
- Registered repositories are read-only inspection targets.
- Repository content is untrusted and is never executed.
- Model output is untrusted and cannot directly mutate verified facts.
- Committed and in-progress evidence remain distinguishable.
- Provider credentials never enter the database, logs, exports, or repository.
- Use temporary repositories and databases in tests.
- Do not add production dependencies or async Rust.
- Run `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-targets --all-features` before completion.

---

### Task 1: Canonical proposal validation

**Files:**
- Modify: `src/provider.rs`
- Modify: `src/storage.rs`
- Test: `tests/provider.rs`

**Interfaces:**
- `provider::validate_response` continues validating schema and evidence paths.
- Add a validation step that receives `&EvidenceBundle` and rejects a `committed` proposal when any cited file is not `EvidenceState::Committed`.
- Align finalization acceptance with `ProposedConfidence::{Exact, Inferred, UserConfirmed}`.

- [x] Write failing tests for committed lifecycle on unstaged evidence and for inferred/user-confirmed finalization.
- [x] Run the focused provider/inspection tests and confirm the new assertions fail for the expected invariant.
- [x] Implement the minimal validation and confidence alignment.
- [x] Run focused tests and confirm they pass.
- [x] Run Clippy for the changed code.

### Task 2: Correct evidence provenance

**Files:**
- Modify: `src/storage.rs`
- Test: `tests/inspection.rs`

**Interfaces:**
- `insert_fact_evidence` must receive `None` for `repository_commit` when the evidence state is staged, unstaged, staged-and-unstaged, or untracked.
- The saved `working_tree_state` and excerpt remain unchanged.

- [x] Write a failing finalization test that inspects unstaged evidence and asserts the fact evidence commit is null.
- [x] Run the focused test and confirm it fails because HEAD is currently stored.
- [x] Implement state-aware commit provenance.
- [x] Run the focused test and the existing finalization tests.

### Task 3: Baseline comparison

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/git.rs` only if snapshot data needs a typed helper
- Test: `tests/inspection.rs`
- Test: `tests/registration.rs`

**Interfaces:**
- Add a pure comparator for saved baseline status/fingerprints versus the current Git snapshot.
- Preserve current CLI output labels, but report only changes relative to the approved baseline.
- Include files that were changed after baseline and exclude files whose dirty state was already approved.

- [x] Write failing tests for an unchanged dirty baseline and a later content change.
- [x] Run them and confirm the current implementation reports the wrong result.
- [x] Implement deterministic status/fingerprint comparison.
- [x] Run focused tests and then all status/inspection tests.

### Task 4: Conservative conflict detection

**Files:**
- Modify: `src/storage.rs`
- Test: `tests/provider.rs`
- Test: `tests/inspection.rs`

**Interfaces:**
- Replace the current negation-only decision with a conservative same-fact-kind/material-evidence check.
- Preserve pending conflict records and require an operator resolution before finalization.

- [x] Write a failing test for positive contradiction such as “uses SQLite” versus “uses PostgreSQL”.
- [x] Run it and confirm no conflict is currently created.
- [x] Implement conservative conflict creation without deleting existing history.
- [x] Run conflict and finalization tests.

### Task 5: Safe repository path reads

**Files:**
- Modify: `src/inventory.rs`
- Modify: `src/code_map.rs`
- Modify: `src/inspection.rs`
- Test: `tests/inventory.rs`
- Test: `tests/code_map.rs`

**Interfaces:**
- Add a repository-root containment helper that canonicalizes a candidate path and returns `None` when it escapes the repository root.
- Apply it before binary detection, parsing, and evidence reads.
- Keep blocked credential paths blocked even when `.pmemcignore` contains a safe override.

- [x] Write a failing symlink-escape fixture test where supported by the Windows test environment.
- [x] Run it and confirm the external target is currently eligible for reading.
- [x] Implement containment checks and safe unsupported-path handling.
- [x] Run inventory, code-map, and inspection tests.

### Task 6: Recover interrupted provider attempts

**Files:**
- Modify: `src/storage.rs`
- Modify: `src/lib.rs`
- Test: `tests/inspection.rs`

**Interfaces:**
- Treat both `provider_failed` and `staged_pending_provider` as recoverable attempts.
- Prevent staging a second attempt while either recoverable attempt exists.
- Preserve prior lifecycle and all existing audit records.

- [x] Write a failing recovery test using a staged provider attempt.
- [x] Run it and confirm a second inspection can currently bypass recovery.
- [x] Implement recovery lookup and retry transition.
- [x] Run focused recovery tests.

### Task 7: Full verification and Rust review

**Files:**
- Review all changed files.
- Modify documentation only if behavior or current implementation status changed.

- [x] Run `cargo fmt --all --check`.
- [x] Run `cargo clippy --all-targets --all-features -- -D warnings`.
- [x] Run `cargo test --all-targets --all-features`.
- [x] Run the Rust review checklist for ownership, error handling, unsafe usage, SQL parameterization, command invocation, and secret handling.
- [x] Inspect `git diff` and confirm no future-version scope was added.
- [x] Report remaining blockers without claiming completion unless every verification command passes.

## Known documentation conflict

`README.md` and `docs/IMPLEMENTATION_PLAN_V1.md` still describe the repository as Phase 0 even though later Version 1 modules exist. This plan treats the existing implementation as the current hardening target and does not rewrite the normative specification.
