//! Per-tool compression rules applied to *older* turns.
//!
//! The active turn is always preserved verbatim (handled by the renderer).
//! For each older turn, this module:
//!   - drops Reasoning items entirely,
//!   - drops AssistantText (content is summarized into the World State block instead),
//!   - keeps tool calls but compresses their args and outputs per Class A/B/C/D rules,
//!   - drops tool outputs that are superseded by a later same-signature call,
//!   - drops `text_editor`/`read_file` outputs whose path was later modified
//!     by an `apply_patch` or `text_editor` write,
//!   - never drops anything whose output indicates an error.

use std::collections::HashMap;
use std::collections::HashSet;

use super::items::ParsedTranscript;
use super::items::TrimItem;
use super::signatures::path_from_signature;

/// Result of compressing the older portion of the transcript.
#[derive(Debug, Clone, Default)]
pub struct CompressedOlder {
    /// Compressed items in original order, ready for the renderer to fold into
    /// per-turn summary messages.
    pub items: Vec<TrimItem>,
    pub collapsed_turn_count: u32,
    pub stale_reads_dropped: u32,
    pub superseded_outputs_dropped: u32,
    pub elided_chars: usize,
}

/// Compress everything in `parsed.items` whose `turn_id < active_turn`.
pub fn compress_older_turns(parsed: &ParsedTranscript, active_turn: u32) -> CompressedOlder {
    if active_turn == 0 || parsed.items.is_empty() {
        return CompressedOlder::default();
    }

    // Pass 1: build supersession + modification indexes from the WHOLE
    // transcript (active turn included so older-turn reads can be invalidated
    // by later writes).
    let latest_for_signature = build_latest_for_signature(&parsed.items);
    let modified_paths = build_modified_paths(&parsed.items);

    let mut out = Vec::new();
    let mut elided = 0usize;
    let mut stale_reads = 0u32;
    let mut superseded = 0u32;
    let mut turn_seen: HashSet<u32> = HashSet::new();

    for (idx, item) in parsed.items.iter().enumerate() {
        let turn = item.turn_id();
        if turn >= active_turn {
            // Active turn — handled by renderer, not this pass.
            continue;
        }
        turn_seen.insert(turn);

        match item {
            TrimItem::User { .. } => {
                // Always keep older user messages verbatim — they're authoritative.
                out.push(item.clone());
            }
            TrimItem::AssistantText { .. } => {
                // Summarized into the per-turn header by the renderer; drop here.
            }
            TrimItem::Reasoning { .. } => {
                // Old reasoning is single-use exhaust.
            }
            TrimItem::ToolCall {
                tool_name,
                args,
                signature,
                ..
            } => {
                let compressed_args = compress_call_args(tool_name, args, &mut elided);
                let mut new_item = item.clone();
                if let TrimItem::ToolCall { args, .. } = &mut new_item {
                    *args = compressed_args;
                }
                let _ = signature;
                out.push(new_item);
            }
            TrimItem::ToolOutput {
                tool_name,
                signature,
                success,
                content,
                ..
            } => {
                // Errors are sticky regardless of age.
                if !success {
                    out.push(item.clone());
                    continue;
                }

                // Superseded by a later same-signature output? Drop.
                if matches!(latest_for_signature.get(signature), Some(&later_idx) if later_idx > idx)
                {
                    superseded = superseded.saturating_add(1);
                    elided = elided.saturating_add(content.len());
                    continue;
                }

                // Stale-after-modify: a `text_editor` read of path P followed
                // by any later modification of P invalidates the read.
                if is_read_call(tool_name)
                    && let Some(path) = path_from_signature(signature)
                    && let Some(&modify_idx) = modified_paths.get(path)
                    && modify_idx > idx
                {
                    stale_reads = stale_reads.saturating_add(1);
                    elided = elided.saturating_add(content.len());
                    continue;
                }

                // Action-shaped tools: the action receipt in the prelude
                // ([Actions taken]) is enough; drop the raw output bytes.
                if is_action_only_tool(tool_name) {
                    elided = elided.saturating_add(content.len());
                    continue;
                }

                let compressed = compress_output(tool_name, content, &mut elided);
                let mut new_item = item.clone();
                if let TrimItem::ToolOutput { content, .. } = &mut new_item {
                    *content = compressed;
                }
                out.push(new_item);
            }
            TrimItem::Other { .. } => {
                // Unknown item types in older turns are dropped — they can't
                // be safely summarized and aren't typically informative once
                // the active turn is past them.
            }
        }
    }

    CompressedOlder {
        items: out,
        collapsed_turn_count: turn_seen.len() as u32,
        stale_reads_dropped: stale_reads,
        superseded_outputs_dropped: superseded,
        elided_chars: elided,
    }
}

