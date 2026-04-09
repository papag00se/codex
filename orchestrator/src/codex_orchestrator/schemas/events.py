"""Event types and envelope."""
from __future__ import annotations

import time
from typing import Optional

from pydantic import BaseModel, Field

from .id import generate_id


class Event(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("evt"))
    type: str
    run_id: str
    task_id: Optional[str] = None

    timestamp: int = Field(default_factory=lambda: int(time.time()))
    sequence: int

    data: dict = Field(default_factory=dict)


# Event type constants
RUN_CREATED = "run.created"
PLAN_REQUESTED = "plan.requested"
PLAN_GENERATED = "plan.generated"
TASK_CREATED = "task.created"
TASK_ASSIGNED = "task.assigned"
ROUTE_SELECTED = "route.selected"
TASK_STARTED = "task.started"
TASK_COMPLETED = "task.completed"
TASK_FAILED = "task.failed"
VERIFICATION_REQUESTED = "verification.requested"
VERIFICATION_PASSED = "verification.passed"
VERIFICATION_FAILED = "verification.failed"
APPROVAL_REQUESTED = "approval.requested"
APPROVAL_GRANTED = "approval.granted"
APPROVAL_DENIED = "approval.denied"
RETRY_SCHEDULED = "retry.scheduled"
RUN_PAUSED = "run.paused"
RUN_RESUMED = "run.resumed"
RUN_CANCELLED = "run.cancelled"
RUN_COMPLETED = "run.completed"
ARTIFACT_PUBLISHED = "artifact.published"
