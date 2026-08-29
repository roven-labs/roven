# Changelog

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
