# Initial Interfaces and Contracts

[< Spec Index](index.md) | [Product Index](../product/index.md)

These types are used throughout the [Orchestrator](operational-model.md), [Routing Engine](routing-architecture.md), and [State Store](state-model.md).

## Run

```python
from pydantic import BaseModel, Field
from typing import Optional, Literal
from datetime import datetime
import nanoid

class Run(BaseModel):
    id: str = Field(default_factory=lambda: f"r_{nanoid.generate(size=12)}")
    goal: str
    status: Literal["planned", "running", "paused", "completed", "failed", "cancelled"] = "planned"
    
    config_snapshot: dict = Field(default_factory=dict)
    repo_path: str
    base_branch: str
    result_branch: Optional[str] = None
    
    created_at: int  # Unix seconds
    updated_at: int
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
```

**Sample payload:**
```json
{
  "id": "r_a8Kx3mP2vQ1n",
  "goal": "Add rate limiting to the API gateway with Redis backend",
  "status": "running",
  "repo_path": "/home/user/project",
  "base_branch": "main",
  "result_branch": "codex/r_a8Kx3mP2vQ1n/result",
  "created_at": 1712600000,
  "updated_at": 1712600120,
  "started_at": 1712600005,
  "total_tasks": 7,
  "completed_tasks": 3,
  "budget_limit": 5.0,
  "budget_spent": 0.28,
  "max_iterations": 50,
  "current_iteration": 12
}
```

## Task

```python
class Task(BaseModel):
    id: str = Field(default_factory=lambda: f"task_{nanoid.generate(size=8)}")
    run_id: str
    description: str
    task_type: Literal["code", "review", "test", "plan", "docs"]
    
    status: Literal[
        "planned", "routed", "assigned", "running",
        "verifying", "awaiting_approval",
        "completed", "failed", "cancelled", "skipped"
    ] = "planned"
    
    dependencies: list[str] = Field(default_factory=list)
    estimated_complexity: Optional[Literal["low", "medium", "high"]] = None
    
    created_at: int
    updated_at: int
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
```

**Sample payload:**
```json
{
  "id": "task_Kx3mP2vQ",
  "run_id": "r_a8Kx3mP2vQ1n",
  "description": "Create Redis client wrapper in src/clients/redis.py with connection pooling and retry logic",
  "task_type": "code",
  "status": "completed",
  "dependencies": [],
  "estimated_complexity": "medium",
  "assigned_backend": "claude-code",
  "assigned_model": "claude-opus-4",
  "worktree_path": "/home/user/project/.codex-worktrees/r_a8Kx3mP2vQ1n/task_Kx3mP2vQ",
  "verification_command": "pytest tests/test_redis.py -v",
  "verification_status": "pass",
  "cost_usd": 0.12,
  "tokens_input": 3200,
  "tokens_output": 1800
}
```

## RoutingDecision

```python
class RoutingDecision(BaseModel):
    id: str = Field(default_factory=lambda: f"rd_{nanoid.generate(size=10)}")
    task_id: str
    run_id: str
    
    selected_backend: str
    selected_model: Optional[str] = None
    confidence: float  # 0.0 - 1.0
    reason: str
    
    eligible_backends: list[str]
    scores: dict[str, float]
    factors: dict  # Routing input features
    
    is_retry: bool = False
    previous_backend: Optional[str] = None
    
    created_at: int
```

**Sample payload:**
```json
{
  "id": "rd_mP2vQ1nX8a",
  "task_id": "task_Kx3mP2vQ",
  "run_id": "r_a8Kx3mP2vQ1n",
  "selected_backend": "claude-code",
  "selected_model": "claude-opus-4",
  "confidence": 0.92,
  "reason": "Complex integration task requiring tool use and multi-file editing. Claude Code scored highest for code_generation (0.95) with tool_use support.",
  "eligible_backends": ["claude-code", "openai-api", "ollama"],
  "scores": {"claude-code": 0.92, "openai-api": 0.78, "ollama": 0.45},
  "factors": {
    "task_type": "code",
    "estimated_complexity": "medium",
    "requires_tool_use": true,
    "privacy_sensitive": false,
    "budget_remaining_usd": 4.72
  }
}
```

## ProviderCapability

