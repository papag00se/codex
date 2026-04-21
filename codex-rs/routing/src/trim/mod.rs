//! Deterministic role-aware transcript trimming for local models.
//!
//! Used by both per-request context preparation (replacing the older
//! `context_strip` module) and as the first stage of compaction.
//! Same rules everywhere; mode (regular vs local-only) only changes routing.
//!
//! Design principles (see docs/spec/trim-design.md when written):
//! - Local models always get the same treatment regardless of mode.
//! - Older history is replaced with synthesized state, not just chopped.
//! - The active turn (everything from the most recent user message onward)
//!   is preserved verbatim with no compression.
//! - Per-tool semantic rules — never blanket character truncation.
//! - Errors are sticky: any failed tool output is preserved regardless of age.
//! - System prompt is never stubbed.

mod items;
mod render;
mod rules;
mod signatures;
mod state_extract;

#[cfg(test)]
mod tests;

use codex_protocol::models::ResponseItem;
use serde::Serialize;
use serde_json::Value as JsonValue;

pub use items::TrimItem;

/// Input handed to the trimmer. Decoupled from the codex-core `Prompt` type so
/// the routing crate stays independent of `codex-core`.
#[derive(Debug, Clone)]
pub struct TrimInput<'a> {
    /// Conversation items, oldest first, exactly as they appear in `Prompt::input`.
    pub items: &'a [ResponseItem],
    /// The full Codex system prompt (base instructions). Passed through verbatim.
    pub system_prompt: &'a str,
    /// Project-level user instructions (AGENTS.md / CLAUDE.md content), if any.
    /// These are pinned into the persistent context block.
    pub user_instructions: Option<&'a str>,
}

/// Result of trimming, ready to send to a local model via the Ollama chat API.
#[derive(Debug, Clone)]
pub struct TrimResult {
    /// Combined system prompt: original system prompt followed by the
    /// synthesized state prelude. Sent as the chat `system` field.
    pub system: String,
    /// Chat messages in Ollama format. Older turns are collapsed into single
    /// summary messages; the active turn is preserved verbatim including any
    /// tool calls and tool outputs.
    pub messages: Vec<JsonValue>,
    /// Diagnostics about what was kept, dropped, or collapsed.
    pub summary: TrimSummary,
}

/// Diagnostics emitted by `trim_for_local`. Logged by the caller; not seen by
/// the model.
#[derive(Debug, Clone, Default, Serialize)]
pub struct TrimSummary {
    pub original_items: usize,
    pub kept_items: usize,
    pub older_turns_collapsed: u32,
    pub stale_reads_dropped: u32,
    pub superseded_outputs_dropped: u32,
    pub elided_output_chars: usize,
    pub estimated_input_tokens: usize,
    /// Count of older-turn messages at the head of `messages`. Active-turn
    /// messages start at this offset. Callers can use this to slice older
    /// content out for further processing (e.g. summarization when even
    /// the trimmed transcript exceeds the local model's context budget).
    pub older_turn_message_count: usize,
}

impl TrimSummary {
    /// Render a one-line summary suitable for tracing/info logs.
    pub fn to_log_line(&self) -> String {
        format!(
            "kept {}/{} items; collapsed {} older turns; dropped {} stale reads, {} superseded outputs; elided {} chars; ~{} input tokens",
            self.kept_items,
            self.original_items,
            self.older_turns_collapsed,
            self.stale_reads_dropped,
            self.superseded_outputs_dropped,
            self.elided_output_chars,
            self.estimated_input_tokens,
        )
    }
}

/// Trim a transcript for a local model.
///
/// The resulting `TrimResult` is intended to fit comfortably inside `target_ctx`
/// tokens. If the active turn alone exceeds `target_ctx`, the trimmer still
/// returns it verbatim — failing later in the model is preferable to silently
/// dropping the user's current request.
pub fn trim_for_local(input: &TrimInput, target_ctx: usize) -> TrimResult {
    let parsed = items::parse(input.items);
    let active_turn = parsed.active_turn_id();

    let extracted = state_extract::extract(&parsed, active_turn);
    let compressed_older = rules::compress_older_turns(&parsed, active_turn);

    let prelude = render::render_prelude(input.user_instructions, &extracted);
    let (messages, older_turn_message_count) =
        render::render_messages(&compressed_older, &parsed, active_turn);

    let mut summary = TrimSummary {
        original_items: input.items.len(),
        kept_items: messages.len(),
        older_turns_collapsed: compressed_older.collapsed_turn_count,
        stale_reads_dropped: compressed_older.stale_reads_dropped,
        superseded_outputs_dropped: compressed_older.superseded_outputs_dropped,
        elided_output_chars: compressed_older.elided_chars,
        estimated_input_tokens: 0,
        older_turn_message_count,
    };

    let combined_system = if prelude.is_empty() {
        input.system_prompt.to_string()
    } else {
        format!("{}\n\n{}", input.system_prompt, prelude)
    };

    summary.estimated_input_tokens =
        crate::metrics::estimate_tokens(&combined_system) + estimate_messages_tokens(&messages);

    TrimResult {
        system: combined_system,
        messages,
        summary,
    }
}

fn estimate_messages_tokens(messages: &[JsonValue]) -> usize {
    let joined: String = messages
        .iter()
        .filter_map(|m| {
            m.get("content")
                .and_then(|c| c.as_str())
                .map(str::to_string)
        })
        .collect::<Vec<_>>()
        .join("\n");
    crate::metrics::estimate_tokens(&joined)
}
