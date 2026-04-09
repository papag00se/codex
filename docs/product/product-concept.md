# Product Concept Document

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## Same CLI, smarter

The user types `codex` and starts an interactive session — exactly as they do today. There is no new subcommand, no separate binary, no mode switch. The multi-agent capability is invisible infrastructure: when the Codex agent receives a complex goal, it automatically decomposes it and spawns specialist sub-agents using the existing `multi_agent_v2` system. The user sees sub-agents appear in the TUI — the same way they already appear when Codex spawns helpers today — but backed by intelligent model routing.

## What the CLI feels like

The CLI feels like having a senior engineer who:
- Listens to your goal
- Produces a plan and asks you to review it
- Assigns the right specialist for each task
- Works on multiple things in parallel when safe
- Runs tests after each change
- Asks you before doing anything risky
- Gives you a clear summary when done
- Can pick up where they left off if interrupted

It does **not** feel like:
- A chatbot you have to babysit turn by turn
- A black box that runs for 30 minutes and dumps output
- A system that requires you to understand its internals to use it

## Supported workflows

### 1. Goal-driven execution (primary)
```
$ codex run "Add rate limiting to the API gateway with Redis backend and integration tests"
```
The system plans, executes, verifies, and presents results.

### 2. Plan-only mode
```
$ codex run --plan-only "Migrate from REST to gRPC for the internal service mesh"
```
Produces a reviewable plan without executing.

### 3. Resume interrupted work
```
$ codex run resume r_abc123
```
Picks up from the last completed task.

### 4. Review mode
```
$ codex run review --branch feature/new-auth
```
Runs reviewer and test-interpreter agents on existing changes.

### 5. Single-task execution
```
$ codex run --task "Fix the flaky test in test_payment_flow.py"
```
Skips planning, routes directly to a coder agent.

## How a user invokes multi-agent mode

Multi-agent mode is the **default** for `codex run`. The system decides how many agents and which types based on the goal complexity. A simple bug fix may use one coder agent. A large feature may use a planner + 3 coders + a reviewer + a test-interpreter.

The user does not need to specify agents, backends, or routing. The system handles this automatically with full transparency.

To force single-agent mode: `codex run --single-agent "..."`.

## How they inspect routing decisions

### During execution
The TUI shows a live status panel:
```
Run r_abc123 — 4/7 tasks complete
├─ [done]  task_1: Add Redis client wrapper     → claude-code (reason: complex integration)
├─ [done]  task_2: Create rate limit middleware  → openai/gpt-5.4 (reason: code generation)
├─ [run]   task_3: Write integration tests       → ollama/qwen3-coder (reason: cost, simple tests)
├─ [wait]  task_4: Update API gateway config     → pending routing
├─ [done]  task_5: Add Redis to docker-compose   → ollama/qwen3-coder (reason: template task)
├─ [done]  task_6: Run full test suite           → local (deterministic)
└─ [pend]  task_7: Review all changes            → pending (depends on task_3, task_4)
```

### After execution
```
$ codex run inspect r_abc123 --routing
Task  Backend              Confidence  Reason
────  ───────────────────  ──────────  ──────────────────────────────
1     claude-code          0.92        Complex integration requiring tool use
2     openai/gpt-5.4       0.88        Strong code generation, medium context
3     ollama/qwen3-coder   0.95        Simple test generation, cost-sensitive
4     openai/gpt-5.4       0.85        Config editing, medium complexity
5     ollama/qwen3-coder   0.97        Template/boilerplate, cost-sensitive
6     local                1.00        Deterministic test execution
7     claude-code          0.90        Multi-file review, large context needed
```

## How they approve risky actions

When a task triggers an approval gate:

```
┌─────────────────────────────────────────────────────────────┐
│ APPROVAL REQUIRED                                            │
│                                                              │
│ Task: task_4 — Update API gateway config                     │
│ Agent: coder (openai/gpt-5.4)                               │
│ Action: shell_exec                                           │
│                                                              │
│ Command: docker-compose restart api-gateway                  │
│                                                              │
│ Policy: "restart commands require approval" (policy.toml:12) │
│                                                              │
│ [a]pprove  [d]eny  [s]kip task  [v]iew diff  [p]ause run   │
└─────────────────────────────────────────────────────────────┘
```

In non-interactive mode (see [CLI Interaction Spec](cli-interaction-spec.md)), approval requests are written to a file and the run pauses until the file is updated:
```
$ cat ~/.codex/runs/r_abc123/approvals/pending/apr_xyz.json
$ echo '{"decision": "approved"}' > ~/.codex/runs/r_abc123/approvals/pending/apr_xyz.json
```

## How they review outputs

### Run summary (auto-generated on completion)
```
$ codex run inspect r_abc123

Run r_abc123 — COMPLETED (4m 32s)
Goal: "Add rate limiting to the API gateway with Redis backend and integration tests"

Tasks: 7 completed, 0 failed
Routing: 2 claude-code, 2 openai, 2 ollama, 1 local
Cost: $0.42 (API) + subscription usage
Verification: 14 tests passed, 0 failed

Files changed:
  A src/middleware/rate_limiter.py    (+142)
  A src/clients/redis_client.py      (+87)
  M src/gateway/app.py               (+12, -2)
  A tests/test_rate_limiter.py       (+203)
  A docker-compose.override.yml      (+8)
  M requirements.txt                 (+2)

Approvals: 1 requested, 1 approved (docker restart)

Branch: codex/r_abc123 (ready for review)
```

### Detailed task inspection
```
$ codex run inspect r_abc123 --task task_1 --events
```
Shows the full event stream for a specific task.

### Diff review
```
$ codex run inspect r_abc123 --diff
```
Shows the combined diff of all changes.

## How the system resumes interrupted work

If a run is interrupted (crash, Ctrl+C, machine restart):

1. The event log and SQLite state are the source of truth.
2. On resume, the orchestrator:
   - Reads the last committed state from SQLite
   - Replays the event log to verify consistency
   - Identifies the last completed task
   - Skips completed tasks
   - Re-routes and re-dispatches pending/failed tasks
   - Cleans up orphaned worktrees from crashed agents
3. The user sees:
```
$ codex run resume r_abc123
Resuming run r_abc123 from task_4 (3/7 tasks already complete)
...
```

If a specific task failed and the user wants to retry with a different backend:
```
$ codex run retry r_abc123 --task task_3 --backend claude-code
```
