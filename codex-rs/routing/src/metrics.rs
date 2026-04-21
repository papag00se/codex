//! Task metrics extraction — ported from coding-agent-router/app/task_metrics.py.
//!
//! See docs/spec/routing-logic-reference.md for the full specification.
//! Every regex pattern and metric definition here must match the Python reference.

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

/// Quick token estimate: ~4 characters per token.
/// Matches Python: `max(1, (len(text) + 3) // 4)`
pub fn estimate_tokens(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    (text.len() + 3) / 4
}

/// All 27 task metrics extracted from a request.
/// Field names and semantics match the Python reference exactly.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskMetrics {
    pub user_prompt_chars: usize,
    pub user_prompt_lines: usize,
    pub user_prompt_tokens: usize,
    pub trajectory_chars: usize,
    pub trajectory_lines: usize,
    pub trajectory_tokens: usize,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub tool_message_count: usize,
    pub tool_call_count: usize,
    pub command_count: usize,
    pub command_output_tokens: usize,
    pub file_reference_count: usize,
    pub unique_file_reference_count: usize,
    pub code_block_count: usize,
    pub json_block_count: usize,
    pub diff_line_count: usize,
    pub error_line_count: usize,
    pub stack_trace_count: usize,
    pub prior_failure_count: usize,
    pub question_count: usize,
    pub metadata_key_count: usize,
}

// File extensions recognized for file reference detection.
// Matches Python FILE_EXTENSIONS tuple exactly.
const FILE_EXTENSIONS: &str = "py|js|ts|tsx|jsx|md|yml|yaml|json|toml|go|java|rb|php|rs|cpp|c|h|sql|sh|bash|html|css|scss|vue|svelte|kt|kts";

// Failure statuses that count toward prior_failure_count.
const FAILURE_STATUSES: &[&str] = &[
    "error",
    "failed",
    "failure",
    "timeout",
    "timed_out",
    "cancelled",
    "canceled",
    "low_confidence",
    "malformed_output",
];

static FILE_REFERENCE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // The Python reference uses (?<!\w) look-behind, which Rust regex doesn't support.
    // We use (?:^|[\s`(]) as a start boundary instead — matches start of line, whitespace,
    // backtick, or open paren. The named group `path` captures just the file path.
    let pattern = format!(
        r"(?i)(?:^|[\s`(])(?P<path>(?:[A-Za-z0-9._/\-]+/)*[A-Za-z0-9._-]+\.(?:{FILE_EXTENSIONS})(?:[:#]\d+)?)"
    );
    Regex::new(&pattern).expect("file reference regex")
});

static ERROR_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?im)^.*(?:error|exception|failed|failure|traceback|panic|fatal).*$")
        .expect("error line regex")
});

static STACK_TRACE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?im)(?:traceback \(most recent call last\)|\bat [^\n]+:\d+|file "[^"\n]+", line \d+)"#,
    )
    .expect("stack trace regex")
});

static JSON_BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)```(?:json|javascript)\b").expect("json block regex"));

static DIFF_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Python uses negative lookahead (?!\+\+\+|---) which Rust regex doesn't support.
    // Instead we match lines starting with +/- followed by a non-+/- character.
    // This excludes +++ and --- headers while catching +added and -removed lines.
    Regex::new(r"(?m)^[+][^+].*$|^[-][^-].*$").expect("diff line regex")
});

static FENCED_BLOCK_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"```").expect("fenced block regex"));

static TOOL_CALL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?:tool_call|tool_calls|function_call|recipient_name)")
        .expect("tool call regex")
});

static COMMAND_LINE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?m)^(?:\$ |(?:bash|sh|zsh|fish|python|python3|node|npm|pnpm|yarn|uv|pytest|git|rg|sed|cat|ls|curl|ollama)\b)",
    )
    .expect("command line regex")
});

static QUESTION_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\?").expect("question regex"));

fn line_count(text: &str) -> usize {
    if text.is_empty() {
        return 0;
    }
    text.chars().filter(|&c| c == '\n').count() + 1
}

