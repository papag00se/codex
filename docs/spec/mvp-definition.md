# MVP Definition

[< Spec Index](index.md) | [Product Index](../product/index.md)

## What's in

The smallest useful v1 that demonstrates the full architecture:

| Component | MVP Scope |
|-----------|-----------|
| **CLI** | `codex run <goal>`, `status`, `inspect`, `resume`, `cancel` subcommands. Interactive approval prompts. JSON output mode. |
| **Orchestrator** | Supervisor loop with bounded iterations, timeout, budget tracking. Sequential and parallel task dispatch. |
| **Planner** | Single-shot LLM planning that produces a task graph with dependencies. |
| **Agents (3)** | Coder (implement changes), Reviewer (review diffs), Test-interpreter (run and interpret tests). |
| **Routing** | Task-level routing across 3+ backend categories. Scoring + fallback. Routing decision logging. |
| **Provider Adapters (3+)** | Codex CLI (subprocess), Claude Code (subprocess), Ollama (HTTP). At least one API adapter (OpenAI or Anthropic). |
| **State** | SQLite for runs/tasks/routing. JSONL event log. |
| **Events** | Core event types: run.created, plan.generated, task.created/started/completed/failed, route.selected, verification.passed/failed, approval.requested/granted/denied. |
| **Repo Isolation** | Git worktree per parallel task (see [Repository Isolation](repository-isolation.md)). Merge into result branch. Conflict detection. Cleanup. |
| **Verification** | Run user-configured test command, check exit code, parse output. |
| **Approval** | Pattern-based policy engine. Interactive approval prompts. Timeout with configurable default. |
| **Resume** | Resume interrupted runs from last completed task. Skip completed tasks. Re-route pending tasks. |
| **Observability** | Structured JSON logs (see [Observability](observability.md)). `codex run inspect` with --routing, --events, --summary. Run summary generation. |
| **Config** | TOML config with provider settings, routing policy, approval policy, verification commands. |

## What's out of MVP

| Feature | Reason |
|---------|--------|
| TUI integration (live dashboard) | CLI status output sufficient for v1; TUI is polish |
| Adaptive re-planning | Single-shot planning sufficient; re-plan on resume covers most cases |
| LLM-based code review agent | Deterministic verification (tests) is the priority |
| Cross-repo workflows | Single repo per run |
| Custom agent definitions | Fixed taxonomy in v1 |
| Coverage analysis | Users configure their own verification |
| Cost dashboard / historical analytics | Per-run cost in inspect output sufficient |
| Distributed execution | Single machine only |
| Auto-conflict resolution | Conflicts flagged for human review |
| Full replay tooling | Event log format supports it; tooling is post-MVP |
| Docs writer, dependency analyst, etc. | v2 agents |
| Plugin system for providers | Hardcoded adapter classes in v1 |
| Batch approval ("approve all of type X") | Simple per-request approval in v1 |

## Success criteria for MVP

1. **End-to-end goal execution:** User can run `codex run "Add feature X with tests"` and get a working implementation with passing tests on a result branch.
2. **Multi-backend routing:** At least 2 different backends are used in a single run (e.g., Claude Code for complex task, Ollama for simple task).
3. **Routing transparency:** User can run `codex run inspect <id> --routing` and see why each backend was chosen.
4. **Crash recovery:** Kill the orchestrator mid-run, `codex run resume <id>` picks up from last checkpoint.
5. **Approval gating:** A risky action (e.g., `npm install`) pauses for user approval.
6. **Verification:** Test failures are detected and trigger retry with feedback.
7. **Parallel execution:** Two independent tasks execute in parallel worktrees.
