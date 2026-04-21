//! Prompt-aware transcript chunking — ported from compaction/chunking.py.
//! Splits compactable items at transcript-item boundaries using
//! token budget estimation.

use super::models::TranscriptChunk;
use crate::metrics::estimate_tokens;

/// Split transcript items into chunks by token budget.
pub fn chunk_items(
    items: &[serde_json::Value],
    target_tokens: usize,
    max_tokens: usize,
    overlap_tokens: usize,
) -> Vec<TranscriptChunk> {
    if items.is_empty() {
        return Vec::new();
    }

    let token_counts: Vec<usize> = items
        .iter()
        .map(|item| estimate_tokens(&serde_json::to_string(item).unwrap_or_default()))
        .collect();

    let mut chunks = Vec::new();
    let mut start = 0;
    let mut chunk_id = 1;

    while start < items.len() {
        let mut end = start;
        let mut token_total = 0;

        while end < items.len() {
            let candidate = token_total + token_counts[end];
            if end > start && candidate > target_tokens {
                break;
            }
            if candidate > max_tokens && end > start {
                break;
            }
            token_total = candidate;
            end += 1;
            if token_total >= target_tokens {
                break;
            }
        }

        let overlap_used = if start > 0 {
            overlap_size(&token_counts, start, end)
        } else {
            0
        };

        chunks.push(TranscriptChunk {
            chunk_id,
            start_index: start,
            end_index: end,
            token_count: token_total,
            overlap_from_previous_tokens: overlap_used,
            items: items[start..end].to_vec(),
        });

        chunk_id += 1;

        if end >= items.len() {
            break;
        }

        let next_start = next_chunk_start(&token_counts, start, end, overlap_tokens);
        start = if next_start > start { next_start } else { end };
    }

    chunks
}

/// Split recent raw turns from the end of items.
pub fn split_recent_raw(
    items: &[serde_json::Value],
    keep_tokens: usize,
) -> (Vec<serde_json::Value>, Vec<serde_json::Value>) {
    if keep_tokens == 0 || items.is_empty() {
        return (items.to_vec(), Vec::new());
    }

    let mut kept_tokens = 0;
    let mut split_index = items.len();

    for i in (0..items.len()).rev() {
        let item_tokens = estimate_tokens(&serde_json::to_string(&items[i]).unwrap_or_default());
        if kept_tokens > 0 && kept_tokens + item_tokens > keep_tokens {
            break;
        }
        kept_tokens += item_tokens;
        split_index = i;
        if kept_tokens >= keep_tokens {
            break;
        }
    }

    (items[..split_index].to_vec(), items[split_index..].to_vec())
}

fn overlap_size(token_counts: &[usize], start: usize, end: usize) -> usize {
    if start >= end {
        return 0;
    }
    token_counts[start..end].iter().sum()
}

fn next_chunk_start(
    token_counts: &[usize],
    _start: usize,
    end: usize,
    overlap_tokens: usize,
) -> usize {
    let mut carried = 0;
    let mut overlap_start = end;
    for i in (0..end).rev() {
        if carried + token_counts[i] > overlap_tokens {
            break;
        }
        carried += token_counts[i];
        overlap_start = i;
    }
    overlap_start
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_chunking() {
        let items: Vec<serde_json::Value> = (0..10)
            .map(|i| serde_json::json!({"role": "user", "content": format!("message {} {}", i, "x".repeat(50))}))
            .collect();
        let chunks = chunk_items(&items, 200, 400, 50);
        assert!(!chunks.is_empty());
        // All items should be covered
        let covered: std::collections::HashSet<usize> = chunks
            .iter()
            .flat_map(|c| c.start_index..c.end_index)
            .collect();
        assert_eq!(covered.len(), 10);
    }

    #[test]
    fn test_single_item() {
        let items = vec![serde_json::json!({"role": "user", "content": "hello"})];
        let chunks = chunk_items(&items, 100, 200, 0);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].chunk_id, 1);
    }

    #[test]
    fn test_split_recent_raw() {
        let items: Vec<serde_json::Value> = (0..10)
            .map(|i| serde_json::json!({"role": "user", "content": format!("msg {i}")}))
            .collect();
        let (compactable, recent) = split_recent_raw(&items, 20);
        assert!(!recent.is_empty());
        assert_eq!(compactable.len() + recent.len(), 10);
    }
}
