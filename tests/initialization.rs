mod support;

use std::process::{Command, Stdio};

use pmemc::storage::{DataPaths, initialize};
use rusqlite::Connection;
use support::TemporaryDirectory;

#[test]
fn initialization_creates_an_idempotent_local_store_at_an_injected_path() {
    let temporary_directory = TemporaryDirectory::new();
    let data_paths = DataPaths::from_root(temporary_directory.path().join("PMEMC"));

    let first = initialize(&data_paths).expect("first initialization should succeed");
    let second = initialize(&data_paths).expect("second initialization should succeed");

    assert_eq!(first.database_path(), second.database_path());
    assert!(data_paths.cache_directory().is_dir());
    assert!(data_paths.exports_directory().is_dir());

    let connection = Connection::open(data_paths.database_path())
        .expect("the initialized database should be readable");
    let migration_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("the migration ledger should exist");
    assert_eq!(migration_count, 9);

    let metadata_table_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'pmemc_metadata'",
            [],
            |row| row.get(0),
        )
        .expect("SQLite metadata query should succeed");
    assert_eq!(metadata_table_count, 1);
}

#[test]
fn init_uses_local_app_data_without_touching_the_callers_real_data_directory() {
    let temporary_directory = TemporaryDirectory::new();

    let output = Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .arg("init")
        .env("LOCALAPPDATA", temporary_directory.path())
        .output()
        .expect("pmemc should run");

    assert!(output.status.success());
    assert!(
        temporary_directory
            .path()
            .join("PMEMC")
            .join("pmemc.sqlite3")
            .is_file()
    );
}

#[test]
fn first_run_init_explains_free_model_setup_without_blocking_noninteractive_users() {
    let temporary_directory = TemporaryDirectory::new();

    let output = Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .arg("init")
        .env("LOCALAPPDATA", temporary_directory.path())
        .stdin(Stdio::null())
        .output()
        .expect("pmemc should run");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("openrouter/free"));
    assert!(stdout.contains("pmemc auth set"));
}
