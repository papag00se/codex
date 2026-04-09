# Minimal Service/Process List

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Hard rule: one CLI

The user types `codex run "goal"`. That's it. The Python orchestrator is an invisible subprocess — the user never invokes it, never sees it, never needs to know it exists.

## Processes at runtime

When a user runs `codex run "goal"`, these processes exist:

```
Process tree:

codex run "goal"                    # Rust CLI (parent) — THE user-facing process
│                                   # Renders TUI, handles approvals, shows progress
│
└── python -m codex_orchestrator    # Invisible subprocess (child, stdin/stdout IPC)
    │                               # Plans, routes, dispatches, verifies
    │
    ├── codex exec --cwd /wt/t1    # Codex CLI worker (for Codex-backed tasks)
    ├── claude --cwd /wt/t2        # Claude Code worker (for Claude-backed tasks)
    └── (HTTP to Ollama)           # Direct HTTP via absorbed Ollama client
```

The orchestrator has **no CLI, no --help, no user-facing output**. All communication is JSON lines on stdin/stdout with the parent Codex CLI process. The routing logic, compaction, Ollama client, and tool adapter from coding-agent-router are absorbed as library modules within the orchestrator (see [Routing Logic Reference](routing-logic-reference.md)).

## Module responsibilities

| Module | Process | Language | Purpose |
|--------|---------|----------|---------|
| `codex-rs/cli` | codex run | Rust | CLI entry point, spawn orchestrator, render TUI/JSON, relay approvals |
| `codex_orchestrator` | python -m codex_orchestrator | Python | Supervisor loop, routing, state management, event emission |
| `codex_orchestrator.routing` | (library, absorbed) | Python | Per-task and per-request routing, task metrics, scoring — migrated from coding-agent-router |
| `codex_orchestrator.compaction` | (library, absorbed) | Python | Transcript compaction pipeline — migrated from coding-agent-router |
| `codex_orchestrator.providers.codex_cli` | codex exec (subprocess) | Rust | Execute tasks via Codex CLI engine |
| `codex_orchestrator.providers.claude_code` | claude (subprocess) | N/A | Execute tasks via Claude Code |
| `codex_orchestrator.providers.ollama` | (HTTP client, absorbed) | Python | Ollama API client with per-endpoint serialization — migrated from coding-agent-router |
| `codex_orchestrator.providers.tool_adapter` | (library, absorbed) | Python | Tool call recovery for local models — migrated from coding-agent-router |
| `codex_orchestrator.state.store` | (library) | Python | SQLite state management |
| `codex_orchestrator.state.events` | (library) | Python | JSONL event log |
| `codex_orchestrator.repo.worktree` | git (subprocess) | N/A | Git worktree management |
| `codex_orchestrator.verification.runner` | bash -c (subprocess) | N/A | Run verification commands |

## Startup sequence

```
1. codex run "goal"
   │
   ├─ 2. Load config (TOML)
   ├─ 3. Create Run record in SQLite
   ├─ 4. Spawn: python -m codex_orchestrator --run-id r_xxx
   │     │
   │     ├─ 6. Load state from SQLite
   │     ├─ 7. Probe provider health (Ollama, Codex, Claude)
   │     ├─ 8. Enter supervisor loop
   │     │     │
   │     │     ├─ 9. Plan (LLM call via provider)
   │     │     ├─ 10. Schedule tasks
   │     │     ├─ 11. Route → Dispatch → Verify → Approve loop
   │     │     └─ 12. Complete run
   │     │
   │     └─ 13. Exit with status code
   │
   └─ 14. Display final summary, exit
```

## Shutdown sequence

### Graceful (Ctrl+C)
```
1. CLI sends SIGINT to orchestrator
2. Orchestrator:
   a. Set run status to "paused"
   b. Send SIGTERM to active agent subprocesses
   c. Wait 5s for agents to finish current turn
   d. Persist state (SQLite + event log)
   e. Clean up worktrees for incomplete tasks (keep completed)
   f. Exit
3. CLI displays "Run paused. Resume with: codex run resume r_xxx"
```

### Forced (kill -9)
```
1. Process dies immediately
2. On next startup (resume):
   a. Read SQLite state
   b. Detect stuck tasks (status=running but no process)
   c. Reset stuck tasks to planned (with retry increment)
   d. Clean orphaned worktrees
   e. Continue from last checkpoint
```

## IPC Protocol (CLI ↔ Orchestrator)

### Orchestrator → CLI (stdout)
```json
{"type": "event", "data": {"type": "task.started", "task_id": "task_1", ...}}
{"type": "event", "data": {"type": "route.selected", "task_id": "task_1", ...}}
{"type": "approval_request", "data": {"id": "apr_xyz", "action": "npm install redis", ...}}
{"type": "status", "data": {"completed": 3, "total": 7, "running": ["task_4"]}}
{"type": "complete", "data": {"status": "completed", "summary": "..."}}
```

### CLI → Orchestrator (stdin)
```json
{"type": "approval_response", "data": {"id": "apr_xyz", "decision": "approved"}}
{"type": "cancel", "data": {}}
{"type": "pause", "data": {}}
```

One JSON object per line, newline-delimited. See [Event Model](event-model.md) for the full event schema.
