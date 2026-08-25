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
pub(super) struct ListDirectoryInput {
    pub(super) path: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum ListDirectoryResult {
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
pub(super) struct ListDirectoryEntry {
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
pub(super) enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum DirectoryEntrySizeError {
    PermissionDenied,
    IoError,
    SymlinkNotFollowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ListDirectoryErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotDirectory,
    PermissionDenied,
    IoError,
}

pub(super) struct ListDirectory;

impl ListDirectory {
    pub(super) fn execute(
        &self,
        context: &ToolContext,
        input: ListDirectoryInput,
    ) -> ListDirectoryResult {
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

pub(super) fn human_workspace_path(path: &Path) -> String {
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

pub(super) fn size_error_reason(error: &io::Error) -> DirectoryEntrySizeError {
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
