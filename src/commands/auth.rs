//! Provider credential commands.

use std::io::{self, IsTerminal, Write};

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
            println!("environment-fallback\tOPENROUTER_API_KEY");
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
    if !io::stdin().is_terminal() {
        println!(
            "OpenRouter setup is non-interactive; run `pmemc auth set` before inspection if needed"
        );
        return Ok(());
    }

    if credentials::environment_openrouter_credential_configured() {
        println!("OpenRouter credential: configured");
        return Ok(());
    }
    match credentials::stored_openrouter_credential() {
        Ok(true) => {
            println!("OpenRouter credential: configured");
            return Ok(());
        }
        Ok(false) => {}
        Err(_) => {
            println!(
                "The operating-system credential store is unavailable; run `pmemc auth set` before inspection"
            );
            return Ok(());
        }
    }

    print!("Configure the OpenRouter credential now? [Y/n] ");
    io::stdout().flush()?;
    let mut answer = String::new();
    if io::stdin().read_line(&mut answer).is_err() {
        println!("Run `pmemc auth set` before inspection to configure OpenRouter");
        return Ok(());
    }
    if matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "" | "y" | "yes"
    ) {
        match credentials::prompt_and_store_openrouter_api_key() {
            Ok(()) => {
                println!("OpenRouter credential stored in the operating-system credential store")
            }
            Err(error) => println!(
                "OpenRouter setup was not completed ({error}); run `pmemc auth set` before inspection"
            ),
        }
    } else {
        println!("Run `pmemc auth set` before inspection to configure OpenRouter");
    }
    Ok(())
}
