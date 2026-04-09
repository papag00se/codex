# Implementation Status

[< Spec Index](index.md)

> Last updated: 2026-04-09

## What's built

### Rust crates (49 tests passing)

**`codex-rs/routing/`** (codex-routing) — 36 tests
- Task metrics extraction: all 27 regex patterns ported from Python reference
- Route selection algorithm: full decision flow with context-window filtering, LLM-assisted selection, deterministic fallback
- **LLM-based request classifier**: local qwen3.5-9b:iq4_xs on the 1080 classifies every request into: light_reasoner, light_coder, cloud_fast, cloud_mini, cloud_reasoner, cloud_coder
- **Cloud tier routing**: classifier output drives model slug override — spark/mini/sonnet for secondary buckets, primary only for cloud_coder
- **Weighted model distribution**: cloud roles with multiple entries use weighted random selection from `.codex-multi/config.toml`
- **Project config loader**: reads `.codex-multi/config.toml` with model roles, failover chains, supervisor settings
- Ollama HTTP client: async with per-endpoint semaphore, supports tool passing
- Tool-call recovery: JSON blob recovery, embedded tool blocks, streaming partial drops
- Classifier robustness: `<think>` tag stripping, malformed JSON recovery, 10s timeout fallback

**`codex-rs/supervisor/`** (codex-supervisor) — 13 tests
- Task graph: deterministic state machine (Pending → Running → Evaluating → Completed/Failed/Skipped)
- Supervisor loop: bounded by iterations (default: 50), timeout (default: 2h), max retries (default: 3)
- `SupervisorJudge` trait: plan_tasks, dispatch_task (returns DispatchResult with thread ID), evaluate_completion, verify
- Dependency resolution: tasks with unmet deps wait; failed deps cascade to skip
- **Context resumption**: tracks `last_agent_thread_id` per task, retries fork from previous agent's conversation via `SpawnAgentForkMode::LastNTurns(5)`

### Codex integration

**`codex-rs/core/src/tools/handlers/supervisor.rs`** — supervisor tool handler
- `SupervisorHandler`: registered as `supervisor` tool, model calls it for complex goals
- `CodexJudge`: bridges supervisor to codex-core
  - `plan_tasks`: local Ollama with failover chain (reasoner → backup → Codex)
  - `dispatch_task`: spawns worker, waits for completion, returns thread ID; retries fork from previous context
  - `evaluate_completion`: local Ollama with failover chain, `<think>` tag handling
  - `verify`: runs subprocess in correct cwd, checks exit code

**`codex-rs/core/src/local_routing.rs`** — per-request routing
- Hooks into `ModelClientSession::stream()` — every model API call goes through the classifier
- Local routes: call Ollama directly, translate to `ResponseEvent` stream
- Cloud routes: override `model_info.slug` for this request only (spark/mini/sonnet)
- Loads config from `.codex-multi/config.toml`, falls back to env vars
- Health check via `/api/version` (fast, doesn't cold-load model)

**`.codex-multi/config.toml`** — project config
- Model roles: classifier, light_reasoner, light_reasoner_backup, light_coder, compactor, cloud_fast, cloud_mini, cloud_reasoner, cloud_coder
- Weighted distribution: `entries = [{model, weight}, ...]` for cloud roles
- Failover chains per task type
- Supervisor behavior: max_iterations, timeout, retries, verification_command
- Usage preservation: primary_warn_threshold, prefer_secondary

### Upstream integration footprint

| File | Change |
|------|--------|
| `core/src/tools/handlers/supervisor.rs` | New: supervisor tool handler |
| `core/src/local_routing.rs` | New: per-request routing hook |
| `core/src/lib.rs` | +1 line: `mod local_routing` |
| `core/src/client.rs` | +12 lines: routing hook in `stream()` |
| `core/Cargo.toml` | +2 lines: deps |
| `tools/src/supervisor_tool.rs` | New: tool spec |
| `tools/src/tool_registry_plan_types.rs` | +1 line: enum variant |
| `tools/src/tool_registry_plan.rs` | +7 lines: register |
| `tools/src/lib.rs` | +2 lines: exports |
| `core/src/tools/spec.rs` | +3 lines: match arm |
| `core/src/tools/handlers/mod.rs` | +2 lines: module |

## Live test results

### Per-request routing (2026-04-09)
```
Request: "What is a goroutine?"
Classifier: light_reasoner, tools_potential=false (3.6s on 1080)
Route: local qwen3.5:9b on sakura:11435
Result: ✓ Correct goroutine explanation, 230 tokens
Cost: ZERO cloud tokens — entirely local
```

### Cloud tier classification (2026-04-09)
```
12 test requests classified by local LLM:
- light_reasoner: 4/12 (simple questions, yes/no, architecture)
- light_coder: 3/12 (file reads, docstrings, renames)
- cloud_fast: 2/12 (unit test fix, single-file refactor)
- cloud_mini: 2/12 (Playwright E2E, multi-file investigation)
- cloud_reasoner: 1/12 (security review)
- cloud_coder: 1/12 (full app debug)
All classifications correct. Every tier hit.
```

### Supervisor tool (2026-04-08)
```
Goal: "Create calculator.py + test_calculator.py + run tests"
Result: ✓ Both files created, 5 tests passing (including edge cases)

Goal: "Create math_utils.py with is_prime + tests"
Result: ✓ 12 parametrized pytest tests passing
```

## What's next

| # | Item | Status |
|---|------|--------|
| 7 | Sequential task context sharing (task 2 sees task 1 output) | Not started |
| 8 | Verification loop end-to-end test | Not tested in full routing flow |
| 9 | Supervisor tool handler reads project config | Only local_routing reads it |
| 10 | Planner quality — use cloud for complex goals, local for simple | Local planner only |
| 11 | Usage tracking across buckets | Config has thresholds, no tracking code |
| 12 | Observability — routing decisions in TUI | Logged via tracing only |
| 13 | Port compaction pipeline to Rust | Deferred (proxy handles it) |
| 14 | Agent role configs | Deferred |
| 15 | ToolSpec-to-Ollama format adapter | Infrastructure ready, adapter not ported |

## Build instructions

```bash
cd codex-rs

# Set up build environment (WSL without libssl-dev)
source routing/build-env.sh

# Build
cargo build -p codex-cli

# Run tests
cargo test -p codex-routing -p codex-supervisor

# Run with routing enabled (needs .codex-multi/config.toml in cwd)
RUST_LOG=codex_core::local_routing=info,codex_routing=info ./target/debug/codex
```

## Git log

```
742db265d Cloud tier routing + weighted distribution + classifier robustness
df836893e Wire agent context forking — retries resume from previous agent's conversation
c10fd5540 Track agent thread IDs for context resumption on retries
fe576711a Wire .codex-multi/config.toml into routing — no env vars needed
a4f8bc8f2 Add .codex-multi/config.toml and project config loader
4bd210eac Multi-agent orchestration: routing, supervisor, per-request local model routing
```
