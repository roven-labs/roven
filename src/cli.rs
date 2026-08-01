use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "pmemc",
    version,
    about = "PMEMC — Project Memory CLI",
    long_about = None,
    arg_required_else_help = true
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Initialize PMEMC local storage.
    Init,
    /// Register or display projects.
    Project {
        #[command(subcommand)]
        command: ProjectCommand,
    },
    /// Show repository changes relative to its approved baseline.
    Status {
        /// Registered project identifier. Omit to show every project.
        project_id: Option<String>,
    },
    /// Stage a project inspection for review.
    Inspect {
        /// Registered project identifier.
        project_id: String,
    },
    /// Review pending project-fact proposals.
    Review {
        /// Registered project identifier. Omit to review every project.
        project_id: Option<String>,
    },
    /// Show a project's inspection and decision history.
    History {
        /// Registered project identifier.
        project_id: String,
    },
    /// Manage provider credentials without exposing their values.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum ProjectCommand {
    /// Register a local Git working tree.
    Add {
        /// Path to the local Git working tree.
        path: PathBuf,
    },
    /// List registered projects.
    List,
    /// Show one registered project.
    Show {
        /// Registered project identifier.
        project_id: String,
    },
    /// Irreversibly forget one project's PMEMC memory and registration.
    Forget {
        /// Registered project identifier.
        project_id: String,
        /// Exact display name for non-interactive confirmation.
        #[arg(long, value_name = "PROJECT_NAME")]
        confirm_name: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Store the OpenRouter API key in the operating-system credential store.
    Set,
    /// Report whether a stored OpenRouter credential exists.
    Status,
    /// Remove the stored OpenRouter credential.
    Remove,
}

pub(crate) fn parse() -> Command {
    Cli::parse().command
}
