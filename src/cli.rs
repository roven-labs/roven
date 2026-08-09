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
    /// Manage the OpenRouter API key without exposing its value.
    Auth {
        #[command(subcommand)]
        command: AuthCommand,
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

pub(crate) fn parse() -> Cli {
    Cli::parse()
}
