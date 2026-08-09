# PMEMC Version 1 Specification

## 1. Scope

Version 1 is a native Windows 11 terminal UI preview plus local credential
management. It has one Cargo package. The UI preview is local only: it has no
model provider, credential read, external request, repository access, or
durable session state.

## 2. Public interface

Bare `pmemc` opens a full-screen terminal UI preview. The preview presents a
muted `PMEMC` / `UI Preview` header, a left-aligned transcript, and a
multi-line composer. A submitted non-empty message appends a `You ›` turn and
then the fixed local preview reply. It must restore the terminal screen on
exit.

The supported credential commands are:

```text
pmemc auth set
pmemc auth status
pmemc auth remove
```

## 3. UI-preview behavior

- `Enter` submits; `Alt+Enter` inserts a newline.
- The composer grows to four lines and then scrolls internally.
- Keyboard and mouse-wheel transcript navigation are supported; a scrollbar
  appears only while away from the newest content.
- `Ctrl+C` exits immediately.
- A terminal too small for the layout shows `Resize terminal to continue`.
- Roles are text-labelled and use restrained semantic colors. Message content
  is plain wrapped text; no Markdown, spinner, footer hints, or in-chat
  commands are in scope.

The preview must not read credentials, contact OpenRouter or another service,
stream model output, persist messages, inspect the working directory, access
a repository, expose tools, or execute commands.

## 4. Credential lifecycle

`pmemc auth set` accepts a secret without echoing it and stores it in Windows
Credential Manager. `pmemc auth status` exposes configuration status only.
`pmemc auth remove` deletes the stored credential.

The secret must never be written to SQLite or another PMEMC database, logs,
terminal output, command arguments, repository files, or test fixtures.

## 5. Deferred capabilities

Model-provider transport, OpenRouter integration, streaming, conversational
memory, repository workflows, source discovery, database storage, agent tools,
MCP, plugins, and background monitoring are deferred. They require a new
approved contract defining their public interface, data ownership, approval
boundary, failure behavior, and acceptance tests.

## 6. Verification

Before release, run:

```powershell
cargo fmt --all --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```
