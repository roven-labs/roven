//! Local data-directory and SQLite initialization.

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, OptionalExtension, params};
use serde_json::Value;
use thiserror::Error;

use crate::{
    inspection::{EvidenceBundle, EvidenceFile},
    provider::{
        ProviderFailureCategory, ProviderInvocationMetadata, ProviderResponse, validate_response,
    },
};

const APPLICATION_DIRECTORY: &str = "PMEMC";
const DATABASE_FILE: &str = "pmemc.sqlite3";
const CACHE_DIRECTORY: &str = "cache";
const EXPORTS_DIRECTORY: &str = "exports";
const CODE_MAP_SNAPSHOT_SCHEMA_VERSION: i64 = 1;

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        sql: "
        CREATE TABLE pmemc_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ",
    },
    Migration {
        version: 2,
        sql: "
        CREATE TABLE projects (
            id INTEGER PRIMARY KEY,
            display_name TEXT NOT NULL,
            canonical_path TEXT NOT NULL UNIQUE,
            lifecycle_state TEXT NOT NULL,
            current_branch TEXT,
            head_commit TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
    ",
    },
    Migration {
        version: 3,
        sql: "
        CREATE TABLE inspection_attempts (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL REFERENCES projects(id),
            status TEXT NOT NULL,
            previous_lifecycle_state TEXT NOT NULL,
            bundle_schema_version INTEGER NOT NULL,
            bundle_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
    ",
    },
    Migration {
        version: 4,
        sql: "
        CREATE TABLE provider_invocations (
            id INTEGER PRIMARY KEY,
            inspection_attempt_id INTEGER NOT NULL REFERENCES inspection_attempts(id),
            provider_id TEXT NOT NULL,
            model_id TEXT NOT NULL,
            prompt_schema_version INTEGER NOT NULL,
            status TEXT NOT NULL,
            failure_category TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE proposals (
            id INTEGER PRIMARY KEY,
            inspection_attempt_id INTEGER NOT NULL REFERENCES inspection_attempts(id),
            provider_invocation_id INTEGER NOT NULL REFERENCES provider_invocations(id),
            statement TEXT NOT NULL,
            lifecycle_state TEXT NOT NULL,
            confidence TEXT NOT NULL,
            evidence_paths_json TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE questions (
            id INTEGER PRIMARY KEY,
            inspection_attempt_id INTEGER NOT NULL REFERENCES inspection_attempts(id),
            provider_invocation_id INTEGER NOT NULL REFERENCES provider_invocations(id),
            question TEXT NOT NULL,
            status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
    ",
    },
    Migration {
        version: 5,
        sql: "
        CREATE TABLE code_map_snapshots (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL REFERENCES projects(id),
            schema_version INTEGER NOT NULL,
            serialized_json TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        ALTER TABLE inspection_attempts ADD COLUMN code_map_snapshot_id INTEGER REFERENCES code_map_snapshots(id);
    ",
    },
    Migration {
        version: 6,
        sql: "
        CREATE TABLE verified_facts (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL REFERENCES projects(id),
            fact_kind TEXT NOT NULL,
            statement TEXT NOT NULL,
            lifecycle_state TEXT NOT NULL,
            verification_status TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE fact_evidence (
            id INTEGER PRIMARY KEY,
            fact_id INTEGER NOT NULL REFERENCES verified_facts(id),
            project_id INTEGER NOT NULL REFERENCES projects(id),
            relative_path TEXT NOT NULL,
            repository_commit TEXT,
            working_tree_state TEXT NOT NULL,
            line_start INTEGER,
            line_end INTEGER,
            symbol_id TEXT,
            excerpt TEXT NOT NULL,
            evidence_type TEXT NOT NULL,
            confidence TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE review_decisions (
            id INTEGER PRIMARY KEY,
            proposal_id INTEGER NOT NULL UNIQUE REFERENCES proposals(id),
            action TEXT NOT NULL,
            corrected_statement TEXT,
            reason TEXT,
            finalized_at TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE conflicts (
            id INTEGER PRIMARY KEY,
            proposal_id INTEGER NOT NULL REFERENCES proposals(id),
            existing_fact_id INTEGER NOT NULL REFERENCES verified_facts(id),
            rationale TEXT NOT NULL,
            status TEXT NOT NULL,
            resolution_decision_id INTEGER REFERENCES review_decisions(id),
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
        CREATE TABLE inspection_baselines (
            id INTEGER PRIMARY KEY,
            project_id INTEGER NOT NULL REFERENCES projects(id),
            inspection_attempt_id INTEGER NOT NULL UNIQUE REFERENCES inspection_attempts(id),
            code_map_snapshot_id INTEGER NOT NULL REFERENCES code_map_snapshots(id),
            repository_commit TEXT,
            created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
        );
    ",
    },
    Migration {
        version: 7,
        sql: "
        ALTER TABLE inspection_attempts ADD COLUMN repository_commit TEXT;
    ",
    },
];

struct Migration {
    version: i64,
    sql: &'static str,
}

/// Locations owned by PMEMC inside one local application-data root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataPaths {
    root: PathBuf,
}

impl DataPaths {
    /// Construct paths rooted at an explicitly supplied local directory.
    #[must_use]
    pub fn from_root(root: PathBuf) -> Self {
        Self { root }
    }

    /// Return the root directory for PMEMC-owned local data.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Return the SQLite database file path.
    #[must_use]
    pub fn database_path(&self) -> PathBuf {
        self.root.join(DATABASE_FILE)
    }

    /// Return the local cache directory path.
    #[must_use]
    pub fn cache_directory(&self) -> PathBuf {
        self.root.join(CACHE_DIRECTORY)
    }

    /// Return the human-readable export directory path.
    #[must_use]
    pub fn exports_directory(&self) -> PathBuf {
        self.root.join(EXPORTS_DIRECTORY)
    }
}

/// Successful local-store initialization details.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initialization {
    database_path: PathBuf,
}

/// A registered repository record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Project {
    /// Stable project identifier.
    pub id: i64,
    /// User-visible repository name.
    pub display_name: String,
    /// Canonical repository path.
    pub canonical_path: PathBuf,
    /// Durable lifecycle state.
    pub lifecycle_state: String,
    /// Current Git branch when available.
    pub current_branch: Option<String>,
    /// Current Git HEAD when available.
    pub head_commit: Option<String>,
}

/// A failed inspection retained with its approved evidence for a provider retry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedProviderAttempt {
    /// Persistent inspection-attempt identifier.
    pub id: i64,
    /// The exact operator-approved bundle that failed to reach a provider.
    pub bundle: EvidenceBundle,
}

/// A provider proposal together with the immutable evidence and invocation that
/// an operator must see before making a review decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingReviewProposal {
    /// Persistent proposal identifier.
    pub id: i64,
    /// Persistent inspection attempt identifier.
    pub inspection_attempt_id: i64,
    /// Untrusted provider statement awaiting an operator decision.
    pub statement: String,
    /// Provider-suggested lifecycle state.
    pub lifecycle_state: String,
    /// Provider-suggested confidence.
    pub confidence: String,
    /// Evidence files selected by the provider from the approved bundle.
    pub evidence: Vec<EvidenceFile>,
    /// Commit captured at inspection time, when the repository had a HEAD.
    pub repository_commit: Option<String>,
    /// Structural locations from the immutable code-map snapshot.
    pub evidence_locators: Vec<EvidenceLocator>,
    /// Non-secret provider adapter identifier.
    pub provider_id: String,
    /// Non-secret provider model identifier.
    pub model_id: String,
}

/// A structural location available for a selected evidence path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceLocator {
    /// Evidence-relative path.
    pub path: String,
    /// One-based source line from the code-map snapshot.
    pub line: usize,
    /// Stable code-map symbol identifier.
    pub symbol_id: String,
}

/// An operator's durable decision for one pending proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReviewDecision {
    /// Accept the provider's original statement.
    Approve,
    /// Accept an operator-corrected statement while preserving the original.
    CorrectAndApprove { statement: String },
    /// Reject the proposal, optionally recording why.
    Reject { reason: Option<String> },
}

impl Initialization {
    /// Return the initialized SQLite database file path.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.database_path
    }
}

/// Errors raised while resolving or initializing local PMEMC storage.
#[derive(Debug, Error)]
pub enum StorageError {
    /// Neither Windows application-data environment variable was available.
    #[error(
        "cannot determine the PMEMC data directory; set LOCALAPPDATA before running `pmemc init`"
    )]
    MissingLocalAppData,
    /// PMEMC could not create one of its owned directories.
    #[error("cannot create PMEMC directory at {path}")]
    CreateDirectory {
        /// Directory that could not be created.
        path: PathBuf,
        /// Underlying filesystem error.
        #[source]
        source: std::io::Error,
    },
    /// PMEMC could not open the local SQLite database.
    #[error("cannot open PMEMC database at {path}")]
    OpenDatabase {
        /// Database path that could not be opened.
        path: PathBuf,
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// A schema migration did not complete.
    #[error("cannot migrate the PMEMC database")]
    Migration {
        /// Underlying SQLite error.
        #[source]
        source: rusqlite::Error,
    },
    /// A project with the same canonical path is already registered.
    #[error("the repository at {path} is already registered")]
    DuplicateProject { path: PathBuf },
    /// A database operation for projects failed.
    #[error("cannot update the PMEMC project store")]
    ProjectDatabase {
        #[source]
        source: rusqlite::Error,
    },
    /// A validated provider response could not be encoded for local storage.
    #[error("cannot serialize the validated provider response")]
    SerializeProviderResponse {
        #[source]
        source: serde_json::Error,
    },
    /// A provider response was not valid for the approved evidence bundle.
    #[error("provider response does not match the approved inspection evidence")]
    InvalidProviderResponse,
    /// An inspection attempt contains invalid locally persisted evidence.
    #[error(
        "stored inspection evidence is invalid; run a new inspection after resolving the local store"
    )]
    InvalidStoredEvidence,
    /// A requested review action cannot be applied to a pending proposal.
    #[error("review decision is invalid or the proposal is no longer pending")]
    InvalidReviewDecision,
    /// Inspection provenance did not contain a Git object identifier.
    #[error("inspection commit provenance is invalid")]
    InvalidCommitProvenance,
}