/// Extract all 27 task metrics from a user prompt and trajectory.
///
/// `trajectory_json` is the JSON-serialized trajectory (prior messages).
/// `metadata_key_count` is the number of keys in the request metadata dict.
pub fn extract_task_metrics(
    user_prompt: &str,
    trajectory_json: &str,
    metadata_key_count: usize,
) -> TaskMetrics {
    let combined = if trajectory_json.is_empty() {
        user_prompt.to_string()
    } else {
        format!("{user_prompt}\n\n{trajectory_json}")
    };

    // File references
    let file_refs: Vec<String> = FILE_REFERENCE_RE
        .captures_iter(&combined)
        .filter_map(|c| c.name("path").map(|m| m.as_str().to_string()))
        .collect();
    let unique_file_refs: HashSet<String> = file_refs.iter().map(|p| p.to_lowercase()).collect();

    // Message counting from trajectory JSON
    let (message_count, user_msg, assistant_msg, tool_msg) = count_messages(trajectory_json);

    // Prior failures
    let prior_failure_count = count_prior_failures(trajectory_json);

    // Command output tokens
    let command_output_tokens = estimate_tokens(&extract_command_output(trajectory_json));

    TaskMetrics {
        user_prompt_chars: user_prompt.len(),
        user_prompt_lines: line_count(user_prompt),
        user_prompt_tokens: estimate_tokens(user_prompt),
        trajectory_chars: trajectory_json.len(),
        trajectory_lines: line_count(trajectory_json),
        trajectory_tokens: estimate_tokens(trajectory_json),
        message_count,
        user_message_count: user_msg,
        assistant_message_count: assistant_msg,
        tool_message_count: tool_msg,
        tool_call_count: TOOL_CALL_RE.find_iter(&combined).count(),
        command_count: COMMAND_LINE_RE.find_iter(&combined).count(),
        command_output_tokens,
        file_reference_count: file_refs.len(),
        unique_file_reference_count: unique_file_refs.len(),
        code_block_count: FENCED_BLOCK_RE.find_iter(&combined).count() / 2,
        json_block_count: JSON_BLOCK_RE.find_iter(&combined).count(),
        diff_line_count: DIFF_LINE_RE.find_iter(&combined).count(),
        error_line_count: ERROR_LINE_RE.find_iter(&combined).count(),
        stack_trace_count: STACK_TRACE_RE.find_iter(&combined).count(),
        prior_failure_count,
        question_count: QUESTION_RE.find_iter(user_prompt).count(),
        metadata_key_count,
    }
}

/// Count messages by role in the trajectory JSON.
/// Returns (total, user, assistant, tool).
fn count_messages(trajectory_json: &str) -> (usize, usize, usize, usize) {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trajectory_json) else {
        return (0, 0, 0, 0);
    };

    let items = match &value {
        serde_json::Value::Array(arr) => arr.as_slice(),
        serde_json::Value::Object(obj) => {
            if let Some(serde_json::Value::Array(msgs)) = obj.get("messages") {
                msgs.as_slice()
            } else {
                return (0, 0, 0, 0);
            }
        }
        _ => return (0, 0, 0, 0),
    };

    let mut total = 0;
    let mut user = 0;
    let mut assistant = 0;
    let mut tool = 0;

    for item in items {
        if let Some(role) = item.get("role").and_then(|r| r.as_str()) {
            total += 1;
            match role {
                "user" => user += 1,
                "assistant" => assistant += 1,
                "tool" | "function" => tool += 1,
                _ => {}
            }
        }
    }

    (total, user, assistant, tool)
}

/// Count prior failures from trajectory attempts.
fn count_prior_failures(trajectory_json: &str) -> usize {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trajectory_json) else {
        return 0;
    };

    let Some(attempts) = value.get("attempts").and_then(|a| a.as_array()) else {
        return 0;
    };

    attempts
        .iter()
        .filter(|item| {
            item.get("status")
                .and_then(|s| s.as_str())
                .map(|s| {
                    let lower = s.trim().to_lowercase();
                    FAILURE_STATUSES.contains(&lower.as_str())
                })
                .unwrap_or(false)
        })
        .count()
}

