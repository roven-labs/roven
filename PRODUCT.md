# Roven Product Context

## Current product boundary

Roven is a local terminal assistant for project-scoped conversations and
project registration. At startup it canonicalizes the current directory and
asks the user to trust it for that launch. Trust is runtime-only and is never
persisted across launches.

After trust, Roven loads only the optional root `ROVEN.md`, creates a session on
the first user message, and streams through the selected named
OpenAI-compatible provider profile. The profile supplies the exact HTTPS
chat-completions endpoint, model ID, and operating-system-stored API key.

## User-facing workflow

- `roven auth set` creates a named profile from user-provided endpoint, model,
  and key values.
- `roven auth list`, `use`, `status`, and `remove` manage profiles and the
  explicit default without displaying secrets.
- Bare `roven` opens the trust gate and terminal chat.
- `/resume` lists sessions for the current canonical workspace.
- `Esc` requests cancellation of an active provider stream.

## Tool boundary

The Rust harness, rather than the model prompt, enforces filesystem authority:

- `list_directory` lists only immediate entries inside the trusted workspace.
- `prepare_project` independently canonicalizes and compares its requested path
  with the trusted workspace before project lookup, Git validation, or writes.
- `list_tools` reports the live tool registry.

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
  context.json                         reserved summary state (currently null)
  events.jsonl                         conversation and tool-call events
log.md                                 operational diagnostics only
```

API keys never enter these files. `events.jsonl` records structured
`function_call_output` events with the tool call ID, tool name, input, and
output, allowing the live transcript and resumed provider messages to preserve
tool activity.

## Safety and non-goals

- Folder trust is requested every launch and is not persisted.
- Filesystem-sensitive tools enforce their own Rust-side workspace boundary.
- Roven does not edit files, execute arbitrary commands, recurse through a
  project, or expose API keys.
- Roven does not currently provide a raw project file-reading tool, automatic
  provider retries, or automatic context summarization.
