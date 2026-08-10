use clap::{Parser, Subcommand};

#[derive(Debug, Parser)]
#[command(
    name = "roven",
    version,
    about = "Roven — Project Memory Assistant",
    long_about = None
)]
pub(crate) struct Cli {
    #[command(subcommand)]
    pub(crate) command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub(crate) enum Command {
    /// Manage named OpenAI-compatible provider profiles without exposing API keys.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum AuthCommand {
    /// Interactively create a named provider profile and store its API key.
    Set,
    /// List profile names, endpoints, models, and the selected default.
    List,
    /// Interactively choose the default provider profile.
    Use,
    /// Report the selected default provider profile.
    Status,
    /// Remove one named provider profile and its API key.
    Remove { name: String },
}

pub(crate) fn parse() -> Cli {
    Cli::parse()
}
