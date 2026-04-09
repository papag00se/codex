# State Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Overview

Durable state lives in two stores:
1. **SQLite** — queryable current state (runs, tasks, routing decisions, approvals)
2. **JSONL event log** — append-only history (all events, for audit and replay)

SQLite is the primary read path. The [event log](event-model.md) is the authoritative history. The event log is the authoritative history. On crash recovery, SQLite state is verified against the event log (see [Operational Model](operational-model.md) for recovery flow).

## Entity Relationship

```
Run 1──* Task
Task 1──1 RoutingDecision
Task 1──* Artifact
Task 1──* RetryRecord
Task 0──1 ApprovalRequest
Task 0──1 WorktreeHandle
Run 1──* Event
```

## Run

```sql
CREATE TABLE runs (
    id              TEXT PRIMARY KEY,       -- "r_" + nanoid
    goal            TEXT NOT NULL,
    status          TEXT NOT NULL,           -- planned, running, paused, completed, failed, cancelled
    config_snapshot TEXT NOT NULL,           -- JSON: frozen config at run start
    
    created_at      INTEGER NOT NULL,       -- Unix seconds
    updated_at      INTEGER NOT NULL,
    started_at      INTEGER,
    completed_at    INTEGER,
    
    total_tasks     INTEGER DEFAULT 0,
    completed_tasks INTEGER DEFAULT 0,
    failed_tasks    INTEGER DEFAULT 0,
    
    budget_limit    REAL,                   -- USD, nullable
    budget_spent    REAL DEFAULT 0.0,
    
    max_iterations  INTEGER NOT NULL DEFAULT 50,
    current_iteration INTEGER DEFAULT 0,
    timeout_seconds INTEGER NOT NULL DEFAULT 7200,
    
    plan_json       TEXT,                   -- JSON: task graph from planner
    summary         TEXT,                   -- Generated on completion
    error           TEXT,                   -- Error message if failed
    
    repo_path       TEXT NOT NULL,          -- Absolute path to repository
    base_branch     TEXT NOT NULL,          -- Branch at run start
    result_branch   TEXT                    -- Branch with merged results
);
```

## Task

```sql
CREATE TABLE tasks (
    id              TEXT PRIMARY KEY,       -- "task_" + nanoid
    run_id          TEXT NOT NULL REFERENCES runs(id),
    
    description     TEXT NOT NULL,
    task_type       TEXT NOT NULL,           -- code, review, test, plan, docs
    
    status          TEXT NOT NULL,           -- planned, routed, assigned, running,
                                            --   verifying, awaiting_approval,
                                            --   completed, failed, cancelled, skipped
    
    dependencies    TEXT NOT NULL DEFAULT '[]',  -- JSON array of task IDs
    
    estimated_complexity TEXT,               -- low, medium, high
    
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    started_at      INTEGER,
    completed_at    INTEGER,
    
    assigned_backend TEXT,                   -- Provider ID
    assigned_model   TEXT,                   -- Model name
    worktree_path    TEXT,                   -- Absolute path to worktree
    worktree_branch  TEXT,                   -- Branch name in worktree
    
    max_turns       INTEGER NOT NULL DEFAULT 10,
    current_turn    INTEGER DEFAULT 0,
    timeout_seconds INTEGER NOT NULL DEFAULT 900,
    
    retry_count     INTEGER DEFAULT 0,
    max_retries     INTEGER NOT NULL DEFAULT 3,
    
    verification_command TEXT,               -- e.g., "pytest tests/"
    verification_status  TEXT,               -- pass, fail, error, skipped
    
    result_summary  TEXT,                   -- Agent's summary of what it did
    error           TEXT,                   -- Error if failed
    
    cost_usd        REAL DEFAULT 0.0,
    tokens_input    INTEGER DEFAULT 0,
    tokens_output   INTEGER DEFAULT 0
);

CREATE INDEX idx_tasks_run_id ON tasks(run_id);
CREATE INDEX idx_tasks_status ON tasks(status);
```

## RoutingDecision

```sql
CREATE TABLE routing_decisions (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    run_id          TEXT NOT NULL REFERENCES runs(id),
    
    selected_backend TEXT NOT NULL,
    selected_model   TEXT,
    confidence       REAL NOT NULL,
    reason           TEXT NOT NULL,
    
    eligible_backends TEXT NOT NULL,         -- JSON array
    scores           TEXT NOT NULL,          -- JSON object: {backend: score}
    factors          TEXT NOT NULL,          -- JSON object: routing input features
    
    is_retry         BOOLEAN DEFAULT FALSE,
    previous_backend TEXT,                   -- Backend of previous attempt if retry
    
    created_at       INTEGER NOT NULL
);

CREATE INDEX idx_routing_task_id ON routing_decisions(task_id);
```

## ApprovalRequest

