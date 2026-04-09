# Integration Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

> This document supersedes the previous "subprocess orchestrator" design. The orchestrator is not a separate process or mode — it is a **capability within the existing Codex agent system**.

## Core principle

The user launches `codex` — the normal interactive TUI. They type a goal. The Codex agent decides how to handle it:

- **Simple goal** → single agent handles it directly (existing behavior, unchanged)
- **Complex goal** → the agent engages specialist sub-agents via the existing `multi_agent_v2` spawn system, with routing logic deciding which model backs each agent

There is no `codex run`. There is no separate orchestrator process. There is no Python CLI. The user types `codex`, gives it a goal, and the system figures out the rest.

## How it works — building on what exists

Codex already has everything needed for multi-agent orchestration:

| Existing capability | Where | What it does |
|---|---|---|
| **Agent spawning** | `core/src/agent/control.rs` | `spawn_agent_with_metadata()` — creates child agent threads with config overrides |
| **Agent registry** | `core/src/agent/registry.rs` | Tracks live agents, enforces `agent_max_threads` (default: 6) |
| **Depth limiting** | `core/src/config/mod.rs` | `agent_max_depth` (default: 1) prevents infinite recursion |
| **Agent roles** | `core/src/config/agent_roles.rs` | Named roles with custom instructions, model overrides, personality |
| **Inter-agent messaging** | `multi_agents_v2/message_tool.rs` | `send_message` between agents |
| **Model overrides per agent** | `multi_agents_v2/spawn.rs` | `model` and `reasoning_effort` args on spawn |
| **Fork modes** | `agent/control.rs` | `FullHistory` or `LastNTurns(n)` — child agents can see parent context |
| **Worktree isolation** | `Agent` tool in Claude Code | Agents can run in isolated worktrees |
| **TUI multi-agent view** | `tui/src/multi_agents.rs` | UI for viewing agent threads |
| **Agent roles config** | `config.toml` | `[agents.roles.X]` defines custom agent types |

**The multi-agent orchestration we need is a set of agent roles + routing logic that plugs into this existing system.** We don't need to build a supervisor loop, task state machine, or IPC protocol — those concepts map directly onto what Codex already does.

## Architecture

```
User launches: codex
User types: "Add rate limiting with Redis and write integration tests"

Codex main agent (the supervisor):
│
├─ Thinks: "This needs decomposition — multiple files, tests, config"
│
├─ Spawns agent: role=coder, model=<routed>, task="Create Redis client wrapper"
│   └─ Works in worktree, commits changes
│
├─ Spawns agent: role=coder, model=<routed>, task="Add rate limit middleware"  
│   └─ Works in worktree, commits changes
│
├─ Spawns agent: role=coder, model=<routed>, task="Write integration tests"
│   └─ Works in worktree, commits changes
│
├─ Spawns agent: role=reviewer, model=<routed>, task="Review all changes"
│   └─ Reads diffs, produces review
│
├─ Runs verification: "pytest tests/"
│
└─ Reports results to user in the TUI
```

### Why the main agent is not a sufficient supervisor

The main Codex agent can do the planning and execution — but **it cannot be trusted to control the loop**. LLMs are trained to be cautious: they stop to ask "should I continue?", they produce partial results and wait, they get conservative after failures. This is exactly the behavior we're eliminating.

The supervisor loop is **deterministic code** (Rust, in codex-core) that:
1. Holds a task graph with explicit pending/running/completed/failed states
2. Continues dispatching as long as tasks are pending — no LLM gets to opt out
3. Evaluates completion by asking the LLM "is this task done?" — but the loop itself is not controlled by the LLM
4. Forces retries when verification fails — the LLM doesn't decide whether to retry, the code does
5. Only stops when: all tasks terminal, budget exhausted, max iterations hit, or timeout

The LLM's role is **judgment within the loop**: planning subtasks, evaluating whether output is complete, interpreting test failures, deciding retry strategy. The LLM does not decide whether the loop continues. See [Design Principles](design-principles.md) for the full mantra.

## Where the new code goes

### 1. Agent roles (config)

Define specialist roles in `config.toml` or `.codex/config.toml`:

```toml
[agents.roles.coder]
nickname = "Coder"
base_instructions = "You are a specialist coding agent. Implement the described change. Commit your work."

[agents.roles.reviewer]
nickname = "Reviewer"  
base_instructions = "You are a code reviewer. Review the diff for bugs, security, and style. Do not modify files."

[agents.roles.test-runner]
nickname = "Test Runner"
base_instructions = "Run the specified test command and interpret the results."
```

These roles use the existing `AgentRoleConfig` system — no new infrastructure needed.

### 2. Routing logic (Rust-native, in codex-core)

The routing logic runs as Rust code inside codex-core — no MCP server, no Python at runtime, no separate process. The Python code we migrated from coding-agent-router is the **reference implementation** — the spec to port from, preserved in [Routing Logic Reference](routing-logic-reference.md).

The routing algorithm is straightforward to port:
1. Filter models by context window — arithmetic
2. If one eligible, return it — trivial
3. If multiple, ask a local router LLM to pick — HTTP call to Ollama
4. Parse JSON response — trivial
5. Fallback if parse fails — hardcoded order

Task metrics extraction (27 regex-based features) ports directly to Rust's `regex` crate.

### 3. Compaction (Rust-native, in codex-core, deferred)

The compaction pipeline (~1,500 lines Python) is the same pattern: chunking (arithmetic), Ollama API calls, deterministic merging. It ports to Rust but is not blocking the supervisor loop. It will be implemented when long-running sessions need it.

