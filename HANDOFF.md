# Roven V2 Implementation Handoff

Last updated: 2026-08-30

## Purpose

This file records the implementation state of the V2 project-registration
storage slice. Read it before changing project storage, registration, listing,
or resume evidence. It is the current-state handoff for new sessions and
agents; it is not a replacement for the architecture decision log or an
implementation plan.

- The architecture log records the intended product and design decisions.
- The Superpowers plan records the original work breakdown. It is ignored by
  Git and must not be committed.
- This file records what actually shipped, what differs from that plan, and
  what remains deliberately deferred.

## Current Git state

- Branch: `feat/v2-project-snapshots`
- Base branch: `v2`
- Version: `1.1.0`
- Implementation commit: `2f85db7`
- Documentation commit: `cc77f9c`
- Version commit: `9409bf8`
- The branch tracks `origin/feat/v2-project-snapshots`.
- No pull request was created because the available GitHub integrations lacked
  permission; the branch is ready at the PR URL for this feature branch.
- No V2 migration is included. The V1 storage behavior is intentionally not
  preserved because this feature had no existing users.

## Plan coverage

| Area | Status | What is true now |
| --- | --- | --- |
| `ProjectSnapshot` | Complete | Exact fields are `project_name`, `project_facts`, `user_context_facts`, and `user_contribution_facts`. All fact groups are flat `Vec<String>` values. |
| JSON validation | Complete | Snapshot and metadata JSON use strict unknown-field rejection and reject blank required strings/facts. Malformed or incomplete project data fails storage operations. |
| Deterministic identity | Complete | Project directories use SHA-256 of the canonical project path; the path is not stored in snapshot JSON. |
| Repository metadata | Complete | Rust discovers and validates the GitHub remote and `HEAD` baseline, then writes them to `repository_metadata.json`. |
| Immutable registration | Complete | A canonical path can be registered once, and duplicate project names are rejected. Existing data is never silently overwritten. |
| Storage layout | Complete | `%LOCALAPPDATA%\\Roven\\data\\projects\\<hash>\\project_snapshot.json` and `repository_metadata.json`. Existing atomic JSON writes remain in use. |
| Snapshot listing | Complete | `list_snapshots()` validates every project, rejects unsupported entries instead of returning a partial list, and sorts by `project_name`. |
| Registration preparation | Partial | `prepare_project` accepts the project name and all three fact arrays, performs Git/path checks, and writes only after validation. The surrounding agent-driven analysis/questionnaire flow remains outside Rust. |
| Resume evidence | Partial | Resume preparation loads V2 snapshots, but the provider prompt currently sends only `project_facts`. The saved context and contribution groups are not yet sent to the model. |
| Crash-safe folder commit | Deferred | Writes are still metadata first, snapshot second, with cleanup if the second write reports an error. A temporary-folder plus one-directory-rename transaction was not implemented. |

## Implemented behavior

### Storage

`ProjectRegistry` now owns only V2 project records:

- `register(project_root, snapshot, &metadata)` validates both values, checks
  duplicate names and the canonical-path directory, then writes the two JSON
  files.
- `read(project_root)` reads and validates both V2 files.
- `list_snapshots()` requires every entry below `projects/` to be a directory
  containing valid V2 snapshot and metadata files. It returns names in
  alphabetical order.
- `lookup(project_root)` identifies a registration from the deterministic
  canonical-path directory.
- Direct V1 JSON files, missing files, malformed JSON, wrong types, unknown
  fields, and duplicate names are storage failures.

The snapshot JSON is intentionally limited to:

```json
{
  "project_name": "PayFlow",
  "project_facts": [],
  "user_context_facts": [],
  "user_contribution_facts": []
}
```

Repository metadata is separate and Rust-generated:

```json
{
  "github_remote": "https://github.com/example/payflow.git",
  "baseline_commit": "<commit>"
}
```

### Registration

`prepare_project` requires `path` and `project_name`. The three fact arrays are
optional input and default to empty arrays. Registration is blocked before any
write when the path is invalid or outside the trusted workspace, Git is
unavailable, the directory is not a repository, `HEAD` is absent, no GitHub
remote exists, or the repository is not clean.

The successful tool response still exposes the project name, canonical path,
GitHub remote, and baseline commit. The remote and commit are not copied into
`project_snapshot.json`; they remain in `repository_metadata.json`.

### Listing and resume loading

`list_project` consumes `list_snapshots()` and returns only sorted project
names. Terminal resume preparation also consumes V2 snapshots and does not
reopen the repository or use workspace tools during generation.

## Deliberate deviations from the original plan

1. **This is the storage/integration slice, not the full MVP-0 architecture.**
   Rust does not yet invoke CodeGraph, run the five analysis queries, perform a
   synthesis call, or implement the full mandatory questionnaire orchestration.
   The existing registration prompt instructs the agent to collect the facts
   and pass them to `prepare_project`.
2. **V1 data is invalid, not migrated or ignored.** This is an explicit product
   decision. Do not add migration or compatibility readers unless the user
   changes that decision.
3. **Folder-level crash atomicity is deferred.** `AtomicWriteFile` protects
   each JSON write, but the pair is not committed with one directory rename.
   A process crash between the two writes can leave an incomplete project
   directory. This is the deferred save-integrity issue.
4. **Resume currently sends only `project_facts`.** The context and contribution
   arrays are stored and validated but are not yet included in the provider
   prompt. This was intentionally left for later.
5. **`register` borrows `RepositoryMetadata`.** The original plan shape could
   own metadata, but the implementation uses `&RepositoryMetadata` because the
   caller already owns the validated value and no clone is needed. Behavior is
   unchanged.
6. **Release numbering is `1.1.0`.** The user chose to treat V2 as a
   continuation because there were no V1 users requiring compatibility. Do not
   change this to `2.0.0` without revisiting that decision.

## Files that already implement this scope

- `src/storage.rs` — V2 models, validation, deterministic directories, reads,
  writes, duplicate checks, and storage tests.
- `src/tools/prepare_project.rs` — registration input/output, trusted-path
  checks, GitHub remote and baseline discovery, and registration tests.
- `src/tools/list_project.rs` — alphabetical V2 project-name listing.
- `src/ui/terminal.rs` — V2 snapshot loading for resume preparation.
- `prompts/register-project.md` — V2 fact collection instructions.
- `README.md`, `docs/TOOLS.md`, `docs/SETUP.md`, `docs/RELEASING.md`, and
  `CHANGELOG.md` — current V2 behavior and `1.1.0` release documentation.

## Verification already run

- `cargo fmt --all -- --check` — passed.
- `cargo check --all-targets` — passed.
- `cargo test --all-targets` — 174 unit tests and 3 CLI tests passed.
- `cargo clippy --all-targets -- -D warnings` — passed.
- `git diff --check` — passed.

## Next-agent rules

1. Start with this file and the current source files above; do not recreate the
   V2 storage layer or repeat the registration decisions.
2. Keep `project_snapshot.json` limited to its four fields. Put Git remote and
   baseline data only in `repository_metadata.json`.
3. Preserve write-once behavior, canonical-path hashing, strict validation,
   all-or-nothing listing, and the no-V1-migration decision.
4. When implementing a deferred item, update this file’s status and deviation
   section in the same change.
5. Keep architecture logs and Superpowers plans out of commits; both are
   ignored local working documents.
