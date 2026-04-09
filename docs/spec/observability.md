# Observability Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Design principles

1. All observability data is local (no external services required for v1)
2. All data is structured (JSON, not free-text)
3. All data is queryable via CLI subcommands
4. Correlation by run_id and task_id throughout

## Logs

### Log format
Structured JSON, one object per line:
```json
{
  "timestamp": "2026-04-08T12:00:00Z",
  "level": "info",
  "run_id": "r_abc123",
  "task_id": "task_3",
  "component": "orchestrator",
  "message": "Task routed to ollama-qwen3-coder (confidence: 0.82)",
  "data": { "backend": "ollama-qwen3-coder", "confidence": 0.82 }
}
```

### Log levels
- `debug` — Detailed internal state (event processing, routing calculations)
- `info` — User-visible state changes (task started, completed, routed)
- `warn` — Degraded conditions (provider slow, verification skipped, budget 80%)
- `error` — Failures (task failed, provider down, merge conflict)

### Log locations
```
~/.codex/multi-agent/runs/<run-id>/
├── orchestrator.log      # Orchestrator + routing logs
├── events.jsonl          # Event log (canonical history)
├── tasks/
│   ├── task_1.log        # Per-task agent execution logs
│   ├── task_2.log
│   └── ...
└── summary.json          # Generated on run completion
```

### Log access via CLI
```bash
codex run logs r_abc123                          # All logs
codex run logs r_abc123 --task task_3            # Task-specific
codex run logs r_abc123 --level error            # Errors only
codex run logs r_abc123 --follow                 # Live tail
codex run logs r_abc123 --json                   # Raw JSON
codex run logs r_abc123 --component routing      # Component filter
```

## Metrics

Collected per-run and queryable via `codex run inspect`:

| Metric | Type | Description |
|--------|------|-------------|
| `run.duration_seconds` | gauge | Total run wall time |
| `run.tasks_total` | counter | Number of tasks in plan |
| `run.tasks_completed` | counter | Successfully completed tasks |
| `run.tasks_failed` | counter | Failed tasks |
| `run.tasks_retried` | counter | Tasks that required retry |
| `run.cost_usd` | gauge | Total API cost |
| `run.tokens_input` | counter | Total input tokens across all tasks |
| `run.tokens_output` | counter | Total output tokens |
| `task.duration_seconds` | gauge | Per-task execution time |
| `task.turns` | counter | Agent turns per task |
| `task.retries` | counter | Retry count per task |
| `routing.decision_count` | counter | Number of routing decisions |
| `routing.backend_distribution` | histogram | Tasks per backend |
| `verification.pass_rate` | gauge | Pass/total verifications |
| `verification.duration_seconds` | gauge | Verification time per task |
| `approval.count` | counter | Approval requests |
| `approval.avg_response_seconds` | gauge | Average approval response time |
| `worktree.active_count` | gauge | Current active worktrees |

### Metrics access
```bash
codex run inspect r_abc123 --summary     # Human-readable summary with key metrics
codex run inspect r_abc123 --json        # Full structured metrics
codex run list --json | jq '.[].metrics' # Metrics across runs
```

## Traces

Each task produces a trace of its lifecycle:

```json
{
  "trace_id": "r_abc123/task_1",
  "spans": [
    {"name": "routing", "start": 1712600001, "end": 1712600002, "data": {"backend": "claude-code"}},
    {"name": "worktree_create", "start": 1712600002, "end": 1712600004},
    {"name": "agent_execution", "start": 1712600004, "end": 1712600066, "data": {"turns": 5}},
    {"name": "verification", "start": 1712600066, "end": 1712600074, "data": {"result": "pass"}},
    {"name": "merge", "start": 1712600074, "end": 1712600075}
  ]
}
```

Traces are embedded in the event log — each event has a timestamp, and spans can be reconstructed from event pairs (task.started → task.completed).

## Audit history

The [event log](event-model.md) (`events.jsonl`) is the complete audit trail. It records:
- Every state change with who/what triggered it
- Every routing decision with full scoring details
- Every approval request and decision
- Every verification attempt and result
- Every retry with reason

Audit queries:
```bash
codex run inspect r_abc123 --events                  # Full event stream
codex run inspect r_abc123 --approvals               # Approval history
codex run inspect r_abc123 --routing                  # Routing decisions
codex run inspect r_abc123 --events --type task.failed # Specific event type
```

## Run summaries

Auto-generated on run completion:

```json
{
  "run_id": "r_abc123",
  "goal": "Add rate limiting with Redis",
  "status": "completed",
  "duration_seconds": 272,
  "tasks": {
    "total": 7,
    "completed": 7,
    "failed": 0,
    "retried": 1
  },
  "routing": {
    "claude-code": 2,
    "openai-api": 2,
    "ollama": 2,
    "local": 1
  },
  "cost": {
    "api_usd": 0.42,
    "subscription_units": 2,
    "total_tokens": 28400
  },
  "verification": {
    "total": 6,
    "passed": 6,
    "failed_then_retried": 1
  },
  "approvals": {
    "requested": 1,
    "approved": 1,
    "denied": 0
  },
  "files_changed": 6,
  "result_branch": "codex/r_abc123/result"
}
```

## Routing summaries

```bash
$ codex run inspect r_abc123 --routing

Routing Summary for r_abc123
─────────────────────────────

Backend Distribution:
  claude-code   ██████████ 2 tasks (28.6%)
  openai-api    ██████████ 2 tasks (28.6%)
  ollama        ██████████ 2 tasks (28.6%)
  local         █████      1 task  (14.3%)

Routing Decisions:
  Task  Type   Backend          Confidence  Reason
  ────  ─────  ───────────────  ──────────  ──────
  1     code   claude-code      0.92        Complex integration, tool use
  2     code   openai-api       0.88        Code generation, medium context
  3     code   ollama           0.95        Simple test generation, cost
  4     code   openai-api       0.85        Config editing, medium
  5     code   ollama           0.97        Template/boilerplate, cost
  6     test   local            1.00        Deterministic test execution
  7     review claude-code      0.90        Multi-file review, large context

Retries:
  Task 3: ollama → openai-api (escalated after verification failure)

Total cost: $0.42 API + 2 subscription units
```

## Failure reports

On run failure, a failure report is generated:

```bash
$ codex run inspect r_def456

Run r_def456 — FAILED (1m 23s)
Goal: "Migrate from Postgres to CockroachDB"

FAILURE: task_2 — "Update SQL queries for CockroachDB compatibility"
  Attempts: 3 (ollama → openai → claude-code)
  Last error: Verification failed — 8/24 tests failing after 3 retries
  Failing tests:
    - test_transaction_isolation: CockroachDB uses serializable by default
    - test_upsert_syntax: ON CONFLICT syntax differs
    - ... (6 more)
  
  Suggestion: This task may require manual SQL review.
  Resume: codex run resume r_def456 --from-task task_2
```

## Replay/debug strategy

The event log supports replay for debugging:

1. **Event replay:** Read events.jsonl sequentially to reconstruct the run timeline
2. **State reconstruction:** Apply events to rebuild SQLite state (verify consistency)
3. **Routing replay:** Re-evaluate routing decisions against current provider config to see if decisions would change
4. **Dry replay:** `codex run replay r_abc123 --dry-run` — show what would happen with current config without executing

Full provider-mocked replay (re-running agents against recorded LLM responses) is a post-v1 capability. The event log format is designed to support it, but the replay tooling is not in MVP scope.
