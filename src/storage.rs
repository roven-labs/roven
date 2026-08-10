//! Project-scoped, crash-resistant conversation storage.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const QUALIFIER: &str = "io.github.vishal24p";
const APPLICATION: &str = "Roven";

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("the operating-system local data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("local Roven storage could not be read or written")]
    Io(#[from] std::io::Error),
    #[error("local Roven storage contains invalid structured data")]
    Json(#[from] serde_json::Error),
}

/// Minimal durable identity for a project that has passed preparation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProjectRegistration {
    #[serde(default)]
    pub(crate) name: String,
    pub(crate) canonical_path: String,
    pub(crate) github_remote: String,
    pub(crate) baseline_commit: String,
    pub(crate) registration_state: RegistrationState,
    #[serde(default)]
    pub(crate) project_context: ProjectContext,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProjectContext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) problem_solved: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) role_and_responsibilities: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tech_stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) architecture: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) key_features: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) technical_challenges: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) outcomes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RegistrationState {
    Registered,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrationLookup {
    Absent,
    Registered(Box<ProjectRegistration>),
}

/// Registration storage deliberately separate from conversation storage.
/// It performs no writes until a project has passed all preparation checks.
#[derive(Debug, Clone)]
pub(crate) struct ProjectRegistry {
    data_root: PathBuf,
}

impl ProjectRegistry {
    pub(crate) fn for_current_user() -> Result<Self, StorageError> {
        let project_dirs = ProjectDirs::from(QUALIFIER, "", APPLICATION)
            .ok_or(StorageError::DataDirectoryUnavailable)?;
        Ok(Self::for_data_root(project_dirs.data_local_dir()))
    }

    pub(crate) fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub(crate) fn lookup(&self, project_root: &Path) -> Result<RegistrationLookup, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        if let Some(registration) = self.find_registration(&canonical_root)? {
            return Ok(RegistrationLookup::Registered(Box::new(registration)));
        }
        Ok(RegistrationLookup::Absent)
    }

    pub(crate) fn register(
        &self,
        project_root: &Path,
        github_remote: String,
        baseline_commit: String,
    ) -> Result<ProjectRegistration, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        let name = project_name(&canonical_root);
        let registration = ProjectRegistration {
            name: name.clone(),
            canonical_path: canonical_root.to_string_lossy().into_owned(),
            github_remote,
            baseline_commit,
            registration_state: RegistrationState::Registered,
            project_context: ProjectContext::default(),
        };
        let project_dir = self.projects_dir();
        fs::create_dir_all(&project_dir)?;
        write_json(
            &self.project_registration_file(&canonical_root, &name)?,
            &registration,
        )?;
        Ok(registration)
    }

    fn projects_dir(&self) -> PathBuf {
        self.data_root.join("projects")
    }

    fn find_registration(
        &self,
        project_root: &Path,
    ) -> Result<Option<ProjectRegistration>, StorageError> {
        let projects_dir = self.projects_dir();
        if !projects_dir.exists() {
            return Ok(None);
        }
        let canonical_path = project_root.to_string_lossy();
        for entry in fs::read_dir(projects_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
                continue;
            }
            let mut registration: ProjectRegistration = read_json(&path)?;
            if registration.canonical_path == canonical_path {
                if registration.name.is_empty() {
                    registration.name = project_name(project_root);
                }
                return Ok(Some(registration));
            }
        }
        Ok(None)
    }

    fn project_registration_file(
        &self,
        project_root: &Path,
        name: &str,
    ) -> Result<PathBuf, StorageError> {
        let base = safe_project_file_stem(name);
        let default_path = self.projects_dir().join(format!("{base}.json"));
        if self.file_is_available_for_project(&default_path, project_root)? {
            return Ok(default_path);
        }
        Err(StorageError::Io(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "project registration file name collision",
        )))
    }

    fn file_is_available_for_project(
        &self,
        path: &Path,
        project_root: &Path,
    ) -> Result<bool, StorageError> {
        if !path.exists() {
            return Ok(true);
        }
        let registration: ProjectRegistration = read_json(path)?;
        Ok(registration.canonical_path == project_root.to_string_lossy())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionMeta {
    pub(crate) id: String,
    pub(crate) project_id: String,
    pub(crate) title: String,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum EventKind {
    User,
    Thought,
    Assistant,
    #[serde(alias = "tool")]
    FunctionCallOutput,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationEvent {
    pub(crate) kind: EventKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_output: Option<Value>,
    pub(crate) created_at_ms: u64,
}

impl ConversationEvent {
    pub(crate) fn message(kind: EventKind, content: String, duration_ms: Option<u64>) -> Self {
        Self {
            kind,
            content,
            duration_ms,
            tool_call_id: None,
            tool_name: None,
            tool_input: None,
            tool_output: None,
            created_at_ms: now_ms(),
        }
    }

    pub(crate) fn function_call_output(
        tool_call_id: String,
        tool_name: String,
        tool_input: Value,
        tool_output: Value,
    ) -> Self {
        Self {
            kind: EventKind::FunctionCallOutput,
            content: String::new(),
            duration_ms: None,
            tool_call_id: Some(tool_call_id),
            tool_name: Some(tool_name),
            tool_input: Some(tool_input),
            tool_output: Some(tool_output),
            created_at_ms: now_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct ContextState {
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectStore {
    project_id: String,
    session_root: PathBuf,
}

impl ProjectStore {
    pub(crate) fn for_current_directory() -> Result<Self, StorageError> {
        let project_root = std::env::current_dir()?.canonicalize()?;
        let project_dirs = ProjectDirs::from(QUALIFIER, "", APPLICATION)
            .ok_or(StorageError::DataDirectoryUnavailable)?;
        Self::for_project(project_dirs.data_local_dir(), &project_root)
    }

    pub(crate) fn for_project(data_root: &Path, project_root: &Path) -> Result<Self, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        let project_id = project_id(&canonical_root);
        let session_root = data_root.join("sessions").join(&project_id);
        Ok(Self {
            project_id,
            session_root,
        })
    }

    pub(crate) fn create_session(&self, first_message: &str) -> Result<SessionMeta, StorageError> {
        let created_at_ms = now_ms();
        let id = Uuid::now_v7().to_string();
        let session_dir = self.sessions_dir().join(&id);
        fs::create_dir_all(&session_dir)?;
        let meta = SessionMeta {
            id,
            project_id: self.project_id.clone(),
            title: title_from(first_message),
            created_at_ms,
            updated_at_ms: created_at_ms,
        };
        write_json(&session_dir.join("meta.json"), &meta)?;
        write_json(&session_dir.join("context.json"), &ContextState::default())?;
        Ok(meta)
    }

    pub(crate) fn append_event(
        &self,
        session_id: &str,
        event: &ConversationEvent,
    ) -> Result<(), StorageError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.session_dir(session_id).join("events.jsonl"))?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        file.sync_data()?;
        let meta_path = self.session_dir(session_id).join("meta.json");
        let mut meta: SessionMeta = read_json(&meta_path)?;
        meta.updated_at_ms = now_ms();
        write_json(&meta_path, &meta)?;
        Ok(())
    }

    pub(crate) fn list_sessions(&self) -> Result<Vec<SessionMeta>, StorageError> {
        let mut sessions = Vec::new();
        let path = self.sessions_dir();
        if !path.exists() {
            return Ok(sessions);
        }
        for entry in fs::read_dir(path)? {
            let meta_path = entry?.path().join("meta.json");
            if meta_path.is_file() {
                sessions.push(read_json(&meta_path)?);
            }
        }
        sessions.sort_by_key(|session| std::cmp::Reverse(session.updated_at_ms));
        Ok(sessions)
    }

    pub(crate) fn events(&self, session_id: &str) -> Result<Vec<ConversationEvent>, StorageError> {
        let path = self.session_dir(session_id).join("events.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let contents = fs::read_to_string(path)?;
        contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str)
            .collect::<Result<Vec<_>, _>>()
            .map_err(StorageError::from)
    }

    fn sessions_dir(&self) -> PathBuf {
        self.session_root.clone()
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.sessions_dir().join(session_id)
    }
}

pub(crate) fn project_id(project_root: &Path) -> String {
    let canonical = project_root
        .canonicalize()
        .unwrap_or_else(|_| project_root.into());
    format!(
        "{:x}",
        Sha256::digest(canonical.to_string_lossy().as_bytes())
    )
}

fn project_name(project_root: &Path) -> String {
    project_root
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("project")
        .to_owned()
}

fn safe_project_file_stem(name: &str) -> String {
    let sanitized = name
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            character if character.is_control() => '-',
            character => character,
        })
        .collect::<String>()
        .trim_matches([' ', '.'])
        .to_owned();
    if sanitized.is_empty() {
        "project".to_owned()
    } else {
        sanitized
    }
}

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn title_from(message: &str) -> String {
    let title: String = message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(80)
        .collect();
    if title.trim().is_empty() {
        "New conversation".to_owned()
    } else {
        title
    }
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), StorageError> {
    let bytes = serde_json::to_vec_pretty(value)?;
    let mut file = AtomicWriteFile::options().open(path)?;
    file.write_all(&bytes)?;
    file.commit()?;
    Ok(())
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, StorageError> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::Value;

    use super::{
        ConversationEvent, EventKind, ProjectRegistry, ProjectStore, RegistrationLookup, project_id,
    };

    fn temp_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).expect("temporary root should exist");
        path
    }

    #[test]
    fn project_id_is_stable_for_a_canonical_path() {
        let root = temp_root("project-id");
        assert_eq!(project_id(&root), project_id(&root.canonicalize().unwrap()));
        fs::remove_dir_all(root).expect("temporary root should be removed");
    }

    #[test]
    fn registration_is_written_as_a_project_named_json_file() {
        let data = temp_root("data");
        let project = temp_root("visible-project");
        let registry = ProjectRegistry::for_data_root(&data);

        let registration = registry
            .register(
                &project,
                "https://github.com/roven/visible-project.git".to_owned(),
                "abc123".to_owned(),
            )
            .unwrap();

        let path = data.join("projects").join(format!(
            "{}.json",
            project.file_name().unwrap().to_string_lossy()
        ));
        assert!(path.is_file());
        let value: Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert!(value.get("id").is_none());
        assert!(value.get("created_at_ms").is_none());
        assert!(value.get("updated_at_ms").is_none());
        assert_eq!(value["name"], registration.name);
        assert_eq!(value["canonical_path"], registration.canonical_path);
        assert_eq!(value["project_context"], serde_json::json!({}));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn registry_lookup_scans_project_json_files_by_canonical_path() {
        let data = temp_root("data");
        let project = temp_root("scan-project");
        let registry = ProjectRegistry::for_data_root(&data);
        let registration = registry
            .register(
                &project,
                "https://github.com/roven/scan-project.git".to_owned(),
                "abc123".to_owned(),
            )
            .unwrap();
        let projects_dir = data.join("projects");
        let original = projects_dir.join(format!(
            "{}.json",
            project.file_name().unwrap().to_string_lossy()
        ));
        let renamed = projects_dir.join("user-chosen-name.json");
        fs::rename(original, renamed).unwrap();

        assert_eq!(
            registry.lookup(&project).unwrap(),
            RegistrationLookup::Registered(Box::new(registration))
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn registry_does_not_duplicate_an_existing_canonical_path() {
        let data = temp_root("data");
        let project = temp_root("duplicate-project");
        let registry = ProjectRegistry::for_data_root(&data);
        registry
            .register(
                &project,
                "https://github.com/roven/duplicate-project.git".to_owned(),
                "abc123".to_owned(),
            )
            .unwrap();
        registry
            .register(
                &project,
                "https://github.com/roven/duplicate-project.git".to_owned(),
                "def456".to_owned(),
            )
            .unwrap();

        let files = fs::read_dir(data.join("projects"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|extension| extension.to_str())
                    == Some("json")
            })
            .count();
        assert_eq!(files, 1);
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn same_project_folder_names_do_not_overwrite_each_other() {
        let data = temp_root("data");
        let parent = temp_root("same-name-parent");
        let left = parent.join("left").join("app");
        let right = parent.join("right").join("app");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        let registry = ProjectRegistry::for_data_root(&data);

        registry
            .register(
                &left,
                "https://github.com/roven/left-app.git".to_owned(),
                "abc123".to_owned(),
            )
            .unwrap();
        registry
            .register(
                &right,
                "https://github.com/roven/right-app.git".to_owned(),
                "def456".to_owned(),
            )
            .unwrap_err();

        assert!(data.join("projects").join("app.json").is_file());
        assert_eq!(
            registry.lookup(&left).unwrap(),
            RegistrationLookup::Registered(Box::new(
                registry
                    .find_registration(&left.canonicalize().unwrap())
                    .unwrap()
                    .unwrap(),
            ))
        );
        assert!(matches!(
            registry.lookup(&right).unwrap(),
            RegistrationLookup::Absent
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn session_is_created_only_when_requested_and_events_are_durable() {
        let data = temp_root("data");
        let project = temp_root("project");
        let store = ProjectStore::for_project(&data, &project).expect("store should initialize");
        assert!(
            !data.join("projects").exists(),
            "session setup must not create project registrations"
        );
        assert!(
            !data.join("sessions").exists(),
            "session setup must not create session files until a session starts"
        );
        assert!(store.list_sessions().unwrap().is_empty());

        let session = store.create_session("Investigate the build").unwrap();
        assert!(!data.join("projects").exists());
        assert!(
            data.join("sessions")
                .join(project_id(&project))
                .join(&session.id)
                .is_dir()
        );
        let event =
            ConversationEvent::message(EventKind::User, "Investigate the build".to_owned(), None);
        store.append_event(&session.id, &event).unwrap();
        let thought = ConversationEvent::message(
            EventKind::Thought,
            "Inspect the failing request.".to_owned(),
            Some(757),
        );
        store.append_event(&session.id, &thought).unwrap();

        let listed = store.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, session.id);
        assert!(listed[0].updated_at_ms >= session.updated_at_ms);
        assert_eq!(store.events(&session.id).unwrap(), vec![event, thought]);
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn function_call_output_is_stored_as_structured_json() {
        let data = temp_root("function-call-data");
        let project = temp_root("function-call-project");
        let store = ProjectStore::for_project(&data, &project).unwrap();
        let session = store.create_session("Inspect the workspace").unwrap();
        let event = ConversationEvent::function_call_output(
            "call-1".to_owned(),
            "list_directory".to_owned(),
            serde_json::json!({"path": "."}),
            serde_json::json!({"status": "ok", "entries": ["src"]}),
        );
        store.append_event(&session.id, &event).unwrap();

        let line = fs::read_to_string(
            data.join("sessions")
                .join(project_id(&project))
                .join(&session.id)
                .join("events.jsonl"),
        )
        .unwrap();
        let value: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["kind"], "function_call_output");
        assert_eq!(value["tool_call_id"], "call-1");
        assert_eq!(value["tool_name"], "list_directory");
        assert_eq!(value["tool_input"], serde_json::json!({"path": "."}));
        assert_eq!(
            value["tool_output"],
            serde_json::json!({"status": "ok", "entries": ["src"]})
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }
}
