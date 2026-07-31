//! Approval-gated construction of minimized, local inspection evidence.

use std::{collections::BTreeSet, fs, path::Path};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::{
    code_map::{CodeMap, CodeMapError, build_code_map, serialize_code_map, structural_neighbors},
    git::WorkingTreeStatus,
    inventory::Language,
};

const BUNDLE_SCHEMA_VERSION: u8 = 1;
const MAX_BUNDLE_BYTES: usize = 64 * 1024;
const MAX_FILE_BYTES: usize = 8 * 1024;

/// The repository state represented by an evidence excerpt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceState {
    Committed,
    Staged,
    #[serde(alias = "InProgress")]
    Unstaged,
    StagedAndUnstaged,
    Untracked,
}

/// One minimized source or text excerpt suitable for a future provider request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceFile {
    pub path: String,
    pub state: EvidenceState,
    pub content: String,
    pub redacted: bool,
}

/// A deterministic, size-bounded payload staged after operator approval.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub schema_version: u8,
    pub project_id: String,
    pub initial_inspection: bool,
    pub files: Vec<EvidenceFile>,
}

/// Immutable inspection material retained locally for later review finalization.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionPackage {
    /// The minimized, provider-approved evidence bundle.
    pub bundle: EvidenceBundle,
    /// Deterministic serialization of the structural map used to select evidence.
    pub code_map_json: String,
}

