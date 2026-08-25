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
struct ReadFileInput {
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ReadFileResult {
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
enum ReadFileErrorReason {
    InvalidPath,
    PathNotAllowed,
    NotFile,
    FileTooLarge,
    NotText,
    PermissionDenied,
    IoError,
}

struct ReadFile;

impl ReadFile {
    fn execute(&self, context: &ToolContext, input: ReadFileInput) -> ReadFileResult {
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

fn read_file_io_reason(error: &io::Error) -> ReadFileErrorReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ReadFileErrorReason::PermissionDenied,
        io::ErrorKind::NotFound => ReadFileErrorReason::InvalidPath,
        _ => ReadFileErrorReason::IoError,
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, io,
        path::{Path, PathBuf},
    };

    use serde_json::json;

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
