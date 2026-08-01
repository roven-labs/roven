# PMEMC Version 1 Implementation Plan

This plan breaks Version 1 into independently verifiable phases. It does not authorize work outside `docs/V1_SPEC.md`.

## Delivery rule

Only one phase is active at a time. Do not begin the next phase while the current phase has failing tests, unresolved contract questions, or incomplete exit criteria.

## Phase 0 — Repository foundation

Deliver:

- One Rust Cargo package with `src/lib.rs` and a thin `src/main.rs`.
- Root documentation from this agent pack.
- Initial modules with no speculative implementation.
- Test-support directory using temporary filesystem locations.
- CI-ready local verification commands documented in `AGENTS.md`.

Suggested initial source layout:

```text
src/
├── main.rs
├── lib.rs
├── cli.rs
├── domain.rs
├── git.rs
└── storage.rs
tests/
```

Do not create code-map or provider modules until their phases become active.

Exit criteria:

- `cargo fmt --all --check` passes.
- `cargo clippy --all-targets --all-features -- -D warnings` passes.
- `cargo test --all-targets --all-features` passes.
- `pmemc --help` identifies the program and current command surface without implementing future commands.

## Phase 1 — Initialization and local storage

Deliver:

- Domain types for project identity and lifecycle.
- Local data-directory resolution on Windows with test injection.
- SQLite connection and first migration.
- Idempotent `pmemc init`.
- Transaction and migration tests against temporary databases.

Exit criteria:

- Repeated initialization produces the same valid state.
- A failed migration does not leave a partially applied schema.
- Tests do not touch the operator's real application-data directory.

## Phase 2 — Repository registration and status vertical slice

Deliver:

- Git command runner with structured results.
- Canonical Windows repository-path handling.
- `pmemc project add`, `project list`, and `project show`.
- Duplicate-registration protection.
- `pmemc status` for initial, committed, staged, unstaged, untracked, renamed, and deleted states.
- Temporary Git-repository fixtures.

Exit criteria:

- The four foundation commands operate end to end through the real SQLite adapter and Git executable.
- Git errors are actionable and preserve database state.
- No source content is inspected during registration or status.

## Phase 3 — File inventory and compact code map

Deliver:

- `.gitignore` and `.pmemcignore` aware inventory.
- Default safety exclusions.
- Language detection.
- Tree-sitter extraction for Rust, Python, Java, Go, JavaScript, TypeScript, JSX, and TSX.
- Generic fallback for declared text formats.
- Normalized files, symbols, imports, and unambiguous direct calls.
- Deterministic map serialization and focused structural-neighbour queries.

Exit criteria:

- Fixture repositories produce deterministic expected maps.
- Ambiguous names do not produce call edges.
- Malformed and unsupported files do not abort a scan.
- No registered project code is executed.

## Phase 4 — Evidence construction and inspection staging

Deliver:

- Initial and incremental inspection-scope calculation.
- Interactive inspection approval.
- Changed-symbol and structural-neighbour selection.
- Test and manifest relevance selection.
- Secret-file blocking and suspected-secret redaction.
- Versioned, size-bounded evidence-bundle schema.
- Pending inspection-attempt persistence.

Exit criteria:

- Denial produces no source reads beyond Git metadata and no provider call.
- Approved bundles contain only expected fixture evidence.
- Secret fixtures never appear in serialized bundles or logs.

## Phase 5 — OpenRouter proposal adapter

Deliver:

- Narrow model-provider interface.
- Deterministic fake adapter for tests.
- OpenRouter HTTP adapter using `OPENROUTER_API_KEY`.
- Configurable model ID, timeout, and bounded retry policy.
- Versioned prompt and strict response schema.
- Proposal/question validation and provider-invocation metadata.

Exit criteria:

- Core tests run without network access through the fake adapter.
- Invalid, partial, rate-limited, timed-out, and unauthorized responses preserve verified data.
- API keys and authorization headers never appear in output or logs.

## Phase 6 — Review, conflicts, and baseline finalization

Deliver:

- `pmemc review` interactive workflow.
- Approve, correct-and-approve, reject, and skip decisions.
- Conflict detection against existing verified facts.
- Evidence presentation.
- Atomic fact, decision, code-map, and baseline finalization.
- Resumable interrupted reviews.
- `pmemc history`.

Exit criteria:

- Every review action has an automated test.
- Conflicts cannot auto-resolve.
- Corrections preserve original proposals.
- Failure before transaction commit preserves the old facts and baseline.

## Phase 7 — Native Windows pilot and Version 1 release gate

Deliver:

- Release build of `pmemc.exe`.
- Native PowerShell verification using one approved pilot repository.
- Verification of paths with spaces and non-ASCII characters.
- Initial inspection followed by committed and uncommitted change cycles.
- Recovery exercise for provider failure and interrupted review.
- Final audit against every acceptance criterion in `docs/V1_SPEC.md`.

Exit criteria:

- All specification acceptance criteria have recorded evidence.
- No explicit non-goal is present in the source, dependencies, database, or command surface.
- The operator can start a fresh agent session and retrieve the same verified project memory.
- Version 1 is tagged only after operator approval.

## First coding task

Start Phase 0 only:

1. Create the Cargo package.
2. Add the documentation pack.
3. Add `src/lib.rs` and a thin `src/main.rs`.
4. Implement only `pmemc --help` and the declared empty command structure needed for Phase 0.
5. Add tests for CLI help and version output.
6. Run formatting, Clippy, and tests.

Do not implement SQLite, Git inspection, code mapping, or OpenRouter during the first task.
