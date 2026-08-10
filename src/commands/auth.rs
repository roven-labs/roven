//! Named OpenAI-compatible provider-profile commands.

use std::io::{self, Write};

use anyhow::bail;

use crate::{
    cli::AuthCommand,
    credentials::{self, SecretStore},
    profiles::{ProviderProfile, ProviderProfiles},
};

pub(crate) fn run(command: AuthCommand) -> anyhow::Result<()> {
    let profiles = ProviderProfiles::for_current_user()?;
    match command {
        AuthCommand::Set => create_profile(&profiles)?,
        AuthCommand::List => list_profiles(&profiles)?,
        AuthCommand::Use => choose_default(&profiles)?,
        AuthCommand::Status => status(&profiles)?,
        AuthCommand::Remove { name } => remove_profile(&profiles, &name)?,
    }
    Ok(())
}

fn create_profile(profiles: &ProviderProfiles) -> anyhow::Result<()> {
    let name = prompt_required("Profile name: ")?;
    let endpoint = prompt_required("OpenAI-compatible HTTPS chat-completions endpoint: ")?;
    let model = prompt_required("Model ID: ")?;
    let secret = rpassword::prompt_password("API key: ")?;
    let confirmation = rpassword::prompt_password("Confirm API key: ")?;
    let profile = profiles.create(&name, &endpoint, &model)?;
    let store = credentials::OsCredentialStore::for_profile_id(&profile.id);
    if let Err(error) = credentials::store_confirmed_api_key(&store, &secret, &confirmation) {
        let _ = profiles.remove(&profile.id);
        return Err(error.into());
    }
    println!("Provider profile `{}` saved", profile.name);
    Ok(())
}

fn list_profiles(profiles: &ProviderProfiles) -> anyhow::Result<()> {
    let items = profiles.list()?;
    if items.is_empty() {
        println!("No provider profiles. Run `roven auth set`.");
        return Ok(());
    }
    let default_id = profiles.default_profile()?.map(|profile| profile.id);
    print!("{}", format_profile_list(&items, default_id.as_deref()));
    Ok(())
}

fn choose_default(profiles: &ProviderProfiles) -> anyhow::Result<()> {
    let items = profiles.list()?;
    if items.is_empty() {
        bail!("no provider profiles exist; run `roven auth set` first");
    }
    print_numbered_profiles(&items);
    let selection = prompt_required("Choose default provider number: ")?;
    let profile = select_profile(&items, &selection)?;
    profiles.set_default(&profile.id)?;
    println!("Default provider set to `{}`", profile.name);
    Ok(())
}

fn status(profiles: &ProviderProfiles) -> anyhow::Result<()> {
    match profiles.default_profile()? {
        Some(profile) => println!(
            "default\t{}\t{}\t{}",
            profile.name, profile.endpoint, profile.model
        ),
        None => println!("default\tmissing"),
    }
    Ok(())
}

fn remove_profile(profiles: &ProviderProfiles, name: &str) -> anyhow::Result<()> {
    let items = profiles.list()?;
    let profile = items
        .iter()
        .find(|profile| profile.name == name)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("provider profile `{name}` does not exist"))?;
    let was_default = profiles
        .default_profile()?
        .is_some_and(|default| default.id == profile.id);
    let replacement = if was_default {
        let remaining: Vec<_> = items
            .iter()
            .filter(|item| item.id != profile.id)
            .cloned()
            .collect();
        if remaining.is_empty() {
            None
        } else {
            print_numbered_profiles(&remaining);
            let selection = prompt_required("Choose a new default provider number: ")?;
            Some(select_profile(&remaining, &selection)?.clone())
        }
    } else {
        None
    };
    let store = credentials::OsCredentialStore::for_profile_id(&profile.id);
    store.delete()?;
    profiles.remove(&profile.id)?;
    if let Some(replacement) = replacement {
        profiles.set_default(&replacement.id)?;
    }
    println!("Provider profile `{}` removed", profile.name);
    Ok(())
}

fn prompt_required(label: &str) -> anyhow::Result<String> {
    print!("{label}");
    io::stdout().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        bail!("a value is required");
    }
    Ok(value)
}

fn print_numbered_profiles(profiles: &[ProviderProfile]) {
    for (index, profile) in profiles.iter().enumerate() {
        println!(
            "{}. {}\t{}\t{}",
            index + 1,
            profile.name,
            profile.endpoint,
            profile.model
        );
    }
}

fn format_profile_list(profiles: &[ProviderProfile], default_id: Option<&str>) -> String {
    profiles
        .iter()
        .map(|profile| {
            let marker = if Some(profile.id.as_str()) == default_id {
                "*"
            } else {
                " "
            };
            format!(
                "{marker} {}\t{}\t{}\n",
                profile.name, profile.endpoint, profile.model
            )
        })
        .collect()
}

fn select_profile<'a>(
    profiles: &'a [ProviderProfile],
    selection: &str,
) -> anyhow::Result<&'a ProviderProfile> {
    let index: usize = selection
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("choose a provider by its displayed number"))?;
    profiles
        .get(index.saturating_sub(1))
        .ok_or_else(|| anyhow::anyhow!("choose a displayed provider number"))
}

#[cfg(test)]
mod tests {
    use crate::profiles::ProviderProfile;

    use super::{format_profile_list, select_profile};

    fn profile(id: &str, name: &str) -> ProviderProfile {
        ProviderProfile {
            id: id.to_owned(),
            name: name.to_owned(),
            endpoint: "https://api.groq.com/openai/v1".to_owned(),
            model: "openai/gpt-oss-20b".to_owned(),
        }
    }

    #[test]
    fn list_shows_the_user_chosen_name_without_a_secret() {
        let profiles = vec![profile("one", "personal groq")];

        assert_eq!(
            format_profile_list(&profiles, Some("one")),
            "* personal groq\thttps://api.groq.com/openai/v1\topenai/gpt-oss-20b\n"
        );
    }

    #[test]
    fn selection_requires_an_existing_number() {
        let profiles = vec![profile("one", "openrouter")];

        assert!(select_profile(&profiles, "2").is_err());
        assert_eq!(select_profile(&profiles, "1").unwrap().name, "openrouter");
    }
}
