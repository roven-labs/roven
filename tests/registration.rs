mod support;

use std::process::Command;

use support::TemporaryDirectory;

fn pmemc(data_directory: &TemporaryDirectory, arguments: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_pmemc"))
        .args(arguments)
        .env("LOCALAPPDATA", data_directory.path())
        .output()
        .expect("pmemc should run")
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
    let after_duplicate = pmemc(&data_directory, &["project", "list"]);
    assert_eq!(
        String::from_utf8_lossy(&after_duplicate.stdout)
            .matches("project-1")
            .count(),
        1
    );

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
    let after_invalid_add = pmemc(&data_directory, &["project", "list"]);
    assert!(after_invalid_add.status.success());
    assert!(!String::from_utf8_lossy(&after_invalid_add.stdout).contains("project-2"));

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
    assert!(all_status_output.contains("project-1\tinitial inspection required"));
    assert!(all_status_output.contains("project-2\tinitial inspection required"));
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
