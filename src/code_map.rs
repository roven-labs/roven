//! Compact, evidence-oriented structural code map.

use std::{collections::BTreeMap, fs, path::Path};

use serde::Serialize;
use thiserror::Error;
use tree_sitter::{Node, Parser};

use crate::inventory::{InventoryError, Language, contained_path, inventory};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SymbolKind {
    Function,
    Method,
    Class,
    Interface,
    Trait,
    Struct,
    Enum,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Symbol {
    pub path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
    pub container: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectCall {
    pub caller: String,
    pub callee: String,
    pub evidence: Evidence,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Confidence {
    Exact,
    Inferred,
    UserConfirmed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Evidence {
    pub path: String,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Import {
    pub source_path: String,
    pub target_path: String,
    pub evidence: Evidence,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum RelationKind {
    Contains,
    Defines,
    Imports,
    Calls,
    DependsOn,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relationship {
    pub kind: RelationKind,
    pub source: String,
    pub target: String,
    pub evidence: Evidence,
    pub confidence: Confidence,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct CodeMap {
    pub files: Vec<crate::inventory::InventoryFile>,
    pub symbols: Vec<Symbol>,
    pub calls: Vec<DirectCall>,
    pub imports: Vec<Import>,
    pub relationships: Vec<Relationship>,
    pub unsupported_paths: Vec<String>,
}

#[derive(Debug, Error)]
pub enum CodeMapError {
    #[error(transparent)]
    Inventory(#[from] InventoryError),
}

/// Build a deterministic structural map without executing repository code.
pub fn build_code_map(repository: &Path) -> Result<CodeMap, CodeMapError> {
    let root = fs::canonicalize(repository).map_err(|source| InventoryError::Git {
        path: repository.into(),
        message: source.to_string(),
    })?;
    let inventory = inventory(&root)?;
    let known_paths = inventory
        .files
        .iter()
        .map(|file| file.path.clone())
        .collect::<std::collections::BTreeSet<_>>();
    let mut map = CodeMap {
        files: inventory.files.clone(),
        ..CodeMap::default()
    };
    let mut candidate_calls = Vec::new();

    for file in inventory.files {
        if !matches!(
            file.language,
            Language::Rust
                | Language::Python
                | Language::JavaScript
                | Language::TypeScript
                | Language::Jsx
                | Language::Tsx
                | Language::Java
                | Language::Go
        ) {
            if !matches!(file.language, Language::GenericText) {
                map.unsupported_paths.push(file.path);
            }
            continue;
        }
        let Some(path) = contained_path(&root, &file.path) else {
            map.unsupported_paths.push(file.path);
            continue;
        };
        let Ok(source) = fs::read_to_string(path) else {
            map.unsupported_paths.push(file.path);
            continue;
        };
        let tree = match file.language {
            Language::Rust => rust_tree(&source),
            Language::Python => python_tree(&source),
            Language::JavaScript => javascript_tree(&source),
            Language::TypeScript => typescript_tree(&source),
            Language::Jsx => javascript_tree(&source),
            Language::Tsx => tsx_tree(&source),
            Language::Java => java_tree(&source),
            Language::Go => go_tree(&source),
            _ => None,
        };
        let Some(tree) = tree else {
            map.unsupported_paths.push(file.path);
            continue;
        };
        if tree.root_node().has_error() {
            map.unsupported_paths.push(file.path);
            continue;
        }
        match file.language {
            Language::Rust => visit_rust(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            Language::Python => visit_python(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            Language::JavaScript => visit_javascript(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            Language::TypeScript | Language::Jsx | Language::Tsx => visit_javascript(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            Language::Java => visit_java(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            Language::Go => visit_go(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                None,
                &mut map.symbols,
                &mut candidate_calls,
            ),
            _ => unreachable!("only structural languages reach this branch"),
        }
        if file.language == Language::Rust {
            collect_rust_imports(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                &known_paths,
                &mut map.imports,
            );
        } else if matches!(
            file.language,
            Language::JavaScript | Language::TypeScript | Language::Jsx | Language::Tsx
        ) {
            collect_ecmascript_imports(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                &known_paths,
                &mut map.imports,
            );
        } else if file.language == Language::Python {
            collect_python_imports(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                &known_paths,
                &mut map.imports,
            );
        } else if file.language == Language::Java {
            collect_java_imports(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                &known_paths,
                &mut map.imports,
            );
        } else if file.language == Language::Go {
            collect_go_imports(
                tree.root_node(),
                source.as_bytes(),
                &file.path,
                go_module_name(&root).as_deref(),
                &known_paths,
                &mut map.imports,
            );
        }
    }

    let counts = map
        .symbols
        .iter()
        .filter(|symbol| matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method))
        .fold(BTreeMap::new(), |mut counts, symbol| {
            *counts.entry(symbol.name.as_str()).or_insert(0_usize) += 1;
            counts
        });
    map.calls = candidate_calls
        .into_iter()
        .filter(|call| {
            counts.get(call.caller.as_str()) == Some(&1)
                && counts.get(call.callee.as_str()) == Some(&1)
        })
        .collect();
    map.symbols.sort_by(|left, right| {
        (&left.path, left.line, &left.name).cmp(&(&right.path, right.line, &right.name))
    });
    map.calls
        .sort_by(|left, right| (&left.caller, &left.callee).cmp(&(&right.caller, &right.callee)));
    map.unsupported_paths.sort();
    map.imports.sort_by(|left, right| {
        (&left.source_path, &left.target_path, left.evidence.line).cmp(&(
            &right.source_path,
            &right.target_path,
            right.evidence.line,
        ))
    });
    map.relationships = map
        .files
        .iter()
        .map(|file| Relationship {
            kind: RelationKind::Contains,
            source: "repository".into(),
            target: file.path.clone(),
            evidence: Evidence {
                path: file.path.clone(),
                line: 1,
            },
            confidence: Confidence::Exact,
        })
        .chain(map.symbols.iter().map(|symbol| Relationship {
            kind: RelationKind::Defines,
            source: symbol.path.clone(),
            target: symbol_id(symbol),
            evidence: Evidence {
                path: symbol.path.clone(),
                line: symbol.line,
            },
            confidence: Confidence::Exact,
        }))
        .chain(type_member_relationships(&map.symbols))
        .chain(map.imports.iter().map(|import| Relationship {
            kind: RelationKind::Imports,
            source: import.source_path.clone(),
            target: import.target_path.clone(),
            evidence: import.evidence.clone(),
            confidence: import.confidence,
        }))
        .chain(map.calls.iter().map(|call| Relationship {
            kind: RelationKind::Calls,
            source: call.caller.clone(),
            target: call.callee.clone(),
            evidence: call.evidence.clone(),
            confidence: call.confidence,
        }))
        .collect();
    map.relationships.sort_by(|left, right| {
        (&left.source, &left.target, left.kind as u8).cmp(&(
            &right.source,
            &right.target,
            right.kind as u8,
        ))
    });
    Ok(map)
}

fn type_member_relationships(symbols: &[Symbol]) -> Vec<Relationship> {
    symbols
        .iter()
        .filter_map(|member| {
            let container = member.container.as_deref()?;
            let mut types = symbols.iter().filter(|symbol| {
                symbol.path == member.path && symbol.name == container && is_type_symbol(symbol)
            });
            let container = types.next()?;
            types.next().is_none().then(|| Relationship {
                kind: RelationKind::Contains,
                source: symbol_id(container),
                target: symbol_id(member),
                evidence: Evidence {
                    path: member.path.clone(),
                    line: member.line,
                },
                confidence: Confidence::Exact,
            })
        })
        .collect()
}

fn is_type_symbol(symbol: &Symbol) -> bool {
    matches!(
        symbol.kind,
        SymbolKind::Class
            | SymbolKind::Interface
            | SymbolKind::Trait
            | SymbolKind::Struct
            | SymbolKind::Enum
    )
}

fn symbol_id(symbol: &Symbol) -> String {
    format!("{}:{}:{}", symbol.path, symbol.line, symbol.name)
}

/// Serialize the already-sorted compact map deterministically as JSON.
pub fn serialize_code_map(map: &CodeMap) -> Result<String, serde_json::Error> {
    serde_json::to_string(map)
}

/// Return exact direct-call neighbours for one unambiguous symbol name.
#[must_use]
pub fn structural_neighbors(map: &CodeMap, symbol_name: &str) -> Vec<Symbol> {
    let names = map
        .calls
        .iter()
        .filter_map(|call| {
            if call.caller == symbol_name {
                Some(call.callee.as_str())
            } else if call.callee == symbol_name {
                Some(call.caller.as_str())
            } else {
                None
            }
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut neighbours = map
        .symbols
        .iter()
        .filter(|symbol| {
            matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method)
                && names.contains(symbol.name.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    neighbours.sort_by(|left, right| {
        (&left.name, &left.path, left.line).cmp(&(&right.name, &right.path, right.line))
    });
    neighbours
}

fn collect_rust_imports(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    known_paths: &std::collections::BTreeSet<String>,
    imports: &mut Vec<Import>,
) {
    if node.kind() == "use_declaration"
        && let Some(text) = node.utf8_text(source).ok()
        && let Some(target_path) = rust_import_target(text, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    } else if node.kind() == "mod_item"
        && let Some(module) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        && let Some(target_path) = rust_module_target(path, module, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_imports(child, source, path, known_paths, imports);
    }
}

fn rust_import_target(
    declaration: &str,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let module = declaration
        .trim()
        .strip_prefix("use crate::")?
        .split("::")
        .next()?
        .trim();
    [format!("src/{module}.rs"), format!("src/{module}/mod.rs")]
        .into_iter()
        .find(|candidate| known_paths.contains(candidate))
}

fn rust_module_target(
    source_path: &str,
    module: &str,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let directory = source_path
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let prefix = if directory.is_empty() {
        String::new()
    } else {
        format!("{directory}/")
    };
    [
        format!("{prefix}{module}.rs"),
        format!("{prefix}{module}/mod.rs"),
    ]
    .into_iter()
    .find(|candidate| known_paths.contains(candidate))
}

fn collect_ecmascript_imports(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    known_paths: &std::collections::BTreeSet<String>,
    imports: &mut Vec<Import>,
) {
    if node.kind() == "import_statement"
        && let Some(text) = node.utf8_text(source).ok()
        && let Some(module) = text
            .split("from")
            .nth(1)
            .map(str::trim)
            .or_else(|| text.trim().strip_prefix("import").map(str::trim))
        && let Some(module) = module
            .trim_end_matches(';')
            .trim()
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                module
                    .trim_end_matches(';')
                    .trim()
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
        && let Some(target_path) = relative_import_target(path, module, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_ecmascript_imports(child, source, path, known_paths, imports);
    }
}

fn relative_import_target(
    source_path: &str,
    module: &str,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    if !(module.starts_with("./") || module.starts_with("../")) {
        return None;
    }
    let mut components = source_path
        .rsplit_once('/')
        .map_or_else(Vec::new, |(directory, _)| {
            directory.split('/').map(str::to_owned).collect()
        });
    for component in module.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components.pop()?;
            }
            component => components.push(component.into()),
        }
    }
    let candidate = components.join("/");
    let mut candidates = vec![candidate.clone()];
    if !candidate
        .rsplit('/')
        .next()
        .is_some_and(|filename| filename.contains('.'))
    {
        candidates.extend(
            ["js", "mjs", "cjs", "ts", "mts", "cts", "jsx", "tsx"]
                .into_iter()
                .map(|extension| format!("{candidate}.{extension}")),
        );
    }
    let mut matches = candidates
        .into_iter()
        .filter(|candidate| known_paths.contains(candidate));
    let target = matches.next()?;
    matches.next().is_none().then_some(target)
}

fn collect_python_imports(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    known_paths: &std::collections::BTreeSet<String>,
    imports: &mut Vec<Import>,
) {
    if node.kind() == "import_from_statement"
        && let Some(text) = node.utf8_text(source).ok()
        && let Some(module) = text
            .trim()
            .strip_prefix("from ")
            .and_then(|value| value.split(" import ").next())
        && let Some(target_path) = python_import_target(path, module, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_python_imports(child, source, path, known_paths, imports);
    }
}

fn python_import_target(
    source_path: &str,
    module: &str,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let relative_levels = module
        .chars()
        .take_while(|character| *character == '.')
        .count();
    let module = module.strip_prefix(&".".repeat(relative_levels))?;
    if relative_levels == 0 || module.is_empty() {
        return None;
    }
    let mut components = source_path
        .rsplit_once('/')
        .map_or_else(Vec::new, |(directory, _)| {
            directory.split('/').map(str::to_owned).collect()
        });
    for _ in 1..relative_levels {
        components.pop()?;
    }
    components.extend(module.split('.').map(str::to_owned));
    let candidate = format!("{}.py", components.join("/"));
    known_paths.contains(&candidate).then_some(candidate)
}

fn collect_java_imports(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    known_paths: &std::collections::BTreeSet<String>,
    imports: &mut Vec<Import>,
) {
    if node.kind() == "import_declaration"
        && let Some(text) = node.utf8_text(source).ok()
        && let Some(module) = text
            .trim()
            .strip_prefix("import ")
            .map(str::trim)
            .and_then(|value| value.strip_suffix(';'))
            .filter(|value| !value.starts_with("static ") && !value.ends_with(".*"))
        && let Some(target_path) = java_import_target(module, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_java_imports(child, source, path, known_paths, imports);
    }
}

fn java_import_target(
    module: &str,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let candidate = format!("{}.java", module.replace('.', "/"));
    if known_paths.contains(&candidate) {
        return Some(candidate);
    }
    let suffix = format!("/{candidate}");
    let mut matches = known_paths
        .iter()
        .filter(|path| path.ends_with(&suffix))
        .cloned();
    let target = matches.next()?;
    matches.next().is_none().then_some(target)
}

fn collect_go_imports(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    module_name: Option<&str>,
    known_paths: &std::collections::BTreeSet<String>,
    imports: &mut Vec<Import>,
) {
    if node.kind() == "import_spec"
        && let Some(text) = node.utf8_text(source).ok()
        && let Some(module) = text
            .split_whitespace()
            .last()
            .and_then(|value| value.strip_prefix('"'))
            .and_then(|value| value.strip_suffix('"'))
        && let Some(target_path) = go_import_target(module, module_name, known_paths)
    {
        imports.push(Import {
            source_path: path.into(),
            target_path,
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_go_imports(child, source, path, module_name, known_paths, imports);
    }
}

fn go_module_name(root: &Path) -> Option<String> {
    fs::read_to_string(root.join("go.mod"))
        .ok()?
        .lines()
        .map(str::trim)
        .find_map(|line| line.strip_prefix("module ").map(str::trim))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
}

fn go_import_target(
    import_path: &str,
    module_name: Option<&str>,
    known_paths: &std::collections::BTreeSet<String>,
) -> Option<String> {
    let local_directory = import_path.strip_prefix(module_name?)?.trim_matches('/');
    if local_directory.is_empty() {
        return None;
    }
    let prefix = format!("{local_directory}/");
    known_paths
        .iter()
        .any(|path| path.starts_with(&prefix) && path.ends_with(".go"))
        .then(|| local_directory.to_owned())
}

fn rust_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn python_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_python::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn javascript_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_javascript::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn typescript_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
        .ok()?;
    parser.parse(source, None)
}

fn tsx_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_typescript::LANGUAGE_TSX.into())
        .ok()?;
    parser.parse(source, None)
}

fn java_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_java::LANGUAGE.into())
        .ok()?;
    parser.parse(source, None)
}

fn go_tree(source: &str) -> Option<tree_sitter::Tree> {
    let mut parser = Parser::new();
    parser.set_language(&tree_sitter_go::LANGUAGE.into()).ok()?;
    parser.parse(source, None)
}

fn visit_java(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_function: Option<&str>,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<DirectCall>,
) {
    let mut next_function = current_function;
    if node.kind() == "method_declaration" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: SymbolKind::Method,
                line: node.start_position().row + 1,
                container: enclosing_type_name(node, source),
            });
            next_function = Some(name);
        }
    } else if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "enum_declaration"
    ) {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let kind = match node.kind() {
                "class_declaration" => SymbolKind::Class,
                "interface_declaration" => SymbolKind::Interface,
                "enum_declaration" => SymbolKind::Enum,
                _ => unreachable!("Java symbol kind was filtered above"),
            };
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind,
                line: node.start_position().row + 1,
                container: None,
            });
        }
    } else if node.kind() == "method_invocation"
        && let (Some(caller), Some(callee)) = (
            current_function,
            node.child_by_field_name("name")
                .and_then(|node| node.utf8_text(source).ok()),
        )
        && callee
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        calls.push(DirectCall {
            caller: caller.into(),
            callee: callee.into(),
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_java(child, source, path, next_function, symbols, calls);
    }
}

fn visit_go(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_function: Option<&str>,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<DirectCall>,
) {
    let mut next_function = current_function;
    if matches!(node.kind(), "function_declaration" | "method_declaration") {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let kind = if node.kind() == "method_declaration" {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind,
                line: node.start_position().row + 1,
                container: None,
            });
            next_function = Some(name);
        }
    } else if node.kind() == "call_expression"
        && let (Some(caller), Some(callee)) = (
            current_function,
            node.child_by_field_name("function")
                .and_then(|node| node.utf8_text(source).ok()),
        )
        && callee
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        calls.push(DirectCall {
            caller: caller.into(),
            callee: callee.into(),
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_go(child, source, path, next_function, symbols, calls);
    }
}

fn visit_javascript(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_function: Option<&str>,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<DirectCall>,
) {
    let mut next_function = current_function;
    if matches!(node.kind(), "function_declaration" | "method_definition") {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let container = enclosing_type_name(node, source);
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: if node.kind() == "method_definition" {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                },
                line: node.start_position().row + 1,
                container,
            });
            next_function = Some(name);
        }
    } else if matches!(
        node.kind(),
        "class_declaration" | "interface_declaration" | "enum_declaration"
    ) {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let kind = match node.kind() {
                "class_declaration" => SymbolKind::Class,
                "interface_declaration" => SymbolKind::Interface,
                "enum_declaration" => SymbolKind::Enum,
                _ => unreachable!("JavaScript-family symbol kind was filtered above"),
            };
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind,
                line: node.start_position().row + 1,
                container: None,
            });
        }
    } else if node.kind() == "call_expression"
        && let (Some(caller), Some(callee)) = (
            current_function,
            node.child_by_field_name("function")
                .and_then(|node| node.utf8_text(source).ok()),
        )
        && callee
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        calls.push(DirectCall {
            caller: caller.into(),
            callee: callee.into(),
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_javascript(child, source, path, next_function, symbols, calls);
    }
}

