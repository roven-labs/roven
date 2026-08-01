# PMEMC Project Forget Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a deliberately destructive `pmemc project forget <project>` command that deletes only one project's PMEMC memory after exact-name confirmation while leaving the source repository untouched.

**Architecture:** The CLI performs preview and exact-name confirmation, then calls one storage use case. SQLite deletes all project-owned records inside one transaction in foreign-key-safe order and removes the project registration last. No repository adapter is called, no source file is opened, and no other project's records are touched.

**Tech Stack:** Stable Rust 2024, clap derive, rusqlite bundled SQLite, existing `anyhow` application boundary, `thiserror` storage errors, PowerShell stdin/stdout.

## Global Constraints

- Version 1 remains a local Windows 11 PowerShell CLI.
- The repository is a read-only inspection target; `project forget` must never delete or modify repository files, Git state, credentials, or other projects.
- Destructive memory deletion requires an explicit exact display-name confirmation; mismatched or cancelled confirmation performs no mutation.
- Model output and existing verified memory are not silently replaced; deletion is explicit operator action and must be documented as irreversible.
- SQLite deletion and project-registration removal must be atomic and use bound parameters.
- Do not add a dependency, daemon, export format, background watcher, or future-version feature.
- Run `cargo fmt --all --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets --all-features`, and `git diff --check` before completion.

---

### Task 1: Define the command and storage result

**Files:**
- Modify: `src/cli.rs`
- Modify: `src/storage.rs`
- Test: `tests/registration.rs`

**Interfaces:**
- `ProjectCommand::Forget { project_id: String, confirm_name: Option<String> }`.
- `storage::forget_project(data_paths, project_id) -> Result<ForgetSummary, StorageError>`.
- `ForgetSummary` contains only non-secret counts for the confirmation/result output.

- [x] **Step 1: Write failing storage tests**

Create a temporary PMEMC database with two projects and project-owned attempts, provider invocations, proposals, questions, decisions, conflicts, evidence, facts, baselines, and code-map snapshots. Assert that forgetting one project removes all its rows, preserves the other project's rows, and leaves the repository directory and files unchanged.

- [x] **Step 2: Run the focused storage test and verify it fails**

Run: `cargo test --test registration forget_project -- --nocapture`

Expected: FAIL because the command and storage operation do not exist.

- [x] **Step 3: Implement the storage transaction**

Inside one SQLite transaction, delete project-owned rows in this order: `conflicts`, `fact_evidence`, `review_decisions`, `questions`, `proposals`, `provider_invocations`, `inspection_baselines`, `inspection_attempts`, `code_map_snapshots`, then `projects`. Use subqueries through `project_id` and bound parameters. Return the pre-delete counts in `ForgetSummary`.

- [x] **Step 4: Run the focused storage test and verify it passes**

Run: `cargo test --test registration forget_project -- --nocapture`

Expected: PASS, including the second-project and repository-preservation assertions.

### Task 2: Add the confirmation-gated CLI flow

**Files:**
- Modify: `src/commands/project.rs`
- Modify: `src/commands/mod.rs`
- Test: `tests/registration.rs`

**Interfaces:**
- `project forget` resolves display name or stable ID using the existing resolver.
- Interactive input confirms by exact display name; `--confirm-name <name>` supports noninteractive automation without weakening the exact-match requirement.

- [x] **Step 1: Write failing CLI tests**

Add cases for: help exposes `project forget`; wrong confirmation leaves all rows; cancellation leaves all rows; correct confirmation prints counts and repository-preservation text; correct `--confirm-name` works without stdin; forgetting an unknown project fails without mutation.

- [x] **Step 2: Run the focused CLI tests and verify they fail**

Run: `cargo test --test registration project_forget -- --nocapture`

Expected: FAIL because the subcommand and handler do not exist.

- [x] **Step 3: Implement preview and confirmation**

Show the project name, canonical repository path, counts to be removed, and the sentence `Repository files will not be changed.` Prompt `Type <display-name> to confirm:` unless `--confirm-name` is provided. Proceed only on an exact match; otherwise print cancellation and return success without calling the storage deletion use case.

- [x] **Step 4: Implement result and failure handling**

On success print that PMEMC memory and registration were forgotten, the repository was unchanged, and the project can be added again. On storage failure propagate the existing safe storage error; the transaction must roll back.

- [x] **Step 5: Run the focused CLI tests and verify they pass**

Run: `cargo test --test registration project_forget -- --nocapture`

Expected: PASS.

### Task 3: Update the V1 contract and documentation

**Files:**
- Modify: `docs/V1_SPEC.md`
- Modify: `README.md`

- [x] **Step 1: Document the explicit scope decision**

Add `pmemc project forget <project-id>` to the command surface and specify that it deletes only PMEMC records for the selected project after exact confirmation, never repository files or credentials. State that it is the one explicit exception to ordinary history retention because the operator requested irreversible memory deletion.

- [x] **Step 2: Document the user workflow**

Document:

```powershell
pmemc project forget Siftara
# type Siftara when prompted
pmemc project add "C:\Users\visha\AI AGENTS\Siftara"
pmemc inspect Siftara
```

Also document `--confirm-name Siftara` for noninteractive use.

### Task 4: Complete verification and safe installation

**Files:**
- Inspect: `src/cli.rs`, `src/commands/project.rs`, `src/storage.rs`, `tests/registration.rs`

- [x] **Step 1: Run complete verification**

Run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
git diff --check
```

- [x] **Step 2: Review destructive-action safety**

Confirm the diff contains no `std::fs::remove_file`, repository path deletion, Git mutation, credential deletion, unbound SQL, or global-database reset. Confirm cancellation and transaction-failure tests preserve all rows.

- [x] **Step 3: Rebuild the installed CLI**

Run `powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1` only after all checks pass, then verify `pmemc --help` contains `forget`.
