# Roven Tools

This is a short reference for the tools implemented by Roven. It documents
their public purpose, input, and result shape. It is not an agent prompt or a
workflow definition.

## Slash commands

- `/register` expands Roven's built-in project-registration prompt, which
  prepares the current trusted workspace, reads the codebase, and stores a V2
  project snapshot through `prepare_project`.
- `/resume` opens the saved conversations for the current project.
- `/model` opens the provider and model picker.

## At a glance

| Tool | Main purpose | Writes data |
| --- | --- | --- |
| `list_directory` | List immediate entries in the trusted workspace | No |
| `read_file` | Read a small UTF-8 text file in the trusted workspace | No |
| `prepare_project` | Validate/register the trusted project and store its V2 snapshot | Yes, after validation |
| `list_tools` | Return the live tool catalog | No |
| `list_project` | Return stored project names in alphabetical order | No |

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

Validates and registers the trusted project. The requested path must resolve to
the exact workspace root trusted when Roven launched. The harness checks this
boundary before project lookup, Git commands, or registration writes.

Input:

```json
{
  "path": ".",
  "project_name": "PayFlow",
  "project_facts": ["Uses PostgreSQL for persistent application data."],
  "user_context_facts": ["Built as a team project in 2026."],
  "user_contribution_facts": ["Implemented the authentication flow."]
}
```

`path` and `project_name` are required. The three fact arrays are optional and
default to empty arrays. `project_name` must not be blank.

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

If the project is already registered at the same canonical path, the
registration mode returns `already_added` and does not overwrite it. Duplicate
project names are also rejected. An unauthorized path returns `blocked` with
reason `path_not_allowed`; no Git command or registration write occurs. Other
blocked reasons identify invalid paths, missing Git prerequisites, an
unavailable Git executable, an unclean repository, an invalid project name, or
local storage failure.

The registration file is written to:

```text
%LOCALAPPDATA%\Roven\data\projects\<sha256-of-canonical-project-path>\
  project_snapshot.json
  repository_metadata.json
```

The snapshot JSON has exactly this shape:

```json
{
  "project_name": "PayFlow",
  "project_facts": [],
  "user_context_facts": [],
  "user_contribution_facts": []
}
```

The metadata JSON is generated by Rust and contains `github_remote` and
`baseline_commit`. Both files are strictly validated. Listing validates every
registered project and returns names alphabetically; malformed, incomplete, or
unsupported V1 project data makes listing return `storage_failure` rather than
returning a partial list. V1 files are not migrated.

## `list_tools`

Returns the currently available tool names, descriptions, and input schemas.
It takes an empty object:

```json
{}
```

The result has `status: "ok"` and a `tools` array. It reads the live Roven
tool catalog and does not access the workspace or modify data.

## `list_project`

Returns the names of projects already registered in local Roven storage, in
alphabetical order. It takes an empty object:

```json
{}
```

The result has `status: "ok"` and a `projects` array. It does not access the
workspace or modify data.
