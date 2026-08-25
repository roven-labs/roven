use std::{
    fs,
    io::{self, Read},
    path::Path,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    RovenToolDefinition, ToolContext,
    workspace::{WorkspacePathError, canonical_workspace_path, workspace_relative_path},
};

const READ_FILE_DESCRIPTION: &str = "Read a known workspace-relative text file after locating it with `list_directory`. Paths are relative to the trusted workspace. This tool reads only regular UTF-8 text files up to 50 KiB and does not modify files or access paths outside the trusted workspace.";

pub(super) fn definition() -> RovenToolDefinition {
    RovenToolDefinition {
        name: "read_file".to_owned(),
        description: READ_FILE_DESCRIPTION.to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "Workspace-relative text file path." } },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub(super) fn dispatch(context: &ToolContext, arguments: Value) -> serde_json::Result<Value> {
    match serde_json::from_value::<ReadFileInput>(arguments) {
        Ok(input) => serde_json::to_value(ReadFile.execute(context, input)),
        Err(_) => serde_json::to_value(ReadFileResult::error(ReadFileErrorReason::InvalidPath, "")),
    }
}

const READ_FILE_SIZE_LIMIT: u64 = 50 * 1024;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ReadFileInput {
    pub(super) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum ReadFileResult {
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
pub(super) enum ReadFileErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotFile,
    FileTooLarge,
    NotText,
    PermissionDenied,
    IoError,
}

pub(super) struct ReadFile;

impl ReadFile {
    pub(super) fn execute(&self, context: &ToolContext, input: ReadFileInput) -> ReadFileResult {
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

pub(super) fn read_file_contents(file: &mut fs::File) -> Result<String, ReadFileErrorReason> {
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
pub(super) fn open_workspace_file(
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
pub(super) fn opened_path_is_within_workspace(
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

fn resolve_workspace_file(
    context: &ToolContext,
    path: &str,
) -> Result<std::path::PathBuf, ReadFileErrorReason> {
    canonical_workspace_path(context, path).map_err(workspace_path_error_reason)
}

fn workspace_path_error_reason(error: WorkspacePathError) -> ReadFileErrorReason {
    match error {
        WorkspacePathError::InvalidPath => ReadFileErrorReason::InvalidPath,
        WorkspacePathError::PathNotAllowed => ReadFileErrorReason::PathNotAllowed,
        WorkspacePathError::PermissionDenied => ReadFileErrorReason::PermissionDenied,
        WorkspacePathError::IoError => ReadFileErrorReason::IoError,
    }
}

pub(super) fn read_file_io_reason(error: &io::Error) -> ReadFileErrorReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ReadFileErrorReason::PermissionDenied,
        io::ErrorKind::NotFound => ReadFileErrorReason::InvalidPath,
        _ => ReadFileErrorReason::IoError,
    }
}
