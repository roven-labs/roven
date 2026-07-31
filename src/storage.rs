//! Local data-directory and SQLite initialization.

use std::{
    collections::BTreeSet,
    env, fs,
    path::{Path, PathBuf},
};

use rusqlite::{Connection, params};
use thiserror::Error;

const APPLICATION_DIRECTORY: &str = "PMEMC";
const DATABASE_FILE: &str = "pmemc.sqlite3";
const CACHE_DIRECTORY: &str = "cache";
const EXPORTS_DIRECTORY: &str = "exports";

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    sql: "
        CREATE TABLE pmemc_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );
    ",
}];

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
        Connection::open(&database_path).map_err(|source| StorageError::OpenDatabase {
            path: database_path.clone(),
            source,
        })?;
    apply_migrations(&mut connection, MIGRATIONS)?;

    Ok(Initialization { database_path })
}

fn create_directory(path: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(path).map_err(|source| StorageError::CreateDirectory {
        path: path.to_path_buf(),
        source,
    })
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
}
