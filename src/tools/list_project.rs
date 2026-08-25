use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::storage::ProjectRegistry;

use super::RovenToolDefinition;

const LIST_PROJECT_DESCRIPTION: &str = "List the projects currently registered with Roven. Use this when the user asks which stored projects exist. Takes no arguments and returns only project names in deterministic alphabetical order; an empty registry returns an empty projects array. This does not inspect project directories or modify storage.";

pub(super) fn definition() -> RovenToolDefinition {
    RovenToolDefinition {
        name: "list_project".to_owned(),
        description: LIST_PROJECT_DESCRIPTION.to_owned(),
        input_schema: json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    }
}

pub(super) fn dispatch(arguments: Value) -> serde_json::Result<Value> {
    match serde_json::from_value::<ListProjectInput>(arguments) {
        Ok(_) => serde_json::to_value(ListProject::for_current_user().execute()),
        Err(_) => serde_json::to_value(ListProjectResult::InvalidInput),
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListProjectInput {}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum ListProjectResult {
    Ok { projects: Vec<String> },
    Error { reason: ListProjectErrorReason },
    InvalidInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum ListProjectErrorReason {
    StorageFailure,
}

struct ListProject {
    registry: Result<ProjectRegistry, ()>,
}

impl ListProject {
    fn for_current_user() -> Self {
        Self {
            registry: ProjectRegistry::for_current_user().map_err(|_| ()),
        }
    }

    fn execute(&self) -> ListProjectResult {
        let Ok(registry) = &self.registry else {
            return ListProjectResult::Error {
                reason: ListProjectErrorReason::StorageFailure,
            };
        };
        match registry.list() {
            Ok(projects) => ListProjectResult::Ok {
                projects: projects.into_iter().map(|project| project.name).collect(),
            },
            Err(_) => ListProjectResult::Error {
                reason: ListProjectErrorReason::StorageFailure,
            },
        }
    }
}
