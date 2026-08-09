<p align="center">
  <img src="assets/roven-logo-white.png" alt="Roven" width="320">
</p>

Roven is a native terminal chat client for a project directory. It starts a
new conversation, asks you to trust the current folder for that launch, and
streams replies from the fixed OpenRouter model
`openai/gpt-oss-20b:free`.

## What works today

- `roven auth set`, `roven auth status`, and `roven auth remove` manage the
  OpenRouter key through the operating-system credential store.
- A trusted chat reads only the optional root `<project>/ROVEN.md` once, then
  sends that text with the conversation.
- The agent can list immediate directory-entry names and kinds inside the
  trusted workspace without reading file contents or recursing.
- Replies stream live. Provider-supplied reasoning appears as a muted
  `Thought` block; the active status changes from working to thinking to
  writing a response.
- `Esc` stops the local stream and keeps received text. `/resume` opens a
  picker for conversations from the current directory only.
- Sessions are stored outside the repository in the operating system's local
  application-data directory, using `meta.json`, `events.jsonl`, and
  `context.json`.
- The same local application-data directory contains append-only `log.md`.
  It records runtime operations and failures (UI, storage, agent, provider,
  and tool events), but never prompts, replies, or credentials.
- After trust, the agent can call `prepare_project` to register the current
  project only after fixed local Git validation confirms a GitHub remote,
  baseline commit, and clean worktree.

Roven never asks for or prints the API key in the chat UI, edits project files,
or runs arbitrary commands.

See [setup](docs/SETUP.md) to store an OpenRouter API key.

## Not implemented yet

Roven does not yet offer project file reading or search tools, broader Git
inspection, CodeGraph queries or indexing, context compaction, or automatic
provider retries. It therefore sends no source code or CodeGraph output to
OpenRouter.

## Install

From this repository in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new PowerShell session, change to the project directory, and run:

```powershell
roven
```
