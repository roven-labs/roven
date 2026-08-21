# Roven Tools

This is a short reference for the tools implemented by Roven. It documents
their public purpose, input, and result shape. It is not an agent prompt or a
workflow definition.

## Slash commands

- `/register` submits Roven's built-in project-registration prompt, prepares
  the current trusted workspace, reads the codebase, and stores a concise
  report through `prepare_project`.

## At a glance

| Tool | Main purpose | Writes data |
| --- | --- | --- |
| `list_directory` | List immediate entries in the trusted workspace | No |
| `read_file` | Read a small UTF-8 text file in the trusted workspace | No |
| `prepare_project` | Validate/register the trusted project or replace its local summary | Yes, after validation |
| `list_tools` | Return the live tool catalog | No |

## `list_directory`

Lists the immediate files, directories, symlinks, and other entries at a path
inside the trusted workspace. It does not read file contents, recurse, modify
files, or follow a path outside the trusted workspace.

Input:

```json
{ "path": "." }
```

The path is workspace-relative; absolute paths and `..` traversal are rejected.
Results contain at most 100 immediate entries in deterministic order. When more
entries exist, `truncated` is `true`; retry with a narrower directory path to
inspect the rest. Results include:

```json
{
  "status": "ok",
  "path": ".",
  "workspace_path": "C:\\Projects\\my-app",
  "entries": [
    { "name": "src", "path": "src", "kind": "directory" },
    { "name": "main.rs", "path": "src/main.rs", "kind": "file", "size_kb": 3.5 },
    {
      "name": "latest",
      "path": "latest",
      "kind": "symlink",
      "size_error": "symlink_not_followed"
    }
  ],
  "truncated": false
}
```

Every regular file includes `size_kb`, calculated as `file_size_in_bytes / 1024`
and rounded to two decimal places. This applies to all regular files, including
executables, binaries, and model files; listing metadata does not read their
contents. Directories and `other` entries omit size fields. Symlinks are never
followed and report `size_error: "symlink_not_followed"`. If a regular file's
metadata cannot be read, the entry remains in the result and reports either
`size_error: "permission_denied"` or `size_error: "io_error"`.

Possible errors include `invalid_path`, `path_not_allowed`, `not_directory`,
`permission_denied`, and `io_error`. For `invalid_path` or `path_not_allowed`,
retry with a relative path under the trusted workspace. For `not_directory`,
pass a directory path.

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

Validates and registers the trusted project, or replaces its local `summary`
section. The requested path must resolve to the exact workspace root trusted
when Roven launched. The harness checks this boundary before project lookup,
Git commands, or registration writes.

Input:

```json
{ "path": "." }
```

After registration, the version-one section update accepts only this shape:

```json
{
  "path": ".",
  "section_name": "summary",
  "text": "concise codebase report",
  "operation": "replace"
}
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

If the project is already registered, the registration mode returns
`already_added`. A successful section update returns `summary_saved`. An
unauthorized path returns `blocked` with reason `path_not_allowed`; no Git
command or registration write occurs. Other blocked reasons identify invalid
paths, invalid section updates, an unregistered update, missing Git
prerequisites, an unavailable Git executable, an unclean repository, or local
storage failure.

The registration file is written to:

```text
%LOCALAPPDATA%\Roven\data\projects\<project-name>.json
```

The JSON keeps registration identity and an optional `sections` map. A saved
report is stored as `sections.summary`; it is not written into the project
repository.

## `list_tools`

Returns the currently available tool names, descriptions, and input schemas.
It takes an empty object:

```json
{}
```

The result has `status: "ok"` and a `tools` array. It reads the live Roven
tool catalog and does not access the workspace or modify data.
