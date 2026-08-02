mod support;

use std::{
    io::Write,
    process::{Command, Stdio},
};

use pmemc::{
    inspection::{EvidenceBundle, EvidenceFile, EvidenceState},
    provider::{
        Proposal, ProposedConfidence, ProposedLifecycle, ProviderInvocationMetadata,
        ProviderResponse,
    },
    storage::{self, BaselineProvenance, ReviewDecision},
};
use support::TemporaryDirectory;

fn pmemc(data_directory: &TemporaryDirectory, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .env("LOCALAPPDATA", data_directory.path())
        .output()
        .expect("pmemc should run")
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
        .expect("pmemc should run");
    process
        .stdin
        .take()
        .expect("stdin should be available")
        .write_all(input)
        .expect("input should be written");
    process.wait_with_output().expect("pmemc should finish")
}

fn git(repository: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should run");
    assert!(output.status.success(), "git failed: {:?}", output);
}

#[test]
fn project_add_registers_a_git_working_tree_without_reading_source_content() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    git(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "PMEMC Test"]);
    let nested_path = repository.path().join("nested");
    std::fs::create_dir(&nested_path).expect("nested fixture directory should be created");

    let output = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            nested_path.to_str().expect("UTF-8 test path"),
        ],
    );

    assert!(output.status.success());
    let repository_name = repository
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temporary repository should have a name");
    let add_output = String::from_utf8_lossy(&output.stdout);
    assert!(add_output.contains(&format!("as {repository_name}")));
    assert!(!add_output.contains("project-1"));
    let list = pmemc(&data_directory, &["project", "list"]);
    assert!(list.status.success());
    let list_output = String::from_utf8_lossy(&list.stdout);
    assert!(list_output.contains("registered_needs_inspection"));
    assert!(list_output.contains("branch="));
    assert!(list_output.contains("last-approved-inspection=none"));

    let show = pmemc(&data_directory, &["project", "show", "project-1"]);
    assert!(show.status.success());
    let show_output = String::from_utf8_lossy(&show.stdout);
    assert!(show_output.contains("registered_needs_inspection"));
    assert!(show_output.contains("branch="));
    assert!(show_output.contains("head=unborn"));

    let status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(status.status.success());
    let status_output = String::from_utf8_lossy(&status.stdout);
    assert!(status_output.contains("initial inspection required"));
    assert!(status_output.contains("branch\t"));
    assert!(status_output.contains("head\tunborn"));
    assert!(status_output.contains("commits-since-baseline\tnot-applicable"));

    let named_status = pmemc(&data_directory, &["status", repository_name]);
    assert!(named_status.status.success());
    assert!(String::from_utf8_lossy(&named_status.stdout).contains("initial inspection required"));

    std::fs::write(repository.path().join("untracked.txt"), "not committed")
        .expect("fixture file should be written");
    let changed_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(changed_status.status.success());
    assert!(String::from_utf8_lossy(&changed_status.stdout).contains("untracked.txt"));

    std::fs::write(repository.path().join("tracked.txt"), "tracked")
        .expect("fixture file should be written");
    git(repository.path(), &["add", "tracked.txt"]);
    let staged_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(staged_status.status.success());
    let staged_status_output = String::from_utf8_lossy(&staged_status.stdout);
    assert!(staged_status_output.contains("staged\ttracked.txt"));
    assert!(staged_status_output.contains("added\ttracked.txt"));

    std::fs::write(repository.path().join("tracked.txt"), "modified")
        .expect("fixture file should be modified");
    let unstaged_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(unstaged_status.status.success());
    let unstaged_status_output = String::from_utf8_lossy(&unstaged_status.stdout);
    assert!(unstaged_status_output.contains("unstaged\ttracked.txt"));
    assert!(unstaged_status_output.contains("modified\ttracked.txt"));

    let duplicate = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(!duplicate.status.success());
    let connection = rusqlite::Connection::open(data_directory.path().join("PMEMC/pmemc.sqlite3"))
        .expect("database should be readable after duplicate registration");
    let project_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .expect("project count should be readable");
    assert_eq!(project_count, 1);

    let non_repository = TemporaryDirectory::new();
    let invalid_add = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            non_repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(!invalid_add.status.success());
    assert!(
        String::from_utf8_lossy(&invalid_add.stderr).contains("Git could not inspect"),
        "registration error should identify the Git inspection failure"
    );
    let project_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .expect("project count should be readable");
    assert_eq!(project_count, 1);

    std::fs::remove_file(repository.path().join("tracked.txt"))
        .expect("fixture file should be deleted");
    let deleted_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(deleted_status.status.success());
    assert!(String::from_utf8_lossy(&deleted_status.stdout).contains("deleted\ttracked.txt"));

    std::fs::write(repository.path().join("old-name.txt"), "rename me")
        .expect("fixture file should be written");
    git(repository.path(), &["add", "old-name.txt"]);
    git(repository.path(), &["commit", "-m", "add rename fixture"]);
    git(repository.path(), &["mv", "old-name.txt", "new-name.txt"]);
    let renamed_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(renamed_status.status.success());
    assert!(
        String::from_utf8_lossy(&renamed_status.stdout)
            .contains("renamed\told-name.txt\tnew-name.txt")
    );

    let second_repository = TemporaryDirectory::new();
    git(second_repository.path(), &["init"]);
    let second_add = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            second_repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(second_add.status.success());
    let all_status = pmemc(&data_directory, &["status"]);
    assert!(all_status.status.success());
    let all_status_output = String::from_utf8_lossy(&all_status.stdout);
    assert!(all_status_output.contains(&repository.path().display().to_string()));
    assert!(all_status_output.contains(&second_repository.path().display().to_string()));
    let project_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .expect("project count should be readable");
    assert_eq!(project_count, 2);
}

