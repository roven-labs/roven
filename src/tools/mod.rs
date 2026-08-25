//! Roven-owned tool definitions, dispatch, and deterministic tool execution.

use std::{io, path::PathBuf};

use serde::Serialize;
use serde_json::{Value, json};

mod list_directory;
mod list_project;
mod list_tools;
mod prepare_project;
mod read_file;
mod workspace;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolDefinition {
    pub(crate) name: String,
    pub(crate) description: String,
    pub(crate) input_schema: Value,
}

pub(crate) fn definitions() -> Vec<RovenToolDefinition> {
    vec![
        prepare_project::definition(),
        list_directory::definition(),
        read_file::definition(),
        list_tools::definition(),
        list_project::definition(),
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RovenToolCall {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub(crate) struct RovenToolResult {
    pub(crate) tool_call_id: String,
    pub(crate) name: String,
    pub(crate) result: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct ToolContext {
    trusted_workspace: PathBuf,
}

impl ToolContext {
    pub(crate) fn new(trusted_workspace: PathBuf) -> io::Result<Self> {
        let trusted_workspace = trusted_workspace.canonicalize()?;
        Ok(Self { trusted_workspace })
    }
}

pub(crate) fn dispatch(context: &ToolContext, call: RovenToolCall) -> RovenToolResult {
    let result = match call.name.as_str() {
        "prepare_project" => prepare_project::dispatch(context, call.arguments),
        "list_directory" => list_directory::dispatch(context, call.arguments),
        "read_file" => read_file::dispatch(context, call.arguments),
        "list_tools" => list_tools::dispatch(call.arguments),
        "list_project" => list_project::dispatch(call.arguments),
        _ => Ok(json!({ "status": "error", "reason": "unknown_tool" })),
    };
    RovenToolResult {
        tool_call_id: call.id,
        name: call.name,
        result: result.expect("tool results are serializable"),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use serde_json::json;

    use super::{RovenToolCall, ToolContext, dispatch};

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn context(path: &Path) -> ToolContext {
        ToolContext::new(path.canonicalize().unwrap()).unwrap()
    }
    #[test]
    fn dispatch_reads_file_contents_from_the_trusted_workspace() {
        let workspace = temp_root("read-file-dispatch");
        fs::write(workspace.join("notes.txt"), "dispatch contents\n").unwrap();

        let result = dispatch(
            &context(&workspace),
            RovenToolCall {
                id: "call_read_file".to_owned(),
                name: "read_file".to_owned(),
                arguments: json!({ "path": "notes.txt" }),
            },
        );

        assert_eq!(
            result.result,
            json!({
                "status": "ok",
                "path": "notes.txt",
                "content": "dispatch contents\n"
            })
        );
        fs::remove_dir_all(workspace).unwrap();
    }
}
