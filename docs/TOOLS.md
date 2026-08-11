# Roven Tools

This is a short reference for the tools implemented by Roven. It documents
their public purpose, input, and result shape. It is not an agent prompt or a
workflow definition.

## At a glance

| Tool | Main purpose | Writes data |
| --- | --- | --- |
| `list_directory` | List immediate entries in the trusted workspace | No |
| `prepare_project` | Validate and register the trusted project | Yes, after validation |
| `list_tools` | Return the live tool catalog | No |
| CodeGraph MCP tools | Every tool returned by the configured CodeGraph MCP server | No |

## `list_directory`

Lists the immediate files, directories, symlinks, and other entries at a path
inside the trusted workspace. It does not read file contents, recurse, modify
files, or follow a path outside the trusted workspace.

Input:

```json
{ "path": "." }
```

The path is workspace-relative. Absolute paths and `..` traversal are rejected.
Results are capped at 100 entries and include:

```json
{
  "status": "ok",
  "path": ".",
  "workspace_path": "C:\\Projects\\my-app",
  "entries": [
    { "name": "src", "path": "src", "kind": "directory" }
  ],
  "truncated": false
}
```

Possible errors include `invalid_path`, `path_not_allowed`, `not_directory`,
`permission_denied`, and `io_error`.

## `prepare_project`

Validates and registers the trusted project. The requested path must resolve to
the exact workspace root trusted when Roven launched. The harness checks this
boundary before project lookup, Git commands, or registration writes.

Input:

```json
{ "path": "." }
```

Registration also requires a usable Git repository with:

- a GitHub remote;
- at least one committed baseline; and
- a clean working tree.

Successful result:

```json
{
  "status": "prepared",
  "project": {
    "name": "my-app",
    "path": "C:\\Projects\\my-app",
    "github_remote": "https://github.com/example/my-app.git",
    "baseline_commit": "abc123..."
  }
}
```

If the project is already registered, the result is `already_added`. An
unauthorized path returns `blocked` with reason `path_not_allowed`; no Git
command or registration write occurs. Other blocked reasons identify invalid
paths, missing Git prerequisites, an unavailable Git executable, an unclean
repository, or local storage failure.

The registration file is written to:

```text
%LOCALAPPDATA%\Roven\data\projects\<project-name>.json
```

## `list_tools`

Returns the currently available tool names, descriptions, input schemas, and
MCP connection status. When MCP startup fails, local tools remain available and
the response includes the preserved startup error.
It takes an empty object:

```json
{}
```

The result has `status: "ok"` and a `tools` array. It reads the live Roven
tool catalog and does not access the workspace or modify data.

## CodeGraph MCP tools

This is not a Roven wrapper around CodeGraph. Roven launches the installed
CodeGraph stdio MCP server in the trusted workspace, discovers the tool through
MCP `tools/list`, and publishes the server-provided name, description, and
`inputSchema` to the model unchanged. The model's arguments are forwarded to
MCP `tools/call` unchanged, and the MCP result is retained unchanged.

The live CodeGraph contract is therefore authoritative. In the current
CodeGraph release this includes `codegraph_explore`, which accepts the server's
documented fields such as a natural-language `query` plus optional `maxFiles`
and `projectPath` values. If a future CodeGraph MCP release advertises more
tools, Roven exposes those too without a code change. Roven does not add a
file-type allowlist or restrict exploration to selected extensions. The server
is read-only from Roven's perspective; Roven does not initialize, rebuild, or
sync the CodeGraph index. The optional `projectPath` remains part of the exact
server contract, but Roven rejects values outside the trusted workspace.

If the MCP server cannot start, its tools are omitted from the live catalog and
`list_tools` reports the unavailable status and startup error.
An unindexed workspace can still expose the server's tools, but CodeGraph may
return its own index-related error. Roven does not silently pretend that source
inspection happened.
