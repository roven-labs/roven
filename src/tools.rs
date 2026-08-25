//! Roven-owned tool definitions, dispatch, and deterministic tool execution.

use std::{
    fs,
    io::{self, Read},
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::storage::{ProjectRegistration, ProjectRegistry, RegistrationLookup};

pub(crate) const PREPARE_PROJECT_DESCRIPTION: &str = "Validate and register the currently trusted project for first-time use with Roven, or replace its concise `summary` section after registration. Pass `.` as the path for the current trusted workspace on every call. Registration validates the project path, existing Roven registration, Git repository, GitHub remote, committed baseline, and clean working state, then stores the minimal project registration. Section updates accept only section_name `summary`, text, and operation `replace`; they update local Roven registration data and do not inspect or modify the project repository.";
pub(crate) const LIST_DIRECTORY_DESCRIPTION: &str = "List the immediate contents of a directory inside the currently trusted Roven workspace. Use this when you need to inspect workspace structure or locate a file or subdirectory before calling another filesystem tool. Pass a workspace-relative directory path such as `.` or `src`; do not pass an absolute path or a path containing `..`. Returns up to 100 immediate entries in deterministic order with `status`, `path`, `workspace_path`, `entries`, and `truncated`; if more entries exist, `truncated` is true. Each entry includes `name`, workspace-relative `path`, and `kind`. Every regular file also includes `size_kb`, measured as bytes divided by 1024 and rounded to two decimal places. Directories and other entries omit size fields. Symlinks are not followed and include `size_error: \"symlink_not_followed\"`; regular-file metadata failures keep the entry and include `size_error: \"permission_denied\"` or \"io_error\". For `invalid_path` or `path_not_allowed`, retry with a relative path under the workspace; for `not_directory`, pass a directory path. This tool does not read file contents, search recursively, modify files, register projects, or access paths outside the trusted workspace.";
pub(crate) const READ_FILE_DESCRIPTION: &str = "Read a known workspace-relative text file after locating it with `list_directory`. Paths are relative to the trusted workspace. This tool reads only regular UTF-8 text files up to 50 KiB and does not modify files or access paths outside the trusted workspace.";
pub(crate) const LIST_TOOLS_DESCRIPTION: &str = "List the Roven tools available to you in this turn, with their exact descriptions and input schemas. Use this when you need to check which Roven capabilities are currently available before selecting a tool. This reports the live Roven tool registry and does not access the workspace or modify anything.";
pub(crate) const LIST_PROJECT_DESCRIPTION: &str = "List the projects currently registered with Roven. Use this when the user asks which stored projects exist. Takes no arguments and returns only project names in deterministic alphabetical order; an empty registry returns an empty projects array. This does not inspect project directories or modify storage.";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolDefinition {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

pub(crate) fn definitions() -> Vec<RovenToolDefinition> {
    vec![
        RovenToolDefinition {
            name: "prepare_project".to_owned(),
            description: PREPARE_PROJECT_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the currently trusted project directory."
                    },
                    "section_name": {
                        "type": "string",
                        "enum": ["summary"],
                        "description": "Registration section to replace; version one accepts only summary."
                    },
                    "text": {
                        "type": "string",
                        "description": "Non-empty concise report text for the selected section."
                    },
                    "operation": {
                        "type": "string",
                        "enum": ["replace"],
                        "description": "Update operation; version one accepts only replace."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        RovenToolDefinition {
            name: "list_directory".to_owned(),
            description: LIST_DIRECTORY_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path; use `.` for the workspace root."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        RovenToolDefinition {
            name: "read_file".to_owned(),
            description: READ_FILE_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative text file path."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        RovenToolDefinition {
            name: "list_tools".to_owned(),
            description: LIST_TOOLS_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
        RovenToolDefinition {
            name: "list_project".to_owned(),
            description: LIST_PROJECT_DESCRIPTION.to_owned(),
            input_schema: json!({
                "type": "object",
                "properties": {},
                "additionalProperties": false
            }),
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RovenToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolResult {
    pub(crate) tool_call_id: String,
    pub(crate) name: String,
    pub(crate) result: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolContext {
    trusted_workspace: PathBuf,
}

impl ToolContext {
    pub(crate) fn new(trusted_workspace: PathBuf) -> io::Result<Self> {
        let trusted_workspace = trusted_workspace.canonicalize()?;
        Ok(Self { trusted_workspace })
    }
}

pub(crate) fn dispatch(context: &ToolContext, call: RovenToolCall) -> RovenToolResult {
    let result = match call.name.as_str() {
        "prepare_project" => {
            let has_null_section_field = ["section_name", "text", "operation"]
                .iter()
                .any(|key| call.arguments.get(*key).is_some_and(Value::is_null));
            if has_null_section_field {
                serde_json::to_value(PrepareProjectResult::blocked(
                    BlockedReason::InvalidSectionUpdate,
                ))
            } else {
                match serde_json::from_value::<PrepareProjectInput>(call.arguments) {
                    Ok(input) => serde_json::to_value(
                        PrepareProject::for_current_user().execute(context, input),
                    ),
                    Err(_) => serde_json::to_value(PrepareProjectResult::blocked(
                        BlockedReason::InvalidPath,
                    )),
                }
            }
        }
        "list_directory" => match serde_json::from_value::<ListDirectoryInput>(call.arguments) {
            Ok(input) => serde_json::to_value(ListDirectory.execute(context, input)),
            Err(_) => serde_json::to_value(ListDirectoryResult::error(
                ListDirectoryErrorReason::InvalidPath,
                "",
            )),
        },
        "read_file" => match serde_json::from_value::<ReadFileInput>(call.arguments) {
            Ok(input) => serde_json::to_value(ReadFile.execute(context, input)),
            Err(_) => {
                serde_json::to_value(ReadFileResult::error(ReadFileErrorReason::InvalidPath, ""))
            }
        },
        "list_tools" => match serde_json::from_value::<ListToolsInput>(call.arguments) {
            Ok(_) => serde_json::to_value(ListToolsResult::Ok {
                tools: definitions(),
            }),
            Err(_) => serde_json::to_value(ListToolsResult::InvalidInput),
        },
        "list_project" => match serde_json::from_value::<ListProjectInput>(call.arguments) {
            Ok(_) => serde_json::to_value(ListProject::for_current_user().execute()),
            Err(_) => serde_json::to_value(ListProjectResult::InvalidInput),
        },
        _ => Ok(json!({ "status": "error", "reason": "unknown_tool" })),
    };
    RovenToolResult {
        tool_call_id: call.id,
        name: call.name,
        result: result.expect("tool results are serializable"),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListToolsInput {}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProjectInput {}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListProjectResult {
    Ok { projects: Vec<String> },
    Error { reason: ListProjectErrorReason },
    InvalidInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ListProjectErrorReason {
    StorageFailure,
}

struct ListProject {
    registry: Result<ProjectRegistry, ()>,
}

impl ListProject {
    fn for_current_user() -> Self {
        Self {
            registry: ProjectRegistry::for_current_user().map_err(|_| ()),
        }
    }

    fn execute(&self) -> ListProjectResult {
        let Ok(registry) = &self.registry else {
            return ListProjectResult::Error {
                reason: ListProjectErrorReason::StorageFailure,
            };
        };
        match registry.list() {
            Ok(projects) => ListProjectResult::Ok {
                projects: projects.into_iter().map(|project| project.name).collect(),
            },
            Err(_) => ListProjectResult::Error {
                reason: ListProjectErrorReason::StorageFailure,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListToolsResult {
    Ok { tools: Vec<RovenToolDefinition> },
    InvalidInput,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrepareProjectInput {
    pub(crate) path: String,
    pub(crate) section_name: Option<String>,
    pub(crate) text: Option<String>,
    pub(crate) operation: Option<String>,
}

const DIRECTORY_LIST_LIMIT: usize = 100;
const READ_FILE_SIZE_LIMIT: u64 = 50 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListDirectoryInput {
    pub(crate) path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ReadFileInput {
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ReadFileResult {
    Ok {
        path: String,
        content: String,
    },
    Error {
        reason: ReadFileErrorReason,
        path: String,
    },
}

impl ReadFileResult {
    fn error(reason: ReadFileErrorReason, path: impl Into<String>) -> Self {
        Self::Error {
            reason,
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ReadFileErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotFile,
    FileTooLarge,
    NotText,
    PermissionDenied,
    IoError,
}

pub(crate) struct ReadFile;

impl ReadFile {
    pub(crate) fn execute(&self, context: &ToolContext, input: ReadFileInput) -> ReadFileResult {
        let target = match resolve_workspace_file(context, &input.path) {
            Ok(path) => path,
            Err(reason) => return ReadFileResult::error(reason, input.path),
        };
        let path = workspace_relative_path(&context.trusted_workspace, &target);
        let mut file = match open_workspace_file(context, &target) {
            Ok(file) => file,
            Err(reason) => return ReadFileResult::error(reason, path),
        };
        let metadata = match file.metadata() {
            Ok(metadata) => metadata,
            Err(error) => return ReadFileResult::error(read_file_io_reason(&error), path),
        };
        if !metadata.is_file() {
            return ReadFileResult::error(ReadFileErrorReason::NotFile, path);
        }
        if metadata.len() > READ_FILE_SIZE_LIMIT {
            return ReadFileResult::error(ReadFileErrorReason::FileTooLarge, path);
        }
        match read_file_contents(&mut file) {
            Ok(content) => ReadFileResult::Ok { path, content },
            Err(reason) => ReadFileResult::error(reason, path),
        }
    }
}

fn read_file_contents(file: &mut fs::File) -> Result<String, ReadFileErrorReason> {
    let mut bytes = Vec::new();
    file.take(READ_FILE_SIZE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| read_file_io_reason(&error))?;
    if bytes.len() > READ_FILE_SIZE_LIMIT as usize {
        return Err(ReadFileErrorReason::FileTooLarge);
    }
    String::from_utf8(bytes).map_err(|_| ReadFileErrorReason::NotText)
}

#[cfg(windows)]
fn open_workspace_file(
    context: &ToolContext,
    path: &Path,
) -> Result<fs::File, ReadFileErrorReason> {
    use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .map_err(|error| read_file_io_reason(&error))?;
    let attributes = file
        .metadata()
        .map_err(|error| read_file_io_reason(&error))?
        .file_attributes();
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(ReadFileErrorReason::PathNotAllowed);
    }
    if !opened_path_is_within_workspace(&opened_path(&file)?, &context.trusted_workspace)? {
        return Err(ReadFileErrorReason::PathNotAllowed);
    }
    Ok(file)
}

#[cfg(not(windows))]
fn open_workspace_file(
    _context: &ToolContext,
    path: &Path,
) -> Result<fs::File, ReadFileErrorReason> {
    fs::File::open(path).map_err(|error| read_file_io_reason(&error))
}

#[cfg(windows)]
fn opened_path(file: &fs::File) -> Result<std::ffi::OsString, ReadFileErrorReason> {
    use std::{
        ffi::{OsString, c_void},
        os::windows::{ffi::OsStringExt, io::AsRawHandle},
    };

    unsafe extern "system" {
        fn GetFinalPathNameByHandleW(
            file: *mut c_void,
            path: *mut u16,
            path_len: u32,
            flags: u32,
        ) -> u32;
    }

    let mut path_len = 260;
    loop {
        let mut buffer = vec![0; path_len as usize];
        let result = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), buffer.as_mut_ptr(), path_len, 0)
        };
        if result == 0 {
            return Err(ReadFileErrorReason::IoError);
        }
        if result < path_len {
            return Ok(OsString::from_wide(&buffer[..result as usize]));
        }
        path_len = result.checked_add(1).ok_or(ReadFileErrorReason::IoError)?;
    }
}

#[cfg(windows)]
fn opened_path_is_within_workspace(
    opened_path: &std::ffi::OsStr,
    trusted_workspace: &Path,
) -> Result<bool, ReadFileErrorReason> {
    use std::os::windows::ffi::OsStrExt;

    const CSTR_EQUAL: i32 = 2;

    unsafe extern "system" {
        fn CompareStringOrdinal(
            string1: *const u16,
            string1_len: i32,
            string2: *const u16,
            string2_len: i32,
            ignore_case: i32,
        ) -> i32;
    }

    let opened_path: Vec<u16> = opened_path.encode_wide().collect();
    let trusted_workspace: Vec<u16> = trusted_workspace.as_os_str().encode_wide().collect();
    if opened_path.len() < trusted_workspace.len() {
        return Ok(false);
    }
    let trusted_len =
        i32::try_from(trusted_workspace.len()).map_err(|_| ReadFileErrorReason::IoError)?;
    let comparison = unsafe {
        CompareStringOrdinal(
            opened_path.as_ptr(),
            trusted_len,
            trusted_workspace.as_ptr(),
            trusted_len,
            1,
        )
    };
    if comparison == 0 {
        return Err(ReadFileErrorReason::IoError);
    }
    let has_component_boundary = opened_path
        .get(trusted_workspace.len())
        .is_some_and(|character| *character == b'\\' as u16 || *character == b'/' as u16);
    let trusted_root_ends_with_separator = trusted_workspace
        .last()
        .is_some_and(|character| *character == b'\\' as u16 || *character == b'/' as u16);
    Ok(comparison == CSTR_EQUAL
        && (opened_path.len() == trusted_workspace.len()
            || trusted_root_ends_with_separator
            || has_component_boundary))
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum ListDirectoryResult {
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
pub(crate) struct ListDirectoryEntry {
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
pub(crate) enum DirectoryEntryKind {
    File,
    Directory,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum DirectoryEntrySizeError {
    PermissionDenied,
    IoError,
    SymlinkNotFollowed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ListDirectoryErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotDirectory,
    PermissionDenied,
    IoError,
}

pub(crate) struct ListDirectory;

impl ListDirectory {
    pub(crate) fn execute(
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
    let relative = Path::new(path);
    if relative
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(ListDirectoryErrorReason::InvalidPath);
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ListDirectoryErrorReason::PathNotAllowed);
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':')))
    {
        return Err(ListDirectoryErrorReason::InvalidPath);
    }
    let target = context
        .trusted_workspace
        .join(relative)
        .canonicalize()
        .map_err(|error| io_reason(&error))?;
    if !target.starts_with(&context.trusted_workspace) {
        return Err(ListDirectoryErrorReason::PathNotAllowed);
    }
    target
        .is_dir()
        .then_some(target)
        .ok_or(ListDirectoryErrorReason::NotDirectory)
}

fn resolve_workspace_file(
    context: &ToolContext,
    path: &str,
) -> Result<PathBuf, ReadFileErrorReason> {
    let relative = Path::new(path);
    if relative
        .components()
        .any(|component| matches!(component, Component::Prefix(_) | Component::RootDir))
    {
        return Err(ReadFileErrorReason::InvalidPath);
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ReadFileErrorReason::PathNotAllowed);
    }
    if relative
        .components()
        .any(|component| matches!(component, Component::Normal(name) if name.to_string_lossy().contains(':')))
    {
        return Err(ReadFileErrorReason::InvalidPath);
    }
    let target = context
        .trusted_workspace
        .join(relative)
        .canonicalize()
        .map_err(|error| read_file_io_reason(&error))?;
    if !target.starts_with(&context.trusted_workspace) {
        return Err(ReadFileErrorReason::PathNotAllowed);
    }
    Ok(target)
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    let relative = path
        .strip_prefix(root)
        .expect("authorized path remains under trusted workspace");
    workspace_relative_path_from_relative(relative)
}

fn workspace_relative_path_from_relative(path: &Path) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    if path.is_empty() {
        ".".to_owned()
    } else {
        path
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

fn read_file_io_reason(error: &io::Error) -> ReadFileErrorReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ReadFileErrorReason::PermissionDenied,
        io::ErrorKind::NotFound => ReadFileErrorReason::InvalidPath,
        _ => ReadFileErrorReason::IoError,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum PrepareProjectResult {
    Prepared { project: PreparedProject },
    AlreadyAdded { project: ExistingProject },
    SummarySaved { project: ExistingProject },
    Blocked { reason: BlockedReason },
}

impl PrepareProjectResult {
    fn blocked(reason: BlockedReason) -> Self {
        Self::Blocked { reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PreparedProject {
    name: String,
    path: String,
    github_remote: String,
    baseline_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExistingProject {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum BlockedReason {
    InvalidPath,
    PathNotAllowed,
    GitUnavailable,
    NotGitRepository,
    NoCommitBaseline,
    NoGithubRemote,
    RepositoryNotClean,
    StorageFailure,
    InvalidSectionUpdate,
    NotRegistered,
}

pub(crate) struct PrepareProject {
    registry: Result<ProjectRegistry, ()>,
}

impl PrepareProject {
    pub(crate) fn for_current_user() -> Self {
        Self {
            registry: ProjectRegistry::for_current_user().map_err(|_| ()),
        }
    }

    #[cfg(test)]
    fn for_data_root(data_root: &Path) -> Self {
        Self {
            registry: Ok(ProjectRegistry::for_data_root(data_root)),
        }
    }

    pub(crate) fn execute(
        &self,
        context: &ToolContext,
        input: PrepareProjectInput,
    ) -> PrepareProjectResult {
        let project_path = match canonical_project_path(context, &input.path) {
            Some(path) => path,
            None => return PrepareProjectResult::blocked(BlockedReason::InvalidPath),
        };
        if project_path != context.trusted_workspace {
            return PrepareProjectResult::blocked(BlockedReason::PathNotAllowed);
        }
        let registry = match &self.registry {
            Ok(registry) => registry,
            Err(()) => return PrepareProjectResult::blocked(BlockedReason::StorageFailure),
        };
        match (input.section_name, input.text, input.operation) {
            (None, None, None) => {}
            (Some(section_name), Some(text), Some(operation)) => {
                if section_name != "summary" || operation != "replace" || text.trim().is_empty() {
                    return PrepareProjectResult::blocked(BlockedReason::InvalidSectionUpdate);
                }
                return match registry.replace_section(&project_path, &section_name, &text) {
                    Ok(Some(registration)) => PrepareProjectResult::SummarySaved {
                        project: existing_project(registration),
                    },
                    Ok(None) => PrepareProjectResult::blocked(BlockedReason::NotRegistered),
                    Err(_) => PrepareProjectResult::blocked(BlockedReason::StorageFailure),
                };
            }
            _ => return PrepareProjectResult::blocked(BlockedReason::InvalidSectionUpdate),
        }
        match registry.lookup(&project_path) {
            Ok(RegistrationLookup::Registered(registration)) => {
                return PrepareProjectResult::AlreadyAdded {
                    project: existing_project(*registration),
                };
            }
            Ok(RegistrationLookup::Absent) => {}
            Err(_) => return PrepareProjectResult::blocked(BlockedReason::StorageFailure),
        }
        if !git_available() {
            return PrepareProjectResult::blocked(BlockedReason::GitUnavailable);
        }
        if !git_is_repository(&project_path) {
            return PrepareProjectResult::blocked(BlockedReason::NotGitRepository);
        }
        let baseline_commit = match git_head_commit(&project_path) {
            Some(commit) => commit,
            None => return PrepareProjectResult::blocked(BlockedReason::NoCommitBaseline),
        };
        let github_remote = match git_github_remote(&project_path) {
            Some(remote) => remote,
            None => return PrepareProjectResult::blocked(BlockedReason::NoGithubRemote),
        };
        if !git_is_clean(&project_path) || git_operation_in_progress(&project_path) {
            return PrepareProjectResult::blocked(BlockedReason::RepositoryNotClean);
        }
        match registry.register(&project_path, github_remote, baseline_commit) {
            Ok(registration) => PrepareProjectResult::Prepared {
                project: prepared_project(registration),
            },
            Err(_) => PrepareProjectResult::blocked(BlockedReason::StorageFailure),
        }
    }
}

/// Resolves a registration target against the launch-scoped trusted workspace.
/// The caller must still require exact equality with that workspace before any
/// project lookup, Git inspection, or registration write.
fn canonical_project_path(context: &ToolContext, path: &str) -> Option<PathBuf> {
    let requested = Path::new(path);
    let resolved = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        context.trusted_workspace.join(requested)
    };
    let canonical = resolved.canonicalize().ok()?;
    canonical.is_dir().then_some(canonical)
}

fn existing_project(registration: ProjectRegistration) -> ExistingProject {
    ExistingProject {
        name: registration.name,
        path: registration.canonical_path,
    }
}

fn prepared_project(registration: ProjectRegistration) -> PreparedProject {
    PreparedProject {
        name: registration.name,
        path: registration.canonical_path,
        github_remote: registration.github_remote,
        baseline_commit: registration.baseline_commit,
    }
}

fn git_available() -> bool {
    git_output(None, &["--version"]).is_some()
}

fn git_is_repository(project_path: &Path) -> bool {
    git_output(Some(project_path), &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|output| output.trim() == "true")
}

fn git_head_commit(project_path: &Path) -> Option<String> {
    git_output(
        Some(project_path),
        &["rev-parse", "--verify", "HEAD^{commit}"],
    )
    .map(|output| output.trim().to_owned())
    .filter(|output| !output.is_empty())
}

fn git_github_remote(project_path: &Path) -> Option<String> {
    let remotes = git_output(Some(project_path), &["remote"])?;
    let remotes = remotes
        .lines()
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    if let Some(origin) = remotes.iter().find(|name| **name == "origin") {
        candidates.push(*origin);
    }
    candidates.extend(remotes.into_iter().filter(|name| *name != "origin"));
    for remote in candidates {
        let fetch = git_output(Some(project_path), &["remote", "get-url", remote]);
        let push = git_output(Some(project_path), &["remote", "get-url", "--push", remote]);
        if let Some(url) = fetch.filter(|url| is_github_url(url.trim())) {
            return Some(url.trim().to_owned());
        }
        if let Some(url) = push.filter(|url| is_github_url(url.trim())) {
            return Some(url.trim().to_owned());
        }
    }
    None
}

fn git_is_clean(project_path: &Path) -> bool {
    git_output(
        Some(project_path),
        &["status", "--porcelain=v1", "--untracked-files=all"],
    )
    .is_some_and(|output| !has_blocking_status(&output))
}

fn git_operation_in_progress(project_path: &Path) -> bool {
    [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "REBASE_HEAD",
        "BISECT_LOG",
        "rebase-apply",
        "rebase-merge",
    ]
    .iter()
    .any(|marker| git_path_exists(project_path, marker))
}

fn git_output(project_path: Option<&Path>, arguments: &[&str]) -> Option<String> {
    let mut command = Command::new("git");
    if let Some(project_path) = project_path {
        command.arg("-C").arg(project_path);
    }
    let output = command.args(arguments).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

fn git_path_exists(project_path: &Path, marker: &str) -> bool {
    let Some(path) = git_output(Some(project_path), &["rev-parse", "--git-path", marker]) else {
        return false;
    };
    let path = PathBuf::from(path.trim());
    let path = if path.is_absolute() {
        path
    } else {
        project_path.join(path)
    };
    path.exists()
}

fn has_blocking_status(status: &str) -> bool {
    status.lines().any(|line| !line.trim().is_empty())
}

fn is_github_url(url: &str) -> bool {
    let url = url.trim().to_ascii_lowercase();
    let authority = if let Some((_, remainder)) = url.split_once("://") {
        remainder.split('/').next().unwrap_or_default()
    } else {
        url.split(':').next().unwrap_or_default()
    };
    let host = authority
        .rsplit('@')
        .next()
        .unwrap_or_default()
        .split(':')
        .next()
        .unwrap_or_default();
    host == "github.com"
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    use serde_json::json;

    use crate::storage::{ProjectRegistry, RegistrationLookup};

    use super::{
        BlockedReason, LIST_DIRECTORY_DESCRIPTION, LIST_PROJECT_DESCRIPTION, ListDirectory,
        ListDirectoryInput, PrepareProject, PrepareProjectInput, PrepareProjectResult,
        READ_FILE_DESCRIPTION, ReadFile, ReadFileInput, RovenToolCall, ToolContext, definitions,
        dispatch,
    };

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn context(path: &Path) -> ToolContext {
        ToolContext::new(path.canonicalize().unwrap()).unwrap()
    }

    fn input(path: &Path) -> PrepareProjectInput {
        PrepareProjectInput {
            path: path.to_string_lossy().into_owned(),
            section_name: None,
            text: None,
            operation: None,
        }
    }

    fn input_value(path: &str) -> PrepareProjectInput {
        PrepareProjectInput {
            path: path.to_owned(),
            section_name: None,
            text: None,
            operation: None,
        }
    }

    fn git(project: &Path, arguments: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(project)
            .args(arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("git should start");
        assert!(status.success(), "git {arguments:?} should succeed");
    }

    fn ready_project(name: &str) -> PathBuf {
        let project = temp_root(name);
        git(&project, &["init"]);
        git(
            &project,
            &[
                "-c",
                "user.name=Roven Test",
                "-c",
                "user.email=roven@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        git(
            &project,
            &[
                "remote",
                "add",
                "origin",
                "git@github.com:roven/example.git",
            ],
        );
        project
    }

    #[test]
    fn prepare_project_blocks_a_sibling_before_git_or_registration() {
        let data = temp_root("prepare-data");
        let parent = temp_root("project-parent");
        let trusted = parent.join("project-one");
        let sibling = parent.join("project-two");
        fs::create_dir_all(&trusted).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let registry = ProjectRegistry::for_data_root(&data);
        let existing = registry
            .register(
                &sibling,
                "https://github.com/roven/project-two".to_owned(),
                "existing-baseline".to_owned(),
            )
            .unwrap();
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&trusted), input(&sibling));

        assert_eq!(
            result,
            PrepareProjectResult::Blocked {
                reason: BlockedReason::PathNotAllowed
            }
        );
        assert_eq!(
            registry.lookup(&sibling).unwrap(),
            RegistrationLookup::Registered(Box::new(existing))
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn prepare_project_resolves_dot_against_the_trusted_workspace() {
        let data = temp_root("prepare-data");
        let trusted = ready_project("trusted");
        let registry = ProjectRegistry::for_data_root(&data);
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&trusted), input_value("."));

        assert!(matches!(result, PrepareProjectResult::Prepared { .. }));
        assert!(matches!(
            registry.lookup(&trusted).unwrap(),
            RegistrationLookup::Registered(_)
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(trusted).unwrap();
    }

    #[test]
    fn prepare_project_blocks_parent_directory_escapes_before_git_or_registration() {
        let data = temp_root("prepare-data");
        let parent = temp_root("project-parent");
        let trusted = parent.join("project-one");
        let sibling = parent.join("project-two");
        fs::create_dir_all(&trusted).unwrap();
        fs::create_dir_all(&sibling).unwrap();
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&trusted), input_value("../project-two"));

        assert_eq!(
            result,
            PrepareProjectResult::Blocked {
                reason: BlockedReason::PathNotAllowed
            }
        );
        assert!(!data.join("projects").exists());
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(parent).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn prepare_project_blocks_symlinks_that_resolve_outside_the_trusted_workspace() {
        use std::os::windows::fs::symlink_dir;

        let data = temp_root("prepare-data");
        let trusted = temp_root("trusted");
        let outside = temp_root("outside");
        let outside_link = trusted.join("outside-link");
        match symlink_dir(&outside, &outside_link) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(data).unwrap();
                fs::remove_dir_all(trusted).unwrap();
                fs::remove_dir_all(outside).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&trusted), input(&outside_link));

        assert_eq!(
            result,
            PrepareProjectResult::Blocked {
                reason: BlockedReason::PathNotAllowed
            }
        );
        assert!(!data.join("projects").exists());
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(trusted).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn prepare_project_accepts_the_exact_trusted_workspace_path() {
        let data = temp_root("prepare-data");
        let project = ready_project("project");
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&project), input(&project));
        let value = serde_json::to_value(result).unwrap();

        assert_eq!(value["status"], "prepared");
        assert_eq!(
            value["project"]["path"],
            project.canonicalize().unwrap().to_string_lossy().as_ref()
        );
        assert_eq!(
            value["project"]["github_remote"],
            "git@github.com:roven/example.git"
        );
        assert!(
            value["project"]["baseline_commit"]
                .as_str()
                .is_some_and(|commit| !commit.is_empty())
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn already_registered_project_skips_git_validation() {
        let data = temp_root("prepare-data");
        let project = temp_root("project");
        let registry = ProjectRegistry::for_data_root(&data);
        registry
            .register(
                &project,
                "https://github.com/roven/example".to_owned(),
                "abc123".to_owned(),
            )
            .unwrap();
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&project), input(&project));

        assert_eq!(
            serde_json::to_value(result).unwrap()["status"],
            "already_added"
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn repository_state_blocks_registration() {
        let data = temp_root("prepare-data");
        let project = ready_project("project");
        fs::write(project.join("untracked.txt"), "untracked").unwrap();
        let tool = PrepareProject::for_data_root(&data);

        let result = tool.execute(&context(&project), input(&project));

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "blocked",
                "reason": "repository_not_clean"
            })
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn prepare_project_input_rejects_values_beyond_the_project_path() {
        assert!(
            serde_json::from_value::<PrepareProjectInput>(serde_json::json!({
                "path": "C:/project",
                "unexpected": true
            }))
            .is_err()
        );
    }

    #[test]
    fn prepare_project_schema_allows_only_the_summary_replace_update() {
        let prepare_project = definitions()
            .into_iter()
            .find(|definition| definition.name == "prepare_project")
            .expect("prepare_project must be registered");
        assert_eq!(
            prepare_project.input_schema["properties"]["section_name"]["enum"],
            json!(["summary"])
        );
        assert_eq!(
            prepare_project.input_schema["properties"]["operation"]["enum"],
            json!(["replace"])
        );
        assert_eq!(prepare_project.input_schema["required"], json!(["path"]));
        assert_eq!(prepare_project.input_schema["additionalProperties"], false);
    }

    #[test]
    fn prepare_project_dispatch_blocks_explicit_null_section_fields() {
        let workspace = temp_root("null-section-workspace");
        let result = dispatch(
            &context(&workspace),
            RovenToolCall {
                id: "null-section".to_owned(),
                name: "prepare_project".to_owned(),
                arguments: json!({ "path": ".", "section_name": null }),
            },
        );

        assert_eq!(
            result.result,
            json!({
                "status": "blocked",
                "reason": "invalid_section_update"
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn summary_update_requires_a_registered_project_and_replaces_text() {
        let data = temp_root("summary-data");
        let project = ready_project("summary-project");
        let registry = ProjectRegistry::for_data_root(&data);
        let tool = PrepareProject::for_data_root(&data);
        let update = |text: &str| PrepareProjectInput {
            path: ".".to_owned(),
            section_name: Some("summary".to_owned()),
            text: Some(text.to_owned()),
            operation: Some("replace".to_owned()),
        };

        assert_eq!(
            tool.execute(&context(&project), update("report")),
            PrepareProjectResult::Blocked {
                reason: BlockedReason::NotRegistered
            }
        );
        assert!(matches!(
            tool.execute(&context(&project), input_value(".")),
            PrepareProjectResult::Prepared { .. }
        ));
        let result = tool.execute(&context(&project), update("report"));
        assert_eq!(
            serde_json::to_value(result).unwrap()["status"],
            "summary_saved"
        );
        let saved = registry.lookup(&project).unwrap();
        let RegistrationLookup::Registered(saved) = saved else {
            panic!("summary update should keep registration");
        };
        assert_eq!(saved.sections["summary"], "report");
        let registration_files = fs::read_dir(data.join("projects"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("json")
            })
            .count();
        assert_eq!(registration_files, 1);
        assert_eq!(
            tool.execute(
                &context(&project),
                PrepareProjectInput {
                    path: ".".to_owned(),
                    section_name: Some("summary".to_owned()),
                    text: Some("  ".to_owned()),
                    operation: Some("replace".to_owned()),
                },
            ),
            PrepareProjectResult::Blocked {
                reason: BlockedReason::InvalidSectionUpdate
            }
        );
        for invalid in [
            PrepareProjectInput {
                path: ".".to_owned(),
                section_name: Some("summary".to_owned()),
                text: Some("report".to_owned()),
                operation: None,
            },
            PrepareProjectInput {
                path: ".".to_owned(),
                section_name: Some("details".to_owned()),
                text: Some("report".to_owned()),
                operation: Some("replace".to_owned()),
            },
            PrepareProjectInput {
                path: ".".to_owned(),
                section_name: Some("summary".to_owned()),
                text: Some("report".to_owned()),
                operation: Some("append".to_owned()),
            },
        ] {
            assert_eq!(
                tool.execute(&context(&project), invalid),
                PrepareProjectResult::Blocked {
                    reason: BlockedReason::InvalidSectionUpdate
                }
            );
        }
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn preparation_requires_a_committed_repository_with_a_github_remote() {
        let data = temp_root("prepare-data");
        let tool = PrepareProject::for_data_root(&data);
        let plain_project = temp_root("plain-project");
        assert_eq!(
            tool.execute(&context(&plain_project), input(&plain_project)),
            PrepareProjectResult::Blocked {
                reason: BlockedReason::NotGitRepository
            }
        );

        let uncommitted_project = temp_root("uncommitted-project");
        git(&uncommitted_project, &["init"]);
        assert_eq!(
            tool.execute(&context(&uncommitted_project), input(&uncommitted_project)),
            PrepareProjectResult::Blocked {
                reason: BlockedReason::NoCommitBaseline
            }
        );

        let no_remote_project = temp_root("no-remote-project");
        git(&no_remote_project, &["init"]);
        git(
            &no_remote_project,
            &[
                "-c",
                "user.name=Roven Test",
                "-c",
                "user.email=roven@example.test",
                "commit",
                "--allow-empty",
                "-m",
                "initial",
            ],
        );
        assert_eq!(
            tool.execute(&context(&no_remote_project), input(&no_remote_project)),
            PrepareProjectResult::Blocked {
                reason: BlockedReason::NoGithubRemote
            }
        );

        for project in [&plain_project, &uncommitted_project, &no_remote_project] {
            assert!(matches!(
                ProjectRegistry::for_data_root(&data)
                    .lookup(project)
                    .unwrap(),
                RegistrationLookup::Absent
            ));
        }
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(plain_project).unwrap();
        fs::remove_dir_all(uncommitted_project).unwrap();
        fs::remove_dir_all(no_remote_project).unwrap();
    }

    #[test]
    fn git_remote_prefers_origin_then_another_github_fetch_remote() {
        let project = temp_root("git-remote");
        git(&project, &["init"]);
        git(
            &project,
            &[
                "remote",
                "add",
                "origin",
                "https://gitlab.com/roven/example.git",
            ],
        );
        git(
            &project,
            &[
                "remote",
                "add",
                "upstream",
                "https://github.com/roven/example.git",
            ],
        );

        assert_eq!(
            super::git_github_remote(&project),
            Some("https://github.com/roven/example.git".to_owned())
        );
        git(
            &project,
            &[
                "remote",
                "set-url",
                "--push",
                "origin",
                "git@github.com:roven/origin-push.git",
            ],
        );
        assert_eq!(
            super::git_github_remote(&project),
            Some("git@github.com:roven/origin-push.git".to_owned())
        );
        git(
            &project,
            &[
                "remote",
                "set-url",
                "origin",
                "git@github.com:roven/origin.git",
            ],
        );
        assert_eq!(
            super::git_github_remote(&project),
            Some("git@github.com:roven/origin.git".to_owned())
        );
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn git_clean_check_ignores_ignored_files_and_blocks_untracked_files() {
        let project = temp_root("git-clean");
        git(&project, &["init"]);
        fs::write(project.join(".gitignore"), "ignored.txt\n").unwrap();
        fs::write(project.join("ignored.txt"), "ignored").unwrap();

        assert!(
            !super::git_is_clean(&project),
            "the untracked .gitignore blocks preparation"
        );
        git(&project, &["add", ".gitignore"]);
        git(
            &project,
            &[
                "-c",
                "user.name=Roven Test",
                "-c",
                "user.email=roven@example.test",
                "commit",
                "-m",
                "ignore",
            ],
        );
        assert!(super::git_is_clean(&project));
        fs::write(project.join("untracked.txt"), "untracked").unwrap();
        assert!(!super::git_is_clean(&project));
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn git_status_is_blocked_when_any_change_is_present() {
        assert!(super::has_blocking_status("?? index.bin\n"));
        assert!(super::has_blocking_status(" M source.rs\n"));
        assert!(super::has_blocking_status("?? source.rs\n"));
    }

    #[test]
    fn github_remote_detection_accepts_standard_git_url_forms_only() {
        for url in [
            "https://github.com/roven/example.git",
            "ssh://git@github.com/roven/example.git",
            "git://github.com/roven/example.git",
            "git@github.com:roven/example.git",
        ] {
            assert!(super::is_github_url(url), "{url} should be accepted");
        }
        assert!(!super::is_github_url(
            "https://github.com.example/roven/example.git"
        ));
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

    #[test]
    fn read_file_returns_utf8_contents_from_the_trusted_workspace() {
        let workspace = temp_root("read-file");
        fs::write(workspace.join("notes.txt"), "first line\nsecond line\n").unwrap();

        let result = ReadFile.execute(
            &context(&workspace),
            ReadFileInput {
                path: "notes.txt".to_owned(),
            },
        );

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "ok",
                "path": "notes.txt",
                "content": "first line\nsecond line\n"
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn read_file_accepts_a_file_exactly_at_the_fifty_kibibyte_limit() {
        let workspace = temp_root("read-file-limit");
        let content = "x".repeat(50 * 1024);
        fs::write(workspace.join("limit.txt"), &content).unwrap();

        let result = ReadFile.execute(
            &context(&workspace),
            ReadFileInput {
                path: "limit.txt".to_owned(),
            },
        );

        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "ok",
                "path": "limit.txt",
                "content": content
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn read_file_caps_an_open_file_that_grows_after_metadata_is_observed() {
        use std::io::Write;

        let workspace = temp_root("read-file-growth");
        let path = workspace.join("notes.txt");
        fs::write(&path, "small").unwrap();
        let mut file = fs::File::open(&path).unwrap();
        assert_eq!(file.metadata().unwrap().len(), 5);

        let mut appender = fs::OpenOptions::new().append(true).open(&path).unwrap();
        appender.write_all(&vec![b'x'; 50 * 1024]).unwrap();
        drop(appender);

        assert_eq!(
            super::read_file_contents(&mut file),
            Err(super::ReadFileErrorReason::FileTooLarge)
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn read_file_rejects_path_escapes_directories_large_files_and_non_utf8_content() {
        let workspace = temp_root("read-file");
        let sibling = temp_root("read-file-sibling");
        fs::create_dir_all(workspace.join("folder")).unwrap();
        fs::write(workspace.join("large.txt"), vec![b'x'; 50 * 1024 + 1]).unwrap();
        fs::write(workspace.join("binary.dat"), [0xff]).unwrap();

        for (path, reason) in [
            (sibling.to_string_lossy().into_owned(), "invalid_path"),
            ("../read-file-sibling".to_owned(), "path_not_allowed"),
            ("folder".to_owned(), "not_file"),
            ("large.txt".to_owned(), "file_too_large"),
            ("binary.dat".to_owned(), "not_text"),
        ] {
            let value = serde_json::to_value(
                ReadFile.execute(&context(&workspace), ReadFileInput { path: path.clone() }),
            )
            .unwrap();
            assert_eq!(value["status"], "error");
            assert_eq!(value["reason"], reason);
            assert_eq!(value["path"], path);
        }
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(sibling).unwrap();
    }

    #[test]
    fn dispatch_reads_file_contents_from_the_trusted_workspace() {
        let workspace = temp_root("read-file-dispatch");
        fs::write(workspace.join("notes.txt"), "dispatch contents\n").unwrap();

        let result = dispatch(
            &context(&workspace),
            RovenToolCall {
                id: "call_read_file".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({ "path": "notes.txt" }),
            },
        );

        assert_eq!(
            result.result,
            json!({
                "status": "ok",
                "path": "notes.txt",
                "content": "dispatch contents\n"
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn read_file_input_rejects_extra_fields() {
        assert!(
            serde_json::from_value::<ReadFileInput>(json!({
                "path": "notes.txt",
                "recursive": true
            }))
            .is_err()
        );
    }

    #[test]
    fn read_file_io_reason_classifies_permission_and_other_errors() {
        assert_eq!(
            super::read_file_io_reason(&io::Error::from(io::ErrorKind::PermissionDenied)),
            super::ReadFileErrorReason::PermissionDenied
        );
        assert_eq!(
            super::read_file_io_reason(&io::Error::from(io::ErrorKind::Other)),
            super::ReadFileErrorReason::IoError
        );
    }

    #[test]
    fn list_tools_returns_the_live_registry_with_descriptions_and_schemas() {
        let workspace = temp_root("list-tools");
        let trusted = context(&workspace);
        let expected = serde_json::to_value(definitions()).unwrap();

        let result = dispatch(
            &trusted,
            RovenToolCall {
                id: "call_tools".to_owned(),
                name: "list_tools".to_owned(),
                arguments: json!({}),
            },
        );

        assert_eq!(
            result.result,
            json!({
                "status": "ok",
                "tools": expected,
            })
        );
        let read_file = result.result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "read_file")
            .expect("read_file must be registered");
        assert_eq!(read_file["description"], READ_FILE_DESCRIPTION);
        assert_eq!(
            read_file["input_schema"],
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative text file path."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })
        );
        let list_directory = result.result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "list_directory")
            .expect("list_directory must be registered");
        assert_eq!(list_directory["description"], LIST_DIRECTORY_DESCRIPTION);
        assert_eq!(
            list_directory["input_schema"],
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path; use `.` for the workspace root."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })
        );
        let invalid = dispatch(
            &trusted,
            RovenToolCall {
                id: "call_invalid_tools".to_owned(),
                name: "list_tools".to_owned(),
                arguments: json!({ "unexpected": true }),
            },
        );
        assert_eq!(invalid.result, json!({ "status": "invalid_input" }));
        fs::remove_dir_all(workspace).unwrap();
    }

    #[test]
    fn list_project_rejects_unknown_input_and_exposes_definition() {
        let zeta = ready_project("zeta-project");

        let invalid = dispatch(
            &context(&zeta),
            RovenToolCall {
                id: "invalid-list-project".to_owned(),
                name: "list_project".to_owned(),
                arguments: json!({ "unexpected": true }),
            },
        );
        assert_eq!(invalid.result, json!({ "status": "invalid_input" }));
        assert!(definitions().iter().any(
            |tool| tool.name == "list_project" && tool.description == LIST_PROJECT_DESCRIPTION
        ));

        fs::remove_dir_all(zeta).unwrap();
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

    #[cfg(windows)]
    #[test]
    fn read_file_blocks_symlink_escapes() {
        use std::os::windows::fs::symlink_file;

        let workspace = temp_root("read-file-workspace");
        let outside = temp_root("read-file-outside");
        let outside_file = outside.join("outside.txt");
        let outside_link = workspace.join("outside-link.txt");
        fs::write(&outside_file, "outside").unwrap();
        match symlink_file(&outside_file, &outside_link) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(workspace).unwrap();
                fs::remove_dir_all(outside).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }

        let value = serde_json::to_value(ReadFile.execute(
            &context(&workspace),
            ReadFileInput {
                path: "outside-link.txt".to_owned(),
            },
        ))
        .unwrap();
        assert_eq!(value["status"], "error");
        assert_eq!(value["reason"], "path_not_allowed");
        assert_eq!(value["path"], "outside-link.txt");

        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn open_workspace_file_rejects_a_symlink_handle() {
        use std::os::windows::fs::symlink_file;

        let workspace = temp_root("read-file-reparse");
        let target = workspace.join("target.txt");
        let link = workspace.join("link.txt");
        fs::write(&target, "target").unwrap();
        match symlink_file(&target, &link) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(workspace).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }

        assert!(matches!(
            super::open_workspace_file(&context(&workspace), &link),
            Err(super::ReadFileErrorReason::PathNotAllowed)
        ));
        fs::remove_dir_all(workspace).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn read_file_rejects_final_directory_links_as_paths_not_allowed() {
        use std::os::windows::fs::symlink_dir;

        let workspace = temp_root("read-file-directory-link-workspace");
        let outside = temp_root("read-file-directory-link-outside");
        let link = workspace.join("outside-link");
        match symlink_dir(&outside, &link) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(workspace).unwrap();
                fs::remove_dir_all(outside).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }

        let result = ReadFile.execute(
            &context(&workspace),
            ReadFileInput {
                path: "outside-link".to_owned(),
            },
        );
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            json!({
                "status": "error",
                "reason": "path_not_allowed",
                "path": "outside-link"
            })
        );
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn protected_open_rejects_outside_directory_links() {
        use std::os::windows::fs::symlink_dir;

        let workspace = temp_root("read-file-protected-directory-link-workspace");
        let outside = temp_root("read-file-protected-directory-link-outside");
        let link = workspace.join("outside-link");
        match symlink_dir(&outside, &link) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
                fs::remove_dir_all(workspace).unwrap();
                fs::remove_dir_all(outside).unwrap();
                return;
            }
            Err(error) => panic!("symlink setup failed: {error}"),
        }

        assert!(matches!(
            super::open_workspace_file(&context(&workspace), &link),
            Err(super::ReadFileErrorReason::PathNotAllowed)
        ));
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn opened_path_comparison_is_case_insensitive_and_component_bounded() {
        use std::ffi::OsStr;

        let workspace = Path::new(r"C:\Workspace");
        assert!(
            super::opened_path_is_within_workspace(
                OsStr::new(r"c:\workspace\notes.txt"),
                workspace
            )
            .unwrap()
        );
        assert!(
            super::opened_path_is_within_workspace(OsStr::new(r"C:\WORKSPACE"), workspace).unwrap()
        );
        assert!(
            super::opened_path_is_within_workspace(
                OsStr::new(r"C:\child\notes.txt"),
                Path::new(r"C:\")
            )
            .unwrap()
        );
        assert!(
            !super::opened_path_is_within_workspace(
                OsStr::new(r"C:\WorkspaceElse\notes.txt"),
                workspace
            )
            .unwrap()
        );
    }
}