#[test]
fn status_reports_commits_made_after_registration_without_creating_a_baseline() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    git(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "PMEMC Test"]);
    std::fs::write(repository.path().join("initial.txt"), "initial")
        .expect("fixture file should be written");
    git(repository.path(), &["add", "initial.txt"]);
    git(repository.path(), &["commit", "-m", "initial commit"]);

    let add = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(add.status.success());
    git(
        repository.path(),
        &["commit", "--allow-empty", "-m", "later commit"],
    );

    let status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(status.status.success());
    let status_output = String::from_utf8_lossy(&status.stdout);
    assert!(status_output.contains("committed-since-registration\t1"));
    assert!(status_output.contains("commits-since-baseline\tnot-applicable"));
}

#[test]
fn status_detects_a_staged_copy_without_relying_on_git_configuration() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    git(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "PMEMC Test"]);
    std::fs::write(repository.path().join("source.txt"), "copy fixture content")
        .expect("fixture file should be written");
    git(repository.path(), &["add", "source.txt"]);
    git(repository.path(), &["commit", "-m", "add copy source"]);

    let add = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(add.status.success());
    std::fs::copy(
        repository.path().join("source.txt"),
        repository.path().join("copy.txt"),
    )
    .expect("fixture file should be copied");
    git(repository.path(), &["add", "copy.txt"]);

    let status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(status.status.success());
    let status_output = String::from_utf8_lossy(&status.stdout);
    assert!(
        status_output.contains("copied\tsource.txt\tcopy.txt"),
        "expected copy relationship, got:\n{status_output}"
    );
}

#[test]
fn project_forget_requires_exact_confirmation_and_preserves_repository_files() {
    let data_directory = TemporaryDirectory::new();
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    let source = repository.path().join("keep.txt");
    std::fs::write(&source, "repository content").expect("repository file should be written");
    let add = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );
    assert!(add.status.success());
    let project_name = repository
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temporary repository should have a name");

    let cancelled = pmemc_with_input(
        &data_directory,
        &["project", "forget", "project-1"],
        b"wrong-name\n",
    );
    assert!(cancelled.status.success());
    assert!(String::from_utf8_lossy(&cancelled.stdout).contains("cancelled"));
    let still_registered = pmemc(&data_directory, &["project", "show", "project-1"]);
    assert!(still_registered.status.success());

    let forgotten = pmemc(
        &data_directory,
        &[
            "project",
            "forget",
            "project-1",
            "--confirm-name",
            project_name,
        ],
    );
    assert!(forgotten.status.success());
    let forgotten_output = String::from_utf8_lossy(&forgotten.stdout);
    assert!(forgotten_output.contains("PMEMC memory and registration forgotten"));
    assert!(forgotten_output.contains("Repository files were not changed"));
    assert!(source.is_file());
    assert!(
        !pmemc(&data_directory, &["project", "show", "project-1"])
            .status
            .success()
    );
}

