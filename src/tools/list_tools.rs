use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::RovenToolDefinition;

const LIST_TOOLS_DESCRIPTION: &str = "List the Roven tools available to you in this turn, with their exact descriptions and input schemas. Use this when you need to check which Roven capabilities are currently available before selecting a tool. This reports the live Roven tool registry and does not access the workspace or modify anything.";

pub(super) fn definition() -> RovenToolDefinition {
    RovenToolDefinition {
        name: "list_tools".to_owned(),
        description: LIST_TOOLS_DESCRIPTION.to_owned(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }
}

pub(super) fn dispatch(arguments: Value) -> serde_json::Result<Value> {
    match serde_json::from_value::<ListToolsInput>(arguments) {
        Ok(_) => serde_json::to_value(ListToolsResult::Ok {
            tools: super::definitions(),
        }),
        Err(_) => serde_json::to_value(ListToolsResult::InvalidInput),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListToolsInput {}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListToolsResult {
    Ok { tools: Vec<RovenToolDefinition> },
    InvalidInput,
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::{Path, PathBuf},
    };

    use serde_json::json;

    use super::super::{RovenToolCall, ToolContext, definitions, dispatch};

    fn temp_root(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("roven-{name}-{}", uuid::Uuid::now_v7()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn context(path: &Path) -> ToolContext {
        ToolContext::new(path.canonicalize().unwrap()).unwrap()
    }
    #[test]
    fn list_tools_returns_the_live_registry_with_descriptions_and_schemas() {
        let workspace = temp_root("list-tools");
        let trusted = context(&workspace);
        let expected = serde_json::to_value(definitions()).unwrap();

        let result = dispatch(
            &trusted,
            RovenToolCall {
                id: "call_tools".to_owned(),
                name: "list_tools".to_owned(),
                arguments: json!({}),
            },
        );

        assert_eq!(
            result.result,
            json!({
                "status": "ok",
                "tools": expected,
            })
        );
        let read_file = result.result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "read_file")
            .expect("read_file must be registered");
        assert_eq!(
            read_file["description"],
            "Read a known workspace-relative text file after locating it with `list_directory`. Paths are relative to the trusted workspace. This tool reads only regular UTF-8 text files up to 50 KiB and does not modify files or access paths outside the trusted workspace."
        );
        assert_eq!(
            read_file["input_schema"],
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative text file path."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })
        );
        let list_directory = result.result["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|tool| tool["name"] == "list_directory")
            .expect("list_directory must be registered");
        assert_eq!(
            list_directory["description"],
            "List the immediate contents of a directory inside the currently trusted Roven workspace. Use this when you need to inspect workspace structure or locate a file or subdirectory before calling another filesystem tool. Pass a workspace-relative directory path such as `.` or `src`; do not pass an absolute path or a path containing `..`. Returns up to 100 immediate entries in deterministic order with `status`, `path`, `workspace_path`, `entries`, and `truncated`; if more entries exist, `truncated` is true. Each entry includes `name`, workspace-relative `path`, and `kind`. Every regular file also includes `size_kb`, measured as bytes divided by 1024 and rounded to two decimal places. Directories and other entries omit size fields. Symlinks are not followed and include `size_error: \"symlink_not_followed\"`; regular-file metadata failures keep the entry and include `size_error: \"permission_denied\"` or \"io_error\". For `invalid_path` or `path_not_allowed`, retry with a relative path under the workspace; for `not_directory`, pass a directory path. This tool does not read file contents, search recursively, modify files, register projects, or access paths outside the trusted workspace."
        );
        assert_eq!(
            list_directory["input_schema"],
            json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Workspace-relative directory path; use `.` for the workspace root."
                    }
                },
                "required": ["path"],
                "additionalProperties": false
            })
        );
        let invalid = dispatch(
            &trusted,
            RovenToolCall {
                id: "call_invalid_tools".to_owned(),
                name: "list_tools".to_owned(),
                arguments: json!({ "unexpected": true }),
            },
        );
        assert_eq!(invalid.result, json!({ "status": "invalid_input" }));
        fs::remove_dir_all(workspace).unwrap();
    }
}
