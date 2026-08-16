use std::{fs, io::Write, path::PathBuf};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

const QUALIFIER: &str = "io.github.vishal24p";
const APPLICATION: &str = "Roven";
const PROFILES_FILE: &str = "provider-profiles.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProviderProfile {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) endpoint: String,
    pub(crate) model: String,
}

#[derive(Debug, Error)]
pub(crate) enum ProfileError {
    #[error("the operating-system local data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("provider profile name cannot be empty")]
    EmptyName,
    #[error("provider model ID cannot be empty")]
    EmptyModel,
    #[error("a provider profile named `{0}` already exists")]
    DuplicateName(String),
    #[error("provider profile `{0}` does not exist")]
    NotFound(String),
    #[error("provider endpoint must be an HTTPS URL without credentials, query, or fragment")]
    InvalidEndpoint,
    #[error("local provider-profile storage could not be read or written")]
    Io(#[from] std::io::Error),
    #[error("local provider-profile storage contains invalid structured data")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub(crate) struct ProviderProfiles {
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct ProfileDocument {
    profiles: Vec<ProviderProfile>,
    default_profile_id: Option<String>,
}

impl ProviderProfiles {
    pub(crate) fn for_current_user() -> Result<Self, ProfileError> {
        let directories = ProjectDirs::from(QUALIFIER, "", APPLICATION)
            .ok_or(ProfileError::DataDirectoryUnavailable)?;
        Ok(Self::for_data_root(
            directories.data_local_dir().to_path_buf(),
        ))
    }

    pub(crate) fn for_data_root(data_root: PathBuf) -> Self {
        Self {
            path: data_root.join(PROFILES_FILE),
        }
    }

    pub(crate) fn list(&self) -> Result<Vec<ProviderProfile>, ProfileError> {
        Ok(self.read()?.profiles)
    }

    pub(crate) fn default_profile(&self) -> Result<Option<ProviderProfile>, ProfileError> {
        let document = self.read()?;
        Ok(document.default_profile_id.and_then(|id| {
            document
                .profiles
                .into_iter()
                .find(|profile| profile.id == id)
        }))
    }

    pub(crate) fn create(
        &self,
        name: &str,
        endpoint: &str,
        model: &str,
    ) -> Result<ProviderProfile, ProfileError> {
        let name = name.trim();
        if name.is_empty() {
            return Err(ProfileError::EmptyName);
        }
        let model = model.trim();
        if model.is_empty() {
            return Err(ProfileError::EmptyModel);
        }
        let endpoint = normalize_endpoint(endpoint)?;
        let mut document = self.read()?;
        if document.profiles.iter().any(|profile| profile.name == name) {
            return Err(ProfileError::DuplicateName(name.to_owned()));
        }
        let profile = ProviderProfile {
            id: uuid::Uuid::now_v7().to_string(),
            name: name.to_owned(),
            endpoint,
            model: model.to_owned(),
        };
        if document.default_profile_id.is_none() {
            document.default_profile_id = Some(profile.id.clone());
        }
        document.profiles.push(profile.clone());
        self.write(&document)?;
        Ok(profile)
    }

    pub(crate) fn set_default(&self, id: &str) -> Result<(), ProfileError> {
        let mut document = self.read()?;
        if !document.profiles.iter().any(|profile| profile.id == id) {
            return Err(ProfileError::NotFound(id.to_owned()));
        }
        document.default_profile_id = Some(id.to_owned());
        self.write(&document)
    }

    pub(crate) fn switch_model_and_default(
        &self,
        id: &str,
        model: &str,
    ) -> Result<ProviderProfile, ProfileError> {
        let model = model.trim();
        if model.is_empty() {
            return Err(ProfileError::EmptyModel);
        }
        let mut document = self.read()?;
        let profile = document
            .profiles
            .iter_mut()
            .find(|profile| profile.id == id)
            .ok_or_else(|| ProfileError::NotFound(id.to_owned()))?;
        profile.model = model.to_owned();
        document.default_profile_id = Some(id.to_owned());
        let updated = profile.clone();
        self.write(&document)?;
        Ok(updated)
    }

    pub(crate) fn remove(&self, id: &str) -> Result<ProviderProfile, ProfileError> {
        let mut document = self.read()?;
        let position = document
            .profiles
            .iter()
            .position(|profile| profile.id == id)
            .ok_or_else(|| ProfileError::NotFound(id.to_owned()))?;
        let profile = document.profiles.remove(position);
        if document.default_profile_id.as_deref() == Some(id) {
            document.default_profile_id = None;
        }
        self.write(&document)?;
        Ok(profile)
    }

    fn read(&self) -> Result<ProfileDocument, ProfileError> {
        match fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Ok(ProfileDocument::default())
            }
            Err(error) => Err(ProfileError::Io(error)),
        }
    }