fn is_read_call(tool_name: &str) -> bool {
    matches!(tool_name, "text_editor" | "view_image")
}

/// Tools whose successful output is effectively "did the thing" — once we have
/// an action receipt in the prelude, the raw output bytes carry no further
/// information for the model. Errors from these tools are still preserved
/// (handled separately above).
fn is_action_only_tool(tool_name: &str) -> bool {
    matches!(
        tool_name,
        "shell"
            | "shell_command"
            | "exec_command"
            | "local_shell"
            | "write_stdin"
            | "apply_patch"
            | "view_image"
    )
}

fn build_latest_for_signature(items: &[TrimItem]) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        if let TrimItem::ToolOutput { signature, .. } = item {
            map.insert(signature.clone(), idx);
        }
    }
    map
}

fn build_modified_paths(items: &[TrimItem]) -> HashMap<String, usize> {
    let mut map: HashMap<String, usize> = HashMap::new();
    for (idx, item) in items.iter().enumerate() {
        if let TrimItem::ToolCall {
            tool_name, args, ..
        } = item
            && is_modifying_call(tool_name, args)
            && let Some(path) = extract_modified_path(tool_name, args)
        {
            map.insert(path, idx);
        }
    }
    map
}

fn is_modifying_call(tool_name: &str, args_raw: &str) -> bool {
    match tool_name {
        "apply_patch" => true,
        "text_editor" => {
            // text_editor commands like `create`, `str_replace`, `insert`
            // modify the file. `view`/`read` do not.
            let parsed: serde_json::Value = serde_json::from_str(args_raw).unwrap_or_default();
            let cmd = parsed.get("command").and_then(|c| c.as_str()).unwrap_or("");
            !matches!(cmd, "view" | "read" | "")
        }
        _ => false,
    }
}

fn extract_modified_path(tool_name: &str, args_raw: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(args_raw).ok()?;
    match tool_name {
        "apply_patch" => {
            // Patch input contains *** Update File: <path> markers; extract
            // the first one as the representative path. (Patches over multiple
            // files invalidate each one; we'd extract them all in a richer
            // implementation.)
            let input = parsed
                .get("input")
                .or_else(|| parsed.get("patch"))
                .and_then(|p| p.as_str())?;
            for line in input.lines() {
                if let Some(rest) = line
                    .strip_prefix("*** Update File: ")
                    .or_else(|| line.strip_prefix("*** Add File: "))
                    .or_else(|| line.strip_prefix("*** Delete File: "))
                {
                    return Some(rest.trim().to_string());
                }
            }
            None
        }
        "text_editor" => parsed
            .get("path")
            .and_then(|p| p.as_str())
            .map(str::to_string),
        _ => None,
    }
}

