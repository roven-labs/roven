//! Roven-owned tool definitions, dispatch, and deterministic tool execution.

use std::{
    fs, io,
    path::{Component, Path, PathBuf},
    process::Command,
};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::storage::{ProjectRegistration, ProjectRegistry, RegistrationLookup};

pub(crate) const PREPARE_PROJECT_DESCRIPTION: &str = "Validate and register the currently trusted project for first-time use with Roven. Use this when the user asks to add/register the current project for future project understanding, resume generation, or portfolio updates. Pass `.` as the path for the current trusted workspace. The tool validates the project path, existing Roven registration, Git repository, GitHub remote, committed baseline, and clean working state, then stores the minimal project registration. It does not inspect source code or initialize code-intelligence systems.";
pub(crate) const LIST_DIRECTORY_DESCRIPTION: &str = "List the immediate contents of a directory inside the currently trusted Roven workspace. Use this to inspect the workspace structure and locate files or subdirectories before choosing another filesystem tool. Paths are relative to the trusted workspace; use `.` for the workspace root. This tool does not read file contents, search recursively, modify files, register projects, or access paths outside the trusted workspace.";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolDefinition {
    pub(crate) name: &'static str,
    pub(crate) description: &'static str,
    pub(crate) input_schema: Value,
}

pub(crate) fn definitions() -> Vec<RovenToolDefinition> {
    vec![
        RovenToolDefinition {
            name: "prepare_project",
            description: PREPARE_PROJECT_DESCRIPTION,
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the currently trusted project directory."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        },
        RovenToolDefinition {
            name: "list_directory",
            description: LIST_DIRECTORY_DESCRIPTION,
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
        Ok(Self {
            trusted_workspace: trusted_workspace.canonicalize()?,
        })
    }
}

pub(crate) fn dispatch(context: &ToolContext, call: RovenToolCall) -> RovenToolResult {
    let result = match call.name.as_str() {
        "prepare_project" => match serde_json::from_value::<PrepareProjectInput>(call.arguments) {
            Ok(input) => {
                serde_json::to_value(PrepareProject::for_current_user().execute(context, input))
            }
            Err(_) => {
                serde_json::to_value(PrepareProjectResult::blocked(BlockedReason::InvalidPath))
            }
        },
        "list_directory" => match serde_json::from_value::<ListDirectoryInput>(call.arguments) {
            Ok(input) => serde_json::to_value(ListDirectory.execute(context, input)),
            Err(_) => serde_json::to_value(ListDirectoryResult::error(
                ListDirectoryErrorReason::InvalidPath,
                "",
            )),
        },
        _ => serde_json::to_value(PrepareProjectResult::blocked(BlockedReason::InvalidPath)),
    };
    RovenToolResult {
        tool_call_id: call.id,
        name: call.name,
        result: result.expect("tool results are serializable"),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PrepareProjectInput {
    pub(crate) path: String,
}

const DIRECTORY_LIST_LIMIT: usize = 100;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ListDirectoryInput {
    pub(crate) path: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ListDirectoryEntry {
    name: String,
    path: String,
    kind: DirectoryEntryKind,
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
            let child_relative = target
                .strip_prefix(&context.trusted_workspace)
                .expect("authorized target remains under trusted workspace")
                .join(&name);
            listed_entries.push(ListDirectoryEntry {
                name,
                path: workspace_relative_path_from_relative(&child_relative),
                kind: entry_kind(&file_type),
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

fn io_reason(error: &io::Error) -> ListDirectoryErrorReason {
    match error.kind() {
        io::ErrorKind::PermissionDenied => ListDirectoryErrorReason::PermissionDenied,
        io::ErrorKind::NotFound => ListDirectoryErrorReason::InvalidPath,
        _ => ListDirectoryErrorReason::IoError,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum PrepareProjectResult {
    Prepared { project: PreparedProject },
    AlreadyAdded { project: ExistingProject },
    Blocked { reason: BlockedReason },
}

impl PrepareProjectResult {
    fn blocked(reason: BlockedReason) -> Self {
        Self::Blocked { reason }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct PreparedProject {
    id: String,
    path: String,
    github_remote: String,
    baseline_commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct ExistingProject {
    id: String,
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
}

pub(crate) struct PrepareProject<G = SystemGit> {
    registry: Result<ProjectRegistry, ()>,
    git: G,
}

impl PrepareProject<SystemGit> {
    pub(crate) fn for_current_user() -> Self {
        Self {
            registry: ProjectRegistry::for_current_user().map_err(|_| ()),
            git: SystemGit,
        }
    }
}

impl<G: GitInspector> PrepareProject<G> {
    #[cfg(test)]
    fn with_dependencies(registry: ProjectRegistry, git: G) -> Self {
        Self {
            registry: Ok(registry),
            git,
        }
    }

    pub(crate) fn execute(
        &self,
        context: &ToolContext,
        input: PrepareProjectInput,
    ) -> PrepareProjectResult {
        let project_path = match canonical_project_path(&input.path) {
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
        match registry.lookup(&project_path) {
            Ok(RegistrationLookup::Registered(registration)) => {
                return PrepareProjectResult::AlreadyAdded {
                    project: existing_project(registration),
                };
            }
            Ok(RegistrationLookup::Absent | RegistrationLookup::Legacy) => {}
            Err(_) => return PrepareProjectResult::blocked(BlockedReason::StorageFailure),
        }
        if !self.git.is_available() {
            return PrepareProjectResult::blocked(BlockedReason::GitUnavailable);
        }
        if !self.git.is_repository(&project_path) {
            return PrepareProjectResult::blocked(BlockedReason::NotGitRepository);
        }
        let baseline_commit = match self.git.head_commit(&project_path) {
            Some(commit) => commit,
            None => return PrepareProjectResult::blocked(BlockedReason::NoCommitBaseline),
        };
        let github_remote = match self.git.github_remote(&project_path) {
            Some(remote) => remote,
            None => return PrepareProjectResult::blocked(BlockedReason::NoGithubRemote),
        };
        if !self.git.is_clean(&project_path) || self.git.operation_in_progress(&project_path) {
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

fn canonical_project_path(path: &str) -> Option<PathBuf> {
    let path = Path::new(path);
    let canonical = path.canonicalize().ok()?;
    canonical.is_dir().then_some(canonical)
}

fn existing_project(registration: ProjectRegistration) -> ExistingProject {
    ExistingProject {
        id: registration.id,
        path: registration.canonical_path,
    }
}

fn prepared_project(registration: ProjectRegistration) -> PreparedProject {
    PreparedProject {
        id: registration.id,
        path: registration.canonical_path,
        github_remote: registration.github_remote,
        baseline_commit: registration.baseline_commit,
    }
}

pub(crate) trait GitInspector {
    fn is_available(&self) -> bool;
    fn is_repository(&self, project_path: &Path) -> bool;
    fn head_commit(&self, project_path: &Path) -> Option<String>;
    fn github_remote(&self, project_path: &Path) -> Option<String>;
    fn is_clean(&self, project_path: &Path) -> bool;
    fn operation_in_progress(&self, project_path: &Path) -> bool;
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SystemGit;

impl GitInspector for SystemGit {
    fn is_available(&self) -> bool {
        git_output(None, &["--version"]).is_some()
    }

    fn is_repository(&self, project_path: &Path) -> bool {
        git_output(Some(project_path), &["rev-parse", "--is-inside-work-tree"])
            .is_some_and(|output| output.trim() == "true")
    }

    fn head_commit(&self, project_path: &Path) -> Option<String> {
        git_output(
            Some(project_path),
            &["rev-parse", "--verify", "HEAD^{commit}"],
        )
        .map(|output| output.trim().to_owned())
        .filter(|output| !output.is_empty())
    }

    fn github_remote(&self, project_path: &Path) -> Option<String> {
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

    fn is_clean(&self, project_path: &Path) -> bool {
        git_output(
            Some(project_path),
            &["status", "--porcelain=v1", "--untracked-files=all"],
        )
        .is_some_and(|output| !has_blocking_status(&output))
    }

    fn operation_in_progress(&self, project_path: &Path) -> bool {
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

/// CodeGraph's local index is a generated, untracked project capability. It
/// must not force users to modify their repository just to register it.
fn has_blocking_status(status: &str) -> bool {
    status.lines().any(|line| {
        let untracked_path = line.strip_prefix("?? ");
        !matches!(untracked_path, Some(path) if is_codegraph_path(path))
    })
}

fn is_codegraph_path(path: &str) -> bool {
    path == ".codegraph" || path.starts_with(".codegraph/")
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

    use crate::storage::{ProjectRegistry, RegistrationLookup, project_id};

    use super::{
        BlockedReason, GitInspector, ListDirectory, ListDirectoryInput, PrepareProject,
        PrepareProjectInput, PrepareProjectResult, SystemGit, ToolContext,
    };

    #[derive(Default)]
    struct FakeGit {
        available: bool,
        repository: bool,
        head: Option<String>,
        remote: Option<String>,
        clean: bool,
        operation: bool,
    }

    impl GitInspector for FakeGit {
        fn is_available(&self) -> bool {
            self.available
        }
        fn is_repository(&self, _: &Path) -> bool {
            self.repository
        }
        fn head_commit(&self, _: &Path) -> Option<String> {
            self.head.clone()
        }
        fn github_remote(&self, _: &Path) -> Option<String> {
            self.remote.clone()
        }
        fn is_clean(&self, _: &Path) -> bool {
            self.clean
        }
        fn operation_in_progress(&self, _: &Path) -> bool {
            self.operation
        }
    }

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
        }
    }

    fn ready_git() -> FakeGit {
        FakeGit {
            available: true,
            repository: true,
            head: Some("abc123".to_owned()),
            remote: Some("git@github.com:roven/example.git".to_owned()),
            clean: true,
            operation: false,
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

    #[test]
    fn prepare_project_rejects_a_path_outside_the_trusted_workspace() {
        let data = temp_root("prepare-data");
        let trusted = temp_root("trusted");
        let other = temp_root("other");
        let tool =
            PrepareProject::with_dependencies(ProjectRegistry::for_data_root(&data), ready_git());

        let result = tool.execute(&context(&trusted), input(&other));

        assert_eq!(
            result,
            PrepareProjectResult::Blocked {
                reason: BlockedReason::PathNotAllowed
            }
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(trusted).unwrap();
        fs::remove_dir_all(other).unwrap();
    }

    #[test]
    fn prepare_project_registers_only_a_clean_github_repository() {
        let data = temp_root("prepare-data");
        let project = temp_root("project");
        let tool =
            PrepareProject::with_dependencies(ProjectRegistry::for_data_root(&data), ready_git());

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
        assert_eq!(value["project"]["baseline_commit"], "abc123");
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
        let tool = PrepareProject::with_dependencies(registry, FakeGit::default());

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
        let project = temp_root("project");
        let mut git = ready_git();
        git.clean = false;
        let tool = PrepareProject::with_dependencies(ProjectRegistry::for_data_root(&data), git);

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
    fn validation_reports_each_git_precondition_without_registering() {
        let cases = [
            (FakeGit::default(), BlockedReason::GitUnavailable),
            (
                FakeGit {
                    available: true,
                    ..FakeGit::default()
                },
                BlockedReason::NotGitRepository,
            ),
            (
                FakeGit {
                    available: true,
                    repository: true,
                    ..FakeGit::default()
                },
                BlockedReason::NoCommitBaseline,
            ),
            (
                FakeGit {
                    remote: None,
                    ..ready_git()
                },
                BlockedReason::NoGithubRemote,
            ),
            (
                FakeGit {
                    operation: true,
                    ..ready_git()
                },
                BlockedReason::RepositoryNotClean,
            ),
        ];

        for (git, reason) in cases {
            let data = temp_root("prepare-data");
            let project = temp_root("project");
            let tool =
                PrepareProject::with_dependencies(ProjectRegistry::for_data_root(&data), git);

            assert_eq!(
                tool.execute(&context(&project), input(&project)),
                PrepareProjectResult::Blocked { reason }
            );
            assert!(matches!(
                ProjectRegistry::for_data_root(&data)
                    .lookup(&project)
                    .unwrap(),
                RegistrationLookup::Absent
            ));
            fs::remove_dir_all(data).unwrap();
            fs::remove_dir_all(project).unwrap();
        }
    }

    #[test]
    fn legacy_project_record_is_upgraded_in_place_after_validation() {
        let data = temp_root("prepare-data");
        let project = temp_root("project");
        let canonical = project.canonicalize().unwrap();
        let record_dir = data.join("projects").join(project_id(&canonical));
        fs::create_dir_all(&record_dir).unwrap();
        fs::write(
            record_dir.join("project.json"),
            serde_json::to_vec(&serde_json::json!({
                "canonical_path": canonical.to_string_lossy()
            }))
            .unwrap(),
        )
        .unwrap();
        let registry = ProjectRegistry::for_data_root(&data);
        let tool = PrepareProject::with_dependencies(registry.clone(), ready_git());

        assert!(matches!(
            tool.execute(&context(&project), input(&project)),
            PrepareProjectResult::Prepared { .. }
        ));
        assert!(matches!(
            registry.lookup(&project).unwrap(),
            RegistrationLookup::Registered(_)
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
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
            SystemGit.github_remote(&project),
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
            SystemGit.github_remote(&project),
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
            SystemGit.github_remote(&project),
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
            !SystemGit.is_clean(&project),
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
        assert!(SystemGit.is_clean(&project));
        fs::create_dir_all(project.join(".codegraph")).unwrap();
        fs::write(project.join(".codegraph/index.bin"), "generated index").unwrap();
        assert!(SystemGit.is_clean(&project));
        fs::write(project.join("untracked.txt"), "untracked").unwrap();
        assert!(!SystemGit.is_clean(&project));
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn codegraph_exception_applies_only_to_untracked_codegraph_paths() {
        assert!(!super::has_blocking_status("?? .codegraph/index.bin\n"));
        assert!(!super::has_blocking_status("?? .codegraph\n"));
        assert!(super::has_blocking_status(" M .codegraph/index.bin\n"));
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
                    { "name": "middle.txt", "path": "middle.txt", "kind": "file" }
                ],
                "truncated": false
            })
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
        assert!(
            listed["entries"]
                .as_array()
                .unwrap()
                .iter()
                .any(|entry| { entry["name"] == "internal-link" && entry["kind"] == "symlink" })
        );
        fs::remove_dir_all(workspace).unwrap();
        fs::remove_dir_all(outside).unwrap();
    }
}
