//! Deterministic state extractor.
//!
//! Walks the parsed transcript and synthesizes the four state blocks the
//! local model sees in its prelude:
//!   - World state: files seen / modified, branch (if known), test results
//!   - Actions taken: an audit log derived from successful tool calls
//!   - Unresolved errors: any tool output where `success = false`
//!   - In-flight work: orchestration calls (spawn/wait) that don't have a
//!     terminal status yet
//!
//! Pure data extraction — no LLM calls, no judgment.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use super::items::ParsedTranscript;
use super::items::TrimItem;
use super::signatures::path_from_signature;

#[derive(Debug, Clone, Default)]
pub struct ExtractedState {
    pub files_seen: BTreeSet<String>,
    pub files_modified: BTreeMap<String, ModifiedFile>,
    pub actions: Vec<ActionReceipt>,
    pub unresolved_errors: Vec<UnresolvedError>,
    pub in_flight: Vec<InFlight>,
    pub test_runs: Vec<TestRun>,
    /// When the model is stuck calling the same tool with identical args
    /// repeatedly. Surfaced prominently in the prelude so the model is
    /// nudged to try a different approach.
    pub repetition: Option<RepetitionAlert>,
}

#[derive(Debug, Clone)]
pub struct RepetitionAlert {
    pub tool_name: String,
    pub command_summary: String,
    pub count: usize,
    pub last_output_excerpt: String,
}

