# PMEMC — Project Memory CLI

PMEMC (`pmemc`) is a local, native Windows command-line tool that builds durable, evidence-backed memory about a developer's software projects.

The tool exists because project knowledge is currently scattered across source code, Git history, documentation, old resumes, portfolio records, AI-agent conversations, and the developer's memory. Every new coding-agent session reconstructs that context differently. This causes repeated explanations, inaccurate project claims, inconsistent writing, and wasted time.

`pmemc` makes project knowledge persistent. It inspects registered Git repositories, detects what changed since the previous approved inspection, builds a small structural code map, proposes new project facts, and requires the developer to approve, correct, or reject every fact before it becomes permanent.

## Current delivery target

- Version: **Version 1 only**
- Status: Version 1 implementation in progress; secure local provider credentials supported
- Language: Rust
- Platform: Native Windows 11 and PowerShell
- Operator: One local user
- Repository scale: Fewer than ten registered repositories
- Storage: Local only
- Model provider in Version 1: OpenRouter
- Product name: **PMEMC — Project Memory CLI**
- Binary name: `pmemc`

## Version 1 outcome

After Version 1 is complete, the operator can:

1. Register local Git repositories.
2. Establish an approved inspection baseline for each repository.
3. Detect committed and uncommitted changes since that baseline.
4. Approve whether changed files should be inspected.
5. Build a compact map of files, important symbols, imports, and only reliably resolved direct calls.
6. Receive evidence-backed project-fact proposals from OpenRouter.
7. Review conflicts between new evidence and existing verified facts.
8. Approve, correct, reject, or skip each proposal.
9. Preserve the resulting verified project memory and decision history locally.

The codebase remains the strongest technical evidence, but it is not the final authority. When sources conflict, the tool shows the conflict and the operator makes the final decision.

## Provider credentials

On a fresh installation, run `pmemc init`. It initializes local storage, uses
OpenRouter's `openrouter/free` router by default, and offers to configure the
key when an interactive terminal is available. In scripts or other
non-interactive shells, it never waits for input; run `pmemc auth set` once in
an interactive PowerShell session if inspection needs the provider.

For local Windows use, `pmemc auth set` stores the OpenRouter key in the
operating-system credential store and never prints it. `pmemc auth status`
reports only whether it is configured, and `pmemc auth remove` deletes it. CI
environments may use `OPENROUTER_API_KEY` as a runtime fallback.

The free router may select a different free model for each request. PMEMC
records the model returned by OpenRouter with the provider invocation. Advanced
users may override the router with `PMEMC_OPENROUTER_MODEL`; this is optional.

## Install the CLI globally

From this repository, run:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

The script builds the release binary, installs it under the current user's
`%LOCALAPPDATA%\Programs\PMEMC`, and adds that directory to the user-level
`PATH`. Open a new PowerShell session, then `pmemc --help` works from any
project directory. Re-run the script to upgrade; uninstall with:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1 -Uninstall
```

To start one project from a clean PMEMC state, forget only its local PMEMC
memory and registration:

```powershell
pmemc project forget Siftara
# type Siftara when prompted
pmemc project add "C:\Users\visha\AI AGENTS\Siftara"
pmemc inspect Siftara
```

The command shows a deletion preview and never changes repository files, Git
state, credentials, or other projects. For a controlled non-interactive call,
provide the exact display name with `--confirm-name Siftara`.

## Version 1 workflow

```text
Register repository
        ↓
Detect initial or incremental Git state
        ↓
Ask permission to inspect relevant changes
        ↓
Build/update compact structural code map
        ↓
Create evidence bundle
        ↓
OpenRouter proposes project facts
        ↓
Operator approves, corrects, rejects, or skips
        ↓
Finalize verified facts and new inspection baseline
```

During `pmemc inspect`, the CLI shows these boundaries as numbered terminal
stages: repository check, evidence preparation, local staging, OpenRouter
request, response validation, SQLite storage, and completion. Interactive
PowerShell sessions use green, cyan, yellow, and red status markers; redirected
output is plain text. Progress is terminal-only and never prints keys, source
excerpts, full prompts, or raw provider responses. A provider validation error
reports a safe reason and keeps the retained attempt retryable.

When a repository is registered, PMEMC uses its directory name as the
user-facing project name and retains a stable numeric identifier for backward
compatibility. Commands such as `status`, `inspect`, `review`, and `history`
accept either value:

```powershell
pmemc inspect medlink-ddi
# equivalent: pmemc inspect project-2
```

## Explicitly not in Version 1

Version 1 does not generate or edit resumes, portfolios, Word documents, project documentation, or websites. It does not include a GUI, TUI, MCP server, vector database, graph database, background watcher, scheduled job, cloud synchronization, Git commits, Git pushes, or a complete multilingual call graph.

Agents must not implement, scaffold, or add dependencies for these future capabilities during Version 1.

## Repository documents

- `PRODUCT.md` — why the product exists, product rules, trust model, and long-term context.
- `docs/V1_SPEC.md` — normative Version 1 requirements and acceptance criteria.
- `docs/IMPLEMENTATION_PLAN_V1.md` — ordered delivery plan and phase exit criteria.
- `AGENTS.md` — durable instructions for Codex and other coding agents.

If these documents conflict, `docs/V1_SPEC.md` is authoritative for Version 1 scope. Product truth still requires operator clarification; an agent must not silently invent or resolve a product contradiction.
