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
