//! CLI command handlers around the local credential boundary.

mod auth;

use crate::cli;

pub(crate) fn run(command: cli::Command) -> anyhow::Result<()> {
    match command {
        cli::Command::Auth { command } => auth::run(command),
    }
}
