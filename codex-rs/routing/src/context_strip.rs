//! Context stripping for local models.
//!
//! Strips conversation context to fit within local model context windows.
//! Different stripping levels for reasoner (aggressive) vs coder (moderate).
//!
//! Ported from compaction normalization patterns:
//! - Remove binary blobs (images, base64)
//! - Remove tool responses except the last one
//! - Truncate long tool outputs
//! - Remove reasoning/thinking blocks
//! - Remove encrypted_content
//! - Collapse repeated patterns (poll loops)
//! - Keep only recent conversation turns
//! - Replace system prompt with minimal version

use serde_json::Value as JsonValue;

/// Stripping level — how aggressively to reduce context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StripLevel {
    /// For light coder: keep tools context, recent history, affected files.
    Coder,
    /// For light reasoner: keep only the question and minimal context.
    Reasoner,
}

/// Stripped context ready for a local model.
#[derive(Debug, Clone)]
pub struct StrippedContext {
    /// Minimal system prompt (replaces the full Codex system prompt).
    pub system: Option<String>,
    /// Stripped conversation messages.
    pub messages: Vec<JsonValue>,
    /// Summary of what was stripped (for logging).
    pub strip_summary: String,
}

/// Strip conversation context for a local model.
///
/// Input: the full Ollama messages (from prompt_to_ollama_messages)
/// and an optional system prompt.
///
/// Output: stripped messages and a minimal system prompt.
pub fn strip_context(
    messages: &[JsonValue],
    system: Option<&str>,
    level: StripLevel,
) -> StrippedContext {
    let original_count = messages.len();
    let mut stripped = Vec::new();
    let mut removed_count = 0;
    let mut truncated_count = 0;
    let mut collapsed_polls = 0;

    // Step 1: Determine how many recent turns to keep
    let keep_turns = match level {
        StripLevel::Coder => 6,   // Last 3 exchanges
        StripLevel::Reasoner => 4, // Last 2 exchanges
    };

    // Step 2: Take only recent messages
    let recent: Vec<&JsonValue> = if messages.len() > keep_turns {
        removed_count += messages.len() - keep_turns;
        messages.iter().rev().take(keep_turns).collect::<Vec<_>>().into_iter().rev().collect()
    } else {
        messages.iter().collect()
    };

    // Step 3: Process each message
    let mut last_was_poll = false;
    let mut poll_count = 0;

    for (i, msg) in recent.iter().enumerate() {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

        // Skip empty messages
        if content.is_empty() && !msg.get("tool_calls").is_some_and(|tc| tc.is_array()) {
            removed_count += 1;
            continue;
        }

        // Collapse repeated poll patterns
        if is_poll_message(content) {
            if last_was_poll {
                poll_count += 1;
                collapsed_polls += 1;
                continue;
            }
            last_was_poll = true;
            poll_count = 1;
        } else {
            if last_was_poll && poll_count > 1 {
                stripped.push(serde_json::json!({
                    "role": "assistant",
                    "content": format!("[polled {} times]", poll_count),
                }));
            }
            last_was_poll = false;
            poll_count = 0;
        }

        // Process content
        let processed_content = process_message_content(content, level, &mut truncated_count);

        // Build stripped message
        let mut new_msg = serde_json::json!({
            "role": role,
            "content": processed_content,
        });

        // Keep tool_calls if present and this is the coder level
        if level == StripLevel::Coder {
            if let Some(tool_calls) = msg.get("tool_calls") {
                new_msg["tool_calls"] = tool_calls.clone();
            }
        }

        stripped.push(new_msg);
    }

    // Flush any trailing poll collapse
    if last_was_poll && poll_count > 1 {
        stripped.push(serde_json::json!({
            "role": "assistant",
            "content": format!("[polled {} times]", poll_count),
        }));
    }

    // Step 4: For reasoner, also strip tool responses except the last one
    if level == StripLevel::Reasoner {
        strip_tool_responses(&mut stripped);
    }

    // Step 5: Minimal system prompt
    let minimal_system = match level {
        StripLevel::Coder => {
            system.map(|s| truncate_system_prompt(s, 500))
                .or(Some("You are a coding assistant. Complete the requested task.".into()))
        }
        StripLevel::Reasoner => {
            Some("Answer concisely and directly.".into())
        }
    };

    let strip_summary = format!(
        "stripped: {} messages removed, {} truncated, {} polls collapsed (kept {}/{})",
        removed_count, truncated_count, collapsed_polls,
        stripped.len(), original_count,
    );

    StrippedContext {
        system: minimal_system,
        messages: stripped,
        strip_summary,
    }
}

