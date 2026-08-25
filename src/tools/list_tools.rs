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
            super::super::read_file::READ_FILE_DESCRIPTION
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
            super::super::list_directory::LIST_DIRECTORY_DESCRIPTION
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
