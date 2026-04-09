# Initial Data Models

[< Spec Index](index.md) | [Product Index](../product/index.md)

These are the starter Pydantic models for the orchestrator. See [Initial Interfaces and Contracts](initial-interfaces.md) for full field descriptions and sample payloads.

## schemas/run.py

```python
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


# Valid task status transitions
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
    return target in VALID_TRANSITIONS.get(current, set())
```

## schemas/events.py

```python
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
```

## schemas/routing.py

```python
"""Routing decision and provider capability models."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class RoutingDecision(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("rd"))
    task_id: str
    run_id: str

    selected_backend: str
    selected_model: Optional[str] = None
    confidence: float
    reason: str

    eligible_backends: list[str]
    scores: dict[str, float]
    factors: dict

    is_retry: bool = False
    previous_backend: Optional[str] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))


class ProviderCapability(BaseModel):
    provider_id: str
    provider_type: Literal["agent_backend", "api", "local"]
    cost_category: Literal["subscription", "api", "free"]
    access_method: Literal["subprocess", "http"]

    context_window: int
    max_output_tokens: Optional[int] = None
    supports_tool_use: bool = False
    supports_streaming: bool = False

    strengths: dict[str, float] = Field(default_factory=dict)

    cost_per_1k_input: Optional[float] = None
    cost_per_1k_output: Optional[float] = None
    subscription_weight: Optional[float] = None

    max_concurrent: int = 1
    requires_network: bool = True

    health: Literal["healthy", "degraded", "unavailable"] = "healthy"
    last_health_check: Optional[int] = None


class CostEstimate(BaseModel):
    estimated_input_tokens: int
    estimated_output_tokens: int
    estimated_cost_usd: Optional[float] = None
    subscription_units: Optional[float] = None
    cost_category: Literal["api", "subscription", "free"]


class HealthStatus(BaseModel):
    status: Literal["healthy", "degraded", "unavailable"]
    latency_ms: Optional[int] = None
    error: Optional[str] = None
    last_checked: int = Field(default_factory=lambda: int(time.time()))
    consecutive_failures: int = 0
```

## schemas/worker.py

```python
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
```

## schemas/approval.py

```python
"""Approval request model."""
from __future__ import annotations

import time
from typing import Literal, Optional

from pydantic import BaseModel, Field

from .id import generate_id


class ApprovalRequest(BaseModel):
    id: str = Field(default_factory=lambda: generate_id("apr"))
    task_id: str
    run_id: str

    action_type: Literal[
        "shell_exec", "file_delete", "file_write_risky",
        "git_op", "package_install", "network", "secret_access",
    ]
    action_detail: str
    policy_rule: str

    status: Literal["pending", "approved", "denied", "timeout"] = "pending"
    decided_at: Optional[int] = None
    decided_by: Optional[str] = None

    created_at: int = Field(default_factory=lambda: int(time.time()))
    timeout_seconds: int = 300
```

## schemas/artifacts.py

```python
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
```

## schemas/id.py

```python
"""ID generation utility."""
import secrets
import string

_ALPHABET = string.ascii_lowercase + string.digits


def generate_id(prefix: str, length: int = 12) -> str:
    """Generate a prefixed random ID like 'r_a8kx3mp2vq1n'."""
    body = "".join(secrets.choice(_ALPHABET) for _ in range(length))
    return f"{prefix}_{body}"
```
