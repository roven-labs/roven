//! Testable application entry points for PMEMC.

use std::{
    collections::BTreeSet,
    io::{self, Write},
};

pub mod code_map;
pub mod domain;
pub mod git;
pub mod inspection;
pub mod inventory;
pub mod provider;
pub mod storage;

mod cli;

/// Run the currently implemented Version 1 command.
///
/// # Errors
///
/// Returns an application-boundary error when a command belongs to a later
/// implementation phase or local initialization fails.
pub fn run() -> anyhow::Result<()> {
    match cli::parse() {
        cli::Command::Init => {
            let data_paths = storage::default_data_paths()?;
            storage::initialize(&data_paths)?;
            println!(
                "PMEMC data directory initialized at {}",
                data_paths.root().display()
            );
            Ok(())
        }
        cli::Command::Project { command } => project_command(command),
        cli::Command::Status { project_id } => status_command(project_id),
        cli::Command::Inspect { project_id } => inspect_command(&project_id),
        _ => anyhow::bail!("this command is not available until a later Version 1 phase"),
    }
}

/// Stage an approved bundle, invoke a supplied provider, and retain only
/// pending-review output. Tests supply [`provider::FakeProvider`] here so this
/// full path never needs network access.
///
/// # Errors
///
/// Returns an error after retaining a retryable attempt when provider work
/// fails, or when local persistence cannot complete atomically.
pub fn submit_approved_bundle(
    data_paths: &storage::DataPaths,
    project_id: i64,
    bundle: &inspection::EvidenceBundle,
    provider: &dyn provider::ModelProvider,
) -> anyhow::Result<i64> {
    let bundle_json = serde_json::to_string(bundle)?;
    let attempt_id = storage::stage_inspection_attempt(
        data_paths,
        project_id,
        bundle.schema_version,
        &bundle_json,
    )?;
    submit_staged_bundle(data_paths, attempt_id, bundle, provider)?;
    Ok(attempt_id)
}

fn inspect_command(project_id: &str) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let id = parse_project_id(project_id)?;
    let project = storage::project_by_id(&data_paths, id)?
        .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
    let status = git::working_tree_status(&project.canonical_path)?;
    let retryable_attempt = storage::failed_provider_attempt_for_project(&data_paths, id)?;
    let initial_inspection = project.lifecycle_state == "registered_needs_inspection";
    match &retryable_attempt {
        Some(attempt) => {
            println!(
                "inspection scope for project-{id}: retained approved evidence from attempt {}",
                attempt.id
            );
            println!(
                "current changed paths detected: {}",
                changed_path_count(&status)
            );
            for file in &attempt.bundle.files {
                println!("scope\t{}", file.path);
            }
            print!("Retry the retained approved evidence bundle? [y/N] ");
        }
        None => {
            let scope_description = if initial_inspection {
                "initial repository context"
            } else {
                "changed files and direct structural context"
            };
            println!("inspection scope for project-{id}: {scope_description}");
            println!("changed paths detected: {}", changed_path_count(&status));
            for path in inspection_scope_paths(&status) {
                println!("scope\t{path}");
            }
            print!("Inspect the reported repository files? [y/N] ");
        }
    }
    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    if !matches!(response.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!("inspection cancelled; no repository content was read");
        return Ok(());
    }

    let (bundle, attempt_id) = if let Some(attempt) = retryable_attempt {
        storage::retry_provider_attempt(&data_paths, attempt.id)?;
        (attempt.bundle, attempt.id)
    } else {
        let package = if initial_inspection {
            inspection::build_initial_package(&project.canonical_path, project_id, &status)?
        } else {
            inspection::build_incremental_package(&project.canonical_path, project_id, &status)?
        };
        let bundle_json = serde_json::to_string(&package.bundle)?;
        let attempt_id = storage::stage_inspection_attempt_with_code_map(
            &data_paths,
            id,
            package.bundle.schema_version,
            &bundle_json,
            Some(&package.code_map_json),
        )?;
        (package.bundle, attempt_id)
    };
    let provider = match provider::OpenRouterProvider::from_environment() {
        Ok(provider) => provider,
        Err(error) => {
            let metadata =
                provider::ProviderInvocationMetadata::new("openrouter", "unconfigured", 1)?;
            storage::record_provider_failure(
                &data_paths,
                attempt_id,
                &metadata,
                error
                    .failure_category()
                    .unwrap_or(provider::ProviderFailureCategory::Configuration),
            )?;
            return Err(error.into());
        }
    };
    submit_staged_bundle(&data_paths, attempt_id, &bundle, &provider)?;
    println!("inspection attempt {attempt_id} stored provider proposals for review");
    Ok(())
}

fn submit_staged_bundle(
    data_paths: &storage::DataPaths,
    attempt_id: i64,
    bundle: &inspection::EvidenceBundle,
    provider: &dyn provider::ModelProvider,
) -> anyhow::Result<()> {
    let metadata = provider.metadata();
    let response = match provider.propose(bundle) {
        Ok(response) => response,
        Err(error) => {
            storage::record_provider_failure(
                data_paths,
                attempt_id,
                &metadata,
                error
                    .failure_category()
                    .unwrap_or(provider::ProviderFailureCategory::RequestFailed),
            )?;
            return Err(error.into());
        }
    };
    storage::store_provider_response(data_paths, attempt_id, &metadata, &response)?;
    Ok(())
}

