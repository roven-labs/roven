mod support;

use std::{
    io::Write,
    process::{Command, Stdio},
};

use pmemc::{
    git,
    inspection::{
        EvidenceBundle, EvidenceFile, EvidenceState, build_incremental_bundle, build_initial_bundle,
    },
    provider::{ProviderInvocationMetadata, parse_response},
    storage,
};
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
        .env_remove("OPENROUTER_API_KEY")
        .env_remove("PMEMC_OPENROUTER_MODEL")
        .env_remove("PMEMC_OPENROUTER_TIMEOUT_SECS")
        .env_remove("PMEMC_OPENROUTER_MAX_ATTEMPTS")
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
    std::fs::write(
        repository.path().join("config.toml"),
        "PRIVATE_KEY = \"-----BEGIN PRIVATE KEY-----sensitive-value\"\n",
    )
    .expect("private-key fixture should be written");
    std::fs::write(
        repository.path().join("notes.md"),
        "-----BEGIN PRIVATE KEY-----\nprivate-key-body-must-not-leak\n-----END PRIVATE KEY-----\n",
    )
    .expect("PEM fixture should be written");
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
    assert!(!serialized.contains("sensitive-value"));
    assert!(!serialized.contains("-----BEGIN PRIVATE KEY-----"));
    assert!(!serialized.contains("private-key-body-must-not-leak"));
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
    assert!(output.contains("unread-source.rs"));
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
fn approved_inspection_with_missing_provider_configuration_preserves_a_redacted_retryable_attempt()
{
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
    assert!(!inspection.status.success());
    let stderr = String::from_utf8_lossy(&inspection.stderr);
    assert!(stderr.contains("OPENROUTER_API_KEY"));

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
    assert_eq!(lifecycle_state, "registered_needs_inspection");
    let attempt_status: String = connection
        .query_row("SELECT status FROM inspection_attempts", [], |row| {
            row.get(0)
        })
        .expect("attempt should be retained for retry");
    assert_eq!(attempt_status, "provider_failed");
    let snapshot_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM code_map_snapshots", [], |row| {
            row.get(0)
        })
        .expect("approved inspection should retain one code-map snapshot");
    let linked_snapshot_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM inspection_attempts WHERE code_map_snapshot_id IS NOT NULL",
            [],
            |row| row.get(0),
        )
        .expect("attempt should reference its code-map snapshot");
    assert_eq!((snapshot_count, linked_snapshot_count), (1, 1));
}

#[test]
fn a_later_inspect_retries_the_retained_provider_attempt() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    std::fs::write(repository.path().join("main.rs"), "pub fn run() {}\n")
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

    let first = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"y\n");
    let database_path = data_directory.path().join("PMEMC").join("pmemc.sqlite3");
    let first_connection = rusqlite::Connection::open(&database_path)
        .expect("database should open after the first failed attempt");
    let original_snapshot_id: i64 = first_connection
        .query_row(
            "SELECT code_map_snapshot_id FROM inspection_attempts WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("the first attempt should retain a code-map snapshot");
    std::fs::write(repository.path().join("later.rs"), "pub fn later() {}\n")
        .expect("later source fixture should be written");
    let second = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"y\n");

    assert!(!first.status.success());
    assert!(!second.status.success());
    let retry_output = String::from_utf8_lossy(&second.stdout);
    assert!(retry_output.contains("retained approved evidence"));
    assert!(retry_output.contains("scope\tmain.rs"));
    assert!(!retry_output.contains("scope\tlater.rs"));
    let connection = rusqlite::Connection::open(&database_path).expect("database should open");
    let attempt_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM inspection_attempts", [], |row| {
            row.get(0)
        })
        .expect("attempt count should be readable");
    let invocation_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM provider_invocations", [], |row| {
            row.get(0)
        })
        .expect("invocation count should be readable");
    let retained_snapshot_id: i64 = connection
        .query_row(
            "SELECT code_map_snapshot_id FROM inspection_attempts WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .expect("retry should retain the original code-map snapshot");
    let snapshot_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM code_map_snapshots", [], |row| {
            row.get(0)
        })
        .expect("retry should not create another snapshot");
    assert_eq!((attempt_count, invocation_count, snapshot_count), (1, 2, 1));
    assert_eq!(retained_snapshot_id, original_snapshot_id);
}

