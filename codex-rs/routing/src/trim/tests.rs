//! Table-driven tests for the trim module.
//!
//! Each test builds a small synthetic transcript via the helper constructors
//! at the bottom of this file, runs `trim_for_local`, and asserts on the
//! resulting messages, prelude, and summary counters.

use codex_protocol::models::ContentItem;
use codex_protocol::models::FunctionCallOutputPayload;
use codex_protocol::models::ResponseItem;

use super::TrimInput;
use super::trim_for_local;

#[test]
fn empty_transcript_produces_only_system_prompt() {
    let result = trim_for_local(
        &TrimInput {
            items: &[],
            system_prompt: "You are Codex.",
            user_instructions: None,
        },
        16384,
    );
    assert_eq!(result.system, "You are Codex.");
    assert!(result.messages.is_empty());
    assert_eq!(result.summary.original_items, 0);
}

#[test]
fn user_instructions_appear_in_prelude() {
    let result = trim_for_local(
        &TrimInput {
            items: &[user_msg("hello")],
            system_prompt: "SYS",
            user_instructions: Some("Don't use mocks."),
        },
        16384,
    );
    assert!(
        result.system.contains("[Persistent project context]"),
        "system: {}",
        result.system
    );
    assert!(result.system.contains("Don't use mocks."));
    assert!(result.system.contains("SYS"));
}

#[test]
fn system_prompt_is_never_stubbed_or_truncated() {
    let long = "A".repeat(20_000);
    let result = trim_for_local(
        &TrimInput {
            items: &[user_msg("hi")],
            system_prompt: &long,
            user_instructions: None,
        },
        16384,
    );
    assert!(result.system.starts_with(&long));
}

