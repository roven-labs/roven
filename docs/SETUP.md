# Setup

## Install

From the repository root in PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install.ps1
```

The script builds the release binary, installs it under
`%LOCALAPPDATA%\Programs\Roven`, and adds that directory to the user PATH.
Open a new PowerShell session after installation.

## Create a provider profile

Run:

```powershell
roven auth set
```

Roven asks for:

1. A user-chosen profile name.
2. The complete HTTPS OpenAI-compatible chat-completions endpoint.
3. The model ID accepted by that endpoint.
4. The API key, entered twice for confirmation.

The endpoint must be HTTPS, must not contain credentials, query parameters, or
fragments, and must be the complete endpoint. Roven does not append
`/chat/completions` automatically.

Examples:

```text
https://api.groq.com/openai/v1/chat/completions
https://openrouter.ai/api/v1/chat/completions
https://ollama.com/v1/chat/completions
```

The profile metadata is stored in the local application-data directory. The API
key is stored separately in the operating-system credential store and is never
written to JSON, logs, prompts, or chat messages.

## Manage profiles

```powershell
roven auth list
roven auth use
roven auth status
roven auth remove <profile-name>
```

The first created profile becomes the default when no default exists. Use
`roven auth use` to change it explicitly. If removing the default while other
profiles remain, Roven asks you to choose its replacement.

`auth list` and `auth status` show profile names, endpoints, and model IDs, but
never API keys.

## Start Roven

Change to the project directory and run:

```powershell
roven
```

Roven shows a trust prompt for the canonical current directory. The trust is
valid only for that launch. After accepting it, the first user message creates
a session and the agent can use the trusted-workspace tools.

## Local data locations

On Windows, Roven uses:

```text
%LOCALAPPDATA%\Roven\data\provider-profiles.json
%LOCALAPPDATA%\Roven\data\projects\<project-name>.json
%LOCALAPPDATA%\Roven\data\sessions\<workspace-sha256>\<session-uuid>\
```

Each session directory may contain `meta.json`, `context.json`, and
`events.jsonl`. Tool calls are persisted in `events.jsonl` as structured
`function_call_output` records containing the tool name, input, and output.

## Troubleshooting

- `Provider rejected the request (HTTP 503)` means the configured provider is
  temporarily unavailable or has no capacity. It is not a project or Git error.
- HTTP 429 means the provider rate limit was reached; wait and try again.
- HTTP 404 usually means the endpoint or model ID is wrong. Check that the
  complete chat-completions endpoint was entered exactly.
- If no default profile or API key exists, run `roven auth list`,
  `roven auth use`, or `roven auth set` as appropriate.

Never put secrets in repository files, command-line arguments, Roven's local
conversation files, logs, test fixtures, or terminal output.
