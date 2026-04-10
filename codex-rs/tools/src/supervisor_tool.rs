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
            "You SHOULD use this tool when the task can benefit from supervised multi-step execution rather than a single direct response. \
            Prefer it for: \
            - multi-file or multi-component changes \
            - implementation plus verification with tests, builds, or checks \
            - tasks that require sequential subtasks \
            - retry-until-successful repair or work loops \
            - work that benefits from specialist agent delegation \
            The supervisor decomposes the goal, assigns subtasks, verifies results, and automatically retries failures until completion or a concrete blocker is reached. \
            Do not use for single-file edits, simple questions, small isolated changes, or review-only tasks."
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
