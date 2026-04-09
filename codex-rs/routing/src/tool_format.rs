//! ToolSpec-to-Ollama format translation.
//!
//! Converts Codex tool definitions (OpenAI Responses API format) to
//! Ollama's tool format so local models can receive tool definitions.
//!
//! Ported from coding-agent-router/app/tool_adapter.py normalize_ollama_tools().

use serde_json::Value as JsonValue;

/// Convert a list of tool specs (as JSON values) to Ollama tool format.
///
/// Input format (Codex/OpenAI):
/// ```json
/// {"type": "function", "name": "shell", "description": "...", "parameters": {...}}
/// ```
/// or:
/// ```json
/// {"name": "shell", "description": "...", "input_schema": {...}}
/// ```
///
/// Output format (Ollama):
/// ```json
/// {"type": "function", "function": {"name": "shell", "description": "...", "parameters": {...}}}
/// ```
pub fn to_ollama_tools(specs: &[JsonValue]) -> Vec<JsonValue> {
    specs.iter().filter_map(spec_to_ollama_tool).collect()
}

fn spec_to_ollama_tool(spec: &JsonValue) -> Option<JsonValue> {
    let obj = spec.as_object()?;

    // Already in Ollama format
    if obj.get("type").and_then(|t| t.as_str()) == Some("function")
        && obj.get("function").is_some()
    {
        return Some(spec.clone());
    }

    // Codex/OpenAI format: has "name" at top level
    let name = obj.get("name").and_then(|n| n.as_str())?;
    let description = obj
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("");
    let parameters = obj
        .get("parameters")
        .or_else(|| obj.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}}));

    Some(serde_json::json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": parameters,
        }
    }))
}

/// Extract just the tool names from a list of tool specs.
/// Works with both Codex and Ollama formats.
pub fn extract_tool_names(specs: &[JsonValue]) -> Vec<String> {
    specs
        .iter()
        .filter_map(|spec| {
            let obj = spec.as_object()?;
            // Try Ollama format first
            if let Some(func) = obj.get("function").and_then(|f| f.as_object()) {
                return func.get("name").and_then(|n| n.as_str()).map(String::from);
            }
            // Codex format
            obj.get("name").and_then(|n| n.as_str()).map(String::from)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_codex_format_to_ollama() {
        let specs = vec![serde_json::json!({
            "name": "shell",
            "description": "Run a shell command",
            "parameters": {"type": "object", "properties": {"cmd": {"type": "string"}}}
        })];
        let result = to_ollama_tools(&specs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["type"], "function");
        assert_eq!(result[0]["function"]["name"], "shell");
    }

    #[test]
    fn test_ollama_format_passthrough() {
        let specs = vec![serde_json::json!({
            "type": "function",
            "function": {"name": "test", "parameters": {}}
        })];
        let result = to_ollama_tools(&specs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["function"]["name"], "test");
    }

    #[test]
    fn test_input_schema_variant() {
        let specs = vec![serde_json::json!({
            "name": "read_file",
            "description": "Read a file",
            "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
        })];
        let result = to_ollama_tools(&specs);
        assert_eq!(result[0]["function"]["parameters"]["properties"]["path"]["type"], "string");
    }

    #[test]
    fn test_extract_names() {
        let specs = vec![
            serde_json::json!({"name": "shell"}),
            serde_json::json!({"type": "function", "function": {"name": "read_file"}}),
        ];
        let names = extract_tool_names(&specs);
        assert_eq!(names, vec!["shell", "read_file"]);
    }
}
