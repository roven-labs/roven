# Roven Product Context

## Current product boundary

Roven is a local terminal assistant for project-scoped conversations and
project registration. At startup it canonicalizes the current directory and
asks the user to trust it for that launch. Trust is runtime-only and is never
persisted across launches.

After trust, Roven loads only the optional root `ROVEN.md`, creates a session on
the first user message, and streams through the selected named provider
profile. The profile supplies the exact HTTPS endpoint, model ID, and
operating-system-stored API key. OpenAI-compatible endpoints and Ollama Cloud's
native `/api/chat` endpoint are handled by separate protocol adapters.

## User-facing workflow

- `roven auth set` creates a named profile from user-provided endpoint, model,
  and key values.
- `roven auth list`, `use`, `status`, and `remove` manage profiles and the
  explicit default without displaying secrets.
- Bare `roven` opens the trust gate and terminal chat.
- `/register` validates the current trusted project, inspects it through the
  read-only tools, and saves a compact evidence summary locally.
- `/generate-resume <job description>` generates a Markdown project section
  from the job description and stored project summaries only. Generation does
  not read the workspace or repository, expose provider tools, or invent
  achievements, metrics, technologies, or responsibilities. Output is stored
  at `%LOCALAPPDATA%\Roven\data\resumes\<uuid>.md`.
- `/resume` lists sessions for the current canonical workspace.
- `Esc` requests cancellation of an active provider stream.

## Tool boundary

The Rust harness, rather than the model prompt, enforces filesystem authority:

- `list_directory` lists only immediate entries inside the trusted workspace.
- `read_file` reads regular UTF-8 text files up to 50 KiB inside the trusted
  workspace.
- `prepare_project` independently canonicalizes and compares its requested path
  with the trusted workspace before project lookup, Git validation, or writes.
- `list_tools` reports the live tool registry.
- `list_project` reports stored project names in alphabetical order.

`prepare_project` validates the trusted project’s GitHub remote, committed
baseline, and clean working state before writing
`data/projects/<project-name>.json`. A sibling path, traversal path, or symlink
that resolves outside the trusted workspace is blocked before Git execution.

## Persistent data

Roven uses the operating-system local application-data directory, separate from
the project repository:

```text
provider-profiles.json                 non-secret profile metadata
projects/<project-name>.json           registered project identity and baseline
sessions/<workspace-sha256>/<uuid>/    conversation sessions
  meta.json                            session identity and timestamps
  events.jsonl                         conversation and tool-call events
ollama-stream-failures.log             raw Ollama stream data after failures
resumes/<uuid>.md                       generated resume project sections
log.md                                 operational diagnostics only
```

API keys are stored in the operating-system credential store and are not
intentionally written to these files. `events.jsonl` records structured
`function_call_output` events with the tool call ID, tool name, input, and
output, including successful `read_file` contents. Those results can be sent
back to the selected provider on later turns or resume. The Ollama failure log
can contain raw model output and tool-call arguments.

## Safety and non-goals

- Folder trust is requested every launch and is not persisted.
- Filesystem-sensitive tools enforce their own Rust-side workspace boundary.
- Roven does not edit files, execute arbitrary commands, recurse through a
  project, or expose API keys.
- Roven provides a read-only raw project file-reading tool for regular UTF-8
  files up to 50 KiB inside the trusted workspace. It does not provide
  automatic provider retries or automatic context summarization.
- Resume project-section generation uses stored project summaries as its sole
  project evidence and does not read the workspace/repository or provide tools.
