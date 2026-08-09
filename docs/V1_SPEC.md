# PMEMC Version 1 Specification

## 1. Scope

Version 1 is currently reset to a local Windows 11 credential-management
baseline. It has one Cargo package and no active repository or agent
capability.

## 2. Public interface

The only supported command group is:

```text
pmemc auth set
pmemc auth status
pmemc auth remove
```

Bare `pmemc` must not start a conversational session, inspect the current
directory, access a repository, or make an external request.

## 3. Credential lifecycle

`pmemc auth set` accepts a secret without echoing it and stores it in Windows
Credential Manager. `pmemc auth status` exposes configuration status only.
`pmemc auth remove` deletes the stored credential.

The secret must never be written to SQLite or another PMEMC database, logs,
terminal output, command arguments, repository files, or test fixtures.

## 4. Explicitly absent capabilities

The current V1 executable has no database, project registration, repository
validation, CodeGraph integration, source discovery, model configuration,
external provider transport, conversational session, or model-visible tool
catalog. It also has no inspection, evidence, baseline, proposal, review,
verified-memory, portfolio, resume, filesystem-write, Git-write, MCP,
plugin, or background-monitoring workflow.

No capability in this list may be reintroduced as a hidden fallback,
environment-variable side effect, or undocumented command.

## 5. Future changes

A future capability requires a new approved V1 contract before implementation.
That contract must define its public interface, data ownership, approval
boundary, failure behavior, and acceptance tests. This specification does not
authorize implementation or scaffolding of those capabilities.

## 6. Verification

Before release, run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```