#[test]
fn review_interactively_approves_corrects_rejects_and_skips_without_losing_proposals() {
    let data_directory = TemporaryDirectory::new();
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project = storage::add_project(
        &data_paths,
        "fixture",
        data_directory.path().join("repository").as_path(),
        None,
        None,
    )
    .expect("project should be stored");
    let bundle = EvidenceBundle {
        schema_version: 1,
        project_id: format!("project-{}", project.id),
        initial_inspection: true,
        files: vec![EvidenceFile {
            path: "src/lib.rs".into(),
            state: EvidenceState::Committed,
            content: "pub fn run() {}".into(),
            redacted: false,
        }],
    };
    let bundle_json = serde_json::to_string(&bundle).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt_with_provenance(
        &data_paths,
        project.id,
        1,
        &bundle_json,
        Some(r#"{"symbols":[{"path":"src/lib.rs","line":1,"name":"run"}]}"#),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )
    .expect("attempt should stage");
    let response = parse_response(
        r#"{"schema_version":1,"proposals":[
            {"statement":"approve me","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs"]},
            {"statement":"correct me","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs"]},
            {"statement":"reject me","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs"]},
            {"statement":"skip me","lifecycle":"committed","confidence":"exact","evidence_paths":["src/lib.rs"]}
        ],"questions":[]}"#,
        &bundle,
    )
    .expect("fixture response should validate");
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should validate");
    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("pending proposals should be stored");

    let review = pmemc_with_input(
        &data_directory,
        &["review", &format!("project-{}", project.id)],
        b"a\nc\ncorrected statement\nr\nnot supported by the evidence\ns\n",
    );

    assert!(review.status.success());
    let output = String::from_utf8_lossy(&review.stdout);
    assert!(output.contains("provider\tfake\toffline-test-model"));
    assert!(output.contains("commit\taaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"));
    assert!(output.contains("evidence\tsrc/lib.rs\tcommitted"));
    assert!(output.contains("locator\tsrc/lib.rs\t1\tsrc/lib.rs:1:run"));
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    let proposal_statuses = connection
        .prepare("SELECT status FROM proposals ORDER BY id")
        .expect("status query should prepare")
        .query_map([], |row| row.get::<_, String>(0))
        .expect("statuses should query")
        .collect::<Result<Vec<_>, _>>()
        .expect("statuses should decode");
    assert_eq!(
        proposal_statuses,
        ["approved", "approved", "rejected", "pending_review"]
    );
    let correction: (String, String) = connection
        .query_row(
            "SELECT action, corrected_statement FROM review_decisions WHERE proposal_id = 2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("correction should preserve an action and replacement");
    assert_eq!(
        correction,
        (
            "corrected_and_approved".into(),
            "corrected statement".into()
        )
    );
    let rejection_reason: String = connection
        .query_row(
            "SELECT reason FROM review_decisions WHERE proposal_id = 3",
            [],
            |row| row.get(0),
        )
        .expect("rejection reason should persist");
    assert_eq!(rejection_reason, "not supported by the evidence");
    let blank_correction = storage::record_review_decision(
        &data_paths,
        4,
        &storage::ReviewDecision::CorrectAndApprove {
            statement: "   ".into(),
        },
    );
    assert!(matches!(
        blank_correction,
        Err(storage::StorageError::InvalidReviewDecision)
    ));
    let skipped_state: String = connection
        .query_row("SELECT status FROM proposals WHERE id = 4", [], |row| {
            row.get(0)
        })
        .expect("invalid correction must leave the skipped proposal pending");
    let skipped_decision_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM review_decisions WHERE proposal_id = 4",
            [],
            |row| row.get(0),
        )
        .expect("invalid correction must not create a decision");
    assert_eq!(
        (skipped_state, skipped_decision_count),
        ("pending_review".into(), 0)
    );
    let duplicate_decision =
        storage::record_review_decision(&data_paths, 1, &storage::ReviewDecision::Approve);
    assert!(matches!(
        duplicate_decision,
        Err(storage::StorageError::InvalidReviewDecision)
    ));
    let decision_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM review_decisions WHERE proposal_id = 1",
            [],
            |row| row.get(0),
        )
        .expect("duplicate decision must not create a second record");
    assert_eq!(decision_count, 1);
    connection
        .execute(
            "UPDATE code_map_snapshots SET serialized_json = '{\"symbols\":[{\"path\":\"src/lib.rs\",\"line\":\"not-a-line\",\"name\":\"run\"}]}'",
            [],
        )
        .expect("snapshot fixture should be corruptible for a fail-closed check");
    let corrupted_snapshot = storage::pending_review_proposals(&data_paths, project.id);
    assert!(matches!(
        corrupted_snapshot,
        Err(storage::StorageError::InvalidStoredEvidence)
    ));
    connection
        .execute(
            "UPDATE code_map_snapshots SET serialized_json = '{\"symbols\":[{\"path\":\"src/lib.rs\",\"line\":1,\"name\":\"run\"}]}'",
            [],
        )
        .expect("snapshot fixture should be restorable");
    connection
        .execute(
            "UPDATE inspection_attempts SET repository_commit = 'not-a-git-object' WHERE id = 1",
            [],
        )
        .expect("commit fixture should be corruptible for a fail-closed check");
    let corrupted_commit = storage::pending_review_proposals(&data_paths, project.id);
    assert!(matches!(
        corrupted_commit,
        Err(storage::StorageError::InvalidStoredEvidence)
    ));
}

