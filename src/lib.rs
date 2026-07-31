//! Testable application entry points for PMEMC.

pub mod domain;
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
        _ => anyhow::bail!("this command is not available until a later Version 1 phase"),
    }
}
