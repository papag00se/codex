# Event Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Overview

All events are persisted to an append-only JSONL file per run (stored alongside the [SQLite state](state-model.md)): `~/.codex/multi-agent/runs/<run-id>/events.jsonl`

Every event has a common envelope:

```json
{
  "id": "evt_01HZ...",
  "type": "task.completed",
  "run_id": "r_abc123",
  "task_id": "task_3",
  "timestamp": 1712600000,
  "sequence": 42,
  "data": { ... }
}
```

`sequence` is a monotonically increasing integer per run. Used for ordering and idempotency.

## Event Types

### run.created

**Producer:** CLI (when user starts a new run)
**Consumers:** Orchestrator (starts supervisor loop), Observability (log)
**Payload:**
```json
{
  "goal": "Add rate limiting with Redis",
  "config_snapshot": { ... },
  "repo_path": "/home/user/project",
  "base_branch": "main"
}
```
**Idempotency:** Run ID is generated once. Duplicate detection by run ID.

---

### plan.requested

**Producer:** Orchestrator (after run.created)
**Consumers:** Planner agent, Observability
**Payload:**
```json
{
  "goal": "Add rate limiting with Redis",
  "repo_context": {
    "languages": ["python", "yaml"],
    "file_count": 234,
    "recent_commits": 5
  }
}
```
**Idempotency:** One plan.requested per run (or per replan). Sequence number prevents duplicates.

---

### plan.generated

**Producer:** Planner agent
**Consumers:** Orchestrator (creates tasks), Observability
**Payload:**
```json
{
  "tasks": [
    {
      "id": "task_1",
      "description": "Create Redis client wrapper",
      "type": "code",
      "dependencies": [],
      "estimated_complexity": "medium"
    }
  ],
  "task_count": 7,
  "planning_backend": "claude-code",
  "planning_tokens": 4500
}
```
**Idempotency:** Plan is immutable once generated. Replanning generates a new event with a new sequence.

---

### task.created

**Producer:** Orchestrator (after plan.generated)
**Consumers:** State store, Observability
**Payload:**
```json
{
  "task_id": "task_1",
  "description": "Create Redis client wrapper",
  "type": "code",
  "dependencies": [],
  "estimated_complexity": "medium"
}
```
**Idempotency:** Task ID is unique. Duplicate creation ignored.

---

### task.assigned

**Producer:** Orchestrator (after routing)
**Consumers:** State store, Observability
**Payload:**
```json
{
  "task_id": "task_1",
  "backend": "claude-code",
  "model": "claude-opus-4",
  "worktree_path": "/home/user/project/.codex-worktrees/r_abc123/task_1"
}
```
**Idempotency:** Assignment is idempotent — reassigning to same backend is a no-op.

---

### route.selected

**Producer:** Routing engine
**Consumers:** State store, Observability, CLI (display)
**Payload:**
```json
{
  "task_id": "task_1",
  "decision": {
    "backend": "claude-code",
    "confidence": 0.92,
    "reason": "Complex integration task, strong tool use needed",
    "eligible": ["claude-code", "openai-api", "ollama"],
    "scores": {"claude-code": 0.92, "openai-api": 0.78, "ollama": 0.45}
  }
}
```
**Idempotency:** One routing decision per task attempt. Retries get new routing events.

---

### task.started

**Producer:** Provider adapter (when agent begins execution)
**Consumers:** State store, Observability, CLI (display)
**Payload:**
```json
{
  "task_id": "task_1",
  "backend": "claude-code",
  "worktree_path": "/home/user/project/.codex-worktrees/r_abc123/task_1",
  "attempt": 1
}
```
**Idempotency:** Keyed by task_id + attempt number.

---

### task.completed

**Producer:** Provider adapter (when agent finishes)
**Consumers:** Orchestrator (advance state), State store, CLI
**Payload:**
```json
{
  "task_id": "task_1",
  "status": "success",
  "artifacts": ["diff_task_1.patch", "task_1_log.jsonl"],
  "files_changed": ["src/clients/redis.py"],
  "tokens_input": 3200,
  "tokens_output": 1800,
  "cost_usd": 0.12,
  "duration_seconds": 62
}
```
**Idempotency:** Task completion is final for that attempt. Duplicate events with same sequence ignored.

