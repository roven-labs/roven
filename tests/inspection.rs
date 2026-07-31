mod support;

use std::process::Command;

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
