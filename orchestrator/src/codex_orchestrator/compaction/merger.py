from __future__ import annotations

from typing import Iterable, List, Union

from .models import ChunkExtraction, MergedState


StateLike = Union[ChunkExtraction, MergedState]


def merge_states(states: Iterable[StateLike]) -> MergedState:
    items = list(states)
    merged = MergedState()
    if not items:
        return merged

    merged.objective = _latest_non_empty(state.objective for state in items)
    merged.repo_state = _merge_repo_state(state.repo_state for state in items)
    merged.files_touched = _merge_unique(state.files_touched for state in items)
    merged.commands_run = _merge_unique(state.commands_run for state in items)
    merged.errors = _merge_unique(state.errors for state in items)
    merged.accepted_fixes = _merge_unique(state.accepted_fixes for state in items)
    merged.rejected_ideas = _merge_unique(state.rejected_ideas for state in items)
    merged.constraints = _merge_unique(state.constraints for state in items)
    merged.environment_assumptions = _merge_unique(state.environment_assumptions for state in items)
    merged.pending_todos = _merge_unique(state.pending_todos for state in items)
    merged.unresolved_bugs = _merge_unique(state.unresolved_bugs for state in items)
    merged.test_status = _merge_unique(state.test_status for state in items)
    merged.external_references = _merge_unique(state.external_references for state in items)
    merged.latest_plan = _latest_non_empty_list(state.latest_plan for state in items)
    merged.merged_chunk_count = sum(getattr(state, "merged_chunk_count", 1) for state in items)
    return merged


def _latest_non_empty(values: Iterable[str]) -> str:
    latest = ""
    for value in values:
        if value:
            latest = value
    return latest


def _latest_non_empty_list(values: Iterable[List[str]]) -> List[str]:
    latest: List[str] = []
    for value in values:
        if value:
            latest = list(value)
    return latest


def _merge_repo_state(values: Iterable[dict]) -> dict:
    merged: dict = {}
    for value in values:
        if value:
            merged.update(value)
    return merged


def _merge_unique(groups: Iterable[Iterable[str]]) -> List[str]:
    seen: set[str] = set()
    merged: List[str] = []
    items = list(groups)
    for group in reversed(items):
        for item in group or []:
            key = item.strip().lower()
            if not key or key in seen:
                continue
            seen.add(key)
            merged.append(item)
    return merged
