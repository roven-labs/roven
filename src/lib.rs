//! Testable application entry points for PMEMC.

mod cli;

/// Parse the Version 1 command surface.
pub fn run() {
    cli::parse();
}
