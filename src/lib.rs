//! Testable application entry points for PMEMC.

pub mod code_map;
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
            if let Some(project) = existing_project {
                output::print_startup_registration(&project, "Already registered");
                return Ok(());
            }
            let metadata = git::metadata(&repository.root)?;
            let project = storage::add_project(
                &data_paths,
                &repository.root,
                metadata.branch.as_deref(),
                Some(&repository.head_commit),
            )?;
            output::print_startup_registration(&project, "Registered successfully");
            Ok(())
        }
    }
}

/// Format repository-validation failures for terminal display.
pub fn validation_error_message(error: &anyhow::Error) -> Option<String> {
    error
        .downcast_ref::<git::RepositoryValidationError>()
        .and_then(output::validation_error_message)
}

/// Stage an approved bundle, invoke a supplied provider, and retain only
/// pending-review output.
pub use application::submit_approved_bundle;