/// Resolve the default Windows local-data location used by `pmemc init`.
///
/// `LOCALAPPDATA` is preferred because PMEMC stores local, non-roaming data.
/// `APPDATA` remains a fallback for Windows environments that do not provide it.
///
/// # Errors
///
/// Returns [`StorageError::MissingLocalAppData`] when neither location is set.
pub fn default_data_paths() -> Result<DataPaths, StorageError> {
    data_paths_from_environment(|name| env::var_os(name))
}

fn data_paths_from_environment(
    value: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<DataPaths, StorageError> {
    let base_directory = value("LOCALAPPDATA")
        .filter(|value| !value.is_empty())
        .or_else(|| value("APPDATA").filter(|value| !value.is_empty()))
        .ok_or(StorageError::MissingLocalAppData)?;

    Ok(DataPaths::from_root(
        PathBuf::from(base_directory).join(APPLICATION_DIRECTORY),
    ))
}

/// Create or migrate the local PMEMC store.
///
/// The operation is idempotent. Directory creation can be safely repeated and
/// all migrations are applied in one SQLite transaction.
///
/// # Errors
///
/// Returns an error when the data directories cannot be created, the database
/// cannot be opened, or a migration cannot be committed.
pub fn initialize(data_paths: &DataPaths) -> Result<Initialization, StorageError> {
    create_directory(data_paths.root())?;
    create_directory(&data_paths.cache_directory())?;
    create_directory(&data_paths.exports_directory())?;

    let database_path = data_paths.database_path();
    let mut connection =
        open_connection(&database_path).map_err(|source| StorageError::OpenDatabase {
            path: database_path.clone(),
            source,
        })?;
    apply_migrations(&mut connection, MIGRATIONS)?;

    Ok(Initialization { database_path })
}

/// Store a new registered project without inspecting repository source content.
///
/// # Errors
///
/// Returns an error if the project already exists or SQLite cannot persist it.
pub fn add_project(
    data_paths: &DataPaths,
    display_name: &str,
    canonical_path: &Path,
    current_branch: Option<&str>,
    head_commit: Option<&str>,
) -> Result<Project, StorageError> {
    initialize(data_paths)?;
    let database_path = data_paths.database_path();
    let connection = open_connection(&database_path)
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let result = connection.execute(
        "INSERT INTO projects (display_name, canonical_path, lifecycle_state, current_branch, head_commit) VALUES (?1, ?2, 'registered_needs_inspection', ?3, ?4)",
        params![display_name, canonical_path.to_string_lossy(), current_branch, head_commit],
    );
    match result {
        Ok(_) => Ok(Project {
            id: connection.last_insert_rowid(),
            display_name: display_name.into(),
            canonical_path: canonical_path.into(),
            lifecycle_state: "registered_needs_inspection".into(),
            current_branch: current_branch.map(str::to_owned),
            head_commit: head_commit.map(str::to_owned),
        }),
        Err(rusqlite::Error::SqliteFailure(error, _))
            if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE =>
        {
            Err(StorageError::DuplicateProject {
                path: canonical_path.into(),
            })
        }
        Err(source) => Err(StorageError::ProjectDatabase { source }),
    }
}

/// List every registered project.
///
/// # Errors
///
/// Returns an error if SQLite cannot read the project records.
pub fn list_projects(data_paths: &DataPaths) -> Result<Vec<Project>, StorageError> {
    initialize(data_paths)?;
    let connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let mut statement = connection.prepare("SELECT id, display_name, canonical_path, lifecycle_state, current_branch, head_commit FROM projects ORDER BY id").map_err(|source| StorageError::ProjectDatabase { source })?;
    statement
        .query_map([], |row| {
            Ok(Project {
                id: row.get(0)?,
                display_name: row.get(1)?,
                canonical_path: PathBuf::from(row.get::<_, String>(2)?),
                lifecycle_state: row.get(3)?,
                current_branch: row.get(4)?,
                head_commit: row.get(5)?,
            })
        })
        .map_err(|source| StorageError::ProjectDatabase { source })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| StorageError::ProjectDatabase { source })
}