/// Process a single message's content: remove binary, truncate, clean.
fn process_message_content(content: &str, level: StripLevel, truncated: &mut usize) -> String {
    let mut result = content.to_string();

    // Remove base64 blobs
    result = remove_base64_blobs(&result);

    // Remove <think>...</think> blocks
    result = remove_think_blocks(&result);

    // Remove encrypted_content patterns
    result = remove_encrypted_content(&result);

    // Truncate long content
    let max_len = match level {
        StripLevel::Coder => 4000,
        StripLevel::Reasoner => 2000,
    };
    if result.len() > max_len {
        let truncated_amount = result.len() - max_len;
        result = format!(
            "{}...\n[truncated {} chars]",
            &result[..max_len],
            truncated_amount,
        );
        *truncated += 1;
    }

    result
}

/// Check if a message looks like a poll/ping (PTY session check).
fn is_poll_message(content: &str) -> bool {
    let lower = content.trim().to_lowercase();
    lower.contains("poll") && lower.len() < 50
        || lower == "."
        || lower == ""
        || lower.starts_with("[poll")
}

/// Remove tool responses except the last one.
fn strip_tool_responses(messages: &mut Vec<JsonValue>) {
    // Find the last tool response
    let last_tool_idx = messages.iter().enumerate().rev()
        .find(|(_, m)| {
            m.get("role").and_then(|r| r.as_str()) == Some("tool")
        })
        .map(|(i, _)| i);

    // Remove all tool responses except the last
    if let Some(last_idx) = last_tool_idx {
        let mut i = 0;
        messages.retain(|m| {
            let is_tool = m.get("role").and_then(|r| r.as_str()) == Some("tool");
            let keep = !is_tool || i == last_idx;
            i += 1;
            keep
        });
    }
}

