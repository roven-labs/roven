//! Testable application entry points for Roven.

use std::{io, path::PathBuf};

use directories::ProjectDirs;

mod agent;
mod cli;
mod commands;
mod context;
mod credentials;
mod model_catalog;
mod ollama;
mod openrouter;
mod profiles;
mod provider;
mod runtime_log;
mod storage;
mod tools;
mod ui;

use runtime_log::RuntimeLog;

const QUALIFIER: &str = "io.github.vishal24p";
const APPLICATION: &str = "Roven";

pub(crate) fn app_data_root() -> io::Result<PathBuf> {
    ProjectDirs::from(QUALIFIER, "", APPLICATION)
        .map(|directories| directories.data_local_dir().to_path_buf())
        .ok_or_else(|| io::Error::other("the operating-system local data directory is unavailable"))
}

/// Run the Roven command.
///
/// # Errors
///
/// Returns an application-boundary error when local credential management fails.
pub fn run() -> anyhow::Result<()> {
    let runtime_log = RuntimeLog::for_current_user().ok();
    if let Some(log) = &runtime_log {
        log.record(
            "application",
            "started",
            &format!("log=enabled path={}", log.path().display()),
        );
    }
    let result = match cli::parse().command {
        Some(command) => {
            if let Some(log) = &runtime_log {
                log.record("application", "command_started", "command=auth");
            }
            commands::run(command)
        }
        None => ui::run(runtime_log.clone()),
    };
    if let Some(log) = &runtime_log {
        log.record(
            "application",
            "finished",
            if result.is_ok() {
                "outcome=ok"
            } else {
                "outcome=error"
            },
        );
    }
    result
}