---

### task.failed

**Producer:** Provider adapter or Orchestrator
**Consumers:** Orchestrator (retry logic), State store, CLI
**Payload:**
```json
{
  "task_id": "task_3",
  "error": "Agent exceeded max turns (10) without completing task",
  "error_type": "turn_limit",
  "attempt": 1,
  "backend": "ollama-qwen3-coder",
  "will_retry": true
}
```
**Idempotency:** Keyed by task_id + attempt.

---

### verification.requested

**Producer:** Orchestrator (after task.completed)
**Consumers:** Verifier, Observability
**Payload:**
```json
{
  "task_id": "task_1",
  "verification_command": "pytest tests/test_redis.py -v",
  "worktree_path": "/home/user/project/.codex-worktrees/r_abc123/task_1"
}
```

---

### verification.passed

**Producer:** Verifier
**Consumers:** Orchestrator (advance to approval or complete), State store
**Payload:**
```json
{
  "task_id": "task_1",
  "tests_run": 12,
  "tests_passed": 12,
  "duration_seconds": 8
}
```

---

### verification.failed

**Producer:** Verifier
**Consumers:** Orchestrator (retry or fail), State store
**Payload:**
```json
{
  "task_id": "task_1",
  "tests_run": 12,
  "tests_passed": 10,
  "tests_failed": 2,
  "failures": [
    {"test": "test_connect", "error": "ConnectionRefusedError"}
  ],
  "will_retry": true
}
```

---

### approval.requested

**Producer:** Policy engine (via Orchestrator)
**Consumers:** Approval gate → CLI → User
**Payload:**
```json
{
  "approval_id": "apr_xyz",
  "task_id": "task_4",
  "action_type": "shell_exec",
  "action_detail": "docker-compose restart api-gateway",
  "policy_rule": "restart commands require approval",
  "timeout_seconds": 300
}
```

---

### approval.granted / approval.denied

**Producer:** Approval gate (after user decision)
**Consumers:** Orchestrator (resume or fail), State store
**Payload:**
```json
{
  "approval_id": "apr_xyz",
  "task_id": "task_4",
  "decision": "granted",
  "decided_by": "user",
  "elapsed_seconds": 12
}
```

---

### retry.scheduled

**Producer:** Orchestrator
**Consumers:** State store, Observability
**Payload:**
```json
{
  "task_id": "task_3",
  "attempt": 2,
  "previous_backend": "ollama-qwen3-coder",
  "new_backend": "openai-api",
  "reason": "Escalating after local model failure",
  "backoff_seconds": 5
}
```

---

### run.paused / run.resumed / run.cancelled

**Producer:** Orchestrator (on user action or budget/timeout)
**Consumers:** State store, CLI, Observability
**Payload:**
```json
{
  "reason": "Budget limit reached ($5.00)",
  "tasks_completed": 4,
  "tasks_remaining": 3
}
```

---

### artifact.published

**Producer:** Artifact manager
**Consumers:** State store, CLI
**Payload:**
```json
{
  "task_id": "task_1",
  "artifact_type": "diff",
  "name": "task_1_changes.patch",
  "path": "~/.codex/multi-agent/runs/r_abc123/artifacts/task_1_changes.patch",
  "size_bytes": 2847
}
```

## Event Log Format

One JSON object per line, newline-delimited:

```jsonl
{"id":"evt_01","type":"run.created","run_id":"r_abc123","sequence":1,"timestamp":1712600000,"data":{...}}
{"id":"evt_02","type":"plan.requested","run_id":"r_abc123","sequence":2,"timestamp":1712600001,"data":{...}}
{"id":"evt_03","type":"plan.generated","run_id":"r_abc123","sequence":3,"timestamp":1712600005,"data":{...}}
```

## Idempotency Strategy

Events are idempotent by `(run_id, sequence)`. If the orchestrator crashes after writing an event but before acting on it, on restart it:
1. Reads all events from the log
2. Finds the highest sequence number
3. Compares against SQLite state
4. Replays any events that weren't reflected in SQLite
5. Continues from the next action

This ensures exactly-once processing of events.