/// Remove base64-encoded data from content.
fn remove_base64_blobs(content: &str) -> String {
    // Simple heuristic: lines that are >100 chars of only base64 characters
    content.lines()
        .map(|line| {
            if line.len() > 100 && line.chars().all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=') {
                "[binary data removed]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Remove <think>...</think> blocks.
fn remove_think_blocks(content: &str) -> String {
    let mut result = content.to_string();
    while let Some(start) = result.find("<think>") {
        if let Some(end) = result.find("</think>") {
            result = format!("{}{}", &result[..start], &result[end + 8..]);
        } else {
            result = result[..start].to_string();
            break;
        }
    }
    result
}

/// Remove encrypted_content JSON fields.
fn remove_encrypted_content(content: &str) -> String {
    // Simple: remove lines containing "encrypted_content"
    content.lines()
        .filter(|line| !line.contains("encrypted_content"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Truncate system prompt to max chars, keeping the first part.
fn truncate_system_prompt(prompt: &str, max_len: usize) -> String {
    if prompt.len() <= max_len {
        prompt.to_string()
    } else {
        format!("{}...", &prompt[..max_len])
    }
}

/// Generate a summary of affected files from conversation messages.
pub fn summarize_affected_files(messages: &[JsonValue]) -> Vec<String> {
    let mut files = std::collections::HashSet::new();

    for msg in messages {
        let content = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");

        // Look for file paths in common patterns
        for line in content.lines() {
            let trimmed = line.trim();
            // File paths in diffs
            if trimmed.starts_with("+++ ") || trimmed.starts_with("--- ") {
                let path = trimmed[4..].trim().trim_start_matches("a/").trim_start_matches("b/");
                if !path.is_empty() && path != "/dev/null" {
                    files.insert(path.to_string());
                }
            }
            // File paths in common tool output
            if trimmed.starts_with("M ") || trimmed.starts_with("A ") || trimmed.starts_with("D ") {
                let path = trimmed[2..].trim();
                if path.contains('.') {
                    files.insert(path.to_string());
                }
            }
        }
    }

    let mut sorted: Vec<String> = files.into_iter().collect();
    sorted.sort();
    sorted
}

/// Generate a summary of tool calls from conversation messages.
pub fn summarize_tool_calls(messages: &[JsonValue]) -> String {
    let mut shell_count = 0;
    let mut edit_count = 0;
    let mut read_count = 0;
    let mut other_count = 0;

    for msg in messages {
        if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
            for tc in tool_calls {
                let name = tc.get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                match name {
                    "shell" | "local_shell" | "exec_command" => shell_count += 1,
                    "apply_patch" | "text_editor" | "file_edit" => edit_count += 1,
                    "read_file" | "list_dir" => read_count += 1,
                    _ => other_count += 1,
                }
            }
        }
    }

    let mut parts = Vec::new();
    if shell_count > 0 { parts.push(format!("{shell_count} shell commands")); }
    if edit_count > 0 { parts.push(format!("{edit_count} file edits")); }
    if read_count > 0 { parts.push(format!("{read_count} file reads")); }
    if other_count > 0 { parts.push(format!("{other_count} other tool calls")); }

    if parts.is_empty() {
        "no tool calls".into()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &str) -> JsonValue {
        serde_json::json!({"role": role, "content": content})
    }

    #[test]
    fn test_reasoner_strips_aggressively() {
        let messages = vec![
            msg("user", "What is auth?"),
            msg("assistant", "Auth is authentication"),
            msg("user", "And authorization?"),
            msg("assistant", "Authorization is access control"),
            msg("user", "What about OAuth?"),
            msg("assistant", "OAuth is a protocol"),
            msg("user", "Explain JWT"),
        ];
        let result = strip_context(&messages, None, StripLevel::Reasoner);
        // Should keep only last 4 messages
        assert!(result.messages.len() <= 4);
        assert_eq!(result.system.as_deref(), Some("Answer concisely and directly."));
    }

    #[test]
    fn test_coder_keeps_more() {
        let messages = vec![
            msg("user", "Fix bug"),
            msg("assistant", "Looking at the code"),
            msg("user", "It's in auth.py"),
            msg("assistant", "Found it"),
            msg("user", "Fix it"),
            msg("assistant", "Done"),
            msg("user", "Test it"),
        ];
        let result = strip_context(&messages, None, StripLevel::Coder);
        assert!(result.messages.len() <= 6);
    }

    #[test]
    fn test_base64_removed() {
        let b64 = "A".repeat(200);
        let content = format!("Some text\n{b64}\nMore text");
        let cleaned = remove_base64_blobs(&content);
        assert!(cleaned.contains("[binary data removed]"));
        assert!(cleaned.contains("Some text"));
    }

    #[test]
    fn test_think_blocks_removed() {
        let content = "Before <think>internal reasoning here</think> After";
        let cleaned = remove_think_blocks(content);
        assert_eq!(cleaned.trim(), "Before  After");
    }

    #[test]
    fn test_poll_collapse() {
        let messages = vec![
            msg("user", "Run the test"),
            msg("assistant", "[poll]"),
            msg("assistant", "[poll]"),
            msg("assistant", "[poll]"),
            msg("assistant", "[poll]"),
            msg("assistant", "Test complete: 5 passed"),
        ];
        let result = strip_context(&messages, None, StripLevel::Coder);
        // 4 polls should collapse to 1 summary
        let poll_msgs: Vec<_> = result.messages.iter()
            .filter(|m| {
                m.get("content").and_then(|c| c.as_str())
                    .map(|s| s.contains("polled"))
                    .unwrap_or(false)
            })
            .collect();
        assert_eq!(poll_msgs.len(), 1);
    }

    #[test]
    fn test_truncate_long_content() {
        // Use realistic content (not pure alphanumeric, which triggers base64 removal)
        let long = "This is a sentence that repeats. ".repeat(300);
        let messages = vec![msg("user", &long)];
        let result = strip_context(&messages, None, StripLevel::Reasoner);
        let content = result.messages[0].get("content").unwrap().as_str().unwrap();
        assert!(content.len() < 3000, "Content too long: {} chars", content.len());
        assert!(content.contains("[truncated"), "Missing truncation marker");
    }

    #[test]
    fn test_summarize_tool_calls() {
        let messages = vec![
            serde_json::json!({
                "role": "assistant",
                "tool_calls": [
                    {"function": {"name": "shell", "arguments": {}}},
                    {"function": {"name": "shell", "arguments": {}}},
                    {"function": {"name": "apply_patch", "arguments": {}}},
                    {"function": {"name": "read_file", "arguments": {}}},
                ]
            }),
        ];
        let summary = summarize_tool_calls(&messages);
        assert!(summary.contains("2 shell commands"));
        assert!(summary.contains("1 file edit"));
        assert!(summary.contains("1 file read"));
    }

    #[test]
    fn test_summarize_affected_files() {
        let messages = vec![
            msg("assistant", "+++ b/src/auth.py\n--- a/src/auth.py\nM tests/test_auth.py"),
        ];
        let files = summarize_affected_files(&messages);
        assert!(files.contains(&"src/auth.py".to_string()));
    }
}
