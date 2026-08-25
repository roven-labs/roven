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
struct PrepareProjectInput {
    path: String,
    section_name: Option<String>,
    text: Option<String>,
    operation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PrepareProjectResult {
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
struct PreparedProject {
    name: String,
    path: String,
    github_remote: String,
    baseline_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ExistingProject {
    name: String,
    path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum BlockedReason {
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

struct PrepareProject {
    registry: Result<ProjectRegistry, ()>,
}

impl PrepareProject {
    fn for_current_user() -> Self {
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

    fn execute(&self, context: &ToolContext, input: PrepareProjectInput) -> PrepareProjectResult {
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
        fs,
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    use serde_json::json;

    use crate::storage::{ProjectRegistry, RegistrationLookup};

    use super::super::{RovenToolCall, definitions, dispatch};
    use super::*;
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
}
