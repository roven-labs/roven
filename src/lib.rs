//! Testable application entry points for PMEMC.

pub mod domain;
pub mod git;
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
        _ => anyhow::bail!("this command is not available until a later Version 1 phase"),
    }
}

fn status_command(project_id: Option<String>) -> anyhow::Result<()> {
    let project_id = project_id.ok_or_else(|| {
        anyhow::anyhow!("a project ID is required until multi-project status is implemented")
    })?;
    let id = project_id
        .strip_prefix("project-")
        .ok_or_else(|| anyhow::anyhow!("project ID must use the form project-<number>"))?
        .parse::<i64>()?;
    let data_paths = storage::default_data_paths()?;
    let project = storage::project_by_id(&data_paths, id)?
        .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
    println!(
        "project-{}\tinitial inspection required\t{}",
        project.id,
        project.canonical_path.display()
    );
    for path in git::untracked_paths(&project.canonical_path)? {
        println!("untracked\t{path}");
    }
    for path in git::staged_paths(&project.canonical_path)? {
        println!("staged\t{path}");
    }
    for path in git::unstaged_paths(&project.canonical_path)? {
        println!("unstaged\t{path}");
    }
    for path in git::deleted_paths(&project.canonical_path)? {
        println!("deleted\t{path}");
    }
    Ok(())
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
                println!(
                    "project-{}\t{}\t{}\t{}",
                    project.id,
                    project.display_name,
                    project.canonical_path.display(),
                    project.lifecycle_state
                );
            }
            Ok(())
        }
        cli::ProjectCommand::Show { project_id } => {
            let id = project_id
                .strip_prefix("project-")
                .ok_or_else(|| anyhow::anyhow!("project ID must use the form project-<number>"))?
                .parse::<i64>()?;
            let project = storage::project_by_id(&data_paths, id)?
                .ok_or_else(|| anyhow::anyhow!("project {project_id} is not registered"))?;
            println!(
                "project-{}\t{}\t{}\t{}",
                project.id,
                project.display_name,
                project.canonical_path.display(),
                project.lifecycle_state
            );
            Ok(())
        }
    }
}