#[test]
fn incremental_bundle_selects_changed_code_neighbours_tests_and_manifests() {
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    git_command(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git_command(repository.path(), &["config", "user.name", "PMEMC Test"]);
    std::fs::create_dir_all(repository.path().join("src")).expect("src directory should exist");
    std::fs::create_dir_all(repository.path().join("tests")).expect("test directory should exist");
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn run() { helper(); }\n",
    )
    .expect("source fixture should be written");
    std::fs::write(
        repository.path().join("src/helper.rs"),
        "pub fn helper() {}\n",
    )
    .expect("neighbour fixture should be written");
    std::fs::write(
        repository.path().join("tests/lib.rs"),
        "#[test] fn run_works() {}\n",
    )
    .expect("test fixture should be written");
    std::fs::write(
        repository.path().join("Cargo.toml"),
        "[package]\nname = \"fixture\"\n",
    )
    .expect("manifest fixture should be written");
    git_command(repository.path(), &["add", "."]);
    git_command(repository.path(), &["commit", "-m", "initial fixture"]);
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub fn run() { helper(); }\n// changed\n",
    )
    .expect("changed fixture should be written");

    let status = git::working_tree_status(repository.path()).expect("status should be read");
    let bundle = build_incremental_bundle(repository.path(), "project-1", &status)
        .expect("incremental bundle should be built");
    let paths = bundle
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();

    assert!(!bundle.initial_inspection);
    assert_eq!(
        paths,
        ["Cargo.toml", "src/helper.rs", "src/lib.rs", "tests/lib.rs"]
    );
    let changed_state = bundle
        .files
        .iter()
        .find(|file| file.path == "src/lib.rs")
        .expect("changed source should be bundled")
        .state;
    assert_eq!(changed_state, EvidenceState::Unstaged);
}

#[test]
fn inspect_uses_incremental_selection_after_a_baseline_exists() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    git_command(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git_command(repository.path(), &["config", "user.name", "PMEMC Test"]);
    std::fs::write(repository.path().join("main.rs"), "pub fn run() {}\n")
        .expect("source fixture should be written");
    git_command(repository.path(), &["add", "main.rs"]);
    git_command(repository.path(), &["commit", "-m", "initial fixture"]);
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
    let database_path = data_directory.path().join("PMEMC").join("pmemc.sqlite3");
    let connection = rusqlite::Connection::open(&database_path).expect("database should open");
    connection
        .execute(
            "UPDATE projects SET lifecycle_state = 'baselined' WHERE id = 1",
            [],
        )
        .expect("project should be marked baselined");
    std::fs::write(
        repository.path().join("main.rs"),
        "pub fn run() {}\n// changed\n",
    )
    .expect("source fixture should change");

    let inspection = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"y\n");
    assert!(!inspection.status.success());
    let stored_bundle: String = connection
        .query_row(
            "SELECT bundle_json FROM inspection_attempts ORDER BY id DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .expect("incremental bundle should be staged");
    let bundle: serde_json::Value =
        serde_json::from_str(&stored_bundle).expect("bundle should be valid JSON");
    assert_eq!(bundle["initial_inspection"], false);
}

#[test]
fn initial_bundle_never_exceeds_its_serialized_size_limit() {
    let repository = TemporaryDirectory::new();
    git_command(repository.path(), &["init"]);
    for index in 0..20 {
        std::fs::write(
            repository.path().join(format!("notes-{index}.md")),
            "x".repeat(8 * 1024),
        )
        .expect("large text fixture should be written");
    }

    let status = git::working_tree_status(repository.path()).expect("status should be read");
    let bundle = build_initial_bundle(repository.path(), "project-1", &status)
        .expect("bundle should be built");
    let serialized = serde_json::to_vec(&bundle).expect("bundle should serialize");

    assert!(serialized.len() <= 64 * 1024);
    assert!(bundle.files.len() < 20);
}
