# Changelog

## [1.1.0] - 2026-08-30

### Added

- V2 immutable project snapshots stored under a SHA-256 directory derived from
  the canonical project path.
- Separate Rust-generated repository metadata containing the GitHub remote and
  baseline commit.
- Explicit resume-ready project names and three fact groups: repository facts,
  user context facts, and user contribution facts.
- Strict JSON validation, duplicate-name rejection, and alphabetical snapshot
  listing.

### Changed

- Project registration now writes `project_snapshot.json` and
  `repository_metadata.json` instead of a V1 project JSON file.
- Registration rejects an already-registered canonical path without silently
  overwriting it.

### Removed

- V1 `sections.summary` storage and summary replacement behavior.

### Compatibility

- V1 project files are not migrated. Unsupported or malformed project data
  causes storage operations to fail until it is removed and the project is
  registered again.

## [1.0.0] - 2026-08-29

### Added

- Named provider profiles with operating-system credential storage.
- OpenAI-compatible provider streaming and native Ollama Cloud streaming.
- Launch-time trusted-workspace access with read-only workspace tools.
- GitHub-backed project registration and stored project summaries.
- `/register`, `/resume`, and `/generate-resume` terminal commands.
- Windows PowerShell installation and uninstall support.

### Security

- Provider API keys stay in the operating-system credential store or provider
  environment variables.
- Workspace tools reject traversal, sibling paths, and symlink escapes.
- Resume generation uses stored project summaries without workspace tools.

### Known limitations

- Conversation history is stored locally and can include workspace file content.
- Ollama stream failure diagnostics can contain raw model output and tool-call
  arguments.
- Version 1 targets Windows and does not provide arbitrary command execution.
