mod support;

use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

use pmemc::git;
use support::TemporaryDirectory;

fn git(repository: &Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should run");
    assert!(output.status.success(), "git failed: {output:?}");
}

fn git_output(repository: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should run");
    assert!(output.status.success(), "git failed: {output:?}");
    String::from_utf8(output.stdout)
        .expect("git output should be UTF-8")
        .trim()
        .to_owned()
}

fn committed_repository() -> TemporaryDirectory {
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init", "-b", "main"]);
    git(
        repository.path(),
        &["config", "user.email", "pmemc-test@example.invalid"],
    );
    git(repository.path(), &["config", "user.name", "PMEMC Test"]);
    fs::write(repository.path().join("tracked.txt"), "initial\n")
        .expect("fixture should be written");
    git(repository.path(), &["add", "tracked.txt"]);
    git(repository.path(), &["commit", "-m", "initial"]);
    repository
}

fn validation_error(repository: &Path) -> String {
    git::validate_repository_for_inspection(repository)
        .expect_err("repository should be blocked")
        .to_string()
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
        .expect("inspection input should be written");
    process.wait_with_output().expect("pmemc should finish")
}

fn pmemc_in(repository: &Path, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("pmemc should run")
}

fn pmemc_in_with_data(
    repository: &Path,
    data_directory: &TemporaryDirectory,
    arguments: &[&str],
) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .current_dir(repository)
        .env("LOCALAPPDATA", data_directory.path())
        .env_remove("OPENROUTER_API_KEY")
        .output()
        .expect("pmemc should run")
}

fn confirmed_codegraph_data(repository: &Path) {
    let data_directory = repository.join(".codegraph");
    fs::create_dir_all(&data_directory).expect("CodeGraph data directory should be created");
    fs::write(data_directory.join("codegraph.db"), "fixture\n")
        .expect("CodeGraph database fixture should be written");
    fs::write(data_directory.join(".gitignore"), "*\n!.gitignore\n")
        .expect("CodeGraph ignore fixture should be written");
}

fn local_exclude_path(repository: &Path) -> std::path::PathBuf {
    let path = std::path::PathBuf::from(git_output(
        repository,
        &["rev-parse", "--git-path", "info/exclude"],
    ));
    if path.is_absolute() {
        path
    } else {
        repository.join(path)
    }
}

