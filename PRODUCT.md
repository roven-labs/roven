# Roven Product Context

## Current product boundary

Roven is a local terminal client for persistent, project-scoped chat. Bare
`roven` asks whether to trust the canonical current directory for this launch.
After approval, it loads only the optional root `ROVEN.md`, creates a session
when the first message is sent, and streams a reply from the fixed OpenRouter
model `openai/gpt-oss-20b:free`.

The UI shows real provider reasoning when it is supplied, a live active-status
line, the final answer, inline errors, and a `/resume` picker. Conversations
are scoped by a SHA-256 hash of the canonical project path and stored outside
the repository through `ProjectDirs::data_local_dir()`.

## Safety boundary implemented today

- OpenRouter credentials stay in the operating-system credential store.
- Folder trust is requested for every launch and is not persisted.
- Before trust, Roven does not read `ROVEN.md` or make a provider request.
- The only project file Roven currently reads is the optional root
  `ROVEN.md`, after trust.
- Roven writes append-only runtime diagnostics to `log.md` in its local
  application-data directory, never into the trusted workspace. The log
  records operational metadata and errors, not prompts, replies, or keys.
- Its `list_directory` tool may read immediate directory-entry names and kinds
  inside the trusted workspace; it does not read file contents or recurse.
- Roven does not edit files, execute arbitrary commands, search files, or
  invoke CodeGraph. Its `prepare_project` tool may run fixed local Git
  validation commands for the trusted workspace before registering it.

## Planned, not implemented

Read-only project tools, Rust-enforced file and symlink protections for those
tools, CodeGraph initialization, broader Git inspection, context compaction,
and automatic retry policy remain planned work. They must not be described as
available until their code and tests exist.
