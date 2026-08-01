//! CLI command handlers. Command modules translate input/output around the
//! application and adapter layers; they do not own domain or storage policy.

mod auth;
mod inspect;
mod project;
mod review;
mod status;

use crate::{cli, storage};

pub(crate) fn run(command: cli::Command) -> anyhow::Result<()> {
    match command {
        cli::Command::Init => {
            let data_paths = storage::default_data_paths()?;
            let first_run = !data_paths.database_path().is_file();
            storage::initialize(&data_paths)?;
            println!(
                "PMEMC data directory initialized at {}",
                data_paths.root().display()
            );
            println!(
                "OpenRouter model: {}",
                crate::provider::configured_model_id()
            );
            if first_run {
                auth::first_run_setup()?;
            }
            Ok(())
        }
        cli::Command::Project { command } => project::run(command),
        cli::Command::Status { project_id } => status::run(project_id),
        cli::Command::Inspect { project_id } => inspect::run(&project_id),
        cli::Command::Review { project_id } => review::run(project_id),
        cli::Command::History { project_id } => project::history(&project_id),
        cli::Command::Auth { command } => auth::run(command),
    }
}

pub(crate) fn resolve_project_id(
    data_paths: &storage::DataPaths,
    reference: &str,
) -> anyhow::Result<i64> {
    let projects = storage::list_projects(data_paths)?;
    let named_projects = projects
        .iter()
        .filter(|project| project.display_name == reference)
        .collect::<Vec<_>>();
    match named_projects.as_slice() {
        [project] => return Ok(project.id),
        [] => {}
        _ => anyhow::bail!(
            "project name `{reference}` is ambiguous; use its project-<number> identifier"
        ),
    }

    reference
        .strip_prefix("project-")
        .ok_or_else(|| anyhow::anyhow!("project `{reference}` is not registered"))?
        .parse::<i64>()
        .map_err(|_| anyhow::anyhow!("project reference must be a name or project-<number>"))
}
