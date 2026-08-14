# Roven Tools

This is a short reference for the tools implemented by Roven. It documents
their public purpose, input, and result shape. It is not an agent prompt or a
workflow definition.

## At a glance

| Tool | Main purpose | Writes data |
| --- | --- | --- |
| `list_directory` | List immediate entries in the trusted workspace | No |
| `read_file` | Read a small UTF-8 text file in the trusted workspace | No |
| `prepare_project` | Validate and register the trusted project | Yes, after validation |
| `list_tools` | Return the live tool catalog | No |

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

## `read_file`

Reads a known workspace-relative file after it has been located with
`list_directory`. It reads only regular UTF-8 text files at most 50 KiB inside
the trusted workspace and does not modify files or access paths outside it.

Input:

```json
{ "path": "src/main.rs" }
```

Successful result:

```json
{ "status": "ok", "path": "src/main.rs", "content": "..." }
```

Possible errors include `invalid_path`, `path_not_allowed`, `not_file`,
`file_too_large`, `not_text`, `permission_denied`, and `io_error`.

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

Returns the currently available tool names, descriptions, and input schemas.
It takes an empty object:

```json
{}
```

The result has `status: "ok"` and a `tools` array. It reads the live Roven
tool catalog and does not access the workspace or modify data.