The Python reference is preserved in [Compaction Reference](compaction-reference.md).

### 4. Deterministic supervisor loop (Rust, in codex-core)

This is the most important new component. It is **not** the LLM deciding to continue. It is code.

```rust
// Pseudocode — the actual loop
while supervisor.has_pending_tasks() && !supervisor.limits_exceeded() {
    let ready_tasks = supervisor.get_ready_tasks();  // Deterministic: check dependency graph
    
    for task in ready_tasks {
        let model = routing_mcp.route_request(&task);  // LLM judgment: which model?
        let agent = spawn_agent(task, model);           // Deterministic: spawn
        let result = agent.run_to_completion();          // Agent does its work
        
        let done = evaluator_llm.is_task_complete(      // LLM judgment: is it done?
            &task.description, &result
        );
        
        if done {
            let verified = run_verification(&task);      // Deterministic: run test command
            let interpretation = evaluator_llm.interpret( // LLM judgment: did tests really pass?
                &verification_output
            );
            if interpretation.passed {
                supervisor.mark_complete(task);           // Deterministic: state transition
            } else {
                supervisor.mark_retry(task, interpretation.reason); // Deterministic: retry
            }
        } else {
            supervisor.mark_retry(task, "task incomplete"); // Deterministic: retry
        }
    }
    
    supervisor.increment_iteration();                    // Deterministic: bounded loop
}
```

The LLM makes three judgment calls per task: plan subtasks, evaluate completion, interpret verification. The loop, state transitions, retries, and termination are all deterministic code.

See [Design Principles](design-principles.md) for why this split matters.

### 5. Verification

Run the configured test command (deterministic — subprocess, check exit code). Ask the LLM to interpret the output (judgment — "did these tests actually pass? is the failure related to our changes or a pre-existing issue?"). The supervisor loop decides what to do based on the interpretation (deterministic — retry or fail).

## What changes vs previous design

| Previous design | New design |
|---|---|
| `codex run` subcommand | No new subcommand — normal `codex` TUI |
| Python orchestrator subprocess | No separate process — everything is Rust in codex-core |
| LLM-controlled supervisor | **Deterministic supervisor loop** (Rust) with LLM for judgment calls only |
| Custom task state machine | Supervisor loop manages task graph with explicit states |
| Custom event system | Existing Codex event system (`EventMsg`) |
| Custom approval gate | Existing Codex approval system |
| Custom worktree manager | Existing agent worktree support |
| SQLite state store (new) | Existing SQLite state + JSONL rollout |

## What we actually need to build

| Component | Effort | Description |
|---|---|---|
| **Supervisor loop (Rust)** | Large | Deterministic loop in codex-core that holds a task graph, dispatches agents, evaluates completion via LLM, runs verification, retries on failure. The model cannot opt out of the loop. |
| **Routing engine (Rust)** | Medium | Port routing algorithm from Python reference to Rust crate. Context-window filtering, task metrics, Ollama router call, fallback ordering. See [Routing Logic Reference](routing-logic-reference.md). |
| **Agent role configs** | Small | TOML configs for coder, reviewer, test-runner roles |
| **Evaluator LLM calls** | Medium | LLM calls for: "is this task complete?", "did verification pass?", "what retry strategy?" — used by the supervisor loop for judgment, not control flow |
| **Spawn model override integration** | Small | Wire routing decisions into the `model` field of `spawn_agent` calls |
| **Compaction engine (Rust, deferred)** | Large | Port compaction pipeline from Python reference when long sessions need it. See [Compaction Reference](compaction-reference.md). |

## Role of the Python code

The migrated Python code in `orchestrator/` is the **reference implementation** — the spec to port from. It is not runtime code. Every heuristic, threshold, and algorithm is preserved in:

- [Routing Logic Reference](routing-logic-reference.md) — the complete routing algorithm, all 27 task metrics, tool-call recovery, config knobs
- [Compaction Reference](compaction-reference.md) — the complete 8-step compaction pipeline

The Rust implementation must match these docs. The Python code exists so developers can run it, test it, and compare behavior during porting — then it can be archived.

## Example: what the user sees

```
$ codex
┌─ Codex ──────────────────────────────────────────────────┐
│ > Add rate limiting to the API gateway with Redis         │
│   and write integration tests                             │
│                                                           │
│ I'll break this into subtasks and work on them:           │
│                                                           │
│ ┌─ Coder (qwen3-coder) ─────────────────────────────┐   │
│ │ Creating Redis client wrapper...                    │   │
│ │ ✓ Done — src/clients/redis.py                      │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ ┌─ Coder (gpt-5.4) ─────────────────────────────────┐   │
│ │ Adding rate limit middleware...                     │   │
│ │ ✓ Done — src/middleware/rate_limiter.py             │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ ┌─ Coder (qwen3-coder) ─────────────────────────────┐   │
│ │ Writing integration tests...                       │   │
│ │ ✓ Done — tests/test_rate_limiter.py                │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ Running verification: pytest tests/ -v                    │
│ ✓ 12 tests passed                                        │
│                                                           │
│ ┌─ Reviewer (gpt-5.4) ──────────────────────────────┐   │
│ │ Review: LGTM — one suggestion on connection pooling │   │
│ └─────────────────────────────────────────────────────┘   │
│                                                           │
│ All done. Changes on branch codex/rate-limiting.          │
└───────────────────────────────────────────────────────────┘
```

This is the existing Codex TUI with its existing multi-agent view — just with smarter agent spawning and model routing.
