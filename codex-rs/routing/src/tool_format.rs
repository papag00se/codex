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

    // Already in Ollama format.
    if obj.get("type").and_then(|t| t.as_str()) == Some("function") && obj.get("function").is_some()
    {
        return Some(spec.clone());
    }

    // OpenAI-builtin variants are identified purely by `type` (no top-level
    // `name`). Synthesize the canonical name so they survive the conversion;
    // otherwise local models can't see them at all.
    let synthesized_name = obj
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::to_string)
        .or_else(|| match obj.get("type").and_then(|t| t.as_str()) {
            Some("local_shell") => Some("local_shell".to_string()),
            Some("web_search") => Some("web_search".to_string()),
            Some("image_generation") => Some("image_generation".to_string()),
            Some("tool_search") => Some("tool_search".to_string()),
            _ => None,
        })?;

    let description = obj
        .get("description")
        .and_then(|d| d.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| default_description_for(&synthesized_name));
    let parameters = obj
        .get("parameters")
        .or_else(|| obj.get("input_schema"))
        .cloned()
        .unwrap_or_else(|| default_parameters_for(&synthesized_name));

    Some(serde_json::json!({
        "type": "function",
        "function": {
            "name": synthesized_name,
            "description": description,
            "parameters": parameters,
        }
    }))
}

/// Description fallback for OpenAI-builtin variants that don't carry one.
fn default_description_for(name: &str) -> String {
    match name {
        "local_shell" => "Execute a shell command on the local machine.".to_string(),
        "web_search" => "Search the web and return relevant results.".to_string(),
        "image_generation" => "Generate an image from a text prompt.".to_string(),
        "tool_search" => "Search for tools relevant to a task.".to_string(),
        _ => String::new(),
    }
}

/// Minimal parameter schema for OpenAI-builtin variants. The Codex registry
/// handles the actual dispatch; we just need *some* schema so the model
/// understands how to call it.
fn default_parameters_for(name: &str) -> JsonValue {
    match name {
        "local_shell" => serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to run."
                }
            },
            "required": ["command"]
        }),
        "web_search" => serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query."
                }
            },
            "required": ["query"]
        }),
        "image_generation" => serde_json::json!({
            "type": "object",
            "properties": {
                "prompt": { "type": "string" }
            },
            "required": ["prompt"]
        }),
        "tool_search" => serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" }
            },
            "required": ["query"]
        }),
        _ => serde_json::json!({"type": "object", "properties": {}}),
    }
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
        assert_eq!(
            result[0]["function"]["parameters"]["properties"]["path"]["type"],
            "string"
        );
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

    #[test]
    fn local_shell_variant_survives_with_synthesized_name() {
        // The OpenAI built-in `local_shell` variant has no `name` field — only
        // `type`. Older code dropped it; the fix synthesizes the name.
        let specs = vec![serde_json::json!({"type": "local_shell"})];
        let result = to_ollama_tools(&specs);
        assert_eq!(result.len(), 1, "local_shell should not be dropped");
        assert_eq!(result[0]["function"]["name"], "local_shell");
        assert!(
            result[0]["function"]["parameters"]["properties"]["command"].is_object(),
            "local_shell should have a command parameter"
        );
    }

    #[test]
    fn web_search_variant_survives_with_synthesized_name() {
        let specs = vec![serde_json::json!({
            "type": "web_search",
            "external_web_access": true,
        })];
        let result = to_ollama_tools(&specs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0]["function"]["name"], "web_search");
    }

    #[test]
    fn image_generation_and_tool_search_variants_survive() {
        let specs = vec![
            serde_json::json!({"type": "image_generation", "output_format": "png"}),
            serde_json::json!({"type": "tool_search", "execution": "x", "description": "y", "parameters": {}}),
        ];
        let result = to_ollama_tools(&specs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0]["function"]["name"], "image_generation");
        assert_eq!(result[1]["function"]["name"], "tool_search");
    }
}
