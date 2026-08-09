# PMEMC Version 1 Implementation Plan

## Current phase: local terminal UI preview

The implementation provides a local terminal interaction preview alongside the
existing credential lifecycle. Bare `pmemc` opens the preview; `pmemc auth`
continues to manage the Windows Credential Manager secret.

## Exit criteria

- Bare `pmemc` opens and restores a full-screen local terminal UI with a
  transcript and four-line composer.
- Submitted non-empty text appends a local preview reply; it does not read a
  credential, contact a service, or persist a conversation.
- `pmemc auth set`, `pmemc auth status`, and `pmemc auth remove` use Windows
  Credential Manager without revealing the secret.
- No PMEMC database, repository workflow, CodeGraph flow, model state,
  provider transport, agent tool catalog, or external request is reachable.
- Formatting, Clippy, tests, and the Windows installer verification pass.

## Deferred work

OpenRouter integration, model selection, token streaming, persistent sessions,
repository workflows, source discovery, storage, and agent tools remain
unapproved. Each requires a new contract before implementation.
