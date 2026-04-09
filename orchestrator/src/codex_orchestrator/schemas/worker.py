"""Worker result and repository context models."""
from __future__ import annotations

from typing import Literal, Optional

from pydantic import BaseModel, Field


class WorkerResult(BaseModel):
    task_id: str
    status: Literal["success", "failure", "timeout", "cancelled"]

    files_changed: list[str] = Field(default_factory=list)
    diff: Optional[str] = None

    output_text: Optional[str] = None
    error_text: Optional[str] = None

    tokens_input: int = 0
    tokens_output: int = 0
    cost_usd: float = 0.0
    duration_seconds: float = 0.0
    turns_used: int = 0

    artifacts: list[str] = Field(default_factory=list)
    provider_metadata: dict = Field(default_factory=dict)


class RepositoryContext(BaseModel):
    repo_path: str
    worktree_path: str
    branch: str
    base_branch: str

    languages: list[str] = Field(default_factory=list)
    file_count: Optional[int] = None
    recent_commits: Optional[list[str]] = None
    changed_files: Optional[list[str]] = None
