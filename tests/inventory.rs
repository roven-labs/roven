mod support;

use std::process::Command;

use pmemc::inventory::{Language, inventory};
use support::TemporaryDirectory;

fn git(repository: &std::path::Path, arguments: &[&str]) {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(repository)
        .output()
        .expect("git should run");
    assert!(output.status.success(), "git failed: {output:?}");
}

#[test]
fn inventory_is_deterministic_and_excludes_ignored_unsafe_and_binary_files() {
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    std::fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    std::fs::create_dir_all(repository.path().join("private"))
        .expect("private directory should exist");
    std::fs::create_dir_all(repository.path().join("target"))
        .expect("build directory should exist");
    std::fs::write(repository.path().join(".gitignore"), "ignored.rs\n")
        .expect("gitignore should be written");
    std::fs::write(repository.path().join(".pmemcignore"), "private/**\n")
        .expect("pmemcignore should be written");
    std::fs::write(repository.path().join("src/lib.rs"), "pub fn run() {}\n")
        .expect("Rust fixture should be written");
    std::fs::write(repository.path().join("notes.md"), "# Notes\n")
        .expect("Markdown fixture should be written");
    std::fs::write(repository.path().join("unknown.custom"), "not supported\n")
        .expect("unsupported fixture should be written");
    std::fs::write(
        repository.path().join("private/scratch.py"),
        "print('private')\n",
    )
    .expect("ignored fixture should be written");
    std::fs::write(
        repository.path().join("ignored.rs"),
        "pub fn ignored() {}\n",
    )
    .expect("Git-ignored fixture should be written");
    std::fs::write(
        repository.path().join("target/generated.rs"),
        "pub fn generated() {}\n",
    )
    .expect("build fixture should be written");
    std::fs::write(repository.path().join(".env"), "SECRET=value\n")
        .expect("secret fixture should be written");
    std::fs::write(repository.path().join("key.pem"), "private key\n")
        .expect("key fixture should be written");
    std::fs::write(repository.path().join("binary.dat"), [0_u8, 1, 2])
        .expect("binary fixture should be written");

    let inventory = inventory(repository.path()).expect("inventory should succeed");
    let paths = inventory
        .files
        .iter()
        .map(|file| file.path.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        paths,
        [
            ".gitignore",
            ".pmemcignore",
            "notes.md",
            "src/lib.rs",
            "unknown.custom"
        ]
    );
    assert_eq!(inventory.files[2].language, Language::GenericText);
    assert_eq!(inventory.files[3].language, Language::Rust);
    assert_eq!(inventory.files[4].language, Language::Unsupported);
}
