mod support;

use std::{
    io::Write,
    process::{Command, Stdio},
};

use pmemc::{git, inspection::build_initial_bundle};
use support::TemporaryDirectory;

fn git_command(repository: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should run");
    assert!(output.status.success(), "git failed: {output:?}");
}

fn pmemc_with_input(
    data_directory: &TemporaryDirectory,
    arguments: &[&str],
    input: &[u8],
) -> std::process::Output {
    let mut process = Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .env("LOCALAPPDATA", data_directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("pmemc should start");
    process
        .stdin
        .take()
        .expect("pmemc stdin should be available")
        .write_all(input)
        .expect("inspection response should be written");
    process.wait_with_output().expect("pmemc should finish")
}

#[test]
fn initial_bundle_is_bounded_deterministic_and_redacts_suspected_secrets() {
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    std::fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub const API_TOKEN: &str = \"actual-secret-value\";\npub fn run() {}\n",
    )
    .expect("source fixture should be written");
    std::fs::write(repository.path().join("README.md"), "# Fixture\n")
        .expect("readme fixture should be written");
    std::fs::write(repository.path().join(".env"), "API_KEY=blocked-secret\n")
        .expect("blocked fixture should be written");

    let status = git::working_tree_status(repository.path()).expect("status should be read");
    let first = build_initial_bundle(repository.path(), "project-1", &status)
        .expect("bundle should be built");
    let second = build_initial_bundle(repository.path(), "project-1", &status)
        .expect("repeat bundle should be built");
    let serialized = serde_json::to_string(&first).expect("bundle should serialize");

    assert_eq!(first, second);
    assert!(serialized.contains("[REDACTED]"));
    assert!(!serialized.contains("actual-secret-value"));
    assert!(!serialized.contains("blocked-secret"));
    assert!(first.files.iter().any(|file| file.path == "src/lib.rs"));
    assert!(first.files.iter().any(|file| file.path == "README.md"));
    assert!(first.files.iter().all(|file| file.path != ".env"));
}

#[test]
fn denied_inspection_does_not_read_source_or_create_a_pending_attempt() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    std::fs::write(
        repository.path().join("unread-source.rs"),
        "pub const TOKEN: &str = \"must-never-be-read\";\n",
    )
    .expect("source fixture should be written");
    let add = Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args([
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ])
        .env("LOCALAPPDATA", data_directory.path())
        .output()
        .expect("project should be registered");
    assert!(add.status.success());

    let inspection = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"n\n");
    assert!(inspection.status.success());
    let output = String::from_utf8_lossy(&inspection.stdout);
    assert!(output.contains("inspection cancelled"));
    assert!(!output.contains("must-never-be-read"));

    let connection =
        rusqlite::Connection::open(data_directory.path().join("PMEMC").join("pmemc.sqlite3"))
            .expect("database should open");
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM inspection_attempts", [], |row| {
            row.get(0)
        })
        .expect("attempt table should exist");
    assert_eq!(attempts, 0);
}

#[test]
fn approved_inspection_stages_a_redacted_bundle_and_marks_the_project_pending_review() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    std::fs::write(
        repository.path().join("main.rs"),
        "pub const API_KEY: &str = \"never-store-this-secret\";\n",
    )
    .expect("source fixture should be written");
    let add = Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args([
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ])
        .env("LOCALAPPDATA", data_directory.path())
        .output()
        .expect("project should be registered");
    assert!(add.status.success());

    let inspection = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"yes\n");
    assert!(inspection.status.success());
    assert!(String::from_utf8_lossy(&inspection.stdout).contains("staged for provider processing"));

    let connection =
        rusqlite::Connection::open(data_directory.path().join("PMEMC").join("pmemc.sqlite3"))
            .expect("database should open");
    let stored_bundle: String = connection
        .query_row("SELECT bundle_json FROM inspection_attempts", [], |row| {
            row.get(0)
        })
        .expect("staged bundle should exist");
    assert!(stored_bundle.contains("[REDACTED]"));
    assert!(!stored_bundle.contains("never-store-this-secret"));
    let lifecycle_state: String = connection
        .query_row(
            "SELECT lifecycle_state FROM projects WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("project should exist");
    assert_eq!(lifecycle_state, "inspection_pending_review");
}
