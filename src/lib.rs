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
    commands::run(cli::parse())
}

/// Stage an approved bundle, invoke a supplied provider, and retain only
/// pending-review output.
pub use application::submit_approved_bundle;
