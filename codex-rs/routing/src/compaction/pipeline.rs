//! Compaction pipeline — the full flow from transcript to handoff.
//! Orchestrates: normalize → chunk → extract → merge → refine → render.

use super::chunking::{chunk_items, split_recent_raw};
use super::extract::extract_chunk;
use super::merger::merge_states;
use super::models::*;
use super::normalize::normalize_transcript;
use super::render::{build_handoff, render_compaction_summary, render_durable_memory};
use crate::config::OllamaEndpoint;
use crate::ollama::OllamaClientPool;
use tracing::{info, warn};

/// Configuration for the compaction pipeline.
pub struct CompactionConfig {
    pub target_chunk_tokens: usize,
    pub max_chunk_tokens: usize,
    pub overlap_tokens: usize,
    pub keep_raw_tokens: usize,
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            target_chunk_tokens: 10_000,
            max_chunk_tokens: 10_000,
            overlap_tokens: 1_500,
            keep_raw_tokens: 8_000,
        }
    }
}

/// Run the full compaction pipeline.
///
/// Takes raw transcript items, returns a rendered compaction summary
/// suitable for injection as the model's context.
pub async fn compact_transcript(
    items: &[serde_json::Value],
    current_request: &str,
    pool: &OllamaClientPool,
    endpoint: &OllamaEndpoint,
    config: &CompactionConfig,
) -> Result<String, String> {
    info!(
        items = items.len(),
        current_request = &current_request[..current_request.len().min(100)],
        "Starting compaction pipeline"
    );

    // Step 1: Normalize — strip encrypted, attachments, tool_result blocks
    let normalized = normalize_transcript(items, config.target_chunk_tokens);
    info!(
        compactable = normalized.compactable_items.len(),
        precompacted = normalized.precompacted_items.len(),
        preserved = normalized.preserved_tail.len(),
        "Normalized transcript"
    );

    // Step 2: Split recent raw turns
    let (compactable, recent_raw) = split_recent_raw(
        &normalized.compactable_items,
        config.keep_raw_tokens,
    );
    info!(
        compactable = compactable.len(),
        recent_raw = recent_raw.len(),
        "Split recent raw turns"
    );

    // Step 3: Chunk compactable items
    let chunks = chunk_items(
        &compactable,
        config.target_chunk_tokens,
        config.max_chunk_tokens,
        config.overlap_tokens,
    );
    info!(chunks = chunks.len(), "Chunked transcript");

    if chunks.is_empty() {
        // Nothing to compact — return a minimal summary
        let memory = DurableMemorySet {
            task_state: "No compactable content.".into(),
            ..Default::default()
        };
        return Ok(render_compaction_summary(&memory, current_request));
    }

    // Step 4: Extract durable state from each chunk
    let mut extractions = Vec::new();
    for chunk in &chunks {
        match extract_chunk(chunk, pool, endpoint, None).await {
            Ok(extraction) => {
                info!(
                    chunk_id = chunk.chunk_id,
                    objective = %extraction.objective,
                    files = extraction.files_touched.len(),
                    "Chunk extracted"
                );
                extractions.push(extraction);
            }
            Err(e) => {
                warn!(
                    chunk_id = chunk.chunk_id,
                    error = %e,
                    "Failed to extract chunk, skipping"
                );
            }
        }
    }

    if extractions.is_empty() {
        return Err("All chunk extractions failed".into());
    }

    // Step 5: Merge all extractions deterministically
    let merged = merge_states(&extractions);
    info!(
        objective = %merged.objective,
        files = merged.files_touched.len(),
        errors = merged.errors.len(),
        "Merged state from {} chunks", extractions.len()
    );

    // Step 6: Build handoff and render
    let all_recent: Vec<serde_json::Value> = [
        recent_raw,
        normalized.precompacted_items,
        normalized.preserved_tail,
    ].concat();

    let memory = render_durable_memory(&merged, &all_recent, current_request);
    let summary = render_compaction_summary(&memory, current_request);

    info!(
        summary_len = summary.len(),
        "Compaction complete"
    );

    Ok(summary)
}