#[test]
fn clean_repository_returns_its_root_and_head_commit() {
    let repository = committed_repository();

    let validated = git::validate_repository_for_inspection(repository.path())
        .expect("clean committed repository should validate");

    assert!(!validated.root.to_string_lossy().starts_with(r"\\?\"));
    assert_eq!(
        validated.head_commit,
        git_output(repository.path(), &["rev-parse", "HEAD"])
    );
}

#[test]
fn bare_pmemc_validates_then_registers_a_clean_repository() {
    let repository = committed_repository();
    let data_directory = TemporaryDirectory::new();

    let output = pmemc_in_with_data(repository.path(), &data_directory, &[]);

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let validation = stdout
        .find("[1/2] Repository validation")
        .expect("startup should show repository validation");
    let registration = stdout
        .find("[2/2] Project registration")
        .expect("startup should show project registration");
    assert!(validation < registration);
    assert!(stdout.contains("✓ Clean committed state"));
    assert!(stdout.contains("✓ Registered successfully"));
    let repository_name = repository
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temporary repository should have a name");
    assert!(stdout.contains(&format!("Project: {repository_name}")));
    assert!(!stdout.contains("Project ID:"));
    assert!(stdout.contains(&git_output(repository.path(), &["rev-parse", "HEAD"])));
    assert!(!stdout.contains(r"\\?\"));
}

#[test]
fn bare_pmemc_reuses_an_existing_registration_without_a_duplicate() {
    let repository = committed_repository();
    let data_directory = TemporaryDirectory::new();

    assert!(
        pmemc_in_with_data(repository.path(), &data_directory, &[])
            .status
            .success()
    );
    let repeated = pmemc_in_with_data(repository.path(), &data_directory, &[]);

    assert!(repeated.status.success());
    let stdout = String::from_utf8_lossy(&repeated.stdout);
    assert!(stdout.contains("✓ Already registered"));
    let repository_name = repository
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temporary repository should have a name");
    assert!(stdout.contains(&format!("Project: {repository_name}")));
    assert!(!stdout.contains("Project ID:"));
    let connection = rusqlite::Connection::open(data_directory.path().join("PMEMC/pmemc.sqlite3"))
        .expect("database should open");
    let project_count: i64 = connection
        .query_row("SELECT COUNT(*) FROM projects", [], |row| row.get(0))
        .expect("project count should be queryable");
    assert_eq!(project_count, 1);
}

#[test]
fn bare_pmemc_repairs_a_legacy_project_name_from_the_repository_slug() {
    let repository = committed_repository();
    let data_directory = TemporaryDirectory::new();

    assert!(
        pmemc_in_with_data(repository.path(), &data_directory, &[])
            .status
            .success()
    );
    let database_path = data_directory.path().join("PMEMC/pmemc.sqlite3");
    let connection = rusqlite::Connection::open(&database_path)
        .expect("database should open after registration");
    connection
        .execute("UPDATE projects SET name = 'project-1' WHERE id = 1", [])
        .expect("legacy project name should be stored");
    drop(connection);

    let output = pmemc_in_with_data(repository.path(), &data_directory, &[]);

    assert!(output.status.success());
    let repository_name = repository
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .expect("temporary repository should have a name");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("✓ Already registered"));
    assert!(stdout.contains(&format!("Project: {repository_name}")));
    assert!(!stdout.contains("project-1"));
    let repaired_name: String = rusqlite::Connection::open(&database_path)
        .expect("database should reopen")
        .query_row("SELECT name FROM projects WHERE id = 1", [], |row| {
            row.get(0)
        })
        .expect("repaired project name should be readable");
    assert_eq!(repaired_name, repository_name);
}

#[test]
fn bare_pmemc_stops_before_database_access_when_validation_fails() {
    let repository = committed_repository();
    let data_directory = TemporaryDirectory::new();
    fs::write(repository.path().join("untracked.txt"), "untracked\n").unwrap();

    let output = pmemc_in_with_data(repository.path(), &data_directory, &[]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Untracked files (1)"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("[2/2] Project registration"));
    assert!(!data_directory.path().join("PMEMC/pmemc.sqlite3").exists());
}

#[test]
fn bare_pmemc_reports_a_database_failure_after_validation() {
    let repository = committed_repository();
    let data_directory = TemporaryDirectory::new();
    fs::write(data_directory.path().join("PMEMC"), "not a directory\n").unwrap();

    let output = pmemc_in_with_data(repository.path(), &data_directory, &[]);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("[1/2] Repository validation"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("[2/2] Project registration"));
}

#[test]
fn bare_pmemc_groups_and_sorts_repository_blockers() {
    let repository = committed_repository();
    fs::write(repository.path().join("zeta.txt"), "new\n").unwrap();
    fs::write(repository.path().join("alpha.txt"), "new\n").unwrap();
    fs::write(repository.path().join("tracked.txt"), "modified\n").unwrap();

    let output = pmemc_in(repository.path(), &[]);

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("✗ PMEMC inspection is blocked\n  Repository: "));
    assert!(stderr.contains("  Blocking conditions:\n    ! Unstaged changes (1)\n      - tracked.txt\n    ! Untracked files (2)\n      - alpha.txt\n      - zeta.txt"));
    assert!(stderr.contains(
        "  • Commit, stash, remove, or ignore the listed files as appropriate, then retry."
    ));
    assert!(!stderr.contains("error:"));
}

#[test]
fn repository_without_a_commit_is_blocked() {
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init", "-b", "main"]);

    assert!(validation_error(repository.path()).contains("at least one commit"));
}

#[test]
fn staged_changes_are_blocked() {
    let repository = committed_repository();
    fs::write(repository.path().join("staged.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "staged.txt"]);

    assert!(validation_error(repository.path()).contains("staged changes: staged.txt"));
}

#[test]
fn unstaged_changes_are_blocked() {
    let repository = committed_repository();
    fs::write(repository.path().join("tracked.txt"), "modified\n").unwrap();

    assert!(validation_error(repository.path()).contains("unstaged changes: tracked.txt"));
}

#[test]
fn non_ignored_untracked_files_are_blocked() {
    let repository = committed_repository();
    fs::write(repository.path().join("untracked.txt"), "untracked\n").unwrap();

    assert!(validation_error(repository.path()).contains("untracked files: untracked.txt"));
}

#[test]
fn merge_conflicts_are_blocked() {
    let repository = committed_repository();
    git(repository.path(), &["checkout", "-b", "conflict"]);
    fs::write(repository.path().join("tracked.txt"), "conflict branch\n").unwrap();
    git(repository.path(), &["commit", "-am", "conflict branch"]);
    git(repository.path(), &["checkout", "main"]);
    fs::write(repository.path().join("tracked.txt"), "main branch\n").unwrap();
    git(repository.path(), &["commit", "-am", "main branch"]);
    let merge = Command::new("git")
        .args(["merge", "conflict"])
        .current_dir(repository.path())
        .output()
        .expect("merge should run");
    assert!(!merge.status.success(), "fixture merge should conflict");

    assert!(validation_error(repository.path()).contains("merge conflicts: tracked.txt"));
}

#[test]
fn unfinished_operations_are_blocked() {
    let operations = [
        ("MERGE_HEAD", "unfinished merge"),
        ("rebase-merge", "unfinished rebase"),
        ("CHERRY_PICK_HEAD", "unfinished cherry-pick"),
        ("REVERT_HEAD", "unfinished revert"),
    ];

    for (marker, expected) in operations {
        let repository = committed_repository();
        let marker_path = repository.path().join(".git").join(marker);
        if marker.contains("rebase") {
            fs::create_dir_all(&marker_path).unwrap();
        } else {
            fs::write(marker_path, "fixture\n").unwrap();
        }

        assert!(validation_error(repository.path()).contains(expected));
    }
}

#[test]
fn ignored_files_do_not_block_validation() {
    let repository = committed_repository();
    fs::write(repository.path().join(".gitignore"), "ignored.txt\n").unwrap();
    git(repository.path(), &["add", ".gitignore"]);
    git(repository.path(), &["commit", "-m", "ignore fixture"]);
    fs::write(repository.path().join("ignored.txt"), "ignored\n").unwrap();

    git::validate_repository_for_inspection(repository.path())
        .expect("ignored files should not block validation");
}

#[test]
fn confirmed_codegraph_data_does_not_block_validation() {
    let repository = committed_repository();
    confirmed_codegraph_data(repository.path());
    fs::write(
        repository.path().join(".codegraph/transient.log"),
        "fixture\n",
    )
    .unwrap();

    git::validate_repository_for_inspection(repository.path())
        .expect("confirmed CodeGraph local data should not block validation");
}

#[test]
fn arbitrary_untracked_gitignore_still_blocks_validation() {
    let repository = committed_repository();
    fs::create_dir_all(repository.path().join("notes")).unwrap();
    fs::write(repository.path().join("notes/.gitignore"), "drafts/\n").unwrap();

    assert!(validation_error(repository.path()).contains("notes/.gitignore"));
}

#[test]
fn modified_tracked_codegraph_gitignore_still_blocks_validation() {
    let repository = committed_repository();
    confirmed_codegraph_data(repository.path());
    git(repository.path(), &["add", ".codegraph"]);
    git(
        repository.path(),
        &["commit", "-m", "track CodeGraph fixture"],
    );
    fs::write(
        repository.path().join(".codegraph/.gitignore"),
        "modified\n",
    )
    .unwrap();

    assert!(validation_error(repository.path()).contains(".codegraph/.gitignore"));
}

#[test]
fn validation_preserves_existing_local_exclusions() {
    let repository = committed_repository();
    let exclude_path = local_exclude_path(repository.path());
    fs::write(&exclude_path, "user-local-rule/\n").unwrap();
    confirmed_codegraph_data(repository.path());

    git::validate_repository_for_inspection(repository.path()).unwrap();
    git::validate_repository_for_inspection(repository.path()).unwrap();

    assert_eq!(
        fs::read_to_string(exclude_path).unwrap(),
        "user-local-rule/\n"
    );
}

#[test]
fn registration_remains_available_for_dirty_repositories() {
    let data_directory = TemporaryDirectory::new();
    let repository = committed_repository();
    fs::write(repository.path().join(".gitignore"), "local-only/\n").unwrap();

    let registration = pmemc_in_with_data(
        repository.path(),
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );

    assert!(registration.status.success());
}

#[test]
fn locally_committed_unpushed_changes_are_valid() {
    let remote = TemporaryDirectory::new();
    git(remote.path(), &["init", "--bare"]);
    let repository = committed_repository();
    git(
        repository.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(repository.path(), &["push", "-u", "origin", "main"]);
    git(
        repository.path(),
        &["commit", "--allow-empty", "-m", "local only"],
    );

    git::validate_repository_for_inspection(repository.path())
        .expect("an unpushed local commit should be valid");
}

#[test]
fn dirty_repository_is_blocked_before_the_inspection_workflow_starts() {
    let data_directory = TemporaryDirectory::new();
    let repository = committed_repository();
    let registration = pmemc_with_input(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
        b"",
    );
    assert!(registration.status.success());
    fs::write(repository.path().join("staged.txt"), "staged\n").unwrap();
    git(repository.path(), &["add", "staged.txt"]);

    let inspection = pmemc_with_input(&data_directory, &["inspect", "project-1"], b"yes\n");

    assert!(!inspection.status.success());
    assert!(String::from_utf8_lossy(&inspection.stderr).contains("Staged changes (1)"));
    let connection = rusqlite::Connection::open(data_directory.path().join("PMEMC/pmemc.sqlite3"))
        .expect("database should open");
    let attempts: i64 = connection
        .query_row("SELECT COUNT(*) FROM inspection_attempts", [], |row| {
            row.get(0)
        })
        .expect("attempt count should be queryable");
    assert_eq!(attempts, 0);
}
