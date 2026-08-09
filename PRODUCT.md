# PMEMC Product Context

## Current product boundary

PMEMC is a local terminal client for persistent, project-scoped chat. Bare
`pmemc` asks whether to trust the canonical current directory for this launch.
After approval, it loads only the optional root `PMEMC.md`, creates a session
when the first message is sent, and streams a reply from the fixed OpenRouter
model `openai/gpt-oss-20b:free`.

The UI shows real provider reasoning when it is supplied, a live active-status
line, the final answer, inline errors, and a `/resume` picker. Conversations
are scoped by a SHA-256 hash of the canonical project path and stored outside
the repository through `ProjectDirs::data_local_dir()`.

## Safety boundary implemented today

- OpenRouter credentials stay in the operating-system credential store.
- Folder trust is requested for every launch and is not persisted.
- Before trust, PMEMC does not read `PMEMC.md` or make a provider request.
- The only project file PMEMC currently reads is the optional root
  `PMEMC.md`, after trust.
- PMEMC does not edit files, execute arbitrary commands, invoke Git, search
  files, or invoke CodeGraph.

## Planned, not implemented

Read-only project tools, Rust-enforced file and symlink protections for those
tools, CodeGraph initialization, Git inspection, context compaction, and
automatic retry policy remain planned work. They must not be described as
available until their code and tests exist.