/// Look up a project by its numeric ID.
pub fn project_by_id(data_paths: &DataPaths, id: i64) -> Result<Option<Project>, StorageError> {
    initialize(data_paths)?;
    let connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    connection.query_row("SELECT id, display_name, canonical_path, lifecycle_state, current_branch, head_commit FROM projects WHERE id = ?1", params![id], |row| Ok(Project { id: row.get(0)?, display_name: row.get(1)?, canonical_path: PathBuf::from(row.get::<_, String>(2)?), lifecycle_state: row.get(3)?, current_branch: row.get(4)?, head_commit: row.get(5)? })).optional().map_err(|source| StorageError::ProjectDatabase { source })
}

/// Persist an approval-gated evidence bundle for a later provider invocation.
///
/// Staging never changes the project lifecycle. It remains authoritative until
/// a valid provider result is atomically persisted for review.
///
/// # Errors
///
/// Returns an error when the project is missing or SQLite cannot commit the
/// retained provider attempt.
pub fn stage_inspection_attempt(
    data_paths: &DataPaths,
    project_id: i64,
    bundle_schema_version: u8,
    bundle_json: &str,
) -> Result<i64, StorageError> {
    stage_inspection_attempt_with_code_map(
        data_paths,
        project_id,
        bundle_schema_version,
        bundle_json,
        None,
    )
}

