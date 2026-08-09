//! Testable application entry points for PMEMC.

mod cli;
mod commands;
mod credentials;

/// Run the currently implemented Version 1 command.
///
/// # Errors
///
/// Returns an application-boundary error when local credential management fails.
pub fn run() -> anyhow::Result<()> {
    match cli::parse().command {
        Some(command) => commands::run(command),
        None => cli::print_help(),
    }
}
