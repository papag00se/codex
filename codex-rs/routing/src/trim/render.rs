//! Block renderer.
//!
//! Composes the trimmer's two outputs:
//!   - The synthesized prelude (persistent context + world state + open issues
//!     + in-flight + tests). Concatenated onto the system prompt by `mod.rs`.
//!   - The Ollama-format chat messages: per-turn collapsed summaries for older
//!     turns, then verbatim items for the active turn.
//!
//! Pure formatting — no decisions about what to keep.

use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

use super::items::ParsedTranscript;
use super::items::TrimItem;
use super::rules::CompressedOlder;
use super::state_extract::ExtractedState;
use super::state_extract::ModifyOp;

/// Render the full prelude block. Empty if there's nothing to say (e.g. an
/// empty transcript with no user instructions).
pub fn render_prelude(user_instructions: Option<&str>, state: &ExtractedState) -> String {
    let mut sections: Vec<String> = Vec::new();

    if let Some(inst) = user_instructions
        && !inst.trim().is_empty()
    {
        sections.push(format!("[Persistent project context]\n{}", inst.trim()));
    }

    let world = render_world_state(state);
    if !world.is_empty() {
        sections.push(world);
    }

    let actions = render_actions(state);
    if !actions.is_empty() {
        sections.push(actions);
    }

    let errors = render_errors(state);
    if !errors.is_empty() {
        sections.push(errors);
    }

    let in_flight = render_in_flight(state);
    if !in_flight.is_empty() {
        sections.push(in_flight);
    }

    let tests = render_tests(state);
    if !tests.is_empty() {
        sections.push(tests);
    }

    sections.join("\n\n")
}