#[derive(Debug, Clone)]
pub struct ModifiedFile {
    pub op: ModifyOp,
    pub turn_id: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum ModifyOp {
    Created,
    Edited,
    Deleted,
}

#[derive(Debug, Clone)]
pub struct ActionReceipt {
    pub turn_id: u32,
    pub summary: String,
}

#[derive(Debug, Clone)]
pub struct UnresolvedError {
    pub turn_id: u32,
    pub tool_name: String,
    pub call_id: String,
    pub excerpt: String,
}

#[derive(Debug, Clone)]
pub struct InFlight {
    pub turn_id: u32,
    pub tool_name: String,
    pub call_id: String,
    pub note: String,
}

#[derive(Debug, Clone)]
pub struct TestRun {
    pub turn_id: u32,
    pub command: String,
    pub passed: bool,
    pub summary: String,
}

/// Extract state from the entire transcript. We extract from active turn too —
/// the model benefits from seeing materialized state for very-recent actions
/// it took itself, especially after a long active turn.
pub fn extract(parsed: &ParsedTranscript, _active_turn: u32) -> ExtractedState {
    let mut state = ExtractedState::default();

    // Map call_id -> (tool_name, turn_id) so we can resolve outputs efficiently
    // when extracting actions and errors.
    let mut call_index: BTreeMap<String, (String, u32, String)> = BTreeMap::new();
    for item in &parsed.items {
        if let TrimItem::ToolCall {
            call_id,
            tool_name,
            turn_id,
            args,
            ..
        } = item
        {
            call_index.insert(call_id.clone(), (tool_name.clone(), *turn_id, args.clone()));
        }
    }

    // Track outputs that had a terminal status, so anything left over is
    // considered in-flight.
    let mut completed_calls: BTreeSet<String> = BTreeSet::new();

    for item in &parsed.items {
        match item {
            TrimItem::ToolCall {
                tool_name,
                args,
                signature,
                turn_id,
                ..
            } => {
                if let Some(path) = path_from_signature(signature) {
                    if matches!(
                        tool_name.as_str(),
                        "text_editor" | "view_image" | "list_dir"
                    ) && !path.is_empty()
                        && path != "?"
                    {
                        state.files_seen.insert(path.to_string());
                    }
                }
                if let Some((path, op)) = derive_modification(tool_name, args) {
                    state.files_modified.insert(
                        path,
                        ModifiedFile {
                            op,
                            turn_id: *turn_id,
                        },
                    );
                }
            }
            TrimItem::ToolOutput {
                tool_name,
                call_id,
                success,
                content,
                turn_id,
                ..
            } => {
                completed_calls.insert(call_id.clone());

                if !*success {
                    state.unresolved_errors.push(UnresolvedError {
                        turn_id: *turn_id,
                        tool_name: tool_name.clone(),
                        call_id: call_id.clone(),
                        excerpt: excerpt(content, 240),
                    });
                    continue;
                }

                // Look up the originating call for full context (args).
                let Some((_call_tool, _call_turn, args)) = call_index.get(call_id) else {
                    continue;
                };

                if let Some(receipt) = derive_action_receipt(tool_name, args, content, *turn_id) {
                    state.actions.push(receipt);
                }
                if let Some(test) = derive_test_run(tool_name, args, content, *turn_id) {
                    state.test_runs.push(test);
                }
            }
            _ => {}
        }
    }

    // Anything called but not completed is in-flight.
    for (call_id, (tool_name, turn_id, args)) in &call_index {
        if completed_calls.contains(call_id) {
            continue;
        }
        if !is_orchestration_or_async(tool_name) {
            continue;
        }
        state.in_flight.push(InFlight {
            turn_id: *turn_id,
            tool_name: tool_name.clone(),
            call_id: call_id.clone(),
            note: short_args(args, 80),
        });
    }

    state.repetition = detect_repetition(parsed);

    state
}

/// Detect when the model is stuck calling the same tool with the same args
/// repeatedly. Walk the most recent ToolCall items in order; if the last 3+
/// share the same `(tool_name, signature)`, that's a stuck loop.
///
/// Threshold: 3 consecutive identical calls. Two could be a legitimate retry
/// after a transient error; three means the model isn't learning from the
/// outputs.
fn detect_repetition(parsed: &ParsedTranscript) -> Option<RepetitionAlert> {
    const THRESHOLD: usize = 3;

    // Walk from the end, collecting consecutive ToolCall signatures until we
    // hit a different signature or a non-ToolCall, non-ToolOutput item.
    let mut last_signature: Option<(String, String)> = None;
    let mut count = 0usize;
    let mut last_call_args: Option<String> = None;
    let mut last_call_id: Option<String> = None;

    for item in parsed.items.iter().rev() {
        match item {
            TrimItem::ToolCall {
                tool_name,
                signature,
                args,
                call_id,
                ..
            } => {
                let key = (tool_name.clone(), signature.clone());
                match &last_signature {
                    None => {
                        last_signature = Some(key);
                        last_call_args = Some(args.clone());
                        last_call_id = Some(call_id.clone());
                        count = 1;
                    }
                    Some(prev) if *prev == key => {
                        count += 1;
                    }
                    Some(_) => break,
                }
            }
            // Tool outputs interleave with calls; skip them.
            TrimItem::ToolOutput { .. } => continue,
            // Anything else (user message, assistant text) breaks the streak.
            _ => break,
        }
    }

    if count < THRESHOLD {
        return None;
    }

    let (tool_name, _) = last_signature?;
    let command_summary = short_args(last_call_args.as_deref().unwrap_or(""), 100);

    // Pull the most recent matching output's excerpt for context.
    let last_output_excerpt = last_call_id
        .as_ref()
        .and_then(|id| {
            parsed.items.iter().rev().find_map(|item| {
                if let TrimItem::ToolOutput {
                    call_id, content, ..
                } = item
                    && call_id == id
                {
                    Some(excerpt(content, 200))
                } else {
                    None
                }
            })
        })
        .unwrap_or_default();

    Some(RepetitionAlert {
        tool_name,
        command_summary,
        count,
        last_output_excerpt,
    })
}

fn derive_modification(tool_name: &str, args_raw: &str) -> Option<(String, ModifyOp)> {
    match tool_name {
        "apply_patch" => {
            let parsed: serde_json::Value = serde_json::from_str(args_raw).ok()?;
            let input = parsed
                .get("input")
                .or_else(|| parsed.get("patch"))
                .and_then(|p| p.as_str())?;
            for line in input.lines() {
                if let Some(rest) = line.strip_prefix("*** Add File: ") {
                    return Some((rest.trim().to_string(), ModifyOp::Created));
                }
                if let Some(rest) = line.strip_prefix("*** Update File: ") {
                    return Some((rest.trim().to_string(), ModifyOp::Edited));
                }
                if let Some(rest) = line.strip_prefix("*** Delete File: ") {
                    return Some((rest.trim().to_string(), ModifyOp::Deleted));
                }
            }
            None
        }
        "text_editor" => {
            let parsed: serde_json::Value = serde_json::from_str(args_raw).ok()?;
            let cmd = parsed.get("command").and_then(|c| c.as_str()).unwrap_or("");
            let path = parsed.get("path").and_then(|p| p.as_str())?.to_string();
            let op = match cmd {
                "create" => ModifyOp::Created,
                "str_replace" | "insert" | "edit" | "write" => ModifyOp::Edited,
                "delete" => ModifyOp::Deleted,
                _ => return None,
            };
            Some((path, op))
        }
        _ => None,
    }
}

fn derive_action_receipt(
    tool_name: &str,
    args_raw: &str,
    output: &str,
    turn_id: u32,
) -> Option<ActionReceipt> {
    let summary = match tool_name {
        "apply_patch" => {
            let (op, path) = match derive_modification("apply_patch", args_raw) {
                Some((p, ModifyOp::Created)) => ("Created", p),
                Some((p, ModifyOp::Edited)) => ("Modified", p),
                Some((p, ModifyOp::Deleted)) => ("Deleted", p),
                None => return None,
            };
            format!("{op} {path}")
        }
        "text_editor" => {
            let (op, path) = match derive_modification("text_editor", args_raw) {
                Some((p, ModifyOp::Created)) => ("Created", p),
                Some((p, ModifyOp::Edited)) => ("Modified", p),
                Some((p, ModifyOp::Deleted)) => ("Deleted", p),
                None => return None,
            };
            format!("{op} {path}")
        }
        "shell" | "shell_command" | "exec_command" | "local_shell" => {
            let parsed: serde_json::Value = serde_json::from_str(args_raw).unwrap_or_default();
            let cmd = parsed
                .get("command")
                .or_else(|| parsed.get("cmd"))
                .or_else(|| parsed.get("argv"))
                .map(stringify_command)
                .unwrap_or_default();
            if cmd.is_empty() {
                return None;
            }
            let exit = shell_exit_code(output).unwrap_or(0);
            format!("Ran `{}` → exit {exit}", short_str(&cmd, 80))
        }
        _ => return None,
    };
    Some(ActionReceipt { turn_id, summary })
}

fn derive_test_run(tool_name: &str, args_raw: &str, output: &str, turn_id: u32) -> Option<TestRun> {
    if !matches!(
        tool_name,
        "shell" | "shell_command" | "exec_command" | "local_shell"
    ) {
        return None;
    }
    let parsed: serde_json::Value = serde_json::from_str(args_raw).ok()?;
    let cmd = parsed
        .get("command")
        .or_else(|| parsed.get("cmd"))
        .or_else(|| parsed.get("argv"))
        .map(stringify_command)
        .unwrap_or_default();
    if !looks_like_test_command(&cmd) {
        return None;
    }
    let exit = shell_exit_code(output).unwrap_or(0);
    let passed = exit == 0;
    let summary = summarize_test_output(output);
    Some(TestRun {
        turn_id,
        command: cmd,
        passed,
        summary,
    })
}

fn looks_like_test_command(cmd: &str) -> bool {
    let lower = cmd.to_lowercase();
    lower.contains("cargo test")
        || lower.contains("pytest")
        || lower.contains("npm test")
        || lower.contains("npm run test")
        || lower.contains("yarn test")
        || lower.contains("pnpm test")
        || lower.contains("jest")
        || lower.contains("go test")
        || lower.contains("mvn test")
        || lower.contains("gradle test")
}

fn summarize_test_output(output: &str) -> String {
    // Best-effort: find the last "passed/failed" summary line.
    for line in output.lines().rev() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if lower.contains("passed") || lower.contains("failed") {
            return trimmed.to_string();
        }
    }
    "(no summary line found)".to_string()
}

fn is_orchestration_or_async(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "spawn_agent"
            | "spawn_subagent_v2"
            | "send_input"
            | "send_message_v2"
            | "wait"
            | "wait_agent"
            | "exec_command"
            | "write_stdin"
            | "supervisor"
            | "agent_jobs"
    )
}

fn shell_exit_code(output: &str) -> Option<i32> {
    // Codex's structured shell output places exit_code in JSON.
    let parsed: serde_json::Value = serde_json::from_str(output).ok()?;
    parsed
        .get("metadata")
        .and_then(|m| m.get("exit_code"))
        .and_then(|c| c.as_i64())
        .map(|c| c as i32)
        .or_else(|| {
            parsed
                .get("exit_code")
                .and_then(|c| c.as_i64())
                .map(|c| c as i32)
        })
}

fn excerpt(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

fn short_str(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..n])
    }
}

fn short_args(s: &str, n: usize) -> String {
    let cleaned = s.replace(['\n', '\r'], " ");
    short_str(&cleaned, n)
}

fn stringify_command(v: &serde_json::Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    if let Some(arr) = v.as_array() {
        return arr
            .iter()
            .filter_map(|p| p.as_str())
            .collect::<Vec<_>>()
            .join(" ");
    }
    v.to_string()
}
