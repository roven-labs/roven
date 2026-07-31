mod support;

use std::process::Command;

use pmemc::code_map::{SymbolKind, build_code_map};
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
fn rust_symbols_and_unambiguous_calls_are_deterministic_and_malformed_files_do_not_abort() {
    let repository = TemporaryDirectory::new();
    git(repository.path(), &["init"]);
    std::fs::create_dir_all(repository.path().join("src")).expect("source directory should exist");
    std::fs::write(
        repository.path().join("src/lib.rs"),
        "pub struct Service;\nimpl Service { pub fn start(&self) {} }\npub fn run() { helper(); }\nfn helper() {}\n",
    )
    .expect("Rust fixture should be written");
    std::fs::write(repository.path().join("broken.py"), "def unfinished(:\n")
        .expect("malformed fixture should be written");

    let map = build_code_map(repository.path()).expect("map should be built");

    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Service" && symbol.kind == SymbolKind::Struct)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "start" && symbol.kind == SymbolKind::Method)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "run" && symbol.kind == SymbolKind::Function)
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "run" && call.callee == "helper")
    );
    assert!(map.unsupported_paths.contains(&"broken.py".into()));
}
