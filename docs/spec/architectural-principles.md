# Core Architectural Principles

[< Spec Index](index.md) | [Product Index](../product/index.md)

## P0: Deterministic Control, Intelligent Judgment

This is the foundational principle. See [Design Principles](design-principles.md) for the full treatment.

**Deterministic code** controls: loops, state transitions, retries, timeouts, dispatch, persistence. The program decides when to continue, when to stop, when to retry. The LLM never controls the loop.

**LLM intelligence** provides: task decomposition, completion evaluation, verification interpretation, routing decisions, retry strategy. The LLM makes judgment calls within the loop, but cannot exit it.

**If you're writing a regex to avoid an LLM call, you're doing it wrong. If you're asking an LLM whether to keep looping, you're also doing it wrong.**

## P1: Bounded Loops

Every loop in the system has an explicit termination condition:

- **Supervisor loop**: max iterations (default: 50), total timeout (default: 2h), budget limit
- **Agent execution**: max turns per task (default: 10), per-task timeout (default: 15m)
- **Retry cycles**: max retries per task (default: 3), exponential backoff with jitter
- **Verification loop**: max verification attempts per task (default: 2)

No loop runs indefinitely. The orchestrator increments a counter on every iteration and hard-stops at the limit. This is not configurable to "unlimited" — the maximum configurable limit is capped (e.g., 200 iterations).

**Maps to: supervisor loop**

## P2: Event-Driven Orchestration

All state changes produce events. Events are the source of truth for what happened. The orchestrator reacts to events, not to direct method calls from agents.

- Agent completes → `task.completed` event → orchestrator picks it up
- Verification fails → `verification.failed` event → orchestrator schedules retry
- User approves → `approval.granted` event → orchestrator resumes task

Events are persisted before the orchestrator acts on them (see [Event Model](event-model.md)). This ensures that if the process crashes between event creation and action, the event is not lost.

**Maps to: event-driven orchestration, durable state**

## P3: Durable State

The system can crash at any point and resume without data loss:

- SQLite holds the current state of runs, tasks, and routing decisions (see [State Model](state-model.md))
- JSONL event log holds the full history of what happened
- On restart, the orchestrator reads SQLite state, verifies against event log, and resumes

State writes are transactional (SQLite) or append-only (JSONL). There are no in-memory-only state transitions that would be lost on crash.

**Maps to: durable state**

## P4: Verification-First

No task is marked complete until verification passes:

1. Agent produces artifacts (diffs, files)
2. Orchestrator runs verification (tests, lint, type check)
3. If verification fails, orchestrator can retry the task (bounded retries) or fail it
4. Only verified tasks are marked complete

Verification is configurable per project. If no verification command is configured, the orchestrator logs a warning and marks the task as "unverified-complete" — a distinct state from "verified-complete."

**Maps to: verification loop**

## P5: Provider Abstraction

The orchestrator never speaks a provider's native protocol. All provider interaction goes through adapters that implement a common interface (see [Provider Abstraction](provider-abstraction.md)):

```
ProviderAdapter {
    execute_task(task, context) → WorkerResult
    capabilities() → ProviderCapability
    health() → HealthStatus
}
```

Adding a new provider means implementing one adapter. Nothing else changes.

**Maps to: specialist agents (backend flexibility)**

## P6: Routing Transparency

Every routing decision is:
1. Logged with full context (input features, eligible backends, selected backend, confidence, reason)
2. Queryable via CLI (`codex run inspect <id> --routing`)
3. Overridable by the user (`--backend <backend>`)

There are no hidden routing decisions. If the system chose Ollama over Claude Code for a task, the user can see exactly why.

**Maps to: event-driven orchestration (routing events)**

## P7: Human Escalation

The system defaults to asking rather than assuming for risky operations:

- Operations matching approval policy patterns → pause and ask
- Agent confidence below threshold → flag for review
- Verification failure after max retries → escalate to human
- Routing to a degraded provider → inform the user

The system never silently does something risky. The [approval gate](verification-safety.md) is deterministic (pattern matching against policy), not agentic.

**Maps to: verification loop, supervisor loop**

## P8: Local-First Execution

Everything runs on the developer's machine:
- SQLite, not Postgres
- File-based event log, not Kafka
- Subprocess execution, not container orchestration
- Git worktrees, not remote branches
- Local process supervision, not Kubernetes

Cloud providers are accessed via API, but all orchestration is local.

## P9: Repository Isolation

Parallel agent work is isolated via Git worktrees (see [Repository Isolation](repository-isolation.md)):
- Each agent task that modifies files gets its own worktree
- Worktrees are created from the current branch
- Results are merged back after verification
- Conflicts are detected and flagged (not auto-resolved in v1)
- Worktrees are cleaned up after task completion or failure

**Maps to: specialist agents (parallel execution)**

## P10: Restart/Resume Safety

A run can be safely interrupted and resumed:
- `Ctrl+C` triggers graceful shutdown: agents finish current turn, state is persisted
- `kill -9` is handled on restart: orphaned worktrees are detected and cleaned up
- Resume reads last persisted state and skips completed tasks
- Each task is designed to be re-executable (idempotent or with "already done" detection)

**Maps to: durable state, supervisor loop**
