//! Testable application entry points for PMEMC.

pub mod code_map;
mod codegraph;
mod credentials;
pub mod domain;
pub mod git;
pub mod inspection;
pub mod inventory;
pub mod provider;
pub mod storage;

mod application;
mod baseline;
mod cli;
mod commands;
mod output;

/// Run the currently implemented Version 1 command.
///
/// # Errors
///
/// Returns an application-boundary error when a command belongs to a later
/// implementation phase or local initialization fails.
pub fn run() -> anyhow::Result<()> {
    match cli::parse() {
        Some(command) => commands::run(command),
        None => {
            let current_directory = std::env::current_dir()?;
            let repository = git::validate_repository_for_inspection(&current_directory)?;
            output::print_startup_repository_validation(&repository.root, &repository.head_commit);
            let data_paths = storage::default_data_paths()?;
            let existing_project = storage::list_projects(&data_paths)?
                .into_iter()
                .find(|project| project.canonical_path == repository.root);
            let (project, registration_outcome) = match existing_project {
                Some(project) => (project, "Already registered"),
                None => {
                    let metadata = git::metadata(&repository.root)?;
                    let project = storage::add_project(
                        &data_paths,
                        &repository.root,
                        metadata.branch.as_deref(),
                        Some(&repository.head_commit),
                    )?;
                    (project, "Registered successfully")
                }
            };
            output::print_startup_registration(&project, registration_outcome);
            prepare_codegraph_startup(&repository.root)
        }
    }
}

fn prepare_codegraph_startup(repository: &std::path::Path) -> anyhow::Result<()> {
    output::print_startup_codegraph_preparation();
    codegraph::check_available(repository)?;
    if codegraph::index_exists(repository)? {
        output::print_startup_codegraph_existing_index();
        output::print_startup_codegraph_synchronizing();
        let ready = codegraph::synchronize(repository)?;
        debug_assert_eq!(ready.repository_root, repository);
        output::print_startup_codegraph_ready();
        return Ok(());
    }
    output::print_startup_codegraph_missing();
    if !startup_confirmation()? {
        output::print_startup_codegraph_cancelled();
        return Ok(());
    }
    output::print_startup_codegraph_initializing();
    codegraph::initialize(repository)?;
    output::print_startup_codegraph_building_and_synchronizing();
    let ready = codegraph::synchronize(repository)?;
    debug_assert_eq!(ready.repository_root, repository);
    output::print_startup_codegraph_ready();
    Ok(())
}

fn startup_confirmation() -> anyhow::Result<bool> {
    use std::io::{self, Write};

    io::stdout().flush()?;
    let mut response = String::new();
    io::stdin().read_line(&mut response)?;
    Ok(matches!(
        response.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Format repository-validation failures for terminal display.
pub fn validation_error_message(error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<git::RepositoryValidationError>()
        .and_then(output::validation_error_message)
}

/// Format user-facing failures that require structured terminal presentation.
pub fn user_error_message(error: &anyhow::Error) -> Option<String> {
    validation_error_message(error).or_else(|| {
        error
            .downcast_ref::<codegraph::CodeGraphError>()
            .map(output::codegraph_error_message)
    })
}

/// Stage an approved bundle, invoke a supplied provider, and retain only
/// pending-review output.
pub use application::submit_approved_bundle;
