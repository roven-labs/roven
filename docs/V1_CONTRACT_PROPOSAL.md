# Approved V1 Contract Amendment — Local UI Preview

## Status

Approved by the operator. This amendment supersedes the earlier
capability-free-reset statement that bare `pmemc` only printed help.

## Contract

Bare `pmemc` opens a full-screen local terminal UI preview. The UI accepts
plain text, displays it as a `You ›` turn, and appends a fixed `PMEMC ›`
preview reply. It has no network, model, credential-read, repository, command
execution, or persistence behavior.

PMEMC also retains the local `pmemc auth` credential lifecycle. It accepts,
checks for, and removes a secret through Windows Credential Manager without
printing or otherwise persisting the secret in PMEMC application data.

PMEMC exposes no model-facing tools, does not inspect, register, index, read,
or write repositories, has no project or memory database, and makes no
external request. Provider-backed chat and all other future capabilities must
be separately approved before code or documentation introduces them.