    fn write(&self, document: &ProfileDocument) -> Result<(), ProfileError> {
        let parent = self
            .path
            .parent()
            .expect("profile file always has a parent");
        fs::create_dir_all(parent)?;
        let bytes = serde_json::to_vec_pretty(document)?;
        let mut file = AtomicWriteFile::options().open(&self.path)?;
        file.write_all(&bytes)?;
        file.commit()?;
        Ok(())
    }
}

pub(crate) fn normalize_endpoint(value: &str) -> Result<String, ProfileError> {
    let endpoint = Url::parse(value.trim()).map_err(|_| ProfileError::InvalidEndpoint)?;
    if endpoint.scheme() != "https"
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(ProfileError::InvalidEndpoint);
    }
    Ok(endpoint.as_str().trim_end_matches('/').to_owned())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{PROFILES_FILE, ProfileError, ProviderProfiles, normalize_endpoint};

    fn temp_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn creates_a_named_profile_with_a_normalized_endpoint() {
        let data_root = temp_root("provider-profiles");
        let profiles = ProviderProfiles::for_data_root(data_root.clone());

        let profile = profiles
            .create(
                "personal groq",
                "https://api.groq.com/openai/v1/",
                "openai/gpt-oss-20b",
            )
            .unwrap();

        assert_eq!(profile.name, "personal groq");
        assert_eq!(profile.endpoint, "https://api.groq.com/openai/v1");
        assert_eq!(profiles.list().unwrap(), vec![profile.clone()]);
        assert_eq!(profiles.default_profile().unwrap(), Some(profile));
        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn rejects_unsafe_or_incomplete_endpoints() {
        for endpoint in [
            "http://example.test/v1",
            "https://key@example.test/v1",
            "https://example.test/v1?key=secret",
            "https://example.test/v1#fragment",
            "https://",
        ] {
            assert!(normalize_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }

    #[test]
    fn profile_storage_rejects_invalid_input_and_tracks_default_removal() {
        let data_root = temp_root("provider-profile-validation");
        let profiles = ProviderProfiles::for_data_root(data_root.clone());
        assert!(matches!(
            profiles.create(" ", "https://example.test/v1", "model"),
            Err(ProfileError::EmptyName)
        ));
        assert!(matches!(
            profiles.create("profile", "https://example.test/v1", " "),
            Err(ProfileError::EmptyModel)
        ));

        let first = profiles
            .create("first", "https://example.test/v1", "model-one")
            .unwrap();
        assert!(matches!(
            profiles.create("first", "https://example.test/v1", "model-two"),
            Err(ProfileError::DuplicateName(name)) if name == "first"
        ));
        let second = profiles
            .create("second", "https://example.test/v1", "model-two")
            .unwrap();
        profiles.set_default(&second.id).unwrap();
        assert_eq!(profiles.default_profile().unwrap(), Some(second.clone()));
        assert_eq!(profiles.remove(&second.id).unwrap(), second);
        assert_eq!(profiles.default_profile().unwrap(), None);
        assert!(matches!(
            profiles.remove("missing"),
            Err(ProfileError::NotFound(_))
        ));
        assert_eq!(profiles.remove(&first.id).unwrap(), first);
        assert!(profiles.list().unwrap().is_empty());

        fs::write(data_root.join(PROFILES_FILE), b"not json").unwrap();
        assert!(profiles.list().is_err());
        fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn switch_model_and_default_updates_both_fields_in_one_operation() {
        let data_root = temp_root("provider-profile-switch");
        let profiles = ProviderProfiles::for_data_root(data_root.clone());
        let first = profiles
            .create(
                "openrouter",
                "https://openrouter.ai/api/v1/chat/completions",
                "openai/gpt-oss-20b",
            )
            .unwrap();
        let second = profiles
            .create("ollama", "https://ollama.com/api/chat", "minimax-m3:cloud")
            .unwrap();
        profiles.set_default(&first.id).unwrap();

        let updated = profiles
            .switch_model_and_default(&second.id, "gpt-oss:120b-cloud")
            .unwrap();
        let listed = profiles.list().unwrap();

        assert_eq!(updated.id, second.id);
        assert_eq!(updated.model, "gpt-oss:120b-cloud");
        assert_eq!(profiles.default_profile().unwrap().unwrap().id, second.id);
        assert_eq!(
            listed.iter().find(|profile| profile.id == first.id).unwrap().model,
            "openai/gpt-oss-20b"
        );
        assert_eq!(
            listed
                .iter()
                .find(|profile| profile.id == second.id)
                .unwrap()
                .model,
            "gpt-oss:120b-cloud"
        );
        fs::remove_dir_all(data_root).unwrap();
    }
}
