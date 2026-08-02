# Proposed V1 Contract Amendment — Conversational Study Entry Point

## Status

Proposed. Operator decisions recorded here do not modify `docs/V1_SPEC.md`
until the amendment and its implementation design are approved.

## Amendment

After bare `pmemc` successfully completes repository validation, project
registration, CodeGraph preparation, and provider access, it opens a local
interactive conversational session for that registered repository.

The session accepts slash commands. `/study` is a command within that session;
it is not a PowerShell command, a top-level `pmemc` subcommand, or an
`pmemc inspect` flag.

Opening the session must not itself read repository content, invoke OpenRouter,
or alter verified facts, inspection baselines, or the repository. `/study` is
the first approval-gated LLM workflow in the session.

The final V1 workflow replaces `pmemc inspect`'s static provider submission.
During the approved demo, the existing implementation remains only as reference
and regression coverage; it is not an equal user-facing workflow.

## Unresolved Contract Decision

`/study` uses CodeGraph as its source-discovery engine. It must not receive a
Rust-selected concatenated source bundle.

Rust retains deterministic safety and control only: approved repository root,
clean-state validation, CodeGraph readiness, ignored-file and secret rules,
path containment, output limits, operator approval, provider transport, and
failure handling.

## Consequence for the V1 CLI Surface

The current restriction on new top-level capabilities remains unchanged:
`/study` exists only after the bare-startup session is open.
