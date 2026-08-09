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
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

const QUALIFIER: &str = "io.github.vishal24p";
const APPLICATION: &str = "PMEMC";

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("the operating-system local data directory is unavailable")]
    DataDirectoryUnavailable,
    #[error("local PMEMC storage could not be read or written")]
    Io(#[from] std::io::Error),
    #[error("local PMEMC storage contains invalid structured data")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ProjectMeta {
    pub(crate) canonical_path: String,
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
    Tool,
    Error,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct ConversationEvent {
    pub(crate) kind: EventKind,
    pub(crate) content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) duration_ms: Option<u64>,
    pub(crate) created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub(crate) struct ContextState {
    pub(crate) summary: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct ProjectStore {
    project_id: String,
    project_dir: PathBuf,
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
        let project_dir = data_root.join("projects").join(&project_id);
        fs::create_dir_all(&project_dir)?;
        let project_meta = ProjectMeta {
            canonical_path: canonical_root.to_string_lossy().into_owned(),
        };
        write_json(&project_dir.join("project.json"), &project_meta)?;
        Ok(Self {
            project_id,
            project_dir,
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
        self.project_dir.join("sessions")
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

    use super::{ConversationEvent, EventKind, ProjectStore, project_id};

    fn temp_root(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("pmemc-{name}-{}", uuid::Uuid::now_v7()));
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
    fn session_is_created_only_when_requested_and_events_are_durable() {
        let data = temp_root("data");
        let project = temp_root("project");
        let store = ProjectStore::for_project(&data, &project).expect("store should initialize");
        assert!(store.list_sessions().unwrap().is_empty());

        let session = store.create_session("Investigate the build").unwrap();
        let event = ConversationEvent {
            kind: EventKind::User,
            content: "Investigate the build".to_owned(),
            duration_ms: None,
            created_at_ms: 1,
        };
        store.append_event(&session.id, &event).unwrap();
        let thought = ConversationEvent {
            kind: EventKind::Thought,
            content: "Inspect the failing request.".to_owned(),
            duration_ms: Some(757),
            created_at_ms: 2,
        };
        store.append_event(&session.id, &thought).unwrap();

        let listed = store.list_sessions().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, session.id);
        assert!(listed[0].updated_at_ms >= session.updated_at_ms);
        assert_eq!(store.events(&session.id).unwrap(), vec![event, thought]);
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }
}
