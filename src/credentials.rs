//! Provider credential storage and resolution.

use std::env;

use thiserror::Error;

const SERVICE_NAME: &str = "pmemc";
const OPENROUTER_ACCOUNT: &str = "openrouter";

/// Errors from local credential management without exposing the credential.
#[derive(Debug, Error)]
pub(crate) enum CredentialError {
    #[error("the operating-system credential store is unavailable")]
    StoreUnavailable,
    #[error("the credential value cannot be empty")]
    EmptyValue,
    #[error("the credential confirmation did not match")]
    ConfirmationMismatch,
    #[error("the password prompt failed")]
    Prompt(#[source] std::io::Error),
    #[error("no OpenRouter credential is configured")]
    Missing,
}

/// Minimal secret-store boundary used by the provider and CLI.
pub(crate) trait SecretStore {
    fn get(&self) -> Result<Option<String>, CredentialError>;
    fn set(&self, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self) -> Result<bool, CredentialError>;
}

/// Native operating-system credential store.
#[derive(Debug, Default, Clone, Copy)]
pub(crate) struct OsCredentialStore;

impl OsCredentialStore {
    fn entry() -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(SERVICE_NAME, OPENROUTER_ACCOUNT)
            .map_err(|_| CredentialError::StoreUnavailable)
    }
}

impl SecretStore for OsCredentialStore {
    fn get(&self) -> Result<Option<String>, CredentialError> {
        match Self::entry()?.get_password() {
            Ok(secret) if !secret.trim().is_empty() => Ok(Some(secret)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialError::StoreUnavailable),
        }
    }

    fn set(&self, secret: &str) -> Result<(), CredentialError> {
        validate_secret(secret)?;
        Self::entry()?
            .set_password(secret)
            .map_err(|_| CredentialError::StoreUnavailable)
    }

    fn delete(&self) -> Result<bool, CredentialError> {
        match Self::entry()?.delete_credential() {
            Ok(()) => Ok(true),
            Err(keyring::Error::NoEntry) => Ok(false),
            Err(_) => Err(CredentialError::StoreUnavailable),
        }
    }
}

fn validate_secret(secret: &str) -> Result<(), CredentialError> {
    if secret.trim().is_empty() {
        Err(CredentialError::EmptyValue)
    } else {
        Ok(())
    }
}

/// Resolve the provider key from Credential Manager first, then environment.
pub(crate) fn openrouter_api_key() -> Result<String, CredentialError> {
    resolve_openrouter_api_key(&OsCredentialStore, |name| env::var(name).ok())
}

pub(crate) fn environment_openrouter_credential_configured() -> bool {
    env::var("OPENROUTER_API_KEY").is_ok_and(|value| !value.trim().is_empty())
}

pub(crate) fn resolve_openrouter_api_key(
    store: &impl SecretStore,
    environment: impl Fn(&str) -> Option<String>,
) -> Result<String, CredentialError> {
    let fallback = || {
        environment("OPENROUTER_API_KEY")
            .filter(|value| !value.trim().is_empty())
            .ok_or(CredentialError::Missing)
    };

    match store.get() {
        Ok(Some(secret)) if !secret.trim().is_empty() => Ok(secret),
        Ok(None) | Ok(Some(_)) | Err(CredentialError::StoreUnavailable) => fallback(),
        Err(error) => Err(error),
    }
}

pub(crate) fn prompt_for_openrouter_api_key() -> Result<String, CredentialError> {
    let secret =
        rpassword::prompt_password("OpenRouter API key: ").map_err(CredentialError::Prompt)?;
    if secret.trim().is_empty() {
        return Err(CredentialError::EmptyValue);
    }
    Ok(secret)
}

pub(crate) fn prompt_and_store_openrouter_api_key() -> Result<(), CredentialError> {
    let secret = prompt_for_openrouter_api_key()?;
    let confirmation = rpassword::prompt_password("Confirm OpenRouter API key: ")
        .map_err(CredentialError::Prompt)?;
    if secret != confirmation {
        return Err(CredentialError::ConfirmationMismatch);
    }
    store_openrouter_api_key(&secret)
}

pub(crate) fn store_openrouter_api_key(secret: &str) -> Result<(), CredentialError> {
    OsCredentialStore.set(secret)
}

pub(crate) fn remove_openrouter_api_key() -> Result<bool, CredentialError> {
    OsCredentialStore.delete()
}

pub(crate) fn stored_openrouter_credential() -> Result<bool, CredentialError> {
    Ok(OsCredentialStore.get()?.is_some())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{CredentialError, SecretStore, resolve_openrouter_api_key};

    #[derive(Default)]
    struct MemoryStore {
        value: RefCell<Option<String>>,
        failure: bool,
    }

    impl SecretStore for MemoryStore {
        fn get(&self) -> Result<Option<String>, CredentialError> {
            if self.failure {
                Err(CredentialError::StoreUnavailable)
            } else {
                Ok(self.value.borrow().clone())
            }
        }

        fn set(&self, secret: &str) -> Result<(), CredentialError> {
            *self.value.borrow_mut() = Some(secret.to_owned());
            Ok(())
        }

        fn delete(&self) -> Result<bool, CredentialError> {
            Ok(self.value.borrow_mut().take().is_some())
        }
    }

    #[test]
    fn stored_secret_wins_over_environment_fallback() {
        let store = MemoryStore {
            value: RefCell::new(Some("stored-secret".into())),
            failure: false,
        };
        let result = resolve_openrouter_api_key(&store, |_| Some("environment-secret".into()));
        assert_eq!(
            result.expect("stored secret should resolve"),
            "stored-secret"
        );
    }

    #[test]
    fn missing_store_entry_uses_environment_fallback() {
        let store = MemoryStore::default();
        let result = resolve_openrouter_api_key(&store, |name| {
            (name == "OPENROUTER_API_KEY").then(|| "environment-secret".into())
        });
        assert_eq!(
            result.expect("environment secret should resolve"),
            "environment-secret"
        );
    }

    #[test]
    fn unavailable_store_still_allows_ci_environment_fallback() {
        let store = MemoryStore {
            value: RefCell::new(None),
            failure: true,
        };
        let result = resolve_openrouter_api_key(&store, |_| Some("ci-secret".into()));
        assert_eq!(result.expect("CI fallback should resolve"), "ci-secret");
    }

    #[test]
    fn empty_sources_are_rejected_without_exposing_values() {
        let store = MemoryStore::default();
        let error = resolve_openrouter_api_key(&store, |_| Some("  ".into()))
            .expect_err("empty sources should fail");
        assert!(matches!(error, CredentialError::Missing));
        assert!(!error.to_string().contains(" ".repeat(2).as_str()));
    }

    #[test]
    fn empty_secret_cannot_be_stored() {
        let store = super::OsCredentialStore;
        let error = store
            .set(" \t")
            .expect_err("empty secret should be rejected");
        assert!(matches!(error, CredentialError::EmptyValue));
    }
}