/// Persist an approval-gated evidence bundle and its structural-map snapshot.
///
/// # Errors
///
/// Returns an error when the project is missing or SQLite cannot atomically
/// retain the attempt and supplied map snapshot.
pub fn stage_inspection_attempt_with_code_map(
    data_paths: &DataPaths,
    project_id: i64,
    bundle_schema_version: u8,
    bundle_json: &str,
    code_map_json: Option<&str>,
) -> Result<i64, StorageError> {
    stage_inspection_attempt_with_provenance(
        data_paths,
        project_id,
        bundle_schema_version,
        bundle_json,
        code_map_json,
        None,
    )
}

/// Persist an approved bundle with its immutable map and inspected commit.
pub fn stage_inspection_attempt_with_provenance(
    data_paths: &DataPaths,
    project_id: i64,
    bundle_schema_version: u8,
    bundle_json: &str,
    code_map_json: Option<&str>,
    repository_commit: Option<&str>,
) -> Result<i64, StorageError> {
    if repository_commit.is_some_and(|commit| !valid_repository_commit(commit)) {
        return Err(StorageError::InvalidCommitProvenance);
    }
    initialize(data_paths)?;
    let mut connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let previous_lifecycle_state = transaction
        .query_row(
            "SELECT lifecycle_state FROM projects WHERE id = ?1",
            params![project_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|source| StorageError::ProjectDatabase { source })?
        .ok_or_else(|| StorageError::ProjectDatabase {
            source: rusqlite::Error::QueryReturnedNoRows,
        })?;
    let code_map_snapshot_id = if let Some(code_map_json) = code_map_json {
        transaction
            .execute(
                "INSERT INTO code_map_snapshots (project_id, schema_version, serialized_json) VALUES (?1, ?2, ?3)",
                params![
                    project_id,
                    CODE_MAP_SNAPSHOT_SCHEMA_VERSION,
                    code_map_json
                ],
            )
            .map_err(|source| StorageError::ProjectDatabase { source })?;
        Some(transaction.last_insert_rowid())
    } else {
        None
    };
    transaction
        .execute(
            "INSERT INTO inspection_attempts (project_id, status, previous_lifecycle_state, bundle_schema_version, bundle_json, code_map_snapshot_id, repository_commit) VALUES (?1, 'staged_pending_provider', ?2, ?3, ?4, ?5, ?6)",
            params![project_id, previous_lifecycle_state, bundle_schema_version, bundle_json, code_map_snapshot_id, repository_commit],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let attempt_id = transaction.last_insert_rowid();
    transaction
        .commit()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    Ok(attempt_id)
}

/// Return the latest failed provider attempt for a project, if it can be retried.
///
/// # Errors
///
/// Returns an error when the local store cannot be read or retained evidence is
/// not valid PMEMC evidence-bundle JSON.
pub fn failed_provider_attempt_for_project(
    data_paths: &DataPaths,
    project_id: i64,
) -> Result<Option<FailedProviderAttempt>, StorageError> {
    initialize(data_paths)?;
    let connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let row = connection
        .query_row(
            "SELECT id, bundle_json FROM inspection_attempts WHERE project_id = ?1 AND status = 'provider_failed' ORDER BY id DESC LIMIT 1",
            params![project_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    row.map(|(id, bundle_json)| {
        serde_json::from_str(&bundle_json)
            .map(|bundle| FailedProviderAttempt { id, bundle })
            .map_err(|_| StorageError::InvalidStoredEvidence)
    })
    .transpose()
}

/// Return pending proposals with exactly the provider-selected approved evidence.
///
/// # Errors
///
/// Returns an error when stored evidence JSON is invalid or the database cannot
/// be read.
pub fn pending_review_proposals(
    data_paths: &DataPaths,
    project_id: i64,
) -> Result<Vec<PendingReviewProposal>, StorageError> {
    initialize(data_paths)?;
    let connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let mut statement = connection
        .prepare(
            "SELECT p.id, p.inspection_attempt_id, p.statement, p.lifecycle_state, p.confidence, p.evidence_paths_json, i.bundle_json, i.repository_commit, c.serialized_json, v.provider_id, v.model_id FROM proposals p JOIN inspection_attempts i ON i.id = p.inspection_attempt_id LEFT JOIN code_map_snapshots c ON c.id = i.code_map_snapshot_id JOIN provider_invocations v ON v.id = p.provider_invocation_id AND v.inspection_attempt_id = p.inspection_attempt_id WHERE i.project_id = ?1 AND i.status = 'pending_review' AND p.status = 'pending_review' AND v.status = 'succeeded' ORDER BY p.id",
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let rows = statement
        .query_map(params![project_id], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, String>(10)?,
            ))
        })
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    rows.map(|row| {
        let (
            id,
            inspection_attempt_id,
            statement,
            lifecycle_state,
            confidence,
            evidence_paths_json,
            bundle_json,
            repository_commit,
            code_map_json,
            provider_id,
            model_id,
        ) = row.map_err(|source| StorageError::ProjectDatabase { source })?;
        let evidence_paths: Vec<String> = serde_json::from_str(&evidence_paths_json)
            .map_err(|_| StorageError::InvalidStoredEvidence)?;
        let unique_evidence_paths = evidence_paths.iter().collect::<BTreeSet<_>>();
        if evidence_paths.is_empty() || unique_evidence_paths.len() != evidence_paths.len() {
            return Err(StorageError::InvalidStoredEvidence);
        }
        let bundle: EvidenceBundle =
            serde_json::from_str(&bundle_json).map_err(|_| StorageError::InvalidStoredEvidence)?;
        let evidence = bundle
            .files
            .into_iter()
            .filter(|file| unique_evidence_paths.contains(&file.path))
            .collect::<Vec<_>>();
        if evidence.len() != unique_evidence_paths.len() {
            return Err(StorageError::InvalidStoredEvidence);
        }
        let evidence_locators = code_map_json
            .as_deref()
            .map(|json| evidence_locators(json, &unique_evidence_paths))
            .transpose()?;
        if repository_commit
            .as_deref()
            .is_some_and(|commit| !valid_repository_commit(commit))
        {
            return Err(StorageError::InvalidStoredEvidence);
        }
        Ok(PendingReviewProposal {
            id,
            inspection_attempt_id,
            statement,
            lifecycle_state,
            confidence,
            evidence,
            repository_commit,
            evidence_locators: evidence_locators.unwrap_or_default(),
            provider_id,
            model_id,
        })
    })
    .collect()
}

/// Persist an operator decision and remove the proposal from the pending queue.
///
/// The original provider proposal is never changed. A correction is stored next
/// to it in the durable decision record for later finalization and audit.
///
/// # Errors
///
/// Returns an error if the proposal is not pending or SQLite cannot atomically
/// retain the decision and proposal state.
pub fn record_review_decision(
    data_paths: &DataPaths,
    proposal_id: i64,
    decision: &ReviewDecision,
) -> Result<(), StorageError> {
    let (action, corrected_statement, reason, proposal_status) = match decision {
        ReviewDecision::Approve => ("approved", None, None, "approved"),
        ReviewDecision::CorrectAndApprove { statement } if !statement.trim().is_empty() => (
            "corrected_and_approved",
            Some(statement.as_str()),
            None,
            "approved",
        ),
        ReviewDecision::CorrectAndApprove { .. } => {
            return Err(StorageError::InvalidReviewDecision);
        }
        ReviewDecision::Reject { reason } => (
            "rejected",
            None,
            reason.as_deref().filter(|reason| !reason.trim().is_empty()),
            "rejected",
        ),
    };
    initialize(data_paths)?;
    let mut connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let updated = transaction
        .execute(
            "UPDATE proposals SET status = ?1 WHERE id = ?2 AND status = 'pending_review'",
            params![proposal_status, proposal_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    if updated != 1 {
        return Err(StorageError::InvalidReviewDecision);
    }
    transaction
        .execute(
            "INSERT INTO review_decisions (proposal_id, action, corrected_statement, reason) VALUES (?1, ?2, ?3, ?4)",
            params![proposal_id, action, corrected_statement, reason],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .commit()
        .map_err(|source| StorageError::ProjectDatabase { source })
}

/// Persist a schema-validated provider response as pending review records.
///
/// This single transaction records non-secret invocation metadata, proposals,
/// and questions without changing verified facts or a baseline.
///
/// # Errors
///
/// Returns an error if the staged attempt is unavailable or SQLite cannot
/// commit every pending-review record.
pub fn store_provider_response(
    data_paths: &DataPaths,
    attempt_id: i64,
    metadata: &ProviderInvocationMetadata,
    response: &ProviderResponse,
) -> Result<(), StorageError> {
    initialize(data_paths)?;
    let mut connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let (project_id, bundle_json) = transaction
        .query_row(
            "SELECT project_id, bundle_json FROM inspection_attempts WHERE id = ?1 AND status = 'staged_pending_provider'",
            params![attempt_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|source| StorageError::ProjectDatabase { source })?
        .ok_or_else(|| StorageError::ProjectDatabase {
            source: rusqlite::Error::QueryReturnedNoRows,
        })?;
    let bundle: EvidenceBundle = serde_json::from_str(&bundle_json)
        .map_err(|source| StorageError::SerializeProviderResponse { source })?;
    validate_response(response, &bundle).map_err(|_| StorageError::InvalidProviderResponse)?;
    transaction
        .execute(
            "INSERT INTO provider_invocations (inspection_attempt_id, provider_id, model_id, prompt_schema_version, status) VALUES (?1, ?2, ?3, ?4, 'succeeded')",
            params![attempt_id, metadata.provider_id, metadata.model_id, metadata.prompt_schema_version],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let invocation_id = transaction.last_insert_rowid();
    for proposal in &response.proposals {
        let evidence_paths_json = serde_json::to_string(&proposal.evidence_paths)
            .map_err(|source| StorageError::SerializeProviderResponse { source })?;
        transaction
            .execute(
                "INSERT INTO proposals (inspection_attempt_id, provider_invocation_id, statement, lifecycle_state, confidence, evidence_paths_json, status) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'pending_review')",
                params![
                    attempt_id,
                    invocation_id,
                    proposal.statement,
                    proposal.lifecycle.as_str(),
                    proposal.confidence.as_str(),
                    evidence_paths_json,
                ],
            )
            .map_err(|source| StorageError::ProjectDatabase { source })?;
    }
    for question in &response.questions {
        transaction
            .execute(
                "INSERT INTO questions (inspection_attempt_id, provider_invocation_id, question, status) VALUES (?1, ?2, ?3, 'pending_review')",
                params![attempt_id, invocation_id, question],
            )
            .map_err(|source| StorageError::ProjectDatabase { source })?;
    }
    transaction
        .execute(
            "UPDATE inspection_attempts SET status = 'pending_review' WHERE id = ?1",
            params![attempt_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .execute(
            "UPDATE projects SET lifecycle_state = 'inspection_pending_review' WHERE id = ?1",
            params![project_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .commit()
        .map_err(|source| StorageError::ProjectDatabase { source })
}

/// Mark a provider attempt as failed and restore its prior durable lifecycle.
///
/// # Errors
///
/// Returns an error if the staged attempt cannot be recovered atomically.
pub fn record_provider_failure(
    data_paths: &DataPaths,
    attempt_id: i64,
    metadata: &ProviderInvocationMetadata,
    failure_category: ProviderFailureCategory,
) -> Result<(), StorageError> {
    initialize(data_paths)?;
    let mut connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let (project_id, previous_lifecycle_state) = transaction
        .query_row(
            "SELECT project_id, previous_lifecycle_state FROM inspection_attempts WHERE id = ?1 AND status = 'staged_pending_provider'",
            params![attempt_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|source| StorageError::ProjectDatabase { source })?
        .ok_or_else(|| StorageError::ProjectDatabase {
            source: rusqlite::Error::QueryReturnedNoRows,
        })?;
    transaction
        .execute(
            "INSERT INTO provider_invocations (inspection_attempt_id, provider_id, model_id, prompt_schema_version, status, failure_category) VALUES (?1, ?2, ?3, ?4, 'failed', ?5)",
            params![
                attempt_id,
                metadata.provider_id,
                metadata.model_id,
                metadata.prompt_schema_version,
                failure_category.as_str(),
            ],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .execute(
            "UPDATE inspection_attempts SET status = 'provider_failed' WHERE id = ?1",
            params![attempt_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .execute(
            "UPDATE projects SET lifecycle_state = ?1 WHERE id = ?2",
            params![previous_lifecycle_state, project_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .commit()
        .map_err(|source| StorageError::ProjectDatabase { source })
}

/// Make a retained failed provider attempt ready for another bounded call.
///
/// Prior invocation rows remain in the audit trail. The project lifecycle stays
/// at its previous durable state until a retry produces valid review records.
///
/// # Errors
///
/// Returns an error when the failed attempt cannot be restored transactionally.
pub fn retry_provider_attempt(data_paths: &DataPaths, attempt_id: i64) -> Result<(), StorageError> {
    initialize(data_paths)?;
    let mut connection = open_connection(&data_paths.database_path())
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    let exists = transaction
        .query_row(
            "SELECT 1 FROM inspection_attempts WHERE id = ?1 AND status = 'provider_failed'",
            params![attempt_id],
            |_| Ok(()),
        )
        .optional()
        .map_err(|source| StorageError::ProjectDatabase { source })?
        .is_some();
    if !exists {
        return Err(StorageError::ProjectDatabase {
            source: rusqlite::Error::QueryReturnedNoRows,
        });
    }
    transaction
        .execute(
            "UPDATE inspection_attempts SET status = 'staged_pending_provider' WHERE id = ?1",
            params![attempt_id],
        )
        .map_err(|source| StorageError::ProjectDatabase { source })?;
    transaction
        .commit()
        .map_err(|source| StorageError::ProjectDatabase { source })
}

fn create_directory(path: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(path).map_err(|source| StorageError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })
}

fn evidence_locators(
    code_map_json: &str,
    evidence_paths: &BTreeSet<&String>,
) -> Result<Vec<EvidenceLocator>, StorageError> {
    let code_map: Value =
        serde_json::from_str(code_map_json).map_err(|_| StorageError::InvalidStoredEvidence)?;
    let symbols = code_map
        .get("symbols")
        .and_then(Value::as_array)
        .ok_or(StorageError::InvalidStoredEvidence)?;
    let mut locators = symbols
        .iter()
        .map(|symbol| {
            let path = symbol
                .get("path")
                .and_then(Value::as_str)
                .ok_or(StorageError::InvalidStoredEvidence)?;
            let selected = evidence_paths
                .iter()
                .any(|selected| selected.as_str() == path);
            if !selected {
                return Ok(None);
            }
            let line = symbol
                .get("line")
                .and_then(Value::as_u64)
                .and_then(|line| usize::try_from(line).ok())
                .filter(|line| *line > 0)
                .ok_or(StorageError::InvalidStoredEvidence)?;
            let name = symbol
                .get("name")
                .and_then(Value::as_str)
                .filter(|name| !name.is_empty())
                .ok_or(StorageError::InvalidStoredEvidence)?;
            Ok(Some(EvidenceLocator {
                path: path.into(),
                line,
                symbol_id: format!("{path}:{line}:{name}"),
            }))
        })
        .collect::<Result<Vec<_>, StorageError>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    locators.sort_by(|left, right| {
        (&left.path, left.line, &left.symbol_id).cmp(&(&right.path, right.line, &right.symbol_id))
    });
    Ok(locators)
}

fn valid_repository_commit(commit: &str) -> bool {
    matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn open_connection(path: &Path) -> Result<Connection, rusqlite::Error> {
    let connection = Connection::open(path)?;
    enable_foreign_keys(&connection)?;
    Ok(connection)
}

fn enable_foreign_keys(connection: &Connection) -> Result<(), rusqlite::Error> {
    connection.execute_batch("PRAGMA foreign_keys = ON;")
}

fn apply_migrations(
    connection: &mut Connection,
    migrations: &[Migration],
) -> Result<(), StorageError> {
    let transaction = connection
        .transaction()
        .map_err(|source| StorageError::Migration { source })?;
    transaction
        .execute_batch(
            "
                CREATE TABLE IF NOT EXISTS schema_migrations (
                    version INTEGER PRIMARY KEY,
                    applied_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
                );
            ",
        )
        .map_err(|source| StorageError::Migration { source })?;

    let applied_versions = {
        let mut statement = transaction
            .prepare("SELECT version FROM schema_migrations")
            .map_err(|source| StorageError::Migration { source })?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(|source| StorageError::Migration { source })?
            .collect::<Result<BTreeSet<i64>, _>>()
            .map_err(|source| StorageError::Migration { source })?
    };

    for migration in migrations {
        if !applied_versions.contains(&migration.version) {
            transaction
                .execute_batch(migration.sql)
                .map_err(|source| StorageError::Migration { source })?;
            transaction
                .execute(
                    "INSERT INTO schema_migrations (version) VALUES (?1)",
                    params![migration.version],
                )
                .map_err(|source| StorageError::Migration { source })?;
        }
    }

    transaction
        .commit()
        .map_err(|source| StorageError::Migration { source })
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, ffi::OsString};

    use super::*;

    #[test]
    fn local_app_data_takes_precedence_over_app_data() {
        let values = BTreeMap::from([
            (
                "LOCALAPPDATA",
                OsString::from(r"C:\\Users\\Ada\\AppData\\Local"),
            ),
            (
                "APPDATA",
                OsString::from(r"C:\\Users\\Ada\\AppData\\Roaming"),
            ),
        ]);

        let paths = data_paths_from_environment(|name| values.get(name).cloned())
            .expect("LOCALAPPDATA should resolve a data directory");

        assert_eq!(
            paths.root(),
            Path::new(r"C:\\Users\\Ada\\AppData\\Local").join("PMEMC")
        );
    }

    #[test]
    fn app_data_is_used_when_local_app_data_is_empty() {
        let values = BTreeMap::from([
            ("LOCALAPPDATA", OsString::new()),
            (
                "APPDATA",
                OsString::from(r"C:\\Users\\Ada\\AppData\\Roaming"),
            ),
        ]);

        let paths = data_paths_from_environment(|name| values.get(name).cloned())
            .expect("APPDATA should be the fallback for an empty LOCALAPPDATA");

        assert_eq!(
            paths.root(),
            Path::new(r"C:\\Users\\Ada\\AppData\\Roaming").join("PMEMC")
        );
    }

    #[test]
    fn missing_windows_data_directories_return_an_actionable_error() {
        let result = data_paths_from_environment(|_| None);

        assert!(matches!(result, Err(StorageError::MissingLocalAppData)));
    }

    #[test]
    fn a_failed_migration_rolls_back_the_schema_ledger_and_prior_migrations() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite should open");
        let migrations = [
            Migration {
                version: 1,
                sql: "CREATE TABLE should_rollback (id INTEGER PRIMARY KEY);",
            },
            Migration {
                version: 2,
                sql: "CREATE TABLE broken (id INTEGER PRIMARY KEY;",
            },
        ];

        let result = apply_migrations(&mut connection, &migrations);

        assert!(matches!(result, Err(StorageError::Migration { .. })));
        let table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('schema_migrations', 'should_rollback')",
                [],
                |row| row.get(0),
            )
            .expect("SQLite metadata query should succeed");
        assert_eq!(table_count, 0);
    }

    #[test]
    fn code_map_snapshot_migration_preserves_existing_provider_records() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite should open");
        apply_migrations(&mut connection, &MIGRATIONS[..6])
            .expect("version six database should initialize");
        connection
            .execute(
                "INSERT INTO projects (id, display_name, canonical_path, lifecycle_state) VALUES (1, 'fixture', 'C:/fixture', 'registered_needs_inspection')",
                [],
            )
            .expect("project fixture should insert");
        connection
            .execute(
                "INSERT INTO code_map_snapshots (id, project_id, schema_version, serialized_json) VALUES (1, 1, 1, '{}')",
                [],
            )
            .expect("snapshot fixture should insert");
        connection
            .execute(
                "INSERT INTO inspection_attempts (id, project_id, status, previous_lifecycle_state, bundle_schema_version, bundle_json, code_map_snapshot_id) VALUES (1, 1, 'provider_failed', 'registered_needs_inspection', 1, '{}', 1)",
                [],
            )
            .expect("attempt fixture should insert");
        connection
            .execute(
                "INSERT INTO provider_invocations (inspection_attempt_id, provider_id, model_id, prompt_schema_version, status, failure_category) VALUES (1, 'fake', 'fixture-model', 1, 'failed', 'configuration')",
                [],
            )
            .expect("provider fixture should insert");
        connection
            .execute(
                "INSERT INTO proposals (inspection_attempt_id, provider_invocation_id, statement, lifecycle_state, confidence, evidence_paths_json, status) VALUES (1, 1, 'fixture statement', 'committed', 'exact', '[\"src/lib.rs\"]', 'pending_review')",
                [],
            )
            .expect("proposal fixture should insert");
        connection
            .execute(
                "INSERT INTO questions (inspection_attempt_id, provider_invocation_id, question, status) VALUES (1, 1, 'fixture question', 'pending_review')",
                [],
            )
            .expect("question fixture should insert");

        apply_migrations(&mut connection, MIGRATIONS)
            .expect("inspection-provenance migration should upgrade version six data");

        let preserved_rows: (i64, i64, i64, i64, i64) = connection
            .query_row(
                "SELECT (SELECT COUNT(*) FROM inspection_attempts), (SELECT COUNT(*) FROM provider_invocations), (SELECT COUNT(*) FROM proposals), (SELECT COUNT(*) FROM questions), (SELECT COUNT(*) FROM code_map_snapshots)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
            )
            .expect("existing records should remain readable");
        let original_snapshot: Option<i64> = connection
            .query_row(
                "SELECT code_map_snapshot_id FROM inspection_attempts WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("preexisting attempt should gain a nullable snapshot reference");
        let repository_commit: Option<String> = connection
            .query_row(
                "SELECT repository_commit FROM inspection_attempts WHERE id = 1",
                [],
                |row| row.get(0),
            )
            .expect("preexisting attempt should gain a nullable commit reference");
        let review_table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('verified_facts', 'fact_evidence', 'review_decisions', 'conflicts', 'inspection_baselines')",
                [],
                |row| row.get(0),
            )
            .expect("review tables should exist after migration");

        assert_eq!(preserved_rows, (1, 1, 1, 1, 1));
        assert_eq!(original_snapshot, Some(1));
        assert_eq!(repository_commit, None);
        assert_eq!(review_table_count, 5);
    }

    #[test]
    fn storage_connections_enforce_declared_foreign_keys() {
        let connection = Connection::open_in_memory().expect("in-memory SQLite should open");
        enable_foreign_keys(&connection).expect("foreign keys should be enabled");
        connection
            .execute_batch(
                "
                CREATE TABLE parents (id INTEGER PRIMARY KEY);
                CREATE TABLE children (parent_id INTEGER NOT NULL REFERENCES parents(id));
                ",
            )
            .expect("fixture tables should be created");

        let result = connection.execute("INSERT INTO children (parent_id) VALUES (99)", []);

        assert!(matches!(
            result,
            Err(rusqlite::Error::SqliteFailure(error, _))
                if error.extended_code == rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY
        ));
    }
}
