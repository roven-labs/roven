use std::{
    path::{Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::storage::{ProjectRegistration, ProjectRegistry, RegistrationLookup};

use super::{RovenToolDefinition, ToolContext};

const PREPARE_PROJECT_DESCRIPTION: &str = "Validate and register the currently trusted project for first-time use with Roven, or replace its concise `summary` section after registration. Pass `.` as the path for the current trusted workspace on every call. Registration validates the project path, existing Roven registration, Git repository, GitHub remote, committed baseline, and clean working state, then stores the minimal project registration. Section updates accept only section_name `summary`, text, and operation `replace`; they update local Roven registration data and do not inspect or modify the project repository.";

pub(super) fn definition() -> RovenToolDefinition {
    RovenToolDefinition {
        name: "prepare_project".to_owned(),
        description: PREPARE_PROJECT_DESCRIPTION.to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the currently trusted project directory." },
                "section_name": { "type": "string", "enum": ["summary"], "description": "Registration section to replace; version one accepts only summary." },
                "text": { "type": "string", "description": "Non-empty concise report text for the selected section." },
                "operation": { "type": "string", "enum": ["replace"], "description": "Update operation; version one accepts only replace." }
            },
            "required": ["path"],
            "additionalProperties": false
        }),
    }
}

pub(super) fn dispatch(context: &ToolContext, arguments: Value) -> serde_json::Result<Value> {
    let has_null_section_field = ["section_name", "text", "operation"]
        .iter()
        .any(|key| arguments.get(*key).is_some_and(Value::is_null));
    if has_null_section_field {
        serde_json::to_value(PrepareProjectResult::blocked(
            BlockedReason::InvalidSectionUpdate,
        ))
    } else {
        match serde_json::from_value::<PrepareProjectInput>(arguments) {
            Ok(input) => {
                serde_json::to_value(PrepareProject::for_current_user().execute(context, input))
            }
            Err(_) => {
                serde_json::to_value(PrepareProjectResult::blocked(BlockedReason::InvalidPath))
            }
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PrepareProjectInput {
    pub(super) path: String,
    pub(super) section_name: Option<String>,
    pub(super) text: Option<String>,
    pub(super) operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(super) enum PrepareProjectResult {
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
pub(super) struct PreparedProject {
    name: String,
    path: String,
    github_remote: String,
    baseline_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(super) struct ExistingProject {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum BlockedReason {
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

pub(super) struct PrepareProject {
    registry: Result<ProjectRegistry, ()>,
}

impl PrepareProject {
    pub(super) fn for_current_user() -> Self {
        Self {
            registry: ProjectRegistry::for_current_user().map_err(|_| ()),
        }
    }

    #[cfg(test)]
    pub(super) fn for_data_root(data_root: &Path) -> Self {
        Self {
            registry: Ok(ProjectRegistry::for_data_root(data_root)),
        }
    }

    pub(super) fn execute(
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

pub(super) fn git_github_remote(project_path: &Path) -> Option<String> {
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

pub(super) fn git_is_clean(project_path: &Path) -> bool {
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

pub(super) fn has_blocking_status(status: &str) -> bool {
    status.lines().any(|line| !line.trim().is_empty())
}

pub(super) fn is_github_url(url: &str) -> bool {
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
