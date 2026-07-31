//! Read-only Git metadata adapter.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use thiserror::Error;

/// Read-only metadata required to register a Git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMetadata {
    pub root: PathBuf,
    pub branch: Option<String>,
    pub head_commit: Option<String>,
}

/// A safe Git command failure.
#[derive(Debug, Error)]
pub enum GitError {
    #[error("Git could not inspect {path}: {message}")]
    Command { path: PathBuf, message: String },
}

/// Read Git repository metadata without reading source files.
pub fn metadata(path: &Path) -> Result<RepositoryMetadata, GitError> {
    let root = run(path, &["rev-parse", "--show-toplevel"])?;
    let root = PathBuf::from(root.trim());
    let branch = optional(&root, &["branch", "--show-current"])?;
    let head_commit = optional(&root, &["rev-parse", "HEAD"])?;
    Ok(RepositoryMetadata {
        root,
        branch,
        head_commit,
    })
}

/// Return non-ignored untracked paths without reading their contents.
pub fn untracked_paths(path: &Path) -> Result<Vec<String>, GitError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| record.strip_prefix(b"? "))
        .map(|path| String::from_utf8_lossy(path).into_owned())
        .collect())
}

/// Return paths with an index-side change from porcelain version 2.
pub fn staged_paths(path: &Path) -> Result<Vec<String>, GitError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let mut fields = record.splitn(9, |byte| *byte == b' ');
            let kind = fields.next()?;
            let xy = fields.next()?;
            if kind == b"1" && xy.first().is_some_and(|state| *state != b'.') {
                fields
                    .nth(6)
                    .map(|path| String::from_utf8_lossy(path).into_owned())
            } else {
                None
            }
        })
        .collect())
}

/// Return paths with a worktree-side change from porcelain version 2.
pub fn unstaged_paths(path: &Path) -> Result<Vec<String>, GitError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let mut fields = record.splitn(9, |byte| *byte == b' ');
            let kind = fields.next()?;
            let xy = fields.next()?;
            if kind == b"1" && xy.get(1).is_some_and(|state| *state != b'.') {
                fields
                    .nth(6)
                    .map(|path| String::from_utf8_lossy(path).into_owned())
            } else {
                None
            }
        })
        .collect())
}

/// Return tracked paths deleted from either the index or worktree.
pub fn deleted_paths(path: &Path) -> Result<Vec<String>, GitError> {
    let output = Command::new("git")
        .args(["status", "--porcelain=v2", "-z", "--untracked-files=all"])
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if !output.status.success() {
        return Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        });
    }
    Ok(output
        .stdout
        .split(|byte| *byte == 0)
        .filter_map(|record| {
            let mut fields = record.splitn(9, |byte| *byte == b' ');
            let kind = fields.next()?;
            let xy = fields.next()?;
            if kind == b"1" && xy.contains(&b'D') {
                fields
                    .nth(6)
                    .map(|path| String::from_utf8_lossy(path).into_owned())
            } else {
                None
            }
        })
        .collect())
}

fn optional(path: &Path, arguments: &[&str]) -> Result<Option<String>, GitError> {
    match run(path, arguments) {
        Ok(value) if value.trim().is_empty() => Ok(None),
        Ok(value) => Ok(Some(value.trim().into())),
        Err(_) => Ok(None),
    }
}
fn run(path: &Path, arguments: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
        .map_err(|error| GitError::Command {
            path: path.into(),
            message: error.to_string(),
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(GitError::Command {
            path: path.into(),
            message: String::from_utf8_lossy(&output.stderr).trim().into(),
        })
    }
}