```python
class ProviderCapability(BaseModel):
    provider_id: str
    provider_type: Literal["agent_backend", "api", "local"]
    cost_category: Literal["subscription", "api", "free"]
    access_method: Literal["subprocess", "http"]
    
    context_window: int
    max_output_tokens: Optional[int] = None
    supports_tool_use: bool = False
    supports_streaming: bool = False
    
    strengths: dict[str, float] = Field(default_factory=dict)  # task_type → score 0.0-1.0
    
    cost_per_1k_input: Optional[float] = None
    cost_per_1k_output: Optional[float] = None
    subscription_weight: Optional[float] = None
    
    max_concurrent: int = 1
    requires_network: bool = True
    
    health: Literal["healthy", "degraded", "unavailable"] = "healthy"
    last_health_check: Optional[int] = None
```

## ProviderAdapter

```python
from abc import ABC, abstractmethod

class ProviderAdapter(ABC):
    @abstractmethod
    async def execute_task(self, task: Task, context: RepositoryContext) -> WorkerResult: ...
    
    @abstractmethod
    async def capabilities(self) -> ProviderCapability: ...
    
    @abstractmethod
    async def health(self) -> HealthStatus: ...
    
    @abstractmethod
    async def estimate_cost(self, task: Task) -> CostEstimate: ...
    
    @abstractmethod
    async def cancel(self, task_id: str) -> None: ...
```

## ApprovalRequest

```python
class ApprovalRequest(BaseModel):
    id: str = Field(default_factory=lambda: f"apr_{nanoid.generate(size=8)}")
    task_id: str
    run_id: str
    
    action_type: Literal["shell_exec", "file_delete", "file_write_risky", "git_op", "package_install", "network", "secret_access"]
    action_detail: str
    policy_rule: str
    
    status: Literal["pending", "approved", "denied", "timeout"] = "pending"
    decided_at: Optional[int] = None
    decided_by: Optional[str] = None  # "user" or "timeout_default"
    
    created_at: int
    timeout_seconds: int = 300
```

**Sample payload:**
```json
{
  "id": "apr_vQ1nX8aK",
  "task_id": "task_P2vQx3mK",
  "run_id": "r_a8Kx3mP2vQ1n",
  "action_type": "package_install",
  "action_detail": "npm install redis@^4.0.0",
  "policy_rule": "approval.policy.shell.require_approval contains 'npm install'",
  "status": "approved",
  "decided_at": 1712600145,
  "decided_by": "user"
}
```

## ArtifactRecord

```python
class ArtifactRecord(BaseModel):
    id: str = Field(default_factory=lambda: f"art_{nanoid.generate(size=8)}")
    task_id: str
    run_id: str
    
    artifact_type: Literal["diff", "file", "test_report", "review", "log"]
    name: str
    path: str
    size_bytes: Optional[int] = None
    
    created_at: int
```

## RepositoryContext

```python
class RepositoryContext(BaseModel):
    repo_path: str
    worktree_path: str
    branch: str
    base_branch: str
    
    languages: list[str] = Field(default_factory=list)
    file_count: Optional[int] = None
    
    recent_commits: Optional[list[str]] = None  # Last 5 commit summaries
    changed_files: Optional[list[str]] = None    # Files changed in current branch
```

## WorkerResult

```python
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
    
    artifacts: list[str] = Field(default_factory=list)  # Artifact file paths
    
    provider_metadata: dict = Field(default_factory=dict)  # Provider-specific data
```

**Sample payload:**
```json
{
  "task_id": "task_Kx3mP2vQ",
  "status": "success",
  "files_changed": ["src/clients/redis.py", "tests/test_redis.py"],
  "tokens_input": 3200,
  "tokens_output": 1800,
  "cost_usd": 0.12,
  "duration_seconds": 62.4,
  "turns_used": 5,
  "artifacts": ["task_Kx3mP2vQ_diff.patch", "task_Kx3mP2vQ_log.jsonl"]
}
```

## Event

```python
class Event(BaseModel):
    id: str = Field(default_factory=lambda: f"evt_{nanoid.generate(size=12)}")
    type: str  # e.g., "run.created", "task.completed"
    run_id: str
    task_id: Optional[str] = None
    
    timestamp: int  # Unix seconds
    sequence: int    # Monotonic per run
    
    data: dict       # Event-type-specific payload
```

**Sample payload:**
```json
{
  "id": "evt_a8Kx3mP2vQ1n",
  "type": "task.completed",
  "run_id": "r_a8Kx3mP2vQ1n",
  "task_id": "task_Kx3mP2vQ",
  "timestamp": 1712600067,
  "sequence": 15,
  "data": {
    "status": "success",
    "files_changed": ["src/clients/redis.py"],
    "cost_usd": 0.12,
    "duration_seconds": 62.4
  }
}
```
