# System Architecture Overview

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Hard rule: one CLI, one experience

**The user launches `codex` — the normal interactive TUI.** There is no `codex run`, no separate orchestrator binary, no Python CLI. The multi-agent orchestration is a capability within the existing Codex agent, not a separate mode.

```
User launches: codex
User types:    "Add rate limiting with Redis and write tests"

Codex main agent (the supervisor):
  ├─ Thinks: "This needs multiple specialists"
  ├─ Calls routing engine (Rust, in-process) to decide model per sub-task
  ├─ Spawns sub-agent: role=coder, model=<routed>
  ├─ Spawns sub-agent: role=coder, model=<routed>
  ├─ Spawns sub-agent: role=reviewer, model=<routed>
  ├─ Runs verification
  └─ Reports results in the TUI
```

The orchestration uses the existing `multi_agent_v2` spawn system, agent roles, worktree isolation, and TUI multi-agent view. No new process model, no IPC protocol, no state machine. See [Integration Model](integration-model.md) for the full design.

## Architecture in plain language

The system has six layers, each with a clear responsibility:

1. **CLI Layer** — Parses user input, renders output, handles interactive approval. Knows nothing about models or routing.

2. **Orchestrator Layer** — The [supervisor loop](orchestrator-pseudocode.md). Receives a goal, asks the planner to decompose it, creates a task graph, dispatches tasks to agents, collects results, runs verification, requests approvals, and decides when the run is complete. This is the brain. It is deterministic at the control-plane level — all state transitions follow explicit rules.

3. **[Agent Layer](agent-taxonomy.md)** — Specialist workers that execute bounded tasks. Each agent has a role (coder, reviewer, test-interpreter), receives a task description, and produces artifacts (diffs, reviews, test results). Agents are stateless from the orchestrator's perspective — all state lives in the orchestrator.

4. **[Routing Layer](routing-architecture.md)** — Unified within the orchestrator:
   - **Task-level routing** (which backend category per task)
   - **Request-level routing** (which specific model per API call — logic absorbed from coding-agent-router)

5. **[Provider Layer](provider-abstraction.md)** — Adapters that normalize heterogeneous backends into a uniform interface. Each adapter speaks the provider's native protocol and exposes a common `execute_task` / `complete` interface.

6. **Infrastructure Layer** — [State store](state-model.md) (SQLite), [event log](event-model.md) (JSONL), repository manager (Git worktrees), verification runner (subprocess), observability (structured logs + traces).

## How a run flows

```
User goal
    │
    ▼
CLI parses goal, creates Run record
    │
    ▼
Orchestrator creates planning task
    │
    ▼
Planner agent decomposes goal → task graph
    │
    ▼
Orchestrator persists task graph to state store
    │
    ▼
╔═══════════════════════════════════════╗
║  SUPERVISOR LOOP (bounded)            ║
║                                       ║
║  For each ready task:                 ║
║    1. Route task → backend            ║
║    2. Create worktree (if needed)     ║
║    3. Dispatch to agent               ║
║    4. Agent executes (bounded turns)  ║
║    5. Collect result + artifacts      ║
║    6. Run verification                ║
║    7. If verify fails → retry or fail ║
║    8. If approval needed → pause      ║
║    9. Mark task complete              ║
║   10. Persist event                   ║
║   11. Check run completion            ║
║                                       ║
║  Exit when: all tasks done            ║
║         or: budget exhausted          ║
║         or: max iterations reached    ║
║         or: unrecoverable failure     ║
║         or: user cancellation         ║
╚═══════════════════════════════════════╝
    │
    ▼
Orchestrator merges worktree results
    │
    ▼
Generate run summary
    │
    ▼
CLI presents results to user
```

## What is deterministic vs what is agentic

| Component | Nature | Why |
|-----------|--------|-----|
| State transitions | Deterministic | Predictable, debuggable, replayable |
| Task scheduling | Deterministic | Based on dependency graph, not heuristics |
| Retry logic | Deterministic | Fixed policy: max retries, backoff schedule |
| Timeout enforcement | Deterministic | Wall-clock limits, no negotiation |
| Approval gating | Deterministic | Pattern matching against policy rules |
| Event persistence | Deterministic | Append-only, idempotent |
| Concurrency limits | Deterministic | Hard caps, no dynamic scaling |
| Planning | Agentic | LLM decomposes goal into tasks |
| Code generation | Agentic | LLM writes code |
| Code review | Agentic | LLM analyzes diffs |
| Task-level routing | Agentic (with deterministic fallbacks) | LLM-assisted selection with rule-based overrides |
| Request-level routing | Agentic (with deterministic fallbacks) | Absorbed routing logic (from coding-agent-router) — see [Routing Logic Reference](routing-logic-reference.md) |
| Test interpretation | Agentic | LLM interprets test output |

## Where existing systems fit

| System | Role in architecture |
|--------|---------------------|
| **Codex CLI** | Agent execution engine. Workers dispatch tasks by invoking `codex exec` or using the TypeScript/Python SDK. Codex CLI handles tool execution, sandboxing, and model communication for OpenAI-backed tasks. |
| **coding-agent-router (absorbed)** | Routing logic, compaction, Ollama client, task metrics, and tool adapter code from coding-agent-router are imported as library modules within the orchestrator. No sidecar process. See [Routing Logic Reference](routing-logic-reference.md). |
| **Claude Code** | Agent execution engine (alternative to Codex CLI). For tasks routed to Anthropic/Claude, the Claude Code provider adapter invokes `claude` as a subprocess with the task prompt. |
| **Ollama** | Local model backend. The Ollama provider adapter sends requests directly to the Ollama HTTP API using the absorbed Ollama client (with per-endpoint serialization). |
| **OpenAI API** | Cloud model backend. Accessed through Codex CLI (which uses the Responses API) or directly through a provider adapter. |
| **Anthropic API** | Cloud model backend. Accessed through Claude Code (subprocess) or directly through a provider adapter for non-agentic completions. |
