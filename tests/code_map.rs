mod support;

use std::process::Command;

use pmemc::code_map::{
    RelationKind, SymbolKind, build_code_map, serialize_code_map, structural_neighbors,
};
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
        "mod helper;\nuse crate::helper::utility;\npub trait Worker {}\npub enum State { Ready }\npub struct Service;\nimpl Service { pub fn start(&self) {} }\npub fn run() { helper(); utility(); }\nfn helper() {}\n",
    )
    .expect("Rust fixture should be written");
    std::fs::write(
        repository.path().join("src/helper.rs"),
        "pub fn utility() {}\n",
    )
    .expect("Rust helper fixture should be written");
    std::fs::write(repository.path().join("broken.py"), "def unfinished(:\n")
        .expect("malformed fixture should be written");
    std::fs::write(
        repository.path().join("good.py"),
        "def caller():\n    py_helper()\n\ndef py_helper():\n    pass\n",
    )
    .expect("Python fixture should be written");
    std::fs::write(
        repository.path().join("web.js"),
        "import { utility } from \"./util.js\";\nfunction webCaller() { webHelper(); }\nfunction webHelper() {}\n",
    )
    .expect("JavaScript fixture should be written");
    std::fs::write(
        repository.path().join("util.js"),
        "export function utility() {}\n",
    )
    .expect("JavaScript utility fixture should be written");
    std::fs::write(
        repository.path().join("typed.ts"),
        "class Typed {}\ninterface Shape {}\nenum Mode { Ready }\nfunction typedCaller(): void { typedHelper(); }\nfunction typedHelper(): void {}\n",
    )
    .expect("TypeScript fixture should be written");
    std::fs::write(
        repository.path().join("view.tsx"),
        "function viewCaller() { viewHelper(); return <div />; }\nfunction viewHelper() {}\n",
    )
    .expect("TSX fixture should be written");
    std::fs::write(
        repository.path().join("widget.jsx"),
        "function jsxCaller() { jsxHelper(); return <div />; }\nfunction jsxHelper() {}\n",
    )
    .expect("JSX fixture should be written");
    std::fs::write(
        repository.path().join("Demo.java"),
        "class Demo { void javaCaller() { javaHelper(); } void javaHelper() {} } interface Contract {} enum Color { RED }\n",
    )
    .expect("Java fixture should be written");
    std::fs::write(
        repository.path().join("sample.go"),
        "package sample\nfunc goCaller() { goHelper() }\nfunc goHelper() {}\n",
    )
    .expect("Go fixture should be written");

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
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Worker" && symbol.kind == SymbolKind::Trait)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "State" && symbol.kind == SymbolKind::Enum)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "helper" && symbol.kind == SymbolKind::Module)
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "run" && call.callee == "helper")
    );
    assert!(map.unsupported_paths.contains(&"broken.py".into()));
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "caller" && symbol.kind == SymbolKind::Function)
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "caller" && call.callee == "py_helper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "webCaller" && call.callee == "webHelper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "typedCaller" && call.callee == "typedHelper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "viewCaller" && call.callee == "viewHelper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "jsxCaller" && call.callee == "jsxHelper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "javaCaller" && call.callee == "javaHelper")
    );
    assert!(
        map.calls
            .iter()
            .any(|call| call.caller == "goCaller" && call.callee == "goHelper")
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Typed" && symbol.kind == SymbolKind::Class)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Shape" && symbol.kind == SymbolKind::Interface)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Mode" && symbol.kind == SymbolKind::Enum)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Demo" && symbol.kind == SymbolKind::Class)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Contract" && symbol.kind == SymbolKind::Interface)
    );
    assert!(
        map.symbols
            .iter()
            .any(|symbol| symbol.name == "Color" && symbol.kind == SymbolKind::Enum)
    );
    assert!(map.imports.iter().any(|import| {
        import.source_path == "src/lib.rs" && import.target_path == "src/helper.rs"
    }));
    assert!(
        map.imports
            .iter()
            .any(|import| { import.source_path == "web.js" && import.target_path == "util.js" })
    );
    let neighbors = structural_neighbors(&map, "run");
    let neighbor_names = neighbors
        .iter()
        .map(|symbol| symbol.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(neighbor_names, ["helper"]);
    let repeat = build_code_map(repository.path()).expect("repeat map should be built");
    assert_eq!(
        serialize_code_map(&map).expect("map should serialize"),
        serialize_code_map(&repeat).expect("repeat map should serialize")
    );
    assert!(map.files.iter().any(|file| file.path == "src/lib.rs"));
    assert!(
        map.relationships
            .iter()
            .any(|relationship| relationship.kind == RelationKind::Contains
                && relationship.source == "repository"
                && relationship.target == "src/lib.rs")
    );
    assert!(
        map.relationships
            .iter()
            .any(|relationship| relationship.kind == RelationKind::Defines
                && relationship.source == "src/lib.rs"
                && relationship.target.contains("run"))
    );
}
