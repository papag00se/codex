from __future__ import annotations

from typing import Any, Callable, Dict, List, Tuple

from ..routing.metrics import estimate_tokens
from .models import TranscriptChunk


def chunk_transcript_items(
    items: List[Dict[str, Any]],
    *,
    target_tokens: int,
    max_tokens: int,
    overlap_tokens: int,
) -> List[TranscriptChunk]:
    if not items:
        return []

    token_counts = [_item_token_count(item) for item in items]
    chunks: List[TranscriptChunk] = []
    start = 0
    chunk_id = 1

    while start < len(items):
        end = start
        token_total = 0
        while end < len(items):
            candidate = token_total + token_counts[end]
            if end > start and candidate > target_tokens:
                break
            if candidate > max_tokens and end > start:
                break
            token_total = candidate
            end += 1
            if token_total >= target_tokens:
                break

        overlap_used = 0 if start == 0 else _overlap_size(token_counts, start, end)
        chunks.append(
            TranscriptChunk(
                chunk_id=chunk_id,
                start_index=start,
                end_index=end,
                token_count=token_total,
                overlap_from_previous_tokens=overlap_used,
                items=items[start:end],
            )
        )
        chunk_id += 1
        if end >= len(items):
            break
        next_start = _next_start(token_counts, start, end, overlap_tokens)
        start = next_start if next_start > start else end

    return chunks


def chunk_transcript_items_by_prompt(
    items: List[Dict[str, Any]],
    *,
    target_prompt_tokens: int,
    max_prompt_tokens: int,
    overlap_tokens: int,
    prompt_token_counter: Callable[[TranscriptChunk], int],
    max_chunk_tokens: int | None = None,
) -> tuple[List[TranscriptChunk], List[Dict[str, Any]]]:
    if not items:
        return [], []

    token_counts = [_item_token_count(item) for item in items]
    chunks: List[TranscriptChunk] = []
    skipped_items: List[Dict[str, Any]] = []
    start = 0
    chunk_id = 1

    while start < len(items):
        end = start
        token_total = 0
        best_end = start
        best_token_total = 0

        while end < len(items):
            token_total += token_counts[end]
            if max_chunk_tokens is not None and token_total > max_chunk_tokens:
                break
            candidate_chunk = TranscriptChunk(
                chunk_id=chunk_id,
                start_index=start,
                end_index=end + 1,
                token_count=token_total,
                overlap_from_previous_tokens=0 if start == 0 else _overlap_size(token_counts, start, end + 1),
                items=items[start : end + 1],
            )
            prompt_tokens = prompt_token_counter(candidate_chunk)
            if prompt_tokens > max_prompt_tokens:
                break
            best_end = end + 1
            best_token_total = token_total
            if prompt_tokens >= target_prompt_tokens:
                break
            end += 1

        if best_end == start:
            skipped_items.append(items[start])
            start += 1
            continue

        overlap_used = 0 if start == 0 else _overlap_size(token_counts, start, best_end)
        chunks.append(
            TranscriptChunk(
                chunk_id=chunk_id,
                start_index=start,
                end_index=best_end,
                token_count=best_token_total,
                overlap_from_previous_tokens=overlap_used,
                items=items[start:best_end],
            )
        )
        chunk_id += 1
        if best_end >= len(items):
            break
        next_start = _next_start(token_counts, start, best_end, overlap_tokens)
        start = next_start if next_start > start else best_end

    return chunks, skipped_items


def split_recent_raw_turns(items: List[Dict[str, Any]], keep_tokens: int) -> Tuple[List[Dict[str, Any]], List[Dict[str, Any]]]:
    if keep_tokens <= 0 or not items:
        return items, []

    kept_tokens = 0
    split_index = len(items)
    for index in range(len(items) - 1, -1, -1):
        item_tokens = _item_token_count(items[index])
        if kept_tokens > 0 and kept_tokens + item_tokens > keep_tokens:
            break
        kept_tokens += item_tokens
        split_index = index
        if kept_tokens >= keep_tokens:
            break
    return items[:split_index], items[split_index:]


def _item_token_count(item: Dict[str, Any]) -> int:
    if not isinstance(item, dict):
        return estimate_tokens(item)
    compactable = {key: value for key, value in item.items() if not str(key).startswith('_compaction_')}
    return estimate_tokens(compactable)


def _next_start(token_counts: List[int], start: int, end: int, overlap_tokens: int) -> int:
    overlap_start = end
    carried = 0
    while overlap_start > start and carried + token_counts[overlap_start - 1] <= overlap_tokens:
        overlap_start -= 1
        carried += token_counts[overlap_start]
    return overlap_start if overlap_start > start else end


def _overlap_size(token_counts: List[int], start: int, end: int) -> int:
    if start >= end:
        return 0
    return sum(token_counts[start:end])