/// Errors that prevent evidence construction after the approval gate.
#[derive(Debug, Error)]
pub enum InspectionError {
    #[error(transparent)]
    CodeMap(#[from] CodeMapError),
    #[error("cannot serialize the compact code map")]
    SerializeCodeMap(#[source] serde_json::Error),
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
    Ok(build_initial_package(repository, project_id, status)?.bundle)
}

/// Build the initial provider bundle together with its immutable structural map.
///
/// Callers must obtain operator approval before calling this function because it
/// reads repository content through the compact-map and excerpt steps.
///
/// # Errors
///
/// Returns an error when the compact map cannot be built or serialized.
pub fn build_initial_package(
    repository: &Path,
    project_id: &str,
    status: &WorkingTreeStatus,
) -> Result<InspectionPackage, InspectionError> {
    let code_map = build_code_map(repository)?;
    let code_map_json = serialize_code_map(&code_map).map_err(InspectionError::SerializeCodeMap)?;
    let selected_paths = code_map
        .files
        .iter()
        .filter(|file| !matches!(file.language, Language::Unsupported))
        .map(|file| file.path.clone())
        .collect();
    let bundle = build_bundle(
        repository,
        project_id,
        status,
        true,
        code_map,
        &selected_paths,
    )?;
    Ok(InspectionPackage {
        bundle,
        code_map_json,
    })
}

/// Build a minimized bundle for changed files and their exact local context.
///
/// # Errors
///
/// Returns an error when the compact map cannot inventory the repository.
pub fn build_incremental_bundle(
    repository: &Path,
    project_id: &str,
    status: &WorkingTreeStatus,
) -> Result<EvidenceBundle, InspectionError> {
    Ok(build_incremental_package(repository, project_id, status)?.bundle)
}

/// Build an incremental provider bundle together with its immutable structural map.
///
/// # Errors
///
/// Returns an error when the compact map cannot be built or serialized.
pub fn build_incremental_package(
    repository: &Path,
    project_id: &str,
    status: &WorkingTreeStatus,
) -> Result<InspectionPackage, InspectionError> {
    let code_map = build_code_map(repository)?;
    let code_map_json = serialize_code_map(&code_map).map_err(InspectionError::SerializeCodeMap)?;
    let selected_paths = incremental_paths(&code_map, status);
    let bundle = build_bundle(
        repository,
        project_id,
        status,
        false,
        code_map,
        &selected_paths,
    )?;
    Ok(InspectionPackage {
        bundle,
        code_map_json,
    })
}

fn build_bundle(
    repository: &Path,
    project_id: &str,
    status: &WorkingTreeStatus,
    initial_inspection: bool,
    code_map: CodeMap,
    selected_paths: &BTreeSet<String>,
) -> Result<EvidenceBundle, InspectionError> {
    let mut remaining_bytes = MAX_BUNDLE_BYTES;
    let mut files = Vec::new();

    for file in &code_map.files {
        if !selected_paths.contains(&file.path) || remaining_bytes == 0 {
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
            state: evidence_state(status, &file.path),
            content,
            redacted,
        });
        if !bundle_fits(project_id, initial_inspection, &files) {
            files.pop();
            break;
        }
    }

    Ok(EvidenceBundle {
        schema_version: BUNDLE_SCHEMA_VERSION,
        project_id: project_id.into(),
        initial_inspection,
        files,
    })
}

fn bundle_fits(project_id: &str, initial_inspection: bool, files: &[EvidenceFile]) -> bool {
    serde_json::to_vec(&EvidenceBundle {
        schema_version: BUNDLE_SCHEMA_VERSION,
        project_id: project_id.into(),
        initial_inspection,
        files: files.into(),
    })
    .is_ok_and(|serialized| serialized.len() <= MAX_BUNDLE_BYTES)
}

fn incremental_paths(code_map: &CodeMap, status: &WorkingTreeStatus) -> BTreeSet<String> {
    let changed_paths = changed_paths(status);
    let mut selected_paths = code_map
        .files
        .iter()
        .filter(|file| changed_paths.contains(file.path.as_str()))
        .map(|file| file.path.clone())
        .collect::<BTreeSet<_>>();
    let changed_symbol_names = code_map
        .symbols
        .iter()
        .filter(|symbol| changed_paths.contains(symbol.path.as_str()))
        .map(|symbol| symbol.name.as_str())
        .collect::<BTreeSet<_>>();
    for symbol_name in changed_symbol_names {
        selected_paths.extend(
            structural_neighbors(code_map, symbol_name)
                .into_iter()
                .map(|symbol| symbol.path),
        );
    }

    let changed_stems = selected_paths
        .iter()
        .filter_map(|path| Path::new(path).file_stem()?.to_str())
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let has_changed_code = code_map.files.iter().any(|file| {
        selected_paths.contains(&file.path) && !matches!(file.language, Language::GenericText)
    });
    for file in &code_map.files {
        if (is_manifest(&file.path) && has_changed_code)
            || is_relevant_test(&file.path, &changed_stems)
        {
            selected_paths.insert(file.path.clone());
        }
    }
    selected_paths
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

fn evidence_state(status: &WorkingTreeStatus, path: &str) -> EvidenceState {
    let staged = status.staged_paths.iter().any(|changed| changed == path);
    let unstaged = status.unstaged_paths.iter().any(|changed| changed == path);
    if status.untracked_paths.iter().any(|changed| changed == path) {
        EvidenceState::Untracked
    } else if staged && unstaged {
        EvidenceState::StagedAndUnstaged
    } else if staged {
        EvidenceState::Staged
    } else if unstaged
        || status.added_paths.iter().any(|changed| changed == path)
        || status.modified_paths.iter().any(|changed| changed == path)
    {
        EvidenceState::Unstaged
    } else {
        EvidenceState::Committed
    }
}

fn is_manifest(path: &str) -> bool {
    matches!(
        path,
        "Cargo.toml"
            | "package.json"
            | "pyproject.toml"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
    )
}

fn is_relevant_test(path: &str, changed_stems: &BTreeSet<String>) -> bool {
    path.split('/')
        .any(|component| component == "tests" || component == "test")
        && Path::new(path)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .is_some_and(|stem| changed_stems.contains(stem))
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
    let mut private_key_block = false;
    let mut result = String::new();
    for line in content.split_inclusive('\n') {
        let lower = line.to_ascii_lowercase();
        let begins_private_key = lower.contains("-----begin");
        let ends_private_key = lower.contains("-----end");
        if private_key_block || begins_private_key {
            redacted = true;
            private_key_block = !ends_private_key;
            result.push_str("[REDACTED]\n");
            continue;
        }

        let looks_sensitive = [
            "api_key",
            "api_token",
            "password",
            "secret",
            "token",
            "authorization",
            "private_key",
            "private key",
        ]
        .iter()
        .any(|marker| lower.contains(marker));
        let separator = line.find('=').or_else(|| line.find(':'));
        if looks_sensitive && let Some(separator) = separator {
            redacted = true;
            result.push_str(&line[..=separator]);
            result.push_str(" [REDACTED]\n");
        } else {
            result.push_str(line);
        }
    }
    (result, redacted)
}
