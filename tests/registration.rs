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

    let output = pmemc(
        &data_directory,
        &[
            "project",
            "add",
            repository.path().to_str().expect("UTF-8 test path"),
        ],
    );

    assert!(output.status.success());
    let list = pmemc(&data_directory, &["project", "list"]);
    assert!(list.status.success());
    assert!(String::from_utf8_lossy(&list.stdout).contains("registered_needs_inspection"));

    let show = pmemc(&data_directory, &["project", "show", "project-1"]);
    assert!(show.status.success());
    assert!(String::from_utf8_lossy(&show.stdout).contains("registered_needs_inspection"));

    let status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(status.status.success());
    assert!(String::from_utf8_lossy(&status.stdout).contains("initial inspection required"));

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
    assert!(String::from_utf8_lossy(&staged_status.stdout).contains("staged\ttracked.txt"));

    std::fs::write(repository.path().join("tracked.txt"), "modified")
        .expect("fixture file should be modified");
    let unstaged_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(unstaged_status.status.success());
    assert!(String::from_utf8_lossy(&unstaged_status.stdout).contains("unstaged\ttracked.txt"));

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

    std::fs::remove_file(repository.path().join("tracked.txt"))
        .expect("fixture file should be deleted");
    let deleted_status = pmemc(&data_directory, &["status", "project-1"]);
    assert!(deleted_status.status.success());
    assert!(String::from_utf8_lossy(&deleted_status.stdout).contains("deleted\ttracked.txt"));
}
