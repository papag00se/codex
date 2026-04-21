//! Chunk extraction via compactor LLM — ported from compaction/extractor.py.
//! Calls local Ollama model to extract durable state from a transcript chunk.

use super::models::{ChunkExtraction, TranscriptChunk};
use crate::config::OllamaEndpoint;
use crate::ollama::OllamaClientPool;
use std::collections::HashMap;
use tracing::{info, warn};

/// The extraction system prompt — verbatim from compaction_extraction_system.md.
const EXTRACTION_SYSTEM_PROMPT: &str = "\
You are extracting durable coding-session state for a later Codex handoff.\n\
Return exactly one JSON object and nothing else.\n\
Do not use markdown fences. Do not explain your answer.\n\
\n\
This extraction is chunk-local:\n\
- use only facts present in this chunk\n\
- prefer newer facts over older facts inside the chunk\n\
- if unsure, omit the fact instead of guessing\n\
- empty strings, empty arrays, and empty objects are valid\n\
\n\
Input notes:\n\
- chunk.events is an ordered compact event stream\n\
- event keys: r (role: u=user, a=assistant), k (kind: msg), c (content)\n\
- chronology matters; use event order\n\
\n\
Field rules:\n\
- objective: latest stable task objective visible in the chunk\n\
- repo_state: concrete repo facts only, as {\"key\":\"...\",\"value\":\"...\"} entries\n\
- files_touched: real file paths mentioned or acted on\n\
- commands_run: shell commands actually run\n\
- errors: concrete failures\n\
- accepted_fixes: fixes already applied\n\
- rejected_ideas: ideas explicitly rejected\n\
- constraints: instructions constraining future work\n\
- environment_assumptions: infrastructure assumptions\n\
- pending_todos: remaining concrete tasks\n\
- unresolved_bugs: still-open bugs\n\
- test_status: concrete test outcomes\n\
- external_references: endpoints, services, docs referenced\n\
- latest_plan: most recent active plan steps, otherwise []";

/// Extract durable state from a transcript chunk using the compactor LLM.
pub async fn extract_chunk(
    chunk: &TranscriptChunk,
    pool: &OllamaClientPool,
    endpoint: &OllamaEndpoint,
    repo_context: Option<&serde_json::Value>,
) -> Result<ChunkExtraction, String> {
    // Build the compact event stream from chunk items
    let events = compact_events(&chunk.items);

    let payload = serde_json::json!({
        "task": "Extract chunk-local durable coding-session state.",
        "output_contract": {
            "format": "json_object_only",
        },
        "chunk": {
            "id": chunk.chunk_id,
            "start": chunk.start_index,
            "end": chunk.end_index,
            "tok": chunk.token_count,
            "ov": chunk.overlap_from_previous_tokens,
            "events": events,
        },
        "repo_context": repo_context.unwrap_or(&serde_json::json!({})),
    });

    let payload_str = serde_json::to_string(&payload)
        .map_err(|e| format!("Failed to serialize extraction payload: {e}"))?;

    info!(
        chunk_id = chunk.chunk_id,
        items = chunk.items.len(),
        "Extracting chunk state"
    );

    let response = pool
        .chat(
            &endpoint.base_url,
            &endpoint.model,
            vec![serde_json::json!({"role": "user", "content": payload_str})],
            Some(EXTRACTION_SYSTEM_PROMPT),
            0.0,
            endpoint.num_ctx,
            Some("json"),
            endpoint.timeout_seconds,
        )
        .await;

    let Some(body) = response else {
        return Err("Compactor LLM unreachable".into());
    };

    let content = body
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");

    // Strip think tags
    let content = crate::classifier::strip_think_tags(content);

    parse_extraction(&content, chunk.chunk_id, chunk.token_count)
}

/// Parse the LLM's JSON response into a ChunkExtraction.
fn parse_extraction(
    content: &str,
    chunk_id: usize,
    source_tokens: usize,
) -> Result<ChunkExtraction, String> {
    let parsed: serde_json::Value = serde_json::from_str(content.trim())
        .map_err(|e| format!("Failed to parse extraction JSON: {e}"))?;

    let obj = parsed
        .as_object()
        .ok_or("Extraction response is not a JSON object")?;

    Ok(ChunkExtraction {
        chunk_id,
        objective: obj
            .get("objective")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .into(),
        repo_state: parse_repo_state(obj.get("repo_state")),
        files_touched: parse_string_list(obj.get("files_touched")),
        commands_run: parse_string_list(obj.get("commands_run")),
        errors: parse_string_list(obj.get("errors")),
        accepted_fixes: parse_string_list(obj.get("accepted_fixes")),
        rejected_ideas: parse_string_list(obj.get("rejected_ideas")),
        constraints: parse_string_list(obj.get("constraints")),
        environment_assumptions: parse_string_list(obj.get("environment_assumptions")),
        pending_todos: parse_string_list(obj.get("pending_todos")),
        unresolved_bugs: parse_string_list(obj.get("unresolved_bugs")),
        test_status: parse_string_list(obj.get("test_status")),
        external_references: parse_string_list(obj.get("external_references")),
        latest_plan: parse_string_list(obj.get("latest_plan")),
        source_token_count: source_tokens,
    })
}

/// Parse repo_state — handles both dict and array-of-entries formats.
fn parse_repo_state(value: Option<&serde_json::Value>) -> HashMap<String, String> {
    let Some(value) = value else {
        return HashMap::new();
    };

    if let Some(obj) = value.as_object() {
        return obj
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").into()))
            .collect();
    }

    // Array of {key, value} entries (Ollama structured output format)
    if let Some(arr) = value.as_array() {
        let mut map = HashMap::new();
        for entry in arr {
            if let (Some(k), Some(v)) = (
                entry.get("key").and_then(|k| k.as_str()),
                entry.get("value").and_then(|v| v.as_str()),
            ) {
                if !k.is_empty() {
                    map.insert(k.into(), v.into());
                }
            }
        }
        return map;
    }

    HashMap::new()
}

/// Parse a JSON value into a Vec<String>.
fn parse_string_list(value: Option<&serde_json::Value>) -> Vec<String> {
    let Some(arr) = value.and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(String::from))
        .filter(|s| !s.is_empty())
        .collect()
}

/// Convert transcript items into a compact event stream for the extractor.
fn compact_events(items: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let mut events = Vec::new();

    for item in items {
        let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("?");
        let compact_role = match role {
            "user" => "u",
            "assistant" => "a",
            other if !other.is_empty() => &other[..1],
            _ => "?",
        };

        let content = item.get("content").and_then(|c| c.as_str()).unwrap_or("");
        if !content.is_empty() {
            events.push(serde_json::json!({
                "r": compact_role,
                "k": "msg",
                "c": content,
            }));
        }
    }

    events
}
