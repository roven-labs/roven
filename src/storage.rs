//! Project-scoped, crash-resistant conversation storage.

use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub(crate) enum StorageError {
    #[error("local Roven storage could not be read or written")]
    Io(#[from] std::io::Error),
    #[error("local Roven storage contains invalid structured data")]
    Json(#[from] serde_json::Error),
    #[error("project is already registered")]
    AlreadyRegistered,
    #[error("project name is already registered: {0}")]
    DuplicateProjectName(String),
    #[error("local Roven storage contains invalid project data: {0}")]
    InvalidProjectData(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct ProjectSnapshot {
    pub(crate) project_name: String,
    pub(crate) project_facts: Vec<String>,
    pub(crate) user_context_facts: Vec<String>,
    pub(crate) user_contribution_facts: Vec<String>,
}

impl ProjectSnapshot {
    fn validate(&self) -> Result<(), StorageError> {
        if self.project_name.trim().is_empty() {
            return Err(StorageError::InvalidProjectData(
                "project_name must not be blank".to_owned(),
            ));
        }
        if self
            .project_facts
            .iter()
            .chain(&self.user_context_facts)
            .chain(&self.user_contribution_facts)
            .any(|fact| fact.trim().is_empty())
        {
            return Err(StorageError::InvalidProjectData(
                "fact entries must not be blank".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(crate) struct RepositoryMetadata {
    pub(crate) github_remote: String,
    pub(crate) baseline_commit: String,
}

impl RepositoryMetadata {
    fn validate(&self) -> Result<(), StorageError> {
        if self.github_remote.trim().is_empty() {
            return Err(StorageError::InvalidProjectData(
                "github_remote must not be blank".to_owned(),
            ));
        }
        if self.baseline_commit.trim().is_empty() {
            return Err(StorageError::InvalidProjectData(
                "baseline_commit must not be blank".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RegistrationLookup {
    Absent,
    Registered(Box<ProjectSnapshot>),
}

/// Registration storage deliberately separate from conversation storage.
/// It performs no writes until a project has passed all preparation checks.
#[derive(Debug, Clone)]
pub(crate) struct ProjectRegistry {
    data_root: PathBuf,
}

#[derive(Debug, Clone)]
pub(crate) struct ResumeStore {
    data_root: PathBuf,
}

impl ResumeStore {
    pub(crate) fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub(crate) fn save(&self, markdown: &str) -> Result<PathBuf, StorageError> {
        if markdown.trim().is_empty() {
            return Err(StorageError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "resume Markdown must not be empty",
            )));
        }
        let resumes_dir = self.data_root.join("resumes");
        fs::create_dir_all(&resumes_dir)?;
        let path = resumes_dir.join(format!("{}.md", Uuid::now_v7()));
        let mut file = AtomicWriteFile::options().open(&path)?;
        file.write_all(markdown.as_bytes())?;
        file.commit()?;
        Ok(path)
    }
}

impl ProjectRegistry {
    pub(crate) fn for_current_user() -> Result<Self, StorageError> {
        Ok(Self::for_data_root(crate::app_data_root()?))
    }

    pub(crate) fn for_data_root(data_root: impl Into<PathBuf>) -> Self {
        Self {
            data_root: data_root.into(),
        }
    }

    pub(crate) fn lookup(&self, project_root: &Path) -> Result<RegistrationLookup, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        let project_dir = self.project_dir(&canonical_root);
        if project_dir.exists() {
            let (snapshot, _) = self.read(&canonical_root)?;
            return Ok(RegistrationLookup::Registered(Box::new(snapshot)));
        }
        Ok(RegistrationLookup::Absent)
    }

    pub(crate) fn list_snapshots(&self) -> Result<Vec<ProjectSnapshot>, StorageError> {
        let projects_dir = self.projects_dir();
        if !projects_dir.exists() {
            return Ok(Vec::new());
        }
        let mut names = BTreeSet::new();
        let mut snapshots = Vec::new();
        for entry in fs::read_dir(projects_dir)? {
            let path = entry?.path();
            if !path.is_dir() {
                return Err(StorageError::InvalidProjectData(
                    "project storage entries must be directories".to_owned(),
                ));
            }
            let (snapshot, _) = self.read_project_dir(&path)?;
            if !names.insert(snapshot.project_name.clone()) {
                return Err(StorageError::DuplicateProjectName(snapshot.project_name));
            }
            snapshots.push(snapshot);
        }
        snapshots.sort_by(|left, right| left.project_name.cmp(&right.project_name));
        Ok(snapshots)
    }

    pub(crate) fn register(
        &self,
        project_root: &Path,
        snapshot: ProjectSnapshot,
        metadata: &RepositoryMetadata,
    ) -> Result<ProjectSnapshot, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        snapshot.validate()?;
        metadata.validate()?;
        if self
            .list_snapshots()?
            .iter()
            .any(|existing| existing.project_name == snapshot.project_name)
        {
            return Err(StorageError::DuplicateProjectName(
                snapshot.project_name.clone(),
            ));
        }
        fs::create_dir_all(self.projects_dir())?;
        let project_dir = self.project_dir(&canonical_root);
        if project_dir.exists() {
            return Err(StorageError::AlreadyRegistered);
        }
        fs::create_dir(&project_dir)?;
        if let Err(error) = write_json(&project_dir.join("repository_metadata.json"), &metadata)
            .and_then(|()| write_json(&project_dir.join("project_snapshot.json"), &snapshot))
        {
            let _ = fs::remove_dir_all(&project_dir);
            return Err(error);
        }
        Ok(snapshot)
    }

    pub(crate) fn read(
        &self,
        project_root: &Path,
    ) -> Result<(ProjectSnapshot, RepositoryMetadata), StorageError> {
        let canonical_root = project_root.canonicalize()?;
        self.read_project_dir(&self.project_dir(&canonical_root))
    }

    fn projects_dir(&self) -> PathBuf {
        self.data_root.join("projects")
    }

    fn project_dir(&self, project_root: &Path) -> PathBuf {
        self.projects_dir().join(project_id(project_root))
    }

    fn read_project_dir(
        &self,
        project_dir: &Path,
    ) -> Result<(ProjectSnapshot, RepositoryMetadata), StorageError> {
        let snapshot: ProjectSnapshot = read_json(&project_dir.join("project_snapshot.json"))?;
        let metadata: RepositoryMetadata =
            read_json(&project_dir.join("repository_metadata.json"))?;
        snapshot.validate()?;
        metadata.validate()?;
        Ok((snapshot, metadata))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct SessionMeta {
    pub(crate) id: String,
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

#[derive(Debug, Clone)]
pub(crate) struct ProjectStore {
    session_root: PathBuf,
}

impl ProjectStore {
    pub(crate) fn for_current_directory() -> Result<Self, StorageError> {
        let project_root = std::env::current_dir()?.canonicalize()?;
        let data_root = crate::app_data_root()?;
        Self::for_project(&data_root, &project_root)
    }

    pub(crate) fn for_project(data_root: &Path, project_root: &Path) -> Result<Self, StorageError> {
        let canonical_root = project_root.canonicalize()?;
        let session_root = data_root.join("sessions").join(project_id(&canonical_root));
        Ok(Self { session_root })
    }

    pub(crate) fn create_session(&self, first_message: &str) -> Result<SessionMeta, StorageError> {
        let created_at_ms = now_ms();
        let id = Uuid::now_v7().to_string();
        let session_dir = self.sessions_dir().join(&id);
        fs::create_dir_all(&session_dir)?;
        let meta = SessionMeta {
            id,
            title: title_from(first_message),
            created_at_ms,
            updated_at_ms: created_at_ms,
        };
        write_json(&session_dir.join("meta.json"), &meta)?;
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
        ConversationEvent, EventKind, ProjectRegistry, ProjectSnapshot, ProjectStore,
        RepositoryMetadata, ResumeStore, StorageError, project_id,
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

    fn snapshot(name: &str) -> ProjectSnapshot {
        ProjectSnapshot {
            project_name: name.to_owned(),
            project_facts: vec!["Uses PostgreSQL.".to_owned()],
            user_context_facts: vec!["Team project.".to_owned()],
            user_contribution_facts: vec!["Built authentication.".to_owned()],
        }
    }

    fn metadata() -> RepositoryMetadata {
        RepositoryMetadata {
            github_remote: "https://github.com/example/project.git".to_owned(),
            baseline_commit: "abc123".to_owned(),
        }
    }

    #[test]
    fn writes_and_reads_a_v2_snapshot_with_repository_metadata() {
        let data = temp_root("v2-data");
        let project = temp_root("project");
        let registry = ProjectRegistry::for_data_root(&data);
        let expected = snapshot("PayFlow");
        let expected_metadata = metadata();

        registry
            .register(&project, expected.clone(), &expected_metadata)
            .unwrap();

        let project_dir = data.join("projects").join(project_id(&project));
        assert!(project_dir.join("project_snapshot.json").is_file());
        assert!(project_dir.join("repository_metadata.json").is_file());
        assert_eq!(
            registry.read(&project).unwrap(),
            (expected, expected_metadata)
        );
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn snapshot_json_has_exact_v2_fields() {
        let value = serde_json::to_value(snapshot("Project")).unwrap();
        assert_eq!(
            value,
            serde_json::json!({
                "project_name": "Project",
                "project_facts": ["Uses PostgreSQL."],
                "user_context_facts": ["Team project."],
                "user_contribution_facts": ["Built authentication."]
            })
        );
        assert!(
            serde_json::from_value::<ProjectSnapshot>(serde_json::json!({
                "project_name": "Project",
                "project_facts": [],
                "user_context_facts": [],
                "user_contribution_facts": [],
                "summary": "obsolete"
            }))
            .is_err()
        );
    }

    #[test]
    fn rejects_malformed_json() {
        let data = temp_root("malformed");
        let project = temp_root("project");
        let project_dir = data.join("projects").join(project_id(&project));
        fs::create_dir_all(&project_dir).unwrap();
        fs::write(project_dir.join("project_snapshot.json"), "not json").unwrap();
        fs::write(
            project_dir.join("repository_metadata.json"),
            serde_json::to_vec(&metadata()).unwrap(),
        )
        .unwrap();

        assert!(matches!(
            ProjectRegistry::for_data_root(&data).list_snapshots(),
            Err(StorageError::Json(_))
        ));
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn lists_snapshot_names_alphabetically() {
        let data = temp_root("sorted");
        let registry = ProjectRegistry::for_data_root(&data);
        for (folder, name) in [("zeta", "Zeta"), ("alpha", "Alpha")] {
            let project = temp_root(folder);
            registry
                .register(&project, snapshot(name), &metadata())
                .unwrap();
        }

        let names = registry
            .list_snapshots()
            .unwrap()
            .into_iter()
            .map(|snapshot| snapshot.project_name)
            .collect::<Vec<_>>();
        assert_eq!(names, ["Alpha", "Zeta"]);
        fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn rejects_duplicate_path_without_overwriting() {
        let data = temp_root("duplicate-path");
        let project = temp_root("project");
        let registry = ProjectRegistry::for_data_root(&data);
        let first = snapshot("First");
        registry
            .register(&project, first.clone(), &metadata())
            .unwrap();

        assert!(matches!(
            registry.register(&project, snapshot("Second"), &metadata()),
            Err(StorageError::AlreadyRegistered)
        ));
        assert_eq!(registry.read(&project).unwrap().0, first);
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn rejects_duplicate_project_name_across_paths() {
        let data = temp_root("duplicate-name");
        let first = temp_root("first");
        let second = temp_root("second");
        let registry = ProjectRegistry::for_data_root(&data);
        registry
            .register(&first, snapshot("Same"), &metadata())
            .unwrap();

        assert!(matches!(
            registry.register(&second, snapshot("Same"), &metadata()),
            Err(StorageError::DuplicateProjectName(_))
        ));
        assert!(!data.join("projects").join(project_id(&second)).exists());
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(first).unwrap();
        fs::remove_dir_all(second).unwrap();
    }

    #[test]
    fn rejects_blank_snapshot_and_metadata_strings() {
        let data = temp_root("invalid-values");
        let project = temp_root("project");
        let registry = ProjectRegistry::for_data_root(&data);

        assert!(
            registry
                .register(&project, snapshot(" "), &metadata())
                .is_err()
        );
        assert!(
            registry
                .register(
                    &project,
                    snapshot("Valid"),
                    &RepositoryMetadata {
                        github_remote: " ".to_owned(),
                        baseline_commit: "abc".to_owned(),
                    }
                )
                .is_err()
        );
        assert!(!data.join("projects").exists());
        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(project).unwrap();
    }

    #[test]
    fn unsupported_v1_file_rejects_listing() {
        let data = temp_root("legacy-file");
        let projects = data.join("projects");
        fs::create_dir_all(&projects).unwrap();
        fs::write(
            projects.join("old.json"),
            serde_json::to_vec(&serde_json::json!({
                "name": "Old",
                "canonical_path": "C:\\\\old",
                "github_remote": "https://github.com/example/old.git",
                "baseline_commit": "abc"
            }))
            .unwrap(),
        )
        .unwrap();

        assert!(
            ProjectRegistry::for_data_root(&data)
                .list_snapshots()
                .is_err()
        );
        fs::remove_dir_all(data).unwrap();
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
        let session_dir = data
            .join("sessions")
            .join(project_id(&project))
            .join(&session.id);
        let meta: Value =
            serde_json::from_slice(&fs::read(session_dir.join("meta.json")).unwrap()).unwrap();
        assert!(meta.get("project_id").is_none());
        assert!(!session_dir.join("context.json").exists());
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
    fn sessions_remain_scoped_to_the_canonical_project_workspace() {
        let data = temp_root("scoped-session-data");
        let current_project = temp_root("current-session-project");
        let other_project = temp_root("other-session-project");
        let current = ProjectStore::for_project(&data, &current_project).unwrap();
        let same_current = ProjectStore::for_project(&data, &current_project.join(".")).unwrap();
        let other = ProjectStore::for_project(&data, &other_project).unwrap();

        let current_session = current.create_session("Current session").unwrap();
        let other_session = other.create_session("Other session").unwrap();

        assert_eq!(
            same_current
                .list_sessions()
                .unwrap()
                .into_iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![current_session.id]
        );
        assert_eq!(
            other
                .list_sessions()
                .unwrap()
                .into_iter()
                .map(|session| session.id)
                .collect::<Vec<_>>(),
            vec![other_session.id]
        );

        fs::remove_dir_all(data).unwrap();
        fs::remove_dir_all(current_project).unwrap();
        fs::remove_dir_all(other_project).unwrap();
    }

    #[test]
    fn resume_store_saves_exact_markdown_atomically_below_resumes() {
        let data = temp_root("resume-store-data");
        let path = ResumeStore::for_data_root(data.clone())
            .save("# Projects\n\n- factual result")
            .unwrap();

        assert!(path.starts_with(data.join("resumes")));
        assert_eq!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("md")
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Projects\n\n- factual result"
        );

        fs::remove_dir_all(data).unwrap();
    }

    #[test]
    fn resume_store_rejects_empty_markdown_without_creating_resumes() {
        let data = temp_root("empty-resume-store-data");
        assert!(
            ResumeStore::for_data_root(data.clone())
                .save(" \n\t")
                .is_err()
        );
        assert!(!data.join("resumes").exists());
        fs::remove_dir_all(data).unwrap();
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
