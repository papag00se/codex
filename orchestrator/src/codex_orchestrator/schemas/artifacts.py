"""Artifact record model."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class ArtifactRecord(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("art"))
    task_id: str
    run_id: str

    artifact_type: Literal["diff", "file", "test_report", "review", "log"]
    name: str
    path: str
    size_bytes: Optional[int] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))
