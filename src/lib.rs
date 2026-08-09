//! Testable application entry points for Roven.

mod cli;
mod commands;
mod credentials;
mod provider;
mod storage;
mod ui;

/// Run the Roven command.
///
/// # Errors
///
/// Returns an application-boundary error when local credential management fails.
pub fn run() -> anyhow::Result<()> {
    match cli::parse().command {
        Some(command) => commands::run(command),
        None => ui::run(),
    }
}
