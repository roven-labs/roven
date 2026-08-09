//! OpenRouter API-key storage in Windows Credential Manager.

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
}

/// Minimal secret-store boundary used by the auth CLI.
pub(crate) trait SecretStore {
    fn get(&self) -> Result<Option<String>, CredentialError>;
    fn set(&self, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self) -> Result<bool, CredentialError>;
}

/// Native Windows Credential Manager store for PMEMC's OpenRouter key.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OsCredentialStore;

impl OsCredentialStore {
    fn entry(&self) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(SERVICE_NAME, OPENROUTER_ACCOUNT)
            .map_err(|_| CredentialError::StoreUnavailable)
    }
}

impl SecretStore for OsCredentialStore {
    fn get(&self) -> Result<Option<String>, CredentialError> {
        match self.entry()?.get_password() {
            Ok(secret) if !secret.trim().is_empty() => Ok(Some(secret)),
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(CredentialError::StoreUnavailable),
        }
    }

    fn set(&self, secret: &str) -> Result<(), CredentialError> {
        validate_secret(secret)?;
        self.entry()?
            .set_password(secret)
            .map_err(|_| CredentialError::StoreUnavailable)
    }

    fn delete(&self) -> Result<bool, CredentialError> {
        match self.entry()?.delete_credential() {
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

/// Prompt for the OpenRouter API key twice and persist it.
pub(crate) fn prompt_and_store_openrouter_api_key() -> Result<(), CredentialError> {
    let secret =
        rpassword::prompt_password("OpenRouter API key: ").map_err(CredentialError::Prompt)?;
    validate_secret(&secret)?;
    let confirmation = rpassword::prompt_password("Confirm OpenRouter API key: ")
        .map_err(CredentialError::Prompt)?;
    store_confirmed_openrouter_api_key(&OsCredentialStore, &secret, &confirmation)
}

fn store_confirmed_openrouter_api_key(
    store: &impl SecretStore,
    secret: &str,
    confirmation: &str,
) -> Result<(), CredentialError> {
    validate_secret(secret)?;
    if secret != confirmation {
        return Err(CredentialError::ConfirmationMismatch);
    }
    store.set(secret)
}

#[cfg(test)]
fn store_prompted_openrouter_api_key(
    store: &impl SecretStore,
    prompt: impl FnOnce() -> Result<String, CredentialError>,
) -> Result<(), CredentialError> {
    let secret = prompt()?;
    validate_secret(&secret)?;
    store.set(&secret)
}

pub(crate) fn remove_openrouter_api_key() -> Result<bool, CredentialError> {
    remove_credential(&OsCredentialStore)
}

pub(crate) fn stored_openrouter_credential() -> Result<bool, CredentialError> {
    credential_is_stored(&OsCredentialStore)
}

/// Retrieves the credential only for the provider boundary.
pub(crate) fn load_openrouter_api_key() -> Result<Option<String>, CredentialError> {
    OsCredentialStore.get()
}

fn remove_credential(store: &impl SecretStore) -> Result<bool, CredentialError> {
    store.delete()
}

fn credential_is_stored(store: &impl SecretStore) -> Result<bool, CredentialError> {
    Ok(store.get()?.is_some())
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::{
        CredentialError, OsCredentialStore, SecretStore, credential_is_stored, remove_credential,
        store_confirmed_openrouter_api_key, store_prompted_openrouter_api_key,
    };

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
            if self.failure {
                return Err(CredentialError::StoreUnavailable);
            }
            *self.value.borrow_mut() = Some(secret.to_owned());
            Ok(())
        }

        fn delete(&self) -> Result<bool, CredentialError> {
            if self.failure {
                return Err(CredentialError::StoreUnavailable);
            }
            Ok(self.value.borrow_mut().take().is_some())
        }
    }

    #[test]
    fn prompted_key_is_stored_without_returning_or_printing_it() {
        let store = MemoryStore::default();
        store_prompted_openrouter_api_key(&store, || Ok("prompted-secret".into()))
            .expect("prompted credential should store");
        assert_eq!(
            store.get().expect("stored credential should read"),
            Some("prompted-secret".into())
        );
    }

    #[test]
    fn cancelled_or_empty_prompt_does_not_store_a_credential() {
        let store = MemoryStore::default();
        let cancelled = store_prompted_openrouter_api_key(&store, || {
            Err(CredentialError::Prompt(std::io::Error::other("cancelled")))
        });
        assert!(matches!(cancelled, Err(CredentialError::Prompt(_))));
        assert_eq!(store.get().expect("store should remain readable"), None);

        let empty = store_prompted_openrouter_api_key(&store, || Ok(" ".into()));
        assert!(matches!(empty, Err(CredentialError::EmptyValue)));
        assert_eq!(store.get().expect("store should remain empty"), None);
    }

    #[test]
    fn empty_secret_cannot_be_stored() {
        let error = OsCredentialStore
            .set(" \t")
            .expect_err("empty secret should be rejected");
        assert!(matches!(error, CredentialError::EmptyValue));
    }

    #[test]
    fn credential_status_distinguishes_configured_and_missing_entries() {
        let configured = MemoryStore {
            value: RefCell::new(Some("stored-secret".into())),
            failure: false,
        };
        let missing = MemoryStore::default();

        assert!(credential_is_stored(&configured).expect("configured store should be readable"));
        assert!(!credential_is_stored(&missing).expect("missing store should be readable"));
    }

    #[test]
    fn removal_distinguishes_present_and_absent_entries() {
        let present = MemoryStore {
            value: RefCell::new(Some("stored-secret".into())),
            failure: false,
        };
        let absent = MemoryStore::default();

        assert!(remove_credential(&present).expect("present credential should be removed"));
        assert!(!remove_credential(&absent).expect("absent credential should not be removed"));
        assert_eq!(present.get().expect("store should be readable"), None);
    }

    #[test]
    fn confirmation_mismatch_preserves_an_existing_credential() {
        let store = MemoryStore {
            value: RefCell::new(Some("existing-secret".into())),
            failure: false,
        };

        let error = store_confirmed_openrouter_api_key(&store, "new-secret", "different-secret")
            .expect_err("mismatched confirmation must fail");

        assert!(matches!(error, CredentialError::ConfirmationMismatch));
        assert_eq!(
            store.get().expect("store should be readable"),
            Some("existing-secret".into())
        );
    }

    #[test]
    fn store_failures_return_safe_errors_without_changing_a_retained_credential() {
        let store = MemoryStore {
            value: RefCell::new(Some("existing-secret".into())),
            failure: true,
        };

        let status = credential_is_stored(&store)
            .expect_err("an unavailable store must make status fail safely");
        let replacement = store_confirmed_openrouter_api_key(&store, "new-secret", "new-secret")
            .expect_err("an unavailable store must reject replacement");
        let removal =
            remove_credential(&store).expect_err("an unavailable store must reject removal");

        assert!(matches!(status, CredentialError::StoreUnavailable));
        assert!(matches!(replacement, CredentialError::StoreUnavailable));
        assert!(matches!(removal, CredentialError::StoreUnavailable));
        assert_eq!(
            store.value.borrow().as_deref(),
            Some("existing-secret"),
            "failure must not clear a retained credential"
        );
    }
}
