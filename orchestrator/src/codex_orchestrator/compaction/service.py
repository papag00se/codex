from __future__ import annotations

import logging
from typing import Any, Callable, Dict, List, Optional

from ..schemas.config import settings
from ..routing.metrics import estimate_tokens
from .chunking import chunk_transcript_items_by_prompt, split_recent_raw_turns
from .durable_memory import build_session_handoff, render_durable_memory
from .extractor import CompactionExtractor
from .handoff import build_codex_handoff_flow, validate_codex_handoff_flow
from .merger import merge_states
from .normalize import normalize_transcript_for_compaction
from .models import CodexHandoffFlow, MergedState, SessionHandoff
from .prompts import estimate_extraction_request_tokens, estimate_refinement_request_tokens
from .refiner import CompactionRefiner
from .storage import CompactionStorage

_REFINEMENT_RECENT_RAW_TARGET_TOKENS = 8000
logger = logging.getLogger(__name__)


class CompactionService:
    def __init__(
        self,
        extractor: Optional[CompactionExtractor] = None,
        refiner: Optional[CompactionRefiner] = None,
        storage: Optional[CompactionStorage] = None,
    ) -> None:
        self.extractor = extractor or CompactionExtractor()
        self.refiner = refiner or CompactionRefiner()
        self.storage = storage or CompactionStorage()

    def compact_transcript(
        self,
        session_id: str,
        items: List[Dict[str, Any]],
        *,
        current_request: str,
        repo_context: Optional[Dict[str, Any]] = None,
        progress_callback: Optional[Callable[..., None]] = None,
    ) -> SessionHandoff:
        hard_chunk_tokens = self._extraction_hard_chunk_tokens()
        normalized = normalize_transcript_for_compaction(items, max_item_tokens=hard_chunk_tokens)
        _emit_progress(
            progress_callback,
            'normalize_completed',
            compactable_count=len(normalized.compactable_items),
            precompacted_count=len(normalized.precompacted_items),
            preserved_tail_count=len(normalized.preserved_tail),
        )
        compactable, recent_raw_turns = split_recent_raw_turns(normalized.compactable_items, settings.compactor_keep_raw_tokens)
        recent_raw_turns = _merge_recent_raw_turns(recent_raw_turns, normalized.precompacted_items)
        recent_raw_turns = _merge_recent_raw_turns(recent_raw_turns, normalized.preserved_tail)
        extraction_prompt_limit = self._extraction_max_prompt_tokens()
        chunks, skipped_items = chunk_transcript_items_by_prompt(
            compactable,
            target_prompt_tokens=extraction_prompt_limit,
            max_prompt_tokens=extraction_prompt_limit,
            overlap_tokens=settings.compactor_overlap_tokens,
            prompt_token_counter=lambda chunk: estimate_extraction_request_tokens(
                chunk,
                repo_context,
                model=self.extractor.model,
            ),
            max_chunk_tokens=hard_chunk_tokens,
        )
        _emit_progress(
            progress_callback,
            'chunking_completed',
            chunk_count=len(chunks),
            skipped_item_count=len(skipped_items),
            max_prompt_tokens=extraction_prompt_limit,
            max_chunk_tokens=hard_chunk_tokens,
        )
        if skipped_items:
            recent_raw_turns = _merge_recent_raw_turns(recent_raw_turns, skipped_items)
        extractions = []
        for index, chunk in enumerate(chunks, start=1):
            _emit_progress(
                progress_callback,
                'extract_chunk_started',
                chunk_id=chunk.chunk_id,
                chunk_index=index,
                chunk_count=len(chunks),
                chunk_tokens=chunk.token_count,
            )
            extractions.append(self.extractor.extract_chunk(chunk, repo_context))
            _emit_progress(
                progress_callback,
                'extract_chunk_completed',
                chunk_id=chunk.chunk_id,
                chunk_index=index,
                chunk_count=len(chunks),
            )
        merged = merge_states(extractions)
        _emit_progress(progress_callback, 'merge_completed', merged_chunk_count=merged.merged_chunk_count)
        refined = self._refine_merged_state(
            merged,
            recent_raw_turns,
            current_request=current_request,
            repo_context=repo_context,
            progress_callback=progress_callback,
        )
        final_recent_raw_turns = _merge_recent_raw_turns(recent_raw_turns, normalized.preserved_tail)
        final_recent_raw_turns = [_strip_compaction_fields(item) for item in final_recent_raw_turns]
        memory = render_durable_memory(refined, final_recent_raw_turns, current_request)
        handoff = build_session_handoff(refined, final_recent_raw_turns, current_request)
        _emit_progress(progress_callback, 'render_completed')

        self.storage.save_chunk_extractions(session_id, extractions)
        self.storage.save_merged_state(session_id, merged)
        self.storage.save_refined_state(session_id, refined)
        self.storage.save_durable_memory(session_id, memory)
        self.storage.save_handoff(session_id, handoff)
        return handoff

    def load_latest_handoff(self, session_id: str) -> Optional[SessionHandoff]:
        try:
            return self.storage.load_handoff(session_id)
        except Exception:
            logger.exception("failed to load compaction handoff for session %s", session_id)
            return None

    def build_codex_handoff_flow(self, session_id: str, *, current_request: Optional[str] = None) -> Optional[CodexHandoffFlow]:
        try:
            handoff = self.storage.load_handoff(session_id)
            memory = self.storage.load_durable_memory(session_id)
            if handoff is None or memory is None:
                return None
            return validate_codex_handoff_flow(build_codex_handoff_flow(memory, handoff, current_request=current_request))
        except Exception:
            logger.exception("failed to build codex handoff flow for session %s", session_id)
            return None

    def refresh_if_needed(
        self,
        session_id: str,
        items: List[Dict[str, Any]],
        *,
        current_request: str,
        repo_context: Optional[Dict[str, Any]] = None,
        progress_callback: Optional[Callable[..., None]] = None,
    ) -> SessionHandoff:
        if estimate_tokens(items) < self._extraction_hard_chunk_tokens():
            existing = self.load_latest_handoff(session_id)
            if existing is not None:
                return existing
        return self.compact_transcript(
            session_id,
            items,
            current_request=current_request,
            repo_context=repo_context,
            progress_callback=progress_callback,
        )

    def _refine_merged_state(
        self,
        merged: MergedState,
        recent_raw_turns: List[Dict[str, Any]],
        *,
        current_request: str,
        repo_context: Optional[Dict[str, Any]] = None,
        progress_callback: Optional[Callable[..., None]] = None,
    ) -> MergedState:
        if not recent_raw_turns:
            return merged

        state = merged
        remaining = list(recent_raw_turns)
        iteration = 0
        max_prompt_tokens = self._refinement_max_prompt_tokens()
        target_prompt_tokens = self._refinement_target_prompt_tokens(max_prompt_tokens)
        target_recent_raw_tokens = self._refinement_target_recent_raw_tokens()
        total_iterations = self._estimate_refinement_iterations(
            remaining,
            merged,
            target_prompt_tokens=target_prompt_tokens,
            max_prompt_tokens=max_prompt_tokens,
            target_recent_raw_tokens=target_recent_raw_tokens,
            current_request=current_request,
            repo_context=repo_context,
        )
        base_prompt_tokens = estimate_refinement_request_tokens(
            state,
            [],
            current_request,
            repo_context,
            model=self.refiner.model,
        )
        if base_prompt_tokens > max_prompt_tokens:
            _emit_progress(
                progress_callback,
                'refine_skipped',
                reason='base_prompt_exceeds_budget',
                prompt_tokens=base_prompt_tokens,
                max_prompt_tokens=max_prompt_tokens,
            )
            return merged
        while remaining:
            chunk_items, next_index, skipped_oversize = _take_items_with_prompt_budget(
                remaining,
                target_prompt_tokens=target_prompt_tokens,
                max_prompt_tokens=max_prompt_tokens,
                target_item_tokens=target_recent_raw_tokens,
                item_token_counter=estimate_tokens,
                prompt_token_counter=lambda candidate_items: estimate_refinement_request_tokens(
                    state,
                    candidate_items,
                    current_request,
                    repo_context,
                    model=self.refiner.model,
                ),
            )
            if skipped_oversize:
                _emit_progress(
                    progress_callback,
                    'refine_item_skipped',
                    reason='prompt_exceeds_budget',
                    skipped_count=next_index,
                )
                remaining = remaining[next_index:]
                continue
            if not chunk_items:
                break
            iteration += 1
            _emit_progress(
                progress_callback,
                'refine_iteration_started',
                iteration=iteration,
                iteration_count=total_iterations,
                recent_turn_count=len(chunk_items),
            )
            state = self.refiner.refine_state(
                state,
                chunk_items,
                current_request=current_request,
                repo_context=repo_context,
            )
            remaining = remaining[next_index:]
            _emit_progress(
                progress_callback,
                'refine_iteration_completed',
                iteration=iteration,
                iteration_count=total_iterations,
            )
        return state

    def _refinement_max_prompt_tokens(self) -> int:
        configured = getattr(settings, 'compactor_max_prompt_tokens', self.refiner.max_prompt_tokens())
        return max(1, min(configured, self.refiner.max_prompt_tokens()))

    def _refinement_target_prompt_tokens(self, max_prompt_tokens: int) -> int:
        return max_prompt_tokens

    def _extraction_hard_chunk_tokens(self) -> int:
        configured = getattr(settings, 'compactor_target_chunk_tokens', self.extractor.target_prompt_tokens())
        legacy_max = getattr(settings, 'compactor_max_chunk_tokens', configured)
        return max(1, min(configured, legacy_max))

    def _extraction_max_prompt_tokens(self) -> int:
        configured = getattr(settings, 'compactor_max_prompt_tokens', self.extractor.max_prompt_tokens())
        return max(1, min(configured, self.extractor.max_prompt_tokens()))

    def _refinement_target_recent_raw_tokens(self) -> int:
        return _REFINEMENT_RECENT_RAW_TARGET_TOKENS

    def _estimate_refinement_iterations(
        self,
        items: List[Dict[str, Any]],
        state: MergedState,
        *,
        target_prompt_tokens: int,
        max_prompt_tokens: int,
        target_recent_raw_tokens: int,
        current_request: str,
        repo_context: Optional[Dict[str, Any]] = None,
    ) -> int:
        remaining = list(items)
        iterations = 0
        base_tokens = estimate_refinement_request_tokens(
            state,
            [],
            current_request,
            repo_context,
            model=self.refiner.model,
        )
        if base_tokens > max_prompt_tokens:
            return 0
        while remaining:
            chunk_items, next_index, skipped_oversize = _take_items_with_prompt_budget(
                remaining,
                target_prompt_tokens=target_prompt_tokens,
                max_prompt_tokens=max_prompt_tokens,
                target_item_tokens=target_recent_raw_tokens,
                item_token_counter=estimate_tokens,
                prompt_token_counter=lambda candidate_items: estimate_refinement_request_tokens(
                    state,
                    candidate_items,
                    current_request,
                    repo_context,
                    model=self.refiner.model,
                ),
            )
            if skipped_oversize:
                remaining = remaining[next_index:]
                continue
            if not chunk_items:
                break
            iterations += 1
            remaining = remaining[next_index:]
        return iterations


