<p align="center">
  <img src="assets/roven-logo-white.png" alt="Roven" width="320">
</p>

# Roven

Roven is a native terminal project-memory assistant. The shipped executable is
`pmemc`. It keeps conversations
scoped to the trusted project directory and can register that project for
future resume and portfolio work.

Roven is provider-neutral: each user chooses a provider endpoint, model, and
API key through a named provider profile. It supports OpenAI-compatible
chat-completions endpoints and Ollama Cloud's native HTTPS API. It does not
silently choose a provider, append an endpoint path, or execute arbitrary
commands.

## Current capabilities

- `pmemc auth set` creates a named provider profile.
- `pmemc auth list` shows saved names, endpoints, models, and the default marker.
- `pmemc auth use` lets you choose the default profile by its displayed number.
- `pmemc auth status` reports the selected profile without revealing its key.
- `pmemc auth remove <name>` removes a profile and its operating-system credential.
- Native Ollama Cloud profiles use `https://ollama.com/api/chat` and report the
  provider's real context-window usage as a percentage.
- Bare `pmemc` asks whether to trust the canonical current directory for the
  current launch. Trust is not persisted between launches.
- After trust, the optional root `ROVEN.md` is loaded as project instructions.
- `/resume` opens sessions for the current workspace. `Esc` cancels an active
  provider stream while retaining received content.

## Agent tools

The Rust harness exposes four local tools:

| Tool | Capability | Writes data? |
| --- | --- | --- |
| `list_directory` | Lists immediate entries inside the trusted workspace. It cannot recurse, read file contents, or escape the workspace. | No |
| `read_file` | Reads a regular UTF-8 text file up to 50 KiB using a workspace-relative path inside the trusted workspace. | No |
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
sessions\<workspace-sha256>\<session-uuid>\events.jsonl
log.md
ollama-stream-failures.log  (created only when an Ollama stream fails)
```

API keys are stored only in the operating-system credential store. Session
`events.jsonl` contains user, assistant, reasoning, error, and structured
`function_call_output` records with the tool name, input, and output. Successful
`read_file` results are therefore persisted as part of session history and can
be sent back to the selected provider on later turns or resume. Do not place
secrets in project files. `log.md` contains operational diagnostics only; the
separate Ollama failure log can contain raw model output and tool-call
arguments.

## Current boundaries

Roven can load the optional root `ROVEN.md` and read regular UTF-8 project files
up to 50 KiB through `read_file`, using workspace-relative paths inside the
trusted workspace. It does not provide arbitrary command execution, automatic
provider retries, or automatic project-context summarization. The agent only
receives data returned by the available tools and the optional root `ROVEN.md`.

## Install

From this repository in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

Open a new PowerShell session, change to the project directory, and run:

```powershell
pmemc
```

See [docs/SETUP.md](docs/SETUP.md) for provider profiles and API-key setup.
See [docs/PROVIDERS.md](docs/PROVIDERS.md) for exact Ollama Cloud and
OpenRouter commands, endpoint selection, and context-usage troubleshooting.
See [docs/TOOLS.md](docs/TOOLS.md) for the local tool reference.
