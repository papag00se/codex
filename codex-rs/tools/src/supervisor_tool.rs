use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

pub fn create_supervisor_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "goal".to_string(),
            JsonSchema::string(Some(
                "The engineering goal to accomplish. The supervisor will decompose this into subtasks, route each to the best model, dispatch specialist agents, verify results, and retry failures."
                    .to_string(),
            )),
        ),
        (
            "verification_command".to_string(),
            JsonSchema::string(Some(
                "Optional shell command to verify results (e.g., 'pytest tests/'). If provided, the supervisor runs this after each subtask and retries on failure."
                    .to_string(),
            )),
        ),
    ]);

    ToolSpec::Function(ResponsesApiTool {
        name: "supervisor".to_string(),
        description:
            "Run a supervised multi-agent workflow to accomplish a complex engineering goal. \
             The supervisor decomposes the goal into subtasks, routes each to the best available model, \
             dispatches specialist agents, verifies results, and retries failures automatically. \
             The workflow runs to completion — it does not stop to ask for confirmation. \
             Use this for goals that require multiple files, tests, or sequential steps."
                .to_string(),
        strict: false,
        defer_loading: None,
        parameters: JsonSchema::object(
            properties,
            Some(vec!["goal".to_string()]),
            Some(false.into()),
        ),
        output_schema: None,
    })
}