/// Extract command output text from trajectory for token estimation.
fn extract_command_output(trajectory_json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(trajectory_json) else {
        return String::new();
    };

    let mut parts = Vec::new();
    collect_command_output(&value, &mut parts);
    parts.join("\n")
}

fn collect_command_output(value: &serde_json::Value, parts: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(obj) => {
            for key in &["stdout", "stderr", "output", "result"] {
                if let Some(serde_json::Value::String(s)) = obj.get(*key) {
                    if !s.is_empty() {
                        parts.push(s.clone());
                    }
                }
            }
            for child in obj.values() {
                collect_command_output(child, parts);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr {
                collect_command_output(child, parts);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_estimate_tokens_empty() {
        assert_eq!(estimate_tokens(""), 0);
    }

    #[test]
    fn test_estimate_tokens_hello() {
        // "hello" = 5 chars → (5+3)/4 = 2
        assert_eq!(estimate_tokens("hello"), 2);
    }

    #[test]
    fn test_estimate_tokens_100_chars() {
        let text = "a".repeat(100);
        // (100+3)/4 = 25
        assert_eq!(estimate_tokens(&text), 25);
    }

    #[test]
    fn test_basic_metrics() {
        let metrics = extract_task_metrics("Fix the bug in auth.py", "", 0);
        assert_eq!(metrics.user_prompt_chars, 22);
        assert_eq!(metrics.user_prompt_lines, 1);
        assert!(metrics.user_prompt_tokens > 0);
        assert!(metrics.file_reference_count >= 1); // auth.py
    }

    #[test]
    fn test_file_references() {
        let metrics = extract_task_metrics("Update src/auth.py and tests/test_auth.py", "", 0);
        assert!(metrics.file_reference_count >= 2);
        assert!(metrics.unique_file_reference_count >= 2);
    }

    #[test]
    fn test_command_detection() {
        let metrics = extract_task_metrics("$ npm install\n$ pytest tests/", "", 0);
        assert!(metrics.command_count >= 2);
    }

    #[test]
    fn test_question_count() {
        let metrics = extract_task_metrics("What is wrong? Can you fix it?", "", 0);
        assert_eq!(metrics.question_count, 2);
    }

    #[test]
    fn test_error_detection() {
        let trajectory = r#"[{"role": "assistant", "content": "Traceback (most recent call last):\n  File 'test.py'"}]"#;
        let metrics = extract_task_metrics("There was an error in the build", trajectory, 0);
        assert!(metrics.error_line_count >= 1);
        assert!(metrics.stack_trace_count >= 1);
    }

    #[test]
    fn test_prior_failure_count() {
        let trajectory =
            r#"{"attempts": [{"status": "error"}, {"status": "success"}, {"status": "timeout"}]}"#;
        let metrics = extract_task_metrics("retry", trajectory, 0);
        assert_eq!(metrics.prior_failure_count, 2);
    }

    #[test]
    fn test_message_counting() {
        let trajectory = r#"[
            {"role": "user", "content": "hello"},
            {"role": "assistant", "content": "hi"},
            {"role": "user", "content": "thanks"},
            {"role": "tool", "content": "result"}
        ]"#;
        let metrics = extract_task_metrics("current", trajectory, 3);
        assert_eq!(metrics.message_count, 4);
        assert_eq!(metrics.user_message_count, 2);
        assert_eq!(metrics.assistant_message_count, 1);
        assert_eq!(metrics.tool_message_count, 1);
        assert_eq!(metrics.metadata_key_count, 3);
    }

    #[test]
    fn test_diff_lines() {
        let metrics = extract_task_metrics(
            "+added line\n-removed line\n unchanged\n+another add",
            "",
            0,
        );
        assert_eq!(metrics.diff_line_count, 3);
    }

    #[test]
    fn test_empty_prompt() {
        let metrics = extract_task_metrics("", "", 0);
        assert_eq!(metrics.user_prompt_chars, 0);
        assert_eq!(metrics.user_prompt_tokens, 0);
        assert_eq!(metrics.question_count, 0);
    }
}
