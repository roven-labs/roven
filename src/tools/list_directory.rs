use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    RovenToolDefinition, ToolContext,
    workspace::{
        WorkspacePathError, canonical_workspace_path, workspace_relative_path,
        workspace_relative_path_from_relative,
    },
};

const LIST_DIRECTORY_DESCRIPTION: &str = "List the immediate contents of a directory inside the currently trusted Roven workspace. Use this when you need to inspect workspace structure or locate a file or subdirectory before calling another filesystem tool. Pass a workspace-relative directory path such as `.` or `src`; do not pass an absolute path or a path containing `..`. Returns up to 100 immediate entries in deterministic order with `status`, `path`, `workspace_path`, `entries`, and `truncated`; if more entries exist, `truncated` is true. Each entry includes `name`, workspace-relative `path`, and `kind`. Every regular file also includes `size_kb`, measured as bytes divided by 1024 and rounded to two decimal places. Directories and other entries omit size fields. Symlinks are not followed and include `size_error: \"symlink_not_followed\"`; regular-file metadata failures keep the entry and include `size_error: \"permission_denied\"` or \"io_error\". For `invalid_path` or `path_not_allowed`, retry with a relative path under the workspace; for `not_directory`, pass a directory path. This tool does not read file contents, search recursively, modify files, register projects, or access paths outside the trusted workspace.";

pub(super) fn definition() -> RovenToolDefinition {
    RovenToolDefinition {
        name: "list_directory".to_owned(),
        description: LIST_DIRECTORY_DESCRIPTION.to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "Workspace-relative directory path; use `.` for the workspace root." } },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub(super) fn dispatch(context: &ToolContext, arguments: Value) -> serde_json::Result<Value> {
    match serde_json::from_value::<ListDirectoryInput>(arguments) {
        Ok(input) => serde_json::to_value(ListDirectory.execute(context, input)),
        Err(_) => serde_json::to_value(ListDirectoryResult::error(
            ListDirectoryErrorReason::InvalidPath,
            "",
        )),
    }
}

