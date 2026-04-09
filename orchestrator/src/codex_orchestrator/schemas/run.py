"""Run and Task data models."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class Run(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("r"))
    goal: str
    status: Literal[
        "planned", "running", "paused", "completed", "failed", "cancelled"
    ] = "planned"

    config_snapshot: dict = Field(default_factory=dict)
    repo_path: str
    base_branch: str
    result_branch: Optional[str] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))
    updated_at: int = Field(default_factory=lambda: int(time.time()))
    started_at: Optional[int] = None
    completed_at: Optional[int] = None

    total_tasks: int = 0
    completed_tasks: int = 0
    failed_tasks: int = 0

    budget_limit: Optional[float] = None
    budget_spent: float = 0.0

    max_iterations: int = 50
    current_iteration: int = 0
    timeout_seconds: int = 7200

    plan_json: Optional[dict] = None
    summary: Optional[str] = None
    error: Optional[str] = None


class Task(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("task"))
    run_id: str
    description: str
    task_type: Literal["code", "review", "test", "plan", "docs"]

    status: Literal[
        "planned", "routed", "assigned", "running",
        "verifying", "awaiting_approval",
        "completed", "failed", "cancelled", "skipped",
    ] = "planned"

    dependencies: list[str] = Field(default_factory=list)
    estimated_complexity: Optional[Literal["low", "medium", "high"]] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))
    updated_at: int = Field(default_factory=lambda: int(time.time()))
    started_at: Optional[int] = None
    completed_at: Optional[int] = None

    assigned_backend: Optional[str] = None
    assigned_model: Optional[str] = None
    worktree_path: Optional[str] = None
    worktree_branch: Optional[str] = None

    max_turns: int = 10
    current_turn: int = 0
    timeout_seconds: int = 900

    retry_count: int = 0
    max_retries: int = 3

    verification_command: Optional[str] = None
    verification_status: Optional[Literal["pass", "fail", "error", "skipped"]] = None

    result_summary: Optional[str] = None
    error: Optional[str] = None

    cost_usd: float = 0.0
    tokens_input: int = 0
    tokens_output: int = 0


# Valid task status transitions — see docs/spec/state-model.md for diagram
VALID_TRANSITIONS: dict[str, set[str]] = {
    "planned": {"routed", "skipped", "cancelled"},
    "routed": {"assigned", "cancelled"},
    "assigned": {"running", "cancelled"},
    "running": {"verifying", "failed", "cancelled"},
    "verifying": {"completed", "awaiting_approval", "failed", "cancelled"},
    "awaiting_approval": {"completed", "failed", "cancelled"},
    "failed": {"planned"},  # retry resets to planned
    "completed": set(),
    "cancelled": set(),
    "skipped": set(),
}


def validate_transition(current: str, target: str) -> bool:
    """Return True if the transition from current to target is valid."""
    return target in VALID_TRANSITIONS.get(current, set())