```sql
CREATE TABLE approval_requests (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    run_id          TEXT NOT NULL REFERENCES runs(id),
    
    action_type     TEXT NOT NULL,           -- shell_exec, file_delete, git_op, etc.
    action_detail   TEXT NOT NULL,           -- The specific command/path/operation
    policy_rule     TEXT NOT NULL,           -- Which policy rule triggered this
    
    status          TEXT NOT NULL,           -- pending, approved, denied, timeout
    decided_at      INTEGER,
    decided_by      TEXT,                    -- "user" or "timeout_default"
    
    created_at      INTEGER NOT NULL,
    timeout_seconds INTEGER NOT NULL DEFAULT 300
);

CREATE INDEX idx_approvals_status ON approval_requests(status);
```

## Artifact

```sql
CREATE TABLE artifacts (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    run_id          TEXT NOT NULL REFERENCES runs(id),
    
    artifact_type   TEXT NOT NULL,           -- diff, file, test_report, review, log
    name            TEXT NOT NULL,           -- Human-readable name
    path            TEXT NOT NULL,           -- Filesystem path to artifact
    size_bytes      INTEGER,
    
    created_at      INTEGER NOT NULL
);

CREATE INDEX idx_artifacts_task_id ON artifacts(task_id);
```

## RetryRecord

```sql
CREATE TABLE retry_records (
    id              TEXT PRIMARY KEY,
    task_id         TEXT NOT NULL REFERENCES tasks(id),
    
    attempt         INTEGER NOT NULL,        -- 1, 2, 3...
    backend         TEXT NOT NULL,
    error           TEXT NOT NULL,
    
    started_at      INTEGER NOT NULL,
    failed_at       INTEGER NOT NULL
);
```

## WorktreeHandle

Tracked in the `tasks` table (worktree_path, worktree_branch columns) plus a cleanup table:

```sql
CREATE TABLE worktrees (
    path            TEXT PRIMARY KEY,
    task_id         TEXT REFERENCES tasks(id),
    run_id          TEXT NOT NULL REFERENCES runs(id),
    branch          TEXT NOT NULL,
    status          TEXT NOT NULL,           -- active, merged, cleaned, orphaned
    created_at      INTEGER NOT NULL,
    cleaned_at      INTEGER
);
```

## Task State Machine

```
                                ┌──────────────┐
                                │   planned    │
                                └──────┬───────┘
                                       │ route_task()
                                       ▼
                                ┌──────────────┐
                                │   routed     │
                                └──────┬───────┘
                                       │ assign_agent()
                                       ▼
                                ┌──────────────┐
                                │  assigned    │
                                └──────┬───────┘
                                       │ start_execution()
                                       ▼
                                ┌──────────────┐
                          ┌────►│   running    │◄────────────────┐
                          │     └──────┬───────┘                 │
                          │            │ execution_complete()     │
                          │            ▼                          │
                          │     ┌──────────────┐                 │
                          │     │  verifying   │                 │
                          │     └──────┬───────┘                 │
                          │            │                          │
                          │     ┌──────┴──────┐                  │
                          │     │             │                  │
                          │     ▼             ▼                  │
                          │  [pass]       [fail]                 │
                          │     │             │                  │
                          │     │      ┌──────┴──────┐           │
                          │     │      │             │           │
                          │     │      ▼             ▼           │
                          │     │  [retries     [retries         │
                          │     │   remain]     exhausted]       │
                          │     │      │             │           │
                          │     │      │             ▼           │
                          │     │      │      ┌──────────────┐  │
                          │     │      └─────►│  retrying    │──┘
                          │     │             └──────────────┘
                          │     │
                          │     ├─[needs approval]──►┌───────────────────┐
                          │     │                    │ awaiting_approval │
                          │     │                    └────────┬──────────┘
                          │     │                    ┌────────┴──────────┐
                          │     │                    ▼                   ▼
                          │     │              [approved]           [denied]
                          │     │                    │                   │
                          │     ▼                    ▼                   ▼
                          │  ┌──────────────┐  ┌──────────┐    ┌──────────────┐
                          │  │  completed   │  │completed │    │   failed     │
                          │  └──────────────┘  └──────────┘    └──────────────┘
                          │
                          └── [user cancels] ──► ┌──────────────┐
                                                 │  cancelled   │
                                                 └──────────────┘
```

Valid transitions:
- `planned` → `routed` → `assigned` → `running`
- `running` → `verifying` → `completed` (verified pass)
- `running` → `verifying` → `retrying` → `running` (verified fail, retries remain)
- `running` → `verifying` → `failed` (verified fail, retries exhausted)
- `verifying` → `awaiting_approval` → `completed` (approved)
- `verifying` → `awaiting_approval` → `failed` (denied)
- Any active state → `cancelled` (user cancellation)
- `planned` → `skipped` (dependency failed)