const DIRECTORY_LIST_LIMIT: usize = 100;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListDirectoryInput {
    path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListDirectoryResult {
    Ok {
        path: String,
        workspace_path: String,
        entries: Vec<ListDirectoryEntry>,
        truncated: bool,
    },
    Error {
        reason: ListDirectoryErrorReason,
        path: String,
    },
}

impl ListDirectoryResult {
    fn error(reason: ListDirectoryErrorReason, path: impl Into<String>) -> Self {
        Self::Error {
            reason,
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
struct ListDirectoryEntry {
    name: String,
    path: String,
    kind: DirectoryEntryKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_kb: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size_error: Option<DirectoryEntrySizeError>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum DirectoryEntrySizeError {
    PermissionDenied,
    IoError,
    SymlinkNotFollowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ListDirectoryErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotDirectory,
    PermissionDenied,
    IoError,
}

struct ListDirectory;

impl ListDirectory {
    fn execute(&self, context: &ToolContext, input: ListDirectoryInput) -> ListDirectoryResult {
        let target = match resolve_workspace_directory(context, &input.path) {
            Ok(path) => path,
            Err(reason) => return ListDirectoryResult::error(reason, input.path),
        };
        let relative_path = workspace_relative_path(&context.trusted_workspace, &target);
        let entries = match fs::read_dir(&target) {
            Ok(entries) => entries,
            Err(error) => {
                return ListDirectoryResult::error(io_reason(&error), relative_path);
            }
        };
        let mut listed_entries = Vec::new();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => return ListDirectoryResult::error(io_reason(&error), relative_path),
            };
            let file_type = match entry.file_type() {
                Ok(file_type) => file_type,
                Err(error) => return ListDirectoryResult::error(io_reason(&error), relative_path),
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let (size_kb, size_error) = if file_type.is_file() {
                match entry.metadata() {
                    Ok(metadata) => (Some(file_size_kb(metadata.len())), None),
                    Err(error) => (None, Some(size_error_reason(&error))),
                }
            } else if file_type.is_symlink() {
                (None, Some(DirectoryEntrySizeError::SymlinkNotFollowed))
            } else {
                (None, None)
            };
            let child_relative = target
                .strip_prefix(&context.trusted_workspace)
                .expect("authorized target remains under trusted workspace")
                .join(&name);
            listed_entries.push(ListDirectoryEntry {
                name,
                path: workspace_relative_path_from_relative(&child_relative),
                kind: entry_kind(&file_type),
                size_kb,
                size_error,
            });
        }
        listed_entries.sort_by(|left, right| {
            entry_kind_rank(left.kind)
                .cmp(&entry_kind_rank(right.kind))
                .then_with(|| left.name.cmp(&right.name))
        });
        let truncated = listed_entries.len() > DIRECTORY_LIST_LIMIT;
        listed_entries.truncate(DIRECTORY_LIST_LIMIT);
        ListDirectoryResult::Ok {
            path: relative_path,
            workspace_path: human_workspace_path(&context.trusted_workspace),
            entries: listed_entries,
            truncated,
        }
    }
}

fn resolve_workspace_directory(
    context: &ToolContext,
    path: &str,
) -> Result<PathBuf, ListDirectoryErrorReason> {
    let target = canonical_workspace_path(context, path).map_err(workspace_path_error_reason)?;
    target
        .is_dir()
        .then_some(target)
        .ok_or(ListDirectoryErrorReason::NotDirectory)
}

fn workspace_path_error_reason(error: WorkspacePathError) -> ListDirectoryErrorReason {
    match error {
        WorkspacePathError::InvalidPath => ListDirectoryErrorReason::InvalidPath,
        WorkspacePathError::PathNotAllowed => ListDirectoryErrorReason::PathNotAllowed,
        WorkspacePathError::PermissionDenied => ListDirectoryErrorReason::PermissionDenied,
        WorkspacePathError::IoError => ListDirectoryErrorReason::IoError,
    }
}

fn human_workspace_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    if let Some(unc_path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc_path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

fn entry_kind(file_type: &fs::FileType) -> DirectoryEntryKind {
    if file_type.is_dir() {
        DirectoryEntryKind::Directory
    } else if file_type.is_file() {
        DirectoryEntryKind::File
    } else if file_type.is_symlink() {
        DirectoryEntryKind::Symlink
    } else {
        DirectoryEntryKind::Other
    }
}

fn entry_kind_rank(kind: DirectoryEntryKind) -> u8 {
    match kind {
        DirectoryEntryKind::Directory => 0,
        DirectoryEntryKind::File => 1,
        DirectoryEntryKind::Symlink | DirectoryEntryKind::Other => 2,
    }
}

fn file_size_kb(bytes: u64) -> f64 {
    (bytes as f64 / 1024.0 * 100.0).round() / 100.0
}

fn size_error_reason(error: &io::Error) -> DirectoryEntrySizeError {
    match error.kind() {
        io::ErrorKind::PermissionDenied => DirectoryEntrySizeError::PermissionDenied,
        _ => DirectoryEntrySizeError::IoError,
    }
}

fn io_reason(error: &io::Error) -> ListDirectoryErrorReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ListDirectoryErrorReason::PermissionDenied,
        io::ErrorKind::NotFound => ListDirectoryErrorReason::InvalidPath,
        _ => ListDirectoryErrorReason::IoError,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
    };

    use super::*;

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn context(path: &Path) -> ToolContext {
        ToolContext::new(path.canonicalize().unwrap()).unwrap()
    }
    #[test]
    fn list_directory_returns_only_sorted_immediate_children() {
        let workspace = temp_root("list-workspace");
        fs::create_dir_all(workspace.join("zeta")).unwrap();
        fs::create_dir_all(workspace.join("alpha")).unwrap();
        fs::write(workspace.join("middle.txt"), "file").unwrap();
        fs::write(workspace.join("alpha/nested.txt"), "nested").unwrap();

        let result = ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: ".".to_owned(),
            },
        );

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "status": "ok",
                "path": ".",
                "workspace_path": super::human_workspace_path(&workspace.canonicalize().unwrap()),
                "entries": [
                    { "name": "alpha", "path": "alpha", "kind": "directory" },
                    { "name": "zeta", "path": "zeta", "kind": "directory" },
                    { "name": "middle.txt", "path": "middle.txt", "kind": "file", "size_kb": 0.0 }
                ],
                "truncated": false
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn list_directory_reports_sizes_for_every_regular_file_in_kilobytes() {
        let workspace = temp_root("list-file-sizes");
        fs::create_dir_all(workspace.join("source")).unwrap();
        fs::write(workspace.join("source.rs"), vec![b'a'; 1024]).unwrap();
        fs::write(workspace.join("program.exe"), vec![0u8; 1536]).unwrap();
        fs::write(workspace.join("weights.bin"), vec![0u8; 2048]).unwrap();

        let value = serde_json::to_value(ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: ".".to_owned(),
            },
        ))
        .unwrap();
        let entries = value["entries"].as_array().unwrap();
        let entry = |name: &str| {
            entries
                .iter()
                .find(|entry| entry["name"] == name)
                .unwrap_or_else(|| panic!("missing entry {name}"))
        };

        assert_eq!(entry("source.rs")["size_kb"], 1.0);
        assert_eq!(entry("program.exe")["size_kb"], 1.5);
        assert_eq!(entry("weights.bin")["size_kb"], 2.0);
        assert!(!entry("source").as_object().unwrap().contains_key("size_kb"));
        assert!(
            !entry("source")
                .as_object()
                .unwrap()
                .contains_key("size_error")
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn list_directory_rejects_absolute_and_traversal_paths() {
        let workspace = temp_root("list-workspace");
        let sibling = temp_root("list-sibling");
        let absolute = sibling.to_string_lossy().into_owned();

        for (path, reason) in [
            (absolute.as_str(), "invalid_path"),
            ("../list-sibling", "path_not_allowed"),
            ("src/../other", "path_not_allowed"),
            ("named:stream", "invalid_path"),
        ] {
            let result = ListDirectory.execute(
                &context(&workspace),
                ListDirectoryInput {
                    path: path.to_owned(),
                },
            );
            let value = serde_json::to_value(result).unwrap();
            assert_eq!(value["status"], "error");
            assert_eq!(value["reason"], reason);
            assert_eq!(value["path"], path);
        }
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(sibling).unwrap();
    }

    #[test]
    fn list_directory_rejects_files_and_caps_results_at_one_hundred() {
        let workspace = temp_root("list-workspace");
        fs::write(workspace.join("not-a-directory.txt"), "file").unwrap();
        for number in 0..101 {
            fs::write(workspace.join(format!("file-{number:03}.txt")), "file").unwrap();
        }

        let file_result = ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: "not-a-directory.txt".to_owned(),
            },
        );
        assert_eq!(
            serde_json::to_value(file_result).unwrap(),
            serde_json::json!({
                "status": "error",
                "reason": "not_directory",
                "path": "not-a-directory.txt"
            })
        );

        let directory_result = ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: ".".to_owned(),
            },
        );
        let value = serde_json::to_value(directory_result).unwrap();
        assert_eq!(value["entries"].as_array().unwrap().len(), 100);
        assert_eq!(value["truncated"], true);
        assert!(value["entries"].as_array().unwrap().iter().all(|entry| {
            entry["path"]
                .as_str()
                .is_some_and(|path| !Path::new(path).is_absolute())
        }));
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn list_directory_input_rejects_extra_fields() {
        assert!(
            serde_json::from_value::<ListDirectoryInput>(serde_json::json!({
                "path": ".",
                "recursive": true
            }))
            .is_err()
        );
    }

    #[test]
    fn list_directory_size_errors_use_stable_codes() {
        assert_eq!(
            serde_json::to_value(super::size_error_reason(&io::Error::new(
                io::ErrorKind::PermissionDenied,
                "denied",
            )))
            .unwrap(),
            "permission_denied"
        );
        assert_eq!(
            serde_json::to_value(super::size_error_reason(&io::Error::other(
                "metadata failed",
            )))
            .unwrap(),
            "io_error"
        );
    }

    #[cfg(windows)]
    #[test]
    fn list_directory_blocks_symlink_escapes_and_reports_internal_symlinks() {
        use std::os::windows::fs::symlink_dir;

        let workspace = temp_root("list-workspace");
        let outside = temp_root("list-outside");
        let internal = workspace.join("internal");
        fs::create_dir_all(&internal).unwrap();
        let outside_link = workspace.join("outside-link");
        let internal_link = workspace.join("internal-link");
        match symlink_dir(&outside, &outside_link)
            .and_then(|_| symlink_dir(&internal, &internal_link))
        {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(workspace).unwrap();
                fs::remove_dir_all(outside).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }

        let escaped = ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: "outside-link".to_owned(),
            },
        );
        assert_eq!(
            serde_json::to_value(escaped).unwrap()["reason"],
            "path_not_allowed"
        );
        let listed = serde_json::to_value(ListDirectory.execute(
            &context(&workspace),
            ListDirectoryInput {
                path: ".".to_owned(),
            },
        ))
        .unwrap();
        assert!(listed["entries"].as_array().unwrap().iter().any(|entry| {
            entry["name"] == "internal-link"
                && entry["kind"] == "symlink"
                && !entry.as_object().unwrap().contains_key("size_kb")
                && entry["size_error"] == "symlink_not_followed"
        }));
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
