//! CodeGraph command adapter used before PMEMC inspection workflows.

use std::{path::Path, path::PathBuf, process::Command};

use serde::Deserialize;
use thiserror::Error;

/// A repository whose CodeGraph index was synchronized and verified.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodeGraphReadyState {
    pub(crate) repository_root: PathBuf,
}

/// Failures while preparing CodeGraph for a repository.
#[derive(Debug, Error)]
pub(crate) enum CodeGraphError {
    #[error("CodeGraph is not installed.")]
    Unavailable,
    #[error("CodeGraph {operation} failed: {message}")]
    Command {
        operation: &'static str,
        message: String,
    },
    #[error("CodeGraph returned an invalid index status for {root}")]
    InvalidStatus { root: PathBuf },
    #[error("CodeGraph index is not ready for {root} after synchronization")]
    NotReady { root: PathBuf },
    #[error(
        "CodeGraph index is not initialized for {root}. Run pmemc from that repository and approve CodeGraph initialization first."
    )]
    MissingIndex { root: PathBuf },
}

#[derive(Deserialize)]
struct Status {
    initialized: bool,
    index: Option<Index>,
    #[serde(rename = "worktreeMismatch")]
    worktree_mismatch: Option<serde_json::Value>,
}

#[derive(Deserialize)]
struct Index {
    state: String,
    #[serde(rename = "pendingRefs")]
    pending_refs: u64,
}

/// Confirm that the installed CodeGraph CLI can be invoked.
pub(crate) fn check_available(repository: &Path) -> Result<(), CodeGraphError> {
    let found = Command::new("cmd")
        .args(["/D", "/S", "/C", "where codegraph >nul 2>nul"])
        .current_dir(repository)
        .status()
        .map_err(|error| CodeGraphError::Command {
            operation: "availability",
            message: error.to_string(),
        })?;
    if !found.success() {
        return Err(CodeGraphError::Unavailable);
    }
    run(repository, &["--version"], "availability").map(|_| ())
}

/// Return whether the repository has a CodeGraph index.
pub(crate) fn index_exists(repository: &Path) -> Result<bool, CodeGraphError> {
    Ok(status(repository)?.initialized)
}

/// Synchronize an existing index and verify that it is ready.
pub(crate) fn synchronize(repository: &Path) -> Result<CodeGraphReadyState, CodeGraphError> {
    run(repository, &["sync"], "synchronization")?;
    verify_ready(repository)
}

/// Initialize a CodeGraph index.
pub(crate) fn initialize(repository: &Path) -> Result<(), CodeGraphError> {
    run(repository, &["init"], "initialization").map(|_| ())
}

/// Require a ready existing index before a repository-inspection workflow runs.
pub(crate) fn prepare_existing(repository: &Path) -> Result<CodeGraphReadyState, CodeGraphError> {
    check_available(repository)?;
    if !index_exists(repository)? {
        return Err(CodeGraphError::MissingIndex {
            root: repository.into(),
        });
    }
    synchronize(repository)
}

fn verify_ready(repository: &Path) -> Result<CodeGraphReadyState, CodeGraphError> {
    let status = status(repository)?;
    let ready = status.initialized
        && status.worktree_mismatch.is_none()
        && status
            .index
            .is_some_and(|index| index.state == "complete" && index.pending_refs == 0);
    if !ready {
        return Err(CodeGraphError::NotReady {
            root: repository.into(),
        });
    }
    Ok(CodeGraphReadyState {
        repository_root: repository.into(),
    })
}

fn status(repository: &Path) -> Result<Status, CodeGraphError> {
    let output = run(repository, &["status", "--json"], "status")?;
    serde_json::from_str(&output).map_err(|_| CodeGraphError::InvalidStatus {
        root: repository.into(),
    })
}

fn run(
    repository: &Path,
    arguments: &[&str],
    operation: &'static str,
) -> Result<String, CodeGraphError> {
    let command = format!("codegraph {}", arguments.join(" "));
    let output = Command::new("cmd")
        .args(["/D", "/S", "/C", &command])
        .current_dir(repository)
        .output()
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                CodeGraphError::Unavailable
            } else {
                CodeGraphError::Command {
                    operation,
                    message: error.to_string(),
                }
            }
        })?;
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).into_owned());
    }
    let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    Err(CodeGraphError::Command {
        operation,
        message: if message.is_empty() {
            String::from_utf8_lossy(&output.stdout).trim().to_owned()
        } else {
            message
        },
    })
}
