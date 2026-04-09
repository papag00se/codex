# Implementation Status

[< Spec Index](index.md)

> Last updated: 2026-04-08

## What's built

### Rust crates (44 tests passing)

**`codex-rs/routing/`** (codex-routing) — 31 tests
- Task metrics extraction: all 27 regex patterns ported from Python reference
- Route selection algorithm: full decision flow with context-window filtering, LLM-assisted selection, deterministic fallback
- Ollama HTTP client: async with per-endpoint semaphore serialization
- Tool-call recovery: JSON blob recovery, embedded tool blocks, streaming partial drops
- Config: all knobs with defaults matching coding-agent-router

**`codex-rs/supervisor/`** (codex-supervisor) — 13 tests
- Task graph: deterministic state machine (Pending → Running → Evaluating → Completed/Failed/Skipped)
- Supervisor loop: bounded by iterations (default: 50), timeout (default: 2h), max retries (default: 3)
- `SupervisorJudge` trait: plan_tasks, dispatch_task, evaluate_completion, verify
- Dependency resolution: tasks with unmet deps wait; failed deps cascade to skip
- Tests cover: happy path, retry-then-succeed, max retries, max iterations, verification failure

### Codex integration (compiles clean, builds successfully)

**`codex-rs/core/src/tools/handlers/supervisor.rs`** — ~240 lines
- `SupervisorHandler`: tool handler registered as `supervisor` tool
- `CodexJudge`: bridges `SupervisorJudge` trait to codex-core internals
  - `plan_tasks`: spawns planner sub-agent with structured prompt, parses JSON task list
  - `dispatch_task`: spawns worker sub-agent via `AgentControl::spawn_agent`, waits via `subscribe_status`
  - `evaluate_completion`: spawns evaluator sub-agent, interprets yes/no response
  - `verify`: runs subprocess, checks exit code

**Tool registration** — 5 files touched in upstream code (~15 lines of glue):
- `tools/src/supervisor_tool.rs` (new, 40 lines)
- `tools/src/tool_registry_plan_types.rs` (+1 line)
- `tools/src/tool_registry_plan.rs` (+7 lines)
- `tools/src/lib.rs` (+2 lines)
- `core/src/tools/spec.rs` (+3 lines)

### Python reference implementation (79 tests passing)

**`orchestrator/`** — Reference code from coding-agent-router, NOT runtime
- Routing metrics, tool adapter, Ollama client, compaction pipeline
- Used for verification during porting, will be archived

## Live test results

### Test 1: Simple file creation
```
Goal: "Create hello.py that prints 'Hello from the supervisor'"
Result: ✓ File created, content correct, runs successfully
```

### Test 2: Multi-part task (module + tests) — IMPROVED
```
Goal: "Create calculator.py with 4 functions + test_calculator.py with pytest tests + run tests"
Result: ✓ Supervisor created both files. 5 tests (including divide-by-zero edge case).
        All tests passing. Supervisor reported success.
```

### Test 3: Routing with live Ollama
```
Goal: "Create greet.py with a greet(name) function"
Config: Ollama at sakura-wsl.taile41496.ts.net:11435 (qwen3:8b, qwen3-coder:30b)
Result: ✓ Routing engine runs, logs advisory decisions.
        Coding task dispatched to Codex sub-agent (needs tool access).
        Evaluation uses local Ollama (free, text-in/text-out).
        File created, function works correctly.
```

## What's next

| Priority | Item | Status |
|----------|------|--------|
| Medium | Use Ollama for planning calls too (save tokens on decomposition) | Not started |
| Medium | Add verification_command support to supervisor tool | Implemented but untested with real commands |
| Medium | Retry with escalation (stronger model on retry) | Loop supports it, routing advisory only |
| Medium | Improve planner to produce multiple tasks with JSON schema | Done |
| Medium | Routing engine wired in | Done — advisory for coding tasks (Codex needed for tools), active for evaluation (Ollama) |
| Low | Compaction pipeline port to Rust | Deferred (Python reference available) |
| Low | Agent role configs (coder, reviewer, test-runner) | Deferred |
| Low | Wire local Ollama as Codex model provider (when supported) | Blocked on Codex CLI supporting Ollama directly |

## Build instructions

```bash
cd codex-rs

# Set up build environment (WSL without libssl-dev)
source routing/build-env.sh

# Build
cargo build -p codex-cli

# Run tests
cargo test -p codex-routing -p codex-supervisor

# Run the binary
./target/debug/codex
```
