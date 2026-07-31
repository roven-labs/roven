//! Approval-gated construction of minimized, local inspection evidence.

use std::{collections::BTreeSet, fs, path::Path};

use serde::Serialize;
use thiserror::Error;

use crate::{
    code_map::{CodeMap, CodeMapError, build_code_map},
    git::WorkingTreeStatus,
    inventory::Language,
};

const BUNDLE_SCHEMA_VERSION: u8 = 1;
const MAX_BUNDLE_BYTES: usize = 64 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;

/// The repository state represented by an evidence excerpt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum EvidenceState {
    Committed,
    InProgress,
}

/// One minimized source or text excerpt suitable for a future provider request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceFile {
    pub path: String,
    pub state: EvidenceState,
    pub content: String,
    pub redacted: bool,
}

/// A deterministic, size-bounded payload staged after operator approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EvidenceBundle {
    pub schema_version: u8,
    pub project_id: String,
    pub initial_inspection: bool,
    pub files: Vec<EvidenceFile>,
    pub code_map: CodeMap,
}

/// Errors that prevent evidence construction after the approval gate.
#[derive(Debug, Error)]
pub enum InspectionError {
    #[error(transparent)]
    CodeMap(#[from] CodeMapError),
}

/// Build the initial inspection bundle without executing repository code.
///
/// Callers must obtain operator approval before calling this function because it
/// reads repository content through the compact-map and excerpt steps.
///
/// # Errors
///
/// Returns an error when the compact map cannot inventory the repository.
pub fn build_initial_bundle(
    repository: &Path,
    project_id: &str,
    status: &WorkingTreeStatus,
) -> Result<EvidenceBundle, InspectionError> {
    let code_map = build_code_map(repository)?;
    let changed_paths = changed_paths(status);
    let mut remaining_bytes = MAX_BUNDLE_BYTES;
    let mut files = Vec::new();

    for file in &code_map.files {
        if matches!(file.language, Language::Unsupported) || remaining_bytes == 0 {
            continue;
        }
        let Ok(content) = fs::read_to_string(repository.join(&file.path)) else {
            continue;
        };
        let limit = remaining_bytes.min(MAX_FILE_BYTES);
        let content = truncate_to_byte_limit(&content, limit);
        let (content, redacted) = redact_suspected_secrets(content);
        remaining_bytes = remaining_bytes.saturating_sub(content.len());
        files.push(EvidenceFile {
            path: file.path.clone(),
            state: if changed_paths.contains(file.path.as_str()) {
                EvidenceState::InProgress
            } else {
                EvidenceState::Committed
            },
            content,
            redacted,
        });
    }

    Ok(EvidenceBundle {
        schema_version: BUNDLE_SCHEMA_VERSION,
        project_id: project_id.into(),
        initial_inspection: true,
        files,
        code_map,
    })
}

fn changed_paths(status: &WorkingTreeStatus) -> BTreeSet<&str> {
    status
        .added_paths
        .iter()
        .chain(&status.modified_paths)
        .chain(&status.untracked_paths)
        .chain(&status.staged_paths)
        .chain(&status.unstaged_paths)
        .chain(&status.deleted_paths)
        .map(String::as_str)
        .collect()
}

fn truncate_to_byte_limit(value: &str, limit: usize) -> String {
    if value.len() <= limit {
        return value.into();
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].into()
}

fn redact_suspected_secrets(content: String) -> (String, bool) {
    let mut redacted = false;
    let lines = content
        .split_inclusive('\n')
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            let looks_sensitive = [
                "api_key",
                "api_token",
                "password",
                "secret",
                "token",
                "authorization",
            ]
            .iter()
            .any(|marker| lower.contains(marker));
            let separator = line.find('=').or_else(|| line.find(':'));
            if looks_sensitive && let Some(separator) = separator {
                redacted = true;
                format!("{} [REDACTED]\n", &line[..=separator])
            } else {
                line.into()
            }
        })
        .collect();
    (lines, redacted)
}
