use crate::JsonSchema;
use crate::ResponsesApiTool;
use crate::ToolSpec;
use std::collections::BTreeMap;

pub fn create_supervisor_tool() -> ToolSpec {
    let properties = BTreeMap::from([
        (
            "goal".to_string(),
            JsonSchema::string(Some(
                "The complete engineering goal. Be specific: include file names, test commands, expected behavior. \
                 The supervisor decomposes this into subtasks and dispatches specialist agents. \
                 Example: 'Create src/auth.py with login/logout endpoints, add tests in tests/test_auth.py, verify with pytest tests/test_auth.py'."
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
            "You SHOULD use this tool when the user's goal involves multiple files, \
             multiple sequential steps, running tests, or any task that benefits from \
             automatic retry on failure. \
             The supervisor decomposes the goal into subtasks, routes each to the best available model, \
             dispatches specialist agents, verifies results, and retries failures automatically. \
             The workflow runs to completion without stopping for confirmation. \
             Examples of when to use: 'implement feature X with tests', 'refactor module Y across files', \
             'fix bug Z and verify with test suite', 'create new service with API + tests + docs'. \
             Do NOT use for single-file edits, simple questions, or code review."
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