#[test]
fn forget_project_deletes_only_selected_project_memory_transactionally() {
    let data_directory = TemporaryDirectory::new();
    let repository_a = TemporaryDirectory::new();
    let repository_b = TemporaryDirectory::new();
    let source_a = repository_a.path().join("keep.txt");
    std::fs::write(&source_a, "keep A").expect("repository A file should be written");
    let data_paths = storage::DataPaths::from_root(data_directory.path().join("PMEMC"));
    let project_a = storage::add_project(
        &data_paths,
        repository_a.path(),
        Some("main"),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
    )
    .expect("project A should be stored");
    let project_b = storage::add_project(
        &data_paths,
        repository_b.path(),
        Some("main"),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
    )
    .expect("project B should be stored");
    let bundle = EvidenceBundle {
        schema_version: 1,
        project_id: format!("project-{}", project_a.id),
        initial_inspection: true,
        files: vec![EvidenceFile {
            path: "keep.txt".into(),
            state: EvidenceState::Committed,
            content: "keep A".into(),
            redacted: false,
        }],
    };
    let bundle_json = serde_json::to_string(&bundle).expect("bundle should serialize");
    let attempt_id = storage::stage_inspection_attempt_with_baseline_provenance(
        &data_paths,
        project_a.id,
        1,
        &bundle_json,
        Some(r#"{"symbols":[]}"#),
        &BaselineProvenance {
            repository_commit: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            repository_branch: Some("main".into()),
            working_tree_status_json: "{}".into(),
            uncommitted_fingerprints_json: "{}".into(),
        },
    )
    .expect("attempt should be staged");
    let response = ProviderResponse {
        schema_version: 1,
        proposals: vec![Proposal {
            fact_kind: "repository_observation".into(),
            statement: "Project A has a source file.".into(),
            lifecycle: ProposedLifecycle::Committed,
            confidence: ProposedConfidence::Exact,
            evidence_paths: vec!["keep.txt".into()],
        }],
        questions: vec!["What is the project purpose?".into()],
    };
    let metadata = ProviderInvocationMetadata::new("fake", "offline-test-model", 1)
        .expect("metadata should be valid");
    storage::store_provider_response(&data_paths, attempt_id, &metadata, &response)
        .expect("provider response should be stored");
    storage::record_review_decision(&data_paths, 1, &ReviewDecision::Approve)
        .expect("proposal should be approved");
    storage::finalize_review(&data_paths, project_a.id).expect("review should finalize");

    let second_attempt_id = storage::stage_inspection_attempt_with_baseline_provenance(
        &data_paths,
        project_a.id,
        1,
        &bundle_json,
        Some(r#"{"symbols":[]}"#),
        &BaselineProvenance {
            repository_commit: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            repository_branch: Some("main".into()),
            working_tree_status_json: "{}".into(),
            uncommitted_fingerprints_json: "{}".into(),
        },
    )
    .expect("second attempt should be staged");
    let before_conflict = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable before conflict");
    let existing: (i64, i64) = before_conflict
        .query_row(
            "SELECT (SELECT COUNT(*) FROM verified_facts WHERE project_id = ?1), (SELECT COUNT(*) FROM fact_evidence WHERE project_id = ?1)",
            [project_a.id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("existing fact counts should be readable");
    assert_eq!(existing, (1, 1));
    let mut conflicting_response = response.clone();
    conflicting_response.proposals[0].statement = "Project A has a source directory.".into();
    conflicting_response.questions.clear();
    storage::store_provider_response(
        &data_paths,
        second_attempt_id,
        &metadata,
        &conflicting_response,
    )
    .expect("conflicting response should be stored");
    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable before forget");
    let conflict_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM conflicts", [], |row| row.get(0))
        .expect("conflict count should be readable");
    assert_eq!(conflict_count, 1);

    let summary = storage::forget_project(&data_paths, project_a.id)
        .expect("project memory should be forgotten");

    assert_eq!(summary.project_id, project_a.id);
    assert_eq!(summary.name, project_a.name);
    assert!(summary.verified_fact_count >= 1);
    assert!(summary.evidence_count >= 1);
    assert!(source_a.is_file());
    assert!(
        storage::project_by_id(&data_paths, project_a.id)
            .expect("project lookup should work")
            .is_none()
    );
    assert!(
        storage::project_by_id(&data_paths, project_b.id)
            .expect("other project lookup should work")
            .is_some()
    );

    let connection = rusqlite::Connection::open(data_paths.database_path())
        .expect("database should be readable");
    for (table, query) in [
        (
            "conflicts",
            "SELECT COUNT(*) FROM conflicts JOIN proposals ON proposals.id = conflicts.proposal_id JOIN inspection_attempts ON inspection_attempts.id = proposals.inspection_attempt_id WHERE inspection_attempts.project_id = ?1",
        ),
        (
            "fact_evidence",
            "SELECT COUNT(*) FROM fact_evidence WHERE project_id = ?1",
        ),
        (
            "review_decisions",
            "SELECT COUNT(*) FROM review_decisions JOIN proposals ON proposals.id = review_decisions.proposal_id JOIN inspection_attempts ON inspection_attempts.id = proposals.inspection_attempt_id WHERE inspection_attempts.project_id = ?1",
        ),
        (
            "questions",
            "SELECT COUNT(*) FROM questions JOIN inspection_attempts ON inspection_attempts.id = questions.inspection_attempt_id WHERE inspection_attempts.project_id = ?1",
        ),
        (
            "proposals",
            "SELECT COUNT(*) FROM proposals JOIN inspection_attempts ON inspection_attempts.id = proposals.inspection_attempt_id WHERE inspection_attempts.project_id = ?1",
        ),
        (
            "provider_invocations",
            "SELECT COUNT(*) FROM provider_invocations JOIN inspection_attempts ON inspection_attempts.id = provider_invocations.inspection_attempt_id WHERE inspection_attempts.project_id = ?1",
        ),
        (
            "inspection_baselines",
            "SELECT COUNT(*) FROM inspection_baselines WHERE project_id = ?1",
        ),
        (
            "inspection_attempts",
            "SELECT COUNT(*) FROM inspection_attempts WHERE project_id = ?1",
        ),
        (
            "code_map_snapshots",
            "SELECT COUNT(*) FROM code_map_snapshots WHERE project_id = ?1",
        ),
        (
            "verified_facts",
            "SELECT COUNT(*) FROM verified_facts WHERE project_id = ?1",
        ),
    ] {
        let count: i64 = connection
            .query_row(query, [project_a.id], |row| row.get(0))
            .expect("project-owned row count should be readable");
        assert_eq!(count, 0, "{table} should be empty for forgotten project");
    }
}
