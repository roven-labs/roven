//! Provider credential commands.

use crate::{cli::AuthCommand, credentials};

pub(crate) fn run(command: AuthCommand) -> anyhow::Result<()> {
    match command {
        AuthCommand::Set => {
            credentials::prompt_and_store_openrouter_api_key()?;
            println!("OpenRouter credential stored in the operating-system credential store");
        }
        AuthCommand::Status => {
            let state = if credentials::stored_openrouter_credential()? {
                "configured"
            } else {
                "missing"
            };
            println!("openrouter\t{state}");
            println!("environment\tOPENROUTER_API_KEY");
        }
        AuthCommand::Remove => {
            if credentials::remove_openrouter_api_key()? {
                println!("OpenRouter credential removed");
            } else {
                println!("OpenRouter credential was already absent");
            }
        }
    }
    Ok(())
}

pub(crate) fn first_run_setup() -> anyhow::Result<()> {
    println!(
        "OpenRouter setup starts after CodeGraph preparation when you run `pmemc` in a clean Git repository"
    );
    Ok(())
}