fn changed_path_count(status: &git::WorkingTreeStatus) -> usize {
    status.added_paths.len()
        + status.modified_paths.len()
        + status.untracked_paths.len()
        + status.staged_paths.len()
        + status.unstaged_paths.len()
        + status.deleted_paths.len()
}

fn inspection_scope_paths(status: &git::WorkingTreeStatus) -> BTreeSet<&str> {
    status
        .added_paths
        .iter()
        .chain(&status.modified_paths)
        .chain(&status.untracked_paths)
        .chain(&status.staged_paths)
        .chain(&status.unstaged_paths)
        .chain(&status.deleted_paths)
        .map(String::as_str)
        .chain(
            status.relationships.iter().flat_map(|relationship| {
                [relationship.source.as_str(), relationship.target.as_str()]
            }),
        )
        .collect()
}

fn status_command(project_id: Option<String>) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    let projects = match project_id {
        Some(project_id) => {
            let id = parse_project_id(&project_id)?;
            let project = storage::project_by_id(&data_paths, id)?
                .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
            vec![project]
        }
        None => storage::list_projects(&data_paths)?,
    };

    if projects.is_empty() {
        anyhow::bail!("no projects are registered; run `pmemc project add <path>` first");
    }

    for project in projects {
        print_project_status(&project)?;
    }
    Ok(())
}

fn print_project_status(project: &storage::Project) -> anyhow::Result<()> {
    println!(
        "project-{}\tinitial inspection required\t{}",
        project.id,
        project.canonical_path.display()
    );
    let metadata = git::metadata(&project.canonical_path)?;
    println!(
        "branch\t{}",
        metadata.branch.as_deref().unwrap_or("detached")
    );
    println!(
        "head\t{}",
        metadata.head_commit.as_deref().unwrap_or("unborn")
    );
    match project.head_commit.as_deref() {
        Some(head_commit) => println!(
            "committed-since-registration\t{}",
            git::commit_count_since(&project.canonical_path, head_commit)?
        ),
        None => println!(
            "committed-since-registration\tnot-applicable (repository was unborn at registration)"
        ),
    }
    println!("commits-since-baseline\tnot-applicable (initial inspection required)");
    let status = git::working_tree_status(&project.canonical_path)?;
    for path in status.added_paths {
        println!("added\t{path}");
    }
    for path in status.modified_paths {
        println!("modified\t{path}");
    }
    for path in status.untracked_paths {
        println!("untracked\t{path}");
    }
    for path in status.staged_paths {
        println!("staged\t{path}");
    }
    for path in status.unstaged_paths {
        println!("unstaged\t{path}");
    }
    for path in status.deleted_paths {
        println!("deleted\t{path}");
    }
    for relationship in status.relationships {
        let label = match relationship.kind {
            git::PathRelationshipKind::Renamed => "renamed",
            git::PathRelationshipKind::Copied => "copied",
        };
        println!("{label}\t{}\t{}", relationship.source, relationship.target);
    }
    Ok(())
}

fn parse_project_id(project_id: &str) -> anyhow::Result<i64> {
    project_id
        .strip_prefix("project-")
        .ok_or_else(|| anyhow::anyhow!("project ID must use the form project-<number>"))?
        .parse::<i64>()
        .map_err(Into::into)
}

fn project_command(command: cli::ProjectCommand) -> anyhow::Result<()> {
    let data_paths = storage::default_data_paths()?;
    match command {
        cli::ProjectCommand::Add { path } => {
            let metadata = git::metadata(&path)?;
            let name = metadata
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("project");
            let project = storage::add_project(
                &data_paths,
                name,
                &metadata.root,
                metadata.branch.as_deref(),
                metadata.head_commit.as_deref(),
            )?;
            println!(
                "registered {} as project-{}",
                project.canonical_path.display(),
                project.id
            );
            Ok(())
        }
        cli::ProjectCommand::List => {
            for project in storage::list_projects(&data_paths)? {
                let metadata = git::metadata(&project.canonical_path)?;
                println!(
                    "project-{}\t{}\t{}\t{}\tbranch={}\tlast-approved-inspection=none\tchanges-detected=initial-inspection-required",
                    project.id,
                    project.display_name,
                    project.canonical_path.display(),
                    project.lifecycle_state,
                    metadata.branch.as_deref().unwrap_or("detached")
                );
            }
            Ok(())
        }
        cli::ProjectCommand::Show { project_id } => {
            let id = parse_project_id(&project_id)?;
            let project = storage::project_by_id(&data_paths, id)?
                .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
            println!(
                "project-{}\t{}\t{}\t{}\tbranch={}\thead={}",
                project.id,
                project.display_name,
                project.canonical_path.display(),
                project.lifecycle_state,
                project.current_branch.as_deref().unwrap_or("detached"),
                project.head_commit.as_deref().unwrap_or("unborn")
            );
            Ok(())
        }
    }
}
