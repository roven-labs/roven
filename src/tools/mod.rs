//! Roven-owned tool definitions, dispatch, and deterministic tool execution.

use std::{io, path::PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

mod list_directory;
mod list_project;
mod list_tools;
mod prepare_project;
mod read_file;
mod workspace;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolDefinition {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

pub(crate) fn definitions() -> Vec<RovenToolDefinition> {
    vec![
        prepare_project::definition(),
        list_directory::definition(),
        read_file::definition(),
        list_tools::definition(),
        list_project::definition(),
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
    pub(super) trusted_workspace: PathBuf,
}

impl ToolContext {
    pub(crate) fn new(trusted_workspace: PathBuf) -> io::Result<Self> {
        let trusted_workspace = trusted_workspace.canonicalize()?;
        Ok(Self { trusted_workspace })
    }
}

pub(crate) fn dispatch(context: &ToolContext, call: RovenToolCall) -> RovenToolResult {
    let result = match call.name.as_str() {
        "prepare_project" => prepare_project::dispatch(context, call.arguments),
        "list_directory" => list_directory::dispatch(context, call.arguments),
        "read_file" => read_file::dispatch(context, call.arguments),
        "list_tools" => list_tools::dispatch(call.arguments),
        "list_project" => list_project::dispatch(call.arguments),
        _ => Ok(json!({ "status": "error", "reason": "unknown_tool" })),
    };
    RovenToolResult {
        tool_call_id: call.id,
        name: call.name,
        result: result.expect("tool results are serializable"),
    }
}

#[cfg(test)]
use list_directory::{ListDirectory, ListDirectoryInput, human_workspace_path, size_error_reason};
#[cfg(test)]
use prepare_project::{
    BlockedReason, PrepareProject, PrepareProjectInput, PrepareProjectResult, git_github_remote,
    git_is_clean, has_blocking_status, is_github_url,
};
#[cfg(test)]
use read_file::{
    ReadFile, ReadFileErrorReason, ReadFileInput, read_file_contents, read_file_io_reason,
};
#[cfg(test)]
use read_file::{open_workspace_file, opened_path_is_within_workspace};

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
        BlockedReason, ListDirectory, ListDirectoryInput, PrepareProject, PrepareProjectInput,
        PrepareProjectResult, ReadFile, ReadFileInput, RovenToolCall, ToolContext, definitions,
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
        assert!(definitions().iter().any(|tool| tool.name == "list_project"));

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
