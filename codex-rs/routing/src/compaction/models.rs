//! Compaction data models — ported from compaction/models.py.
//! See docs/spec/compaction-reference.md.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A chunk of transcript items for extraction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptChunk {
    pub chunk_id: usize,
    pub start_index: usize,
    pub end_index: usize,
    pub token_count: usize,
    pub overlap_from_previous_tokens: usize,
    pub items: Vec<serde_json::Value>,
}

/// Extracted durable state from a single chunk.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChunkExtraction {
    pub chunk_id: usize,
    pub objective: String,
    pub repo_state: HashMap<String, String>,
    pub files_touched: Vec<String>,
    pub commands_run: Vec<String>,
    pub errors: Vec<String>,
    pub accepted_fixes: Vec<String>,
    pub rejected_ideas: Vec<String>,
    pub constraints: Vec<String>,
    pub environment_assumptions: Vec<String>,
    pub pending_todos: Vec<String>,
    pub unresolved_bugs: Vec<String>,
    pub test_status: Vec<String>,
    pub external_references: Vec<String>,
    pub latest_plan: Vec<String>,
    pub source_token_count: usize,
}

/// Merged state from multiple chunk extractions.
pub type MergedState = ChunkExtraction;

/// Session handoff — the final output of compaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionHandoff {
    pub stable_task_definition: String,
    pub repo_state: HashMap<String, String>,
    pub key_decisions: Vec<String>,
    pub unresolved_work: Vec<String>,
    pub latest_plan: Vec<String>,
    pub failures_to_avoid: Vec<String>,
    pub recent_raw_turns: Vec<serde_json::Value>,
    pub current_request: String,
}

/// Durable memory set — 5 markdown documents.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DurableMemorySet {
    pub task_state: String,
    pub decisions: String,
    pub failures_to_avoid: String,
    pub next_steps: String,
    pub session_handoff: String,
}