/// Apply per-tool argument compression for older calls.
fn compress_call_args(tool_name: &str, args: &str, elided: &mut usize) -> String {
    match tool_name {
        // Drop the diff body from older `apply_patch` calls — the file state is
        // already in the file. Keep the operation header.
        "apply_patch" => {
            let parsed: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
            let input = parsed
                .get("input")
                .or_else(|| parsed.get("patch"))
                .and_then(|p| p.as_str())
                .unwrap_or("");
            let line_count = input.lines().count();
            *elided = elided.saturating_add(input.len());
            format!("{{\"input\":\"<elided {line_count}-line patch>\"}}")
        }
        // text_editor write commands: keep the command + path, drop the new content.
        "text_editor" => {
            let parsed: serde_json::Value = serde_json::from_str(args).unwrap_or_default();
            let cmd = parsed.get("command").and_then(|c| c.as_str()).unwrap_or("");
            if matches!(cmd, "view" | "read" | "") {
                args.to_string()
            } else {
                let path = parsed.get("path").and_then(|p| p.as_str()).unwrap_or("?");
                let body_chars: usize = ["new_str", "file_text", "content", "old_str"]
                    .iter()
                    .filter_map(|k| parsed.get(*k).and_then(|v| v.as_str()))
                    .map(str::len)
                    .sum();
                *elided = elided.saturating_add(body_chars);
                format!("{{\"command\":\"{cmd}\",\"path\":\"{path}\"}}")
            }
        }
        _ => args.to_string(),
    }
}

/// Apply per-tool output compression for older outputs that survived
/// supersession + stale-read filtering.
fn compress_output(tool_name: &str, content: &str, elided: &mut usize) -> String {
    match tool_name {
        // Shell family: long success outputs get a tail-only window.
        "shell" | "shell_command" | "exec_command" | "local_shell" => {
            let lines: Vec<&str> = content.lines().collect();
            const TAIL_LINES: usize = 100;
            if lines.len() > TAIL_LINES {
                let drop_count = lines.len() - TAIL_LINES;
                let dropped: usize = lines
                    .iter()
                    .take(drop_count)
                    .map(|l| l.len() + 1)
                    .sum::<usize>();
                *elided = elided.saturating_add(dropped);
                let tail = lines[drop_count..].join("\n");
                format!("[earlier {drop_count} lines elided]\n{tail}")
            } else {
                content.to_string()
            }
        }
        // text_editor / view_image outputs that survived: long file contents
        // get a head-window since the model usually wants the top of a file.
        "text_editor" | "view_image" => {
            const HEAD_CHARS: usize = 2000;
            if content.len() > HEAD_CHARS {
                *elided = elided.saturating_add(content.len() - HEAD_CHARS);
                format!(
                    "{}\n[truncated {} chars; re-read with explicit line range if needed]",
                    &content[..HEAD_CHARS],
                    content.len() - HEAD_CHARS
                )
            } else {
                content.to_string()
            }
        }
        // grep_files: cap at 20 match lines.
        "grep_files" => {
            let lines: Vec<&str> = content.lines().collect();
            const MAX_MATCHES: usize = 20;
            if lines.len() > MAX_MATCHES {
                *elided = elided.saturating_add(
                    lines
                        .iter()
                        .skip(MAX_MATCHES)
                        .map(|l| l.len() + 1)
                        .sum::<usize>(),
                );
                let head = lines[..MAX_MATCHES].join("\n");
                format!(
                    "{head}\n[{} more matches elided; refine query if needed]",
                    lines.len() - MAX_MATCHES
                )
            } else {
                content.to_string()
            }
        }
        // list_dir: long listings get capped.
        "list_dir" => {
            const MAX_LINES: usize = 50;
            let lines: Vec<&str> = content.lines().collect();
            if lines.len() > MAX_LINES {
                *elided = elided.saturating_add(
                    lines
                        .iter()
                        .skip(MAX_LINES)
                        .map(|l| l.len() + 1)
                        .sum::<usize>(),
                );
                let head = lines[..MAX_LINES].join("\n");
                format!("{head}\n[{} more entries elided]", lines.len() - MAX_LINES)
            } else {
                content.to_string()
            }
        }
        // apply_patch outputs are usually small status lines; pass through.
        "apply_patch" => content.to_string(),
        // Default for Class C/D: hard cap at 500 chars with marker.
        _ => {
            const DEFAULT_CAP: usize = 500;
            if content.len() > DEFAULT_CAP {
                *elided = elided.saturating_add(content.len() - DEFAULT_CAP);
                format!(
                    "{}\n[truncated {} chars]",
                    &content[..DEFAULT_CAP],
                    content.len() - DEFAULT_CAP
                )
            } else {
                content.to_string()
            }
        }
    }
}
