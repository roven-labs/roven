//! Testable application entry points for Roven.

mod agent;
mod cli;
mod commands;
mod credentials;
mod mcp;
mod profiles;
mod provider;
mod runtime_log;
mod storage;
mod tools;
mod ui;

use runtime_log::RuntimeLog;

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
