# PMEMC — Project Memory CLI

PMEMC (`pmemc`) is a local, native Windows command-line tool that builds durable, evidence-backed memory about a developer's software projects.

The tool exists because project knowledge is currently scattered across source code, Git history, documentation, old resumes, portfolio records, AI-agent conversations, and the developer's memory. Every new coding-agent session reconstructs that context differently. This causes repeated explanations, inaccurate project claims, inconsistent writing, and wasted time.

`pmemc` makes project knowledge persistent. It inspects registered Git repositories, detects what changed since the previous approved inspection, builds a small structural code map, proposes new project facts, and requires the developer to approve, correct, or reject every fact before it becomes permanent.

## Current delivery target

- Version: **Version 1 only**
- Status: Specification locked; implementation not started
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

## Explicitly not in Version 1

Version 1 does not generate or edit resumes, portfolios, Word documents, project documentation, or websites. It does not include a GUI, TUI, MCP server, vector database, graph database, background watcher, scheduled job, cloud synchronization, Git commits, Git pushes, or a complete multilingual call graph.

Agents must not implement, scaffold, or add dependencies for these future capabilities during Version 1.

## Repository documents

- `PRODUCT.md` — why the product exists, product rules, trust model, and long-term context.
- `docs/V1_SPEC.md` — normative Version 1 requirements and acceptance criteria.
- `docs/IMPLEMENTATION_PLAN_V1.md` — ordered delivery plan and phase exit criteria.
- `AGENTS.md` — durable instructions for Codex and other coding agents.

If these documents conflict, `docs/V1_SPEC.md` is authoritative for Version 1 scope. Product truth still requires operator clarification; an agent must not silently invent or resolve a product contradiction.
