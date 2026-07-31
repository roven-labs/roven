//! Compact, evidence-oriented structural code map.

use std::{collections::BTreeMap, fs, path::Path};

use thiserror::Error;
use tree_sitter::{Node, Parser};

use crate::inventory::{InventoryError, Language, inventory};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub path: String,
    pub name: String,
    pub kind: SymbolKind,
    pub line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectCall {
    pub caller: String,
    pub callee: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeMap {
    pub symbols: Vec<Symbol>,
    pub calls: Vec<DirectCall>,
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
    let mut map = CodeMap::default();
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
        let Ok(source) = fs::read_to_string(root.join(&file.path)) else {
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
    }

    let counts = map
        .symbols
        .iter()
        .fold(BTreeMap::new(), |mut counts, symbol| {
            *counts.entry(symbol.name.as_str()).or_insert(0_usize) += 1;
            counts
        });
    map.calls = candidate_calls
        .into_iter()
        .filter(|call| counts.get(call.callee.as_str()) == Some(&1))
        .collect();
    map.symbols.sort_by(|left, right| {
        (&left.path, left.line, &left.name).cmp(&(&right.path, right.line, &right.name))
    });
    map.calls
        .sort_by(|left, right| (&left.caller, &left.callee).cmp(&(&right.caller, &right.callee)));
    map.unsupported_paths.sort();
    Ok(map)
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
            });
            next_function = Some(name);
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
    if node.kind() == "function_declaration" {
        if let Some(name) = node
            .child_by_field_name("name")
            .and_then(|node| node.utf8_text(source).ok())
        {
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: SymbolKind::Function,
                line: node.start_position().row + 1,
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
            symbols.push(Symbol {
                path: path.into(),
                name: name.into(),
                kind: SymbolKind::Function,
                line: node.start_position().row + 1,
            });
            next_function = Some(name);
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
