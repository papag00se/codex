//! Claude CLI subprocess dispatch for Anthropic model routing.
//!
//! When the classifier routes to an Anthropic model (e.g., sonnet-4.6),
//! we invoke the `claude` CLI in print mode rather than sending the slug
//! to the OpenAI Responses API (which doesn't know Anthropic models).
//!
//! Supports session resumption: tracks a session ID per conversation
//! so follow-up turns continue the same Claude conversation.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Mutex;
use tokio::process::Command;
use tracing::{info, warn};

/// Tracks Claude CLI session IDs for context resumption.
#[derive(Debug, Default)]
pub struct ClaudeSessionTracker {
    /// Maps a conversation key to a Claude session ID.
    sessions: Mutex<HashMap<String, String>>,
}

impl ClaudeSessionTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Get a stored session ID for a conversation key.
    pub fn get_session(&self, key: &str) -> Option<String> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    /// Store a session ID for a conversation key.
    pub fn set_session(&self, key: &str, session_id: &str) {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key.to_string(), session_id.to_string());
    }
}

/// Result from a Claude CLI invocation.
#[derive(Debug)]
pub struct ClaudeCliResult {
    pub content: String,
    pub model: String,
    pub session_id: Option<String>,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// Response JSON from `claude -p --output-format json`.
#[derive(Debug, Deserialize)]
struct ClaudeJsonResponse {
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    // Fallback: some versions use "content" instead of "result"
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u64,
    #[serde(default)]
    output_tokens: u64,
}

/// Invoke the Claude CLI in print mode.
///
/// `claude_binary`: path to the `claude` binary (default: "claude")
/// `model`: the Anthropic model to use (e.g., "sonnet-4.6")
/// `prompt`: the user's request
/// `resume_session_id`: if set, resumes this Claude conversation
/// `cwd`: working directory for the subprocess
pub async fn invoke_claude(
    claude_binary: &str,
    model: &str,
    prompt: &str,
    resume_session_id: Option<&str>,
    cwd: Option<&std::path::Path>,
) -> Result<ClaudeCliResult, String> {
    let mut cmd = Command::new(claude_binary);

    // Print mode: non-interactive, outputs result
    cmd.arg("-p").arg(prompt);

    // Model selection
    cmd.arg("--model").arg(model);

    // JSON output for structured parsing
    cmd.arg("--output-format").arg("json");

    // Resume previous session if available
    if let Some(sid) = resume_session_id {
        cmd.arg("--resume").arg(sid);
    }

    // Working directory
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    // Don't inherit stdin
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    info!(
        binary = claude_binary,
        model = model,
        resume = resume_session_id.is_some(),
        "Invoking Claude CLI"
    );

    let output = cmd.output().await.map_err(|e| {
        format!("Failed to spawn claude CLI ({claude_binary}): {e}")
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        warn!(
            exit_code = output.status.code(),
            stderr = %stderr,
            "Claude CLI failed"
        );
        return Err(format!(
            "Claude CLI exited with {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.chars().take(500).collect::<String>(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Try parsing as JSON first
    if let Ok(resp) = serde_json::from_str::<ClaudeJsonResponse>(&stdout) {
        let content = resp.result
            .or(resp.content)
            .unwrap_or_default();
        let (input_tokens, output_tokens) = resp.usage
            .map(|u| (u.input_tokens, u.output_tokens))
            .unwrap_or((0, 0));

        info!(
            content_len = content.len(),
            session_id = resp.session_id.as_deref().unwrap_or("none"),
            input_tokens,
            output_tokens,
            "Claude CLI response received"
        );

        return Ok(ClaudeCliResult {
            content,
            model: resp.model.unwrap_or_else(|| model.to_string()),
            session_id: resp.session_id,
            input_tokens,
            output_tokens,
        });
    }

    // Fallback: treat entire stdout as plain text response
    let content = stdout.trim().to_string();
    if content.is_empty() {
        return Err("Claude CLI returned empty response".into());
    }

    Ok(ClaudeCliResult {
        content,
        model: model.to_string(),
        session_id: None,
        input_tokens: 0,
        output_tokens: 0,
    })
}
