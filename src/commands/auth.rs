//! OpenRouter API-key credential commands.

use crate::{cli::AuthCommand, credentials};

pub(crate) fn run(command: AuthCommand) -> anyhow::Result<()> {
    match command {
        AuthCommand::Set => {
            credentials::prompt_and_store_openrouter_api_key()?;
            println!(
                "OpenRouter credential stored for Roven in the operating-system credential store"
            );
        }
        AuthCommand::Status => {
            let state = if credentials::stored_openrouter_credential()? {
                "configured"
            } else {
                "missing"
            };
            println!("{}", credential_status_message(state));
        }
        AuthCommand::Remove => {
            if credentials::remove_openrouter_api_key()? {
                println!("Roven OpenRouter credential removed");
            } else {
                println!("Roven OpenRouter credential was already absent");
            }
        }
    }
    Ok(())
}

fn credential_status_message(state: &str) -> String {
    format!("openrouter\t{state}")
}

#[cfg(test)]
mod tests {
    #[test]
    fn status_message_reports_only_windows_credential_manager_state() {
        let message = super::credential_status_message("configured");

        assert_eq!(message, "openrouter\tconfigured");
        assert!(!message.contains("environment"));
    }
}