#[test]
fn active_turn_user_message_kept_verbatim() {
    let prompt = "I would like to build a hello world Lambda.";
    let result = trim_for_local(
        &TrimInput {
            items: &[user_msg(prompt)],
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert_eq!(result.messages.len(), 1);
    let content = result.messages[0]
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap();
    assert_eq!(content, prompt);
}

#[test]
fn tool_calls_and_outputs_in_active_turn_are_preserved() {
    let items = vec![
        user_msg("read auth.py"),
        function_call(
            "call_1",
            "text_editor",
            r#"{"command":"view","path":"src/auth.py"}"#,
        ),
        function_output("call_1", "<file contents>", true),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    // user message + assistant tool_call + tool output (rendered as user
    // message wrapping <tool_result>) = 3 messages.
    assert_eq!(result.messages.len(), 3, "messages: {:?}", result.messages);
    assert_eq!(result.messages[1].get("role").unwrap(), "assistant");
    assert!(result.messages[1].get("tool_calls").is_some());
    assert_eq!(result.messages[2].get("role").unwrap(), "user");
    let last = result.messages[2]
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap();
    assert!(
        last.contains("<tool_result"),
        "tool output should be wrapped in <tool_result>: {last}"
    );
    assert!(last.contains("<file contents>"));
}

#[test]
fn old_read_then_patch_drops_old_read_output() {
    let items = vec![
        user_msg("turn 1: read"),
        function_call("c1", "text_editor", r#"{"command":"view","path":"foo.py"}"#),
        function_output("c1", "OLD CONTENT", true),
        user_msg("turn 2: patch"),
        function_call(
            "c2",
            "apply_patch",
            r#"{"input":"*** Update File: foo.py\n@@\n-old\n+new\n"}"#,
        ),
        function_output("c2", "patched", true),
        user_msg("turn 3: do something else"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert!(
        result.summary.stale_reads_dropped >= 1,
        "expected stale read drop, got summary {:?}",
        result.summary
    );
    // The OLD CONTENT string must not appear anywhere in the rendered output.
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("OLD CONTENT"),
        "stale read content leaked into messages:\n{combined}"
    );
}

#[test]
fn duplicate_grep_supersedes_older_output() {
    let args = r#"{"query":"foo","path":"."}"#;
    let items = vec![
        user_msg("turn 1: grep"),
        function_call("g1", "grep_files", args),
        function_output("g1", "OLD MATCHES", true),
        user_msg("turn 2: grep again"),
        function_call("g2", "grep_files", args),
        function_output("g2", "NEW MATCHES", true),
        user_msg("turn 3"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert!(
        result.summary.superseded_outputs_dropped >= 1,
        "expected supersession, got summary {:?}",
        result.summary
    );
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!combined.contains("OLD MATCHES"));
}

#[test]
fn shell_output_with_nonzero_exit_recognized_as_failure() {
    // Codex's shell handler hardcodes `success: Some(true)`, putting the
    // actual exit code in `metadata.exit_code` inside the content. Trim
    // should detect this and treat the output as a failure, surfacing it
    // in [UNRESOLVED ERRORS] AND tagging it as <tool_error>.
    let items = vec![
        user_msg("run the broken command"),
        function_call("call1", "shell", r#"{"command":["bash","-lc","rg '*.ts'"]}"#),
        function_output(
            "call1",
            r#"{"output":"rg: regex parse error","metadata":{"exit_code":2,"duration_seconds":0.1}}"#,
            true,
        ),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert!(
        result.system.contains("[UNRESOLVED ERRORS]"),
        "exit_code 2 should surface as unresolved error:\n{}",
        result.system
    );
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("<tool_error"),
        "failed output should be tagged <tool_error>:\n{combined}"
    );
}

#[test]
fn failed_shell_output_kept_even_in_old_turn() {
    let items = vec![
        user_msg("turn 1: install"),
        function_call("s1", "shell", r#"{"command":"pip install boto3"}"#),
        function_output("s1", "ERROR: pip not found", false),
        user_msg("turn 2"),
        user_msg("turn 3"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert!(
        result.system.contains("[UNRESOLVED ERRORS]"),
        "system block missing unresolved errors header:\n{}",
        result.system
    );
    assert!(result.system.contains("pip not found"));
}

#[test]
fn world_state_lists_modified_files() {
    let items = vec![
        user_msg("turn 1: create"),
        function_call(
            "c1",
            "apply_patch",
            r#"{"input":"*** Add File: src/lambda.py\n+import json\n"}"#,
        ),
        function_output("c1", "ok", true),
        user_msg("turn 2"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    assert!(
        result.system.contains("Created: src/lambda.py"),
        "expected created file in world state:\n{}",
        result.system
    );
}

#[test]
fn old_assistant_text_dropped_from_messages_but_actions_recorded() {
    let items = vec![
        user_msg("turn 1: do it"),
        assistant_msg("I'll do it."),
        function_call(
            "p1",
            "apply_patch",
            r#"{"input":"*** Add File: foo.py\n+x\n"}"#,
        ),
        function_output("p1", "ok", true),
        user_msg("turn 2: now this"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("I'll do it."),
        "old assistant narration leaked:\n{combined}"
    );
    assert!(
        result.system.contains("[Actions taken]"),
        "actions block missing:\n{}",
        result.system
    );
    assert!(result.system.contains("Created foo.py"));
}

#[test]
fn old_successful_shell_output_dropped_action_receipt_kept() {
    // Shell, apply_patch, etc. are "action-only" tools: once we have an
    // [Actions taken] entry in the prelude, the raw output bytes are dead
    // weight. Drop them entirely from older turns.
    let mut long = String::new();
    for i in 0..500 {
        long.push_str(&format!("line {i}\n"));
    }
    let items = vec![
        user_msg("turn 1"),
        function_call("s1", "shell", r#"{"command":"ls -R /"}"#),
        function_output("s1", &long, true),
        user_msg("turn 2 — keep going"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        !combined.contains("line 0") && !combined.contains("line 499"),
        "old shell output bytes leaked into messages:\n{}",
        &combined.chars().take(2000).collect::<String>()
    );
    assert!(
        result.system.contains("[Actions taken]"),
        "actions block missing:\n{}",
        result.system
    );
    assert!(
        result.system.contains("Ran `ls -R /`"),
        "shell action receipt missing:\n{}",
        result.system
    );
}

#[test]
fn old_grep_output_kept_with_match_cap() {
    // Read-shaped tools (grep, list_dir, text_editor view) keep their data
    // because the model may still reference it. Long match lists get capped.
    let mut matches = String::new();
    for i in 0..50 {
        matches.push_str(&format!("src/file_{i}.rs:1: foo\n"));
    }
    let items = vec![
        user_msg("turn 1"),
        function_call("g1", "grep_files", r#"{"query":"foo","path":"."}"#),
        function_output("g1", &matches, true),
        user_msg("turn 2 — keep going"),
    ];
    let result = trim_for_local(
        &TrimInput {
            items: &items,
            system_prompt: "SYS",
            user_instructions: None,
        },
        16384,
    );
    let combined: String = result
        .messages
        .iter()
        .filter_map(|m| m.get("content").and_then(|c| c.as_str()))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        combined.contains("file_0.rs"),
        "head matches not preserved:\n{}",
        &combined.chars().take(2000).collect::<String>()
    );
    assert!(
        combined.contains("more matches elided"),
        "missing grep cap marker"
    );
}

#[test]
fn turn_id_increments_only_on_user_messages() {
    use super::items::parse;
    let items = vec![
        user_msg("first"),
        assistant_msg("response"),
        user_msg("second"),
        assistant_msg("response 2"),
        function_call("c1", "shell", r#"{"command":"ls"}"#),
        function_output("c1", "ok", true),
        user_msg("third"),
    ];
    let parsed = parse(&items);
    assert_eq!(parsed.max_turn_id, 2);
    assert_eq!(parsed.items[0].turn_id(), 0); // first user
    assert_eq!(parsed.items[1].turn_id(), 0); // assistant in turn 0
    assert_eq!(parsed.items[2].turn_id(), 1); // second user
}

// --- helpers ------------------------------------------------------------

fn user_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "user".to_string(),
        content: vec![ContentItem::InputText {
            text: text.to_string(),
        }],
        end_turn: None,
        phase: None,
    }
}

fn assistant_msg(text: &str) -> ResponseItem {
    ResponseItem::Message {
        id: None,
        role: "assistant".to_string(),
        content: vec![ContentItem::OutputText {
            text: text.to_string(),
        }],
        end_turn: None,
        phase: None,
    }
}

fn function_call(call_id: &str, name: &str, arguments: &str) -> ResponseItem {
    ResponseItem::FunctionCall {
        id: None,
        name: name.to_string(),
        namespace: None,
        arguments: arguments.to_string(),
        call_id: call_id.to_string(),
    }
}

fn function_output(call_id: &str, content: &str, success: bool) -> ResponseItem {
    ResponseItem::FunctionCallOutput {
        call_id: call_id.to_string(),
        output: FunctionCallOutputPayload {
            body: codex_protocol::models::FunctionCallOutputBody::Text(content.to_string()),
            success: Some(success),
        },
    }
}
