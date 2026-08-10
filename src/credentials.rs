//! OpenRouter API-key storage in Windows Credential Manager.

use thiserror::Error;

const SERVICE_NAME: &str = "roven";

/// Errors from local credential management without exposing the credential.
#[derive(Debug, Error)]
pub(crate) enum CredentialError {
    #[error("the operating-system credential store is unavailable")]
    StoreUnavailable,
    #[error("the credential value cannot be empty")]
    EmptyValue,
    #[error("the credential confirmation did not match")]
    ConfirmationMismatch,
}

/// Minimal secret-store boundary used by the auth CLI.
pub(crate) trait SecretStore {
    fn get(&self) -> Result<Option<String>, CredentialError>;
    fn set(&self, secret: &str) -> Result<(), CredentialError>;
    fn delete(&self) -> Result<bool, CredentialError>;
}

/// Native operating-system credential-store entry for one provider profile.
#[derive(Debug, Clone)]
pub(crate) struct OsCredentialStore {
    account: String,
}

impl OsCredentialStore {
    pub(crate) fn for_profile_id(profile_id: &str) -> Self {
        Self {
            account: credential_account(profile_id),
        }
    }

    fn entry(&self) -> Result<keyring::Entry, CredentialError> {
        keyring::Entry::new(SERVICE_NAME, &self.account)
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

fn credential_account(profile_id: &str) -> String {
    format!("provider-profile:{profile_id}")
}

pub(crate) fn store_confirmed_api_key(
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
mod tests {
    use std::cell::RefCell;

    use super::{
        CredentialError, OsCredentialStore, SecretStore, credential_account,
        store_confirmed_api_key,
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
    fn confirmed_key_is_stored_without_returning_or_printing_it() {
        let store = MemoryStore::default();
        store_confirmed_api_key(&store, "prompted-secret", "prompted-secret")
            .expect("confirmed credential should store");
        assert_eq!(
            store.get().expect("stored credential should read"),
            Some("prompted-secret".into())
        );
    }

    #[test]
    fn empty_secret_does_not_store_a_credential() {
        let store = MemoryStore::default();
        let empty = store_confirmed_api_key(&store, " ", " ");
        assert!(matches!(empty, Err(CredentialError::EmptyValue)));
        assert_eq!(store.get().expect("store should remain empty"), None);
    }

    #[test]
    fn empty_secret_cannot_be_stored() {
        let error = OsCredentialStore::for_profile_id("test-profile")
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

        assert!(
            configured
                .get()
                .expect("configured store should be readable")
                .is_some()
        );
        assert!(
            missing
                .get()
                .expect("missing store should be readable")
                .is_none()
        );
    }

    #[test]
    fn removal_distinguishes_present_and_absent_entries() {
        let present = MemoryStore {
            value: RefCell::new(Some("stored-secret".into())),
            failure: false,
        };
        let absent = MemoryStore::default();

        assert!(
            present
                .delete()
                .expect("present credential should be removed")
        );
        assert!(
            !absent
                .delete()
                .expect("absent credential should not be removed")
        );
        assert_eq!(present.get().expect("store should be readable"), None);
    }

    #[test]
    fn confirmation_mismatch_preserves_an_existing_credential() {
        let store = MemoryStore {
            value: RefCell::new(Some("existing-secret".into())),
            failure: false,
        };

        let error = store_confirmed_api_key(&store, "new-secret", "different-secret")
            .expect_err("mismatched confirmation must fail");

        assert!(matches!(error, CredentialError::ConfirmationMismatch));
        assert_eq!(
            store.get().expect("store should be readable"),
            Some("existing-secret".into())
        );
    }

    #[test]
    fn profile_keys_use_distinct_credential_accounts() {
        assert_ne!(
            credential_account("profile-a"),
            credential_account("profile-b")
        );
    }

    #[test]
    fn confirmed_profile_key_requires_matching_confirmation() {
        let store = MemoryStore {
            value: RefCell::new(Some("existing-secret".into())),
            failure: false,
        };

        let error = store_confirmed_api_key(&store, "new-secret", "different-secret")
            .expect_err("mismatched confirmation must fail");

        assert!(matches!(error, CredentialError::ConfirmationMismatch));
        assert_eq!(store.get().unwrap().as_deref(), Some("existing-secret"));
    }

    #[test]
    fn store_failures_return_safe_errors_without_changing_a_retained_credential() {
        let store = MemoryStore {
            value: RefCell::new(Some("existing-secret".into())),
            failure: true,
        };

        let status = store
            .get()
            .expect_err("an unavailable store must make status fail safely");
        let replacement = store_confirmed_api_key(&store, "new-secret", "new-secret")
            .expect_err("an unavailable store must reject replacement");
        let removal = store
            .delete()
            .expect_err("an unavailable store must reject removal");

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
