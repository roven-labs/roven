<p align="center">
  <img src="assets/roven-logo-white.png" alt="Roven" width="320">
</p>

# Roven

Roven is a native terminal project-memory assistant. It keeps conversations
scoped to the trusted project directory and can register that project for
future resume and portfolio work.

Roven is provider-neutral: each user chooses an OpenAI-compatible HTTPS
chat-completions endpoint, model, and API key through a named provider profile.
It does not silently choose a provider, append an endpoint path, read source
files, or execute arbitrary commands.

## Current capabilities

- `roven auth set` creates a named provider profile.
- `roven auth list` shows saved names, endpoints, models, and the default marker.
- `roven auth use` lets you choose the default profile by its displayed number.
- `roven auth status` reports the selected profile without revealing its key.
- `roven auth remove <name>` removes a profile and its operating-system credential.
- Bare `roven` asks whether to trust the canonical current directory for the
  current launch. Trust is not persisted between launches.
- After trust, the optional root `ROVEN.md` is loaded as project instructions.
- `/resume` opens sessions for the current workspace. `Esc` cancels an active
  provider stream while retaining received content.

## Agent tools

The Rust harness exposes three local tools:

| Tool | Capability | Writes data? |
| --- | --- | --- |
| `list_directory` | Lists immediate entries inside the trusted workspace. It cannot recurse, read file contents, or escape the workspace. | No |
| `prepare_project` | Independently canonicalizes and authorizes the requested path, then validates Git/GitHub state and registers the trusted project. | Yes, after validation |
| `list_tools` | Returns the available tool names, descriptions, and input schemas. | No |

`prepare_project` rejects paths outside the launch-time trusted workspace before
any Git command, project lookup, or registration write. A successful
registration is stored as:

```text
%LOCALAPPDATA%\Roven\data\projects\<project-name>.json
```

## Local data

Roven stores data in the operating-system application-data directory. On
Windows the default root is `%LOCALAPPDATA%\Roven\data`:

```text
provider-profiles.json
projects\<project-name>.json
sessions\<workspace-sha256>\<session-uuid>\meta.json
sessions\<workspace-sha256>\<session-uuid>\context.json
sessions\<workspace-sha256>\<session-uuid>\events.jsonl
log.md
```

API keys are stored only in the operating-system credential store. Session
`events.jsonl` contains user, assistant, reasoning, error, and structured
`function_call_output` records with the tool name, input, and output. `log.md`
contains operational diagnostics only; it does not contain prompts, replies, or
credentials.

## Current boundaries

Roven does not currently provide a raw project file-reading tool, arbitrary
command execution, automatic provider retries, or automatic project-context
summarization. The agent only receives data returned by the available tools and
the optional root `ROVEN.md`.

## Install

From this repository in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new PowerShell session, change to the project directory, and run:

```powershell
roven
```

See [docs/SETUP.md](docs/SETUP.md) for provider profiles and API-key setup.
