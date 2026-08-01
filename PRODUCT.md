# PMEMC Product Context

## Capability

PMEMC (Project Memory CLI) gives one developer a durable, local, evidence-backed record of what was actually built across their software projects. The record survives individual AI-agent sessions and becomes the trusted source that future resume, portfolio, and documentation workflows can consume.

## Problem

The operator is a student developer who builds projects in multiple languages and frequently uses new AI-agent sessions. Those agents do not know what happened in earlier sessions. Existing resumes and portfolio entries contain only compressed descriptions and can be stale or ambiguous. Asking each new agent to reread full repositories repeatedly is slow, expensive, and inconsistent.

The underlying problem is not resume writing. The underlying problem is the absence of durable project memory with traceable evidence and human approval.

Current failure modes include:

- Reexplaining the same project to every new agent.
- Agents misunderstanding architecture or claiming work that was not completed.
- Losing the reason behind architectural decisions.
- Treating unfinished work as completed work.
- Allowing resumes, portfolios, and repositories to disagree.
- Spending hours or days reconstructing project context instead of building projects.

## Operator

Version 1 has exactly one actor: the local developer operating `pmemc` from PowerShell on Windows.

There are no accounts, teams, permissions, remote users, or shared workspaces.

## Product promises

Version 1 promises that:

1. Project memory persists across agent sessions.
2. Every permanent technical fact has evidence or an explicit operator confirmation.
3. Committed and uncommitted work are never presented as equivalent.
4. Conflicts are shown rather than silently resolved.
5. No model response can write directly to verified memory.
6. An inspection baseline changes only after the operator completes review.
7. Local project memory remains on the operator's laptop.
8. A provider, parsing, or Git failure cannot corrupt already verified facts.

## Evidence and authority policy

Evidence sources can include:

- Source code
- Tests
- Git history and diffs
- Repository documentation
- Configuration and package manifests
- Existing verified project facts
- Operator answers

Source code and tests receive the highest default technical weight because they show practical implementation. Documentation and old written records are supporting evidence and may be stale.

The operator is the final authority. When code, documentation, old records, and operator recollection disagree, the tool must:

1. Show what each source claims.
2. Show the relevant source location and repository state.
3. Explain why the claims conflict.
4. Ask the operator for the final answer.
5. Store the decision and its evidence history.

The tool must never silently replace an approved fact.

## Work-state policy

- Evidence from a committed revision is labelled `committed`.
- Evidence from staged, unstaged, or untracked files is labelled `in_progress`.
- In-progress evidence may be stored, but it must remain visibly in progress.
- In-progress functionality must not be described as completed merely because the model finds code for it.
- If previously in-progress work later appears in a commit, the inspection must reconcile it without creating duplicate facts.

## Inspection policy

The product is on-demand, not continuously monitoring.

When a future consumer asks to create or update an artifact, it will first ask `pmemc` which registered repositories changed since their previous approved inspections. In Version 1, only the project-memory inspection workflow is built. Artifact consumers are future context, not Version 1 scope.

The operator decides whether detected changes should be inspected. Inspection should target changed files and relevant structural neighbours rather than repeatedly sending entire repositories to a model.

## Model policy

The architecture supports multiple model providers, but Version 1 implements only OpenRouter.

The model:

- Receives a minimized evidence bundle after inspection approval.
- Produces structured, untrusted proposals.
- May identify missing context and propose questions.
- Cannot approve facts.
- Cannot finalize a baseline.
- Cannot write directly to verified memory.

Provider keys are supplied through environment variables and are never written to the repository or PMEMC database.

## Long-term context — not authorized Version 1 scope

Verified project memory is intended to support later capabilities such as maintaining a master resume, creating job-specific two-page resumes, updating a portfolio JSON file, and generating project documentation.

This paragraph explains why the memory model must be durable. It does not authorize implementing or scaffolding any of those capabilities during Version 1.

## Version 1 success

Version 1 succeeds when the operator can close every AI-agent session, start a new one, and still retrieve the same verified project facts, supporting evidence, inspection history, and unresolved items through the CLI.