fn render_world_state(state: &ExtractedState) -> String {
    if state.files_seen.is_empty() && state.files_modified.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[World state]".to_string()];
    if !state.files_seen.is_empty() {
        lines.push(format!(
            "Files seen: {}",
            state
                .files_seen
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !state.files_modified.is_empty() {
        let mut by_op: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (path, m) in &state.files_modified {
            let label = match m.op {
                ModifyOp::Created => "Created",
                ModifyOp::Edited => "Edited",
                ModifyOp::Deleted => "Deleted",
            };
            by_op.entry(label).or_default().push(path.as_str());
        }
        for (label, paths) in by_op {
            lines.push(format!("{label}: {}", paths.join(", ")));
        }
    }
    lines.join("\n")
}

fn render_actions(state: &ExtractedState) -> String {
    if state.actions.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[Actions taken]".to_string()];
    for a in &state.actions {
        lines.push(format!("- (turn {}) {}", a.turn_id, a.summary));
    }
    lines.join("\n")
}

fn render_errors(state: &ExtractedState) -> String {
    if state.unresolved_errors.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[UNRESOLVED ERRORS]".to_string()];
    for e in &state.unresolved_errors {
        lines.push(format!(
            "- (turn {}) {}: {}",
            e.turn_id, e.tool_name, e.excerpt
        ));
    }
    lines.join("\n")
}

fn render_in_flight(state: &ExtractedState) -> String {
    if state.in_flight.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[In-flight]".to_string()];
    for f in &state.in_flight {
        lines.push(format!(
            "- (turn {}) {} call_id={} args={}",
            f.turn_id, f.tool_name, f.call_id, f.note
        ));
    }
    lines.join("\n")
}

fn render_tests(state: &ExtractedState) -> String {
    if state.test_runs.is_empty() {
        return String::new();
    }
    let mut lines = vec!["[Tests]".to_string()];
    for t in &state.test_runs {
        let verdict = if t.passed { "PASS" } else { "FAIL" };
        lines.push(format!(
            "- (turn {}) {} `{}` → {}",
            t.turn_id, verdict, t.command, t.summary
        ));
    }
    lines.join("\n")
}

/// Build the chat messages and report how many of the leading messages
/// represent older (collapsed) turns. Active-turn messages occupy
/// `messages[older_turn_message_count..]`. Callers that need to summarize
/// just the older portion (e.g. when even the trimmed transcript exceeds
/// the local model's context budget) can use the count to slice cleanly.
pub fn render_messages(
    older: &CompressedOlder,
    parsed: &ParsedTranscript,
    active_turn: u32,
) -> (Vec<JsonValue>, usize) {
    let mut messages: Vec<JsonValue> = Vec::new();

    // Older turns: render a single user-message-shaped item per turn that
    // contains the verbatim user message + a one-line action summary (already
    // handled by the prelude's [Actions taken] block, so the per-turn message
    // here just preserves the user's words and the call signatures from that
    // turn that survived compression).
    let mut older_by_turn: BTreeMap<u32, Vec<&TrimItem>> = BTreeMap::new();
    for item in &older.items {
        older_by_turn.entry(item.turn_id()).or_default().push(item);
    }
    for (turn, turn_items) in older_by_turn {
        let mut user_text = String::new();
        let mut tool_lines: Vec<String> = Vec::new();
        for item in turn_items {
            match item {
                TrimItem::User { text, .. } => {
                    if !user_text.is_empty() {
                        user_text.push('\n');
                    }
                    user_text.push_str(text);
                }
                TrimItem::ToolCall {
                    tool_name,
                    args,
                    signature,
                    ..
                } => {
                    let _ = signature;
                    tool_lines.push(format!("  - called {tool_name}({})", short(args, 80)));
                }
                TrimItem::ToolOutput {
                    tool_name,
                    success,
                    content,
                    ..
                } => {
                    if !*success {
                        tool_lines.push(format!("  - {tool_name} ERROR: {}", short(content, 200)));
                    } else {
                        // Read-shaped tools (grep, list_dir, text_editor view)
                        // survived older-turn compression — include the data
                        // for the model to reference. Action-only tools were
                        // already dropped by `rules::compress_older_turns`.
                        tool_lines.push(format!("  - {tool_name} output:"));
                        for line in content.lines() {
                            tool_lines.push(format!("    {line}"));
                        }
                    }
                }
                _ => {}
            }
        }
        let mut content = format!("[turn {turn} — user]\n{user_text}");
        if !tool_lines.is_empty() {
            content.push_str("\n[turn ");
            content.push_str(&turn.to_string());
            content.push_str(" — surviving tool activity]\n");
            content.push_str(&tool_lines.join("\n"));
        }
        messages.push(serde_json::json!({
            "role": "user",
            "content": content,
        }));
    }

    let older_turn_message_count = messages.len();

    // Active turn: pass through verbatim, preserving the original
    // role/structure as best Ollama can represent it.
    for item in &parsed.items {
        if item.turn_id() != active_turn {
            continue;
        }
        match item {
            TrimItem::User { text, .. } => {
                messages.push(serde_json::json!({"role": "user", "content": text}));
            }
            TrimItem::AssistantText { text, .. } => {
                messages.push(serde_json::json!({"role": "assistant", "content": text}));
            }
            TrimItem::Reasoning { text, .. } => {
                // Ollama doesn't have a dedicated reasoning role; tag it inline
                // so the model knows it's its own prior thinking.
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": format!("<reasoning>{text}</reasoning>"),
                }));
            }
            TrimItem::ToolCall {
                tool_name,
                args,
                call_id,
                ..
            } => {
                // Ollama's chat API expects `arguments` as a JSON object, not
                // a JSON-encoded string. Parse our stored args (which arrived
                // as a raw string from the model) and embed as an object;
                // fall back to an empty object on parse failure so we never
                // send a malformed message that returns 400.
                //
                // Likewise, `content: null` triggers Ollama's parser to
                // complain about an unclosed object; use an empty string.
                let args_obj: serde_json::Value =
                    serde_json::from_str(args).unwrap_or_else(|_| serde_json::json!({}));
                messages.push(serde_json::json!({
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "id": call_id,
                        "type": "function",
                        "function": {
                            "name": tool_name,
                            "arguments": args_obj,
                        }
                    }]
                }));
            }
            TrimItem::ToolOutput {
                content,
                call_id,
                tool_name,
                success,
                ..
            } => {
                // Render tool output as a `user`-role message wrapped in a
                // `<tool_result>` block (or `<tool_error>` if the call failed)
                // instead of the OpenAI-native `{role: "tool", ...}` form.
                // The `role: "tool"` shape relies on the model's chat template
                // rendering it back into prompt context — many local model
                // templates either skip it or render it with a marker the
                // model wasn't trained to attend to. A user-role wrapper is
                // universally rendered.
                //
                // Distinguishing `<tool_error>` from `<tool_result>` gives
                // the model an obvious visual signal that the previous call
                // failed and needs to be retried with a different approach.
                let tag = if *success { "tool_result" } else { "tool_error" };
                messages.push(serde_json::json!({
                    "role": "user",
                    "content": format!(
                        "<{tag} tool=\"{tool_name}\" call_id=\"{call_id}\">\n{content}\n</{tag}>"
                    ),
                }));
            }
            TrimItem::Other { .. } => {
                // Skip unknown types in the active turn rather than risk
                // sending Ollama-incompatible JSON.
            }
        }
    }

    (messages, older_turn_message_count)
}

fn short(s: &str, n: usize) -> String {
    let cleaned = s.replace(['\n', '\r'], " ");
    if cleaned.len() <= n {
        cleaned
    } else {
        format!("{}…", &cleaned[..n])
    }
}