def _merge_recent_raw_turns(existing: List[Dict[str, Any]], preserved_tail: List[Dict[str, Any]]) -> List[Dict[str, Any]]:
    merged = list(existing)
    for item in preserved_tail:
        if item not in merged:
            merged.append(item)
    return sorted(merged, key=_compaction_sort_key)


def _compaction_sort_key(item: Dict[str, Any]) -> tuple[int, str]:
    index = item.get('_compaction_index') if isinstance(item, dict) else None
    if isinstance(index, int):
        return index, ''
    return 1_000_000_000, str(item)


def _strip_compaction_fields(item: Dict[str, Any]) -> Dict[str, Any]:
    if not isinstance(item, dict):
        return item
    return {key: value for key, value in item.items() if not str(key).startswith('_compaction_')}


def _take_items_with_prompt_budget(
    items: List[Dict[str, Any]],
    *,
    target_prompt_tokens: int,
    max_prompt_tokens: int,
    target_item_tokens: Optional[int] = None,
    item_token_counter: Optional[Callable[[List[Dict[str, Any]]], int]] = None,
    prompt_token_counter: Callable[[List[Dict[str, Any]]], int],
) -> tuple[List[Dict[str, Any]], int, bool]:
    best_end = 0
    for end in range(len(items)):
        candidate_items = items[: end + 1]
        if target_item_tokens is not None and item_token_counter is not None:
            item_tokens = item_token_counter(candidate_items)
            if item_tokens > target_item_tokens and end > 0:
                break
        prompt_tokens = prompt_token_counter(candidate_items)
        if prompt_tokens > max_prompt_tokens:
            break
        best_end = end + 1
        if prompt_tokens >= target_prompt_tokens:
            break
        if (
            target_item_tokens is not None
            and item_token_counter is not None
            and item_tokens >= target_item_tokens
        ):
            break
    if best_end == 0:
        return [], 1 if items else 0, bool(items)
    return items[:best_end], best_end, False


def _emit_progress(
    progress_callback: Optional[Callable[..., None]],
    stage: str,
    **fields: Any,
) -> None:
    if progress_callback is not None:
        progress_callback(stage, **fields)