fn visit_python(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_function: Option<&str>,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<DirectCall>,
) {
    let mut next_function = current_function;
    if node.kind() == "function_definition" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let container = enclosing_type_name(node, source);
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: if container.is_some() {
                    SymbolKind::Method
                } else {
                    SymbolKind::Function
                },
                line: node.start_position().row + 1,
                container,
            });
            next_function = Some(name);
        }
    } else if node.kind() == "class_definition" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: SymbolKind::Class,
                line: node.start_position().row + 1,
                container: None,
            });
        }
    } else if node.kind() == "call"
        && let (Some(caller), Some(callee)) = (
            current_function,
            node.child_by_field_name("function")
                .and_then(|node| node.utf8_text(source).ok()),
        )
        && callee
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        calls.push(DirectCall {
            caller: caller.into(),
            callee: callee.into(),
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_python(child, source, path, next_function, symbols, calls);
    }
}

fn visit_rust(
    node: Node<'_>,
    source: &[u8],
    path: &str,
    current_function: Option<&str>,
    symbols: &mut Vec<Symbol>,
    calls: &mut Vec<DirectCall>,
) {
    let mut next_function = current_function;
    if node.kind() == "function_item" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let kind = if is_method(node) {
                SymbolKind::Method
            } else {
                SymbolKind::Function
            };
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind,
                line: node.start_position().row + 1,
                container: rust_impl_type_name(node, source),
            });
            next_function = Some(name);
        }
    } else if node.kind() == "struct_item" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: SymbolKind::Struct,
                line: node.start_position().row + 1,
                container: None,
            });
        }
    } else if matches!(node.kind(), "trait_item" | "enum_item" | "mod_item") {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            let kind = match node.kind() {
                "trait_item" => SymbolKind::Trait,
                "enum_item" => SymbolKind::Enum,
                "mod_item" => SymbolKind::Module,
                _ => unreachable!("Rust symbol kind was filtered above"),
            };
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind,
                line: node.start_position().row + 1,
                container: None,
            });
        }
    } else if node.kind() == "call_expression"
        && let (Some(caller), Some(callee)) = (
            current_function,
            node.child_by_field_name("function")
                .and_then(|node| node.utf8_text(source).ok()),
        )
        && callee
            .chars()
            .all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        calls.push(DirectCall {
            caller: caller.into(),
            callee: callee.into(),
            evidence: Evidence {
                path: path.into(),
                line: node.start_position().row + 1,
            },
            confidence: Confidence::Exact,
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_rust(child, source, path, next_function, symbols, calls);
    }
}

fn is_method(node: Node<'_>) -> bool {
    node.parent()
        .and_then(|parent| parent.parent())
        .is_some_and(|parent| parent.kind() == "impl_item")
}

fn enclosing_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if matches!(
            ancestor.kind(),
            "class_declaration" | "interface_declaration" | "enum_declaration" | "class_definition"
        ) {
            return ancestor
                .child_by_field_name("name")
                .and_then(|name| name.utf8_text(source).ok())
                .map(str::to_owned);
        }
        current = ancestor.parent();
    }
    None
}

fn rust_impl_type_name(node: Node<'_>, source: &[u8]) -> Option<String> {
    let mut current = node.parent();
    while let Some(ancestor) = current {
        if ancestor.kind() == "impl_item" {
            return ancestor
                .child_by_field_name("type")
                .and_then(|type_node| type_node.utf8_text(source).ok())
                .map(str::to_owned);
        }
        current = ancestor.parent();
    }
    None
}
