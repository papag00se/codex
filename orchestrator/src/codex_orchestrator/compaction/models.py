from __future__ import annotations

from typing import Any, Dict, List

from pydantic import BaseModel, Field


class TranscriptChunk(BaseModel):
    chunk_id: int
    start_index: int
    end_index: int
    token_count: int
    overlap_from_previous_tokens: int = 0
    items: List[Dict[str, Any]] = Field(default_factory=list)


class ChunkExtraction(BaseModel):
    chunk_id: int
    objective: str = ""
    repo_state: Dict[str, Any] = Field(default_factory=dict)
    files_touched: List[str] = Field(default_factory=list)
    commands_run: List[str] = Field(default_factory=list)
    errors: List[str] = Field(default_factory=list)
    accepted_fixes: List[str] = Field(default_factory=list)
    rejected_ideas: List[str] = Field(default_factory=list)
    constraints: List[str] = Field(default_factory=list)
    environment_assumptions: List[str] = Field(default_factory=list)
    pending_todos: List[str] = Field(default_factory=list)
    unresolved_bugs: List[str] = Field(default_factory=list)
    test_status: List[str] = Field(default_factory=list)
    external_references: List[str] = Field(default_factory=list)
    latest_plan: List[str] = Field(default_factory=list)
    source_token_count: int = 0


class MergedState(BaseModel):
    objective: str = ""
    repo_state: Dict[str, Any] = Field(default_factory=dict)
    files_touched: List[str] = Field(default_factory=list)
    commands_run: List[str] = Field(default_factory=list)
    errors: List[str] = Field(default_factory=list)
    accepted_fixes: List[str] = Field(default_factory=list)
    rejected_ideas: List[str] = Field(default_factory=list)
    constraints: List[str] = Field(default_factory=list)
    environment_assumptions: List[str] = Field(default_factory=list)
    pending_todos: List[str] = Field(default_factory=list)
    unresolved_bugs: List[str] = Field(default_factory=list)
    test_status: List[str] = Field(default_factory=list)
    external_references: List[str] = Field(default_factory=list)
    latest_plan: List[str] = Field(default_factory=list)
    merged_chunk_count: int = 0


class MergedStatePatch(BaseModel):
    objective_update: str = ""
    repo_state_updates: Dict[str, Any] = Field(default_factory=dict)
    add_files_touched: List[str] = Field(default_factory=list)
    add_commands_run: List[str] = Field(default_factory=list)
    add_errors: List[str] = Field(default_factory=list)
    add_accepted_fixes: List[str] = Field(default_factory=list)
    add_rejected_ideas: List[str] = Field(default_factory=list)
    add_constraints: List[str] = Field(default_factory=list)
    add_environment_assumptions: List[str] = Field(default_factory=list)
    add_pending_todos: List[str] = Field(default_factory=list)
    add_unresolved_bugs: List[str] = Field(default_factory=list)
    add_test_status: List[str] = Field(default_factory=list)
    add_external_references: List[str] = Field(default_factory=list)
    latest_plan_update: List[str] = Field(default_factory=list)


class DurableMemorySet(BaseModel):
    task_state: str = ""
    decisions: str = ""
    failures_to_avoid: str = ""
    next_steps: str = ""
    session_handoff: str = ""


class SessionHandoff(BaseModel):
    stable_task_definition: str = ""
    repo_state: Dict[str, Any] = Field(default_factory=dict)
    key_decisions: List[str] = Field(default_factory=list)
    unresolved_work: List[str] = Field(default_factory=list)
    latest_plan: List[str] = Field(default_factory=list)
    failures_to_avoid: List[str] = Field(default_factory=list)
    recent_raw_turns: List[Dict[str, Any]] = Field(default_factory=list)
    current_request: str = ""


class CodexHandoffFlow(BaseModel):
    durable_memory: List[Dict[str, str]] = Field(default_factory=list)
    structured_handoff: Dict[str, Any] = Field(default_factory=dict)
    recent_raw_turns: List[Dict[str, Any]] = Field(default_factory=list)
    current_request: str = ""
