# PMEMC Version 1 Specification

This document is the normative scope and behaviour contract for PMEMC (Project Memory CLI) Version 1.

## 1. Capability

Version 1 is a local Windows CLI that registers Git repositories, detects committed and uncommitted changes since an approved inspection baseline, constructs a compact structural code map, creates evidence-backed project-fact proposals through OpenRouter, and requires operator review before changing permanent project memory.

## 2. Fixed decisions

- Product name: PMEMC — Project Memory CLI
- Binary name: `pmemc`
- Implementation language: Rust
- Runtime surface: Native Windows 11 CLI used from PowerShell
- Operator model: One local user
- Repository count: Fewer than ten
- Source repositories: Local Git working trees
- Persistence: Local SQLite database plus human-readable exports where specified
- First model provider: OpenRouter
- Provider architecture: Replaceable provider interface
- Final fact authority: Operator
- Highest default technical evidence: Code and tests
- Uncommitted-work label: `in_progress`
- Trigger: Explicit operator command; no background monitoring

## 3. Trust boundaries and invariants

1. Registered repositories are read-only inspection targets.
2. Repository content is untrusted and is never executed.
3. Model output is untrusted and cannot directly mutate verified facts.
4. A verified fact changes only through an explicit operator review decision.
5. Baseline finalization occurs only after review completes.
6. Already verified memory survives partial failures.
7. Provider credentials never enter the database, logs, exports, or repository.
8. Every stored fact retains evidence provenance or an operator-confirmed marker.
9. Committed and in-progress evidence remain distinguishable.
10. Relationships that cannot be resolved reliably remain unresolved rather than guessed.

## 4. CLI interface

Version 1 owns only these user-facing commands:

```text
pmemc init
pmemc project add <path>
pmemc project list
pmemc project show <project-id>
pmemc status [project-id]
pmemc inspect <project-id>
pmemc review [project-id]
pmemc history <project-id>
```

Commands may gain flags needed to satisfy this specification, but agents must not add new top-level capabilities without an approved specification change.

### 4.1 `pmemc init`

Must:

- Create or migrate the local Version 1 database.
- Create required local cache/export directories.
- Be idempotent.
- Report the resolved local data location without exposing secrets.

Must not:

- Register a repository.
- Call a model provider.
- Modify Git configuration.

### 4.2 `pmemc project add <path>`

Must:

- Resolve and normalize a native Windows path.
- Verify that the path exists and is a Git working tree.
- Read the repository root, current branch when present, and HEAD commit when present.
- Create a stable project ID.
- Reject duplicate registrations of the same canonical repository path.
- Leave the project in `registered_needs_inspection` state.

Must not inspect source content or create verified project facts.

### 4.3 `pmemc project list`

Must show project ID, display name, canonical path, state, current branch if available, last approved inspection time, and whether changes are currently detected.

### 4.4 `pmemc project show <project-id>`

Must show registration details, baseline details, verified facts, unresolved questions, and counts of evidence, proposals, and decisions. It must not invoke OpenRouter.

### 4.5 `pmemc status [project-id]`

Must compare the current repository with its last approved baseline and report:

- Current branch and HEAD
- Commits since baseline
- Added, modified, deleted, copied, and renamed tracked files
- Staged changes
- Unstaged changes
- Untracked non-ignored files
- Whether an initial inspection is required

When no project ID is provided, it must summarize every registered project.

Status is read-only and must not change the baseline.

### 4.6 `pmemc inspect <project-id>`

Must:

1. Run status and display the inspection scope.
2. Ask the operator whether to inspect the reported files.
3. Stop without mutation if permission is denied.
4. Inventory allowed files while respecting `.gitignore` and `.pmemcignore`.
5. Build or update the compact code map.
6. Build a minimized evidence bundle from changed code, relevant structural neighbours, tests, manifests, and documentation.
7. Remove blocked secret files and reject suspected credentials from the provider bundle.
8. Send the approved minimized bundle to OpenRouter.
9. Validate the provider's structured response.
10. Store proposals and questions as pending review, without modifying verified facts or the approved baseline.

An initial inspection examines the repository broadly enough to establish project context. Later inspections prioritize changes since the baseline and their direct structural neighbours.

### 4.7 `pmemc review [project-id]`

For each pending proposal, the command must show:

- Proposed fact
- Proposed lifecycle state
- Confidence
- Evidence sources with paths, commit or working-tree state, and line/symbol positions when available
- Existing fact when a conflict exists
- Provider and model identifier

The operator can:

- Approve
- Correct and approve
- Reject with an optional reason
- Skip and leave pending

The tool must preserve every decision. Correcting a proposal must preserve both the original proposal and the corrected fact.

After all required proposals and conflicts are resolved, the command asks whether to finalize the inspection. Finalization atomically records the accepted facts, decisions, code-map snapshot, and new baseline.

### 4.8 `pmemc history <project-id>`

Must show an ordered audit trail of inspections, proposals, decisions, conflicts, and baseline changes without invoking a provider.

## 5. Project lifecycle

```text
unregistered
  -> registered_needs_inspection
  -> inspection_pending_review
  -> baselined
  -> changes_detected
  -> inspection_pending_review
  -> baselined
```

An inspection failure returns the project to its previous durable state. Pending proposals may remain recoverable, but the previous baseline and verified facts remain authoritative.

## 6. Git model

The Git adapter must use the installed Git executable and machine-readable output where available.

The approved baseline records:

- HEAD object ID, if the repository has commits
- Branch name or detached-HEAD state
- Relevant working-tree status
- Fingerprints for inspected uncommitted files
- Inspection timestamp

The database must not store complete repository copies. It stores evidence excerpts only when required for auditability and must minimize stored source content.

Version 1 supports normal working trees, empty repositories, detached HEAD, renamed files, deleted files, staged changes, unstaged changes, and untracked non-ignored files. Git submodules may be recorded as explicit dependencies but are not recursively registered or inspected automatically.

## 7. Compact code map

### 7.1 Purpose

The code map selects relevant context for project understanding. It is not a general code-intelligence platform and not a complete call graph.

### 7.2 Nodes

- Repository
- File
- Symbol: function, method, class, interface, trait, struct, enum, or module when supported

### 7.3 Relationships

- `contains`: repository to file, or type to member
- `defines`: file to symbol
- `imports`: file to resolvable local file/module
- `calls`: direct symbol call only when unambiguous
- `depends_on`: explicit cross-project/local dependency only

Every relationship records evidence and one confidence value:

- `exact`
- `inferred`
- `user_confirmed`

Ambiguous calls and references remain unresolved and are never converted into edges merely because names match.

### 7.4 Language support

Version 1 structural adapters cover:

- Rust
- Python
- Java
- Go
- JavaScript
- TypeScript
- JSX
- TSX

Tree-sitter grammars are used; custom language grammars are not Version 1 work.

Markdown, JSON, YAML, TOML, and notebook files use a generic text/metadata fallback. Unsupported or malformed files must not abort the inspection.

### 7.5 Exclusions

The inventory excludes Git-ignored content plus `.pmemcignore` patterns. Default safety exclusions include secret/environment files, VCS internals, dependency/vendor directories, build outputs, generated files, and binary content. The implementation must allow explicit safe overrides without allowing blocked credential patterns.

## 8. Evidence and facts

A fact contains:

- Stable fact ID
- Project ID
- Fact kind
- Human-readable statement
- Lifecycle state: `committed`, `in_progress`, or `user_confirmed`
- Creation and update timestamps
- Verification status
- Evidence references

An evidence reference contains, when available:

- Repository and project ID
- Relative path
- Commit object ID or working-tree marker
- Staged/unstaged/untracked state
- Line range or symbol ID
- Short excerpt or content fingerprint
- Evidence type
- Confidence

Facts must not contain invented metrics, results, ownership, team role, motivation, or architectural rationale. When those cannot be proven from repository evidence, the provider produces an operator question instead of a fact.

## 9. Conflict handling

A conflict exists when new evidence materially contradicts a verified fact or when two material sources disagree.

The review must show:

- Existing verified fact and its evidence
- New proposal and its evidence
- Source types and repository states
- Why the tool considers them inconsistent

Code and tests receive the strongest default technical weighting, but the tool cannot auto-resolve the conflict. The operator's decision creates a new decision record and either preserves, supersedes, or corrects the fact without deleting history.

## 10. Provider interface

The core depends on a narrow provider interface that accepts a versioned evidence bundle and returns schema-validated proposals and questions.

Version 1 includes:

- One OpenRouter production adapter
- One deterministic fake adapter for tests
- Configurable OpenRouter model identifier
- Provider/model/prompt-schema metadata recorded with proposals
- API key read from `OPENROUTER_API_KEY`

The provider must not receive blocked files, full Git history, or entire repositories by default. HTTP, authentication, rate-limit, timeout, invalid JSON, and schema-validation failures leave verified facts and baselines unchanged.

No other production provider is implemented in Version 1.

## 11. Local persistence

SQLite is the canonical store. The implementation must use transactions for review finalization and schema migrations for database changes.

Minimum logical records:

- Projects
- Inspection baselines and attempts
- Verified facts
- Evidence
- Proposals and questions
- Review decisions
- Conflicts
- Code-map snapshots or references
- Provider invocation metadata without credentials

Human-readable project summaries may be exported from verified memory. Exports are derived views, not the source of truth.

## 12. Security and privacy

- All canonical memory remains local.
- The tool never executes registered project code.
- OpenRouter receives only operator-approved, minimized, filtered evidence.
- `.env`, private keys, credentials, tokens, and known secret-file patterns are blocked.
- Logs redact authorization headers and suspected secret values.
- The tool does not alter source repositories, Git configuration, branches, index, commits, or remotes.
- SQL uses bound parameters.
- Paths are canonicalized and displayed before inspection.
- Provider keys are never printed.

## 13. Failure and recovery

- Git failure: report the command category and safe diagnostic; preserve state.
- Parser failure: record the file as unsupported/failed and continue with fallback where safe.
- Provider failure: retain an inspection attempt that can be retried; do not create verified facts.
- Validation failure: store no provider proposal as trusted; show the validation problem.
- Database failure before commit: roll back.
- Interrupted review: preserve pending decisions and resume later.
- Repository moved or missing: mark unavailable and ask the operator to resolve the path; do not silently register a replacement.

## 14. Non-functional requirements

- Deterministic scans of the same repository snapshot produce the same structural map before model interpretation.
- Normal read-only commands do not require network access.
- User-facing errors explain the failed operation and recovery action.
- The CLI works with Windows paths containing spaces and non-ASCII characters.
- Version 1 remains usable without Graphify, CodeGraph, Docker, WSL, a graph database, or an MCP host.
- The codebase must pass formatting, Clippy with warnings denied, and automated tests.

## 15. Acceptance criteria

Version 1 is complete only when automated tests and a native Windows pilot demonstrate all of the following:

1. Initialization is idempotent.
2. A repository path can be registered and duplicate canonical paths are rejected.
3. Initial status reports that inspection is required.
4. Status correctly distinguishes committed, staged, unstaged, and untracked changes.
5. Initial inspection creates proposals without creating verified facts.
6. Denying inspection sends no repository content to OpenRouter.
7. The compact map records files, definitions, imports, and only unambiguous direct calls for supported fixtures.
8. Unsupported files do not abort an inspection.
9. Secret fixtures never enter provider requests or logs.
10. Approve, correct, reject, and skip decisions behave as specified.
11. A conflict cannot be finalized without an operator decision.
12. Review finalization atomically changes facts and baseline.
13. Provider and database failures preserve the previous baseline and facts.
14. In-progress evidence remains labelled in progress.
15. New commits and later working-tree changes are detected relative to the approved baseline.
16. History shows original proposals, corrections, decisions, and evidence.
17. A fresh agent session can retrieve the same verified memory through the CLI.

## 16. Explicit non-goals

The following are prohibited Version 1 implementation work:

- Master or job-specific resume generation
- DOCX or PDF editing
- Portfolio JSON updates
- Project-documentation generation
- GUI or full-screen TUI
- MCP server, plugin, or agent skill
- Background watchers or scheduled jobs
- Cloud database, cloud sync, accounts, or collaboration
- Vector search, embeddings, or RAG
- Graph database
- Complete control-flow, data-flow, or multilingual call graph
- Runtime instrumentation or execution of registered code
- Automatic Git commits, pushes, branch creation, or repository edits
- Production providers other than OpenRouter
- Performance optimization without a measured Version 1 bottleneck

An agent must not add placeholder modules, unused interfaces, dependencies, database tables, CLI commands, or configuration for these non-goals.

## 17. Handoff

The capability is ready for direct implementation. Work must follow `docs/IMPLEMENTATION_PLAN_V1.md` one phase at a time. Each phase must satisfy its exit criteria before the next begins.
