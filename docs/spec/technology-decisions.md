# Technology Decision Record

[< Spec Index](index.md) | [Product Index](../product/index.md)

## TDR-1: Integration Strategy

| | |
|---|---|
| **Decision** | Multi-agent orchestration is a **capability within the existing Codex agent**, not a separate process or mode. See [Integration Model](integration-model.md). |
| **Preferred v1** | The user launches `codex` (normal interactive TUI). The main Codex agent decomposes complex goals by spawning specialist sub-agents via the existing `multi_agent_v2` system. Routing logic runs as an MCP server. |
| **Rejected alternatives** | (A) `codex run` subcommand with Python subprocess orchestrator — **tried and rejected twice**. First as a standalone Python CLI, then as an invisible subprocess. Both create a parallel system that duplicates what Codex already has (agent spawning, state, events, TUI, approvals). (B) Separate binary. (C) Fork Codex CLI. |
| **Why this works** | Codex already has: agent spawning (`AgentControl`), registry with thread limits, depth limits, agent roles, inter-agent messaging, model overrides per spawn, worktree isolation, multi-agent TUI, JSONL rollout, SQLite state. Building on this instead of beside it. |
| **Tradeoff** | The main Codex agent must be smart enough to act as supervisor — deciding when to decompose, what roles to spawn, when to verify. This depends on the frontier model's planning capability. Mitigated: a **deterministic supervisor loop** (Rust) drives the process; the LLM provides judgment (planning, evaluation) but cannot opt out. See [Design Principles](design-principles.md). |

## TDR-2: Routing and Compaction Runtime

| | |
|---|---|
| **Decision** | Routing and compaction logic implemented in **Rust**, in codex-core. No Python at runtime, no MCP server, no separate process. |
| **Preferred v1** | Port routing algorithm from Python reference implementation to a Rust crate (`codex-routing` or module within codex-core). Compaction deferred — port when long sessions need it. |
| **Rejected alternatives** | (A) Python MCP server — **rejected: still a separate process**, adds Python runtime dependency, latency on every routing call, another thing to manage. (B) Python subprocess with custom IPC — rejected for the same reasons. (C) Keep coding-agent-router as HTTP sidecar — rejected: adds even more operational complexity. |
| **Why Rust** | The routing algorithm is not complex: filter by context window (arithmetic), check eligibility (boolean), call Ollama HTTP API (reqwest), parse JSON (serde), fallback order (match). Task metrics are 27 regex patterns — Rust's `regex` crate handles them. No Python-specific magic in the logic. |
| **Risk mitigation** | The Python code is preserved as a reference implementation in `orchestrator/` with 79 passing tests. The preservation docs ([Routing Logic Reference](routing-logic-reference.md), [Compaction Reference](compaction-reference.md)) capture every heuristic, threshold, and decision path. Rust port must match these docs. |
| **Tradeoff** | Porting ~1,200 lines of routing Python to Rust. Acceptable: the algorithm is well-documented and testable. Compaction (~1,500 lines) is deferred. |

## TDR-3: Provider / Backend Integration

| | |
|---|---|
| **Decision** | The supervisor loop dispatches work to sub-agents via existing `spawn_agent` infrastructure, passing model overrides from routing decisions. No provider adapter abstraction layer. |
| **Preferred v1** | Each sub-agent task is dispatched by calling `AgentControl::spawn_agent_with_metadata()` with the routed model in the config. The existing Codex agent execution engine handles all provider communication (OpenAI Responses API, etc.). |
| **Why not a provider adapter ABC** | Codex already has a model client (`core/src/client.rs`) that handles provider communication. Building a second abstraction layer on top would duplicate this. The routing decision selects a model; the existing infrastructure talks to that model. |
| **Tradeoff** | Adding a new provider (e.g., direct Ollama) requires changes to the existing model client, not a pluggable adapter. Acceptable for v1 where providers are: OpenAI (via Codex), Ollama (via Codex's Ollama support), and the local router model. |

## TDR-5: State Store

| | |
|---|---|
| **Decision** | SQLite (WAL mode) for queryable state + JSONL for event log |
| **Preferred v1** | SQLite at `~/.codex/multi-agent/state.db`, JSONL at `~/.codex/multi-agent/runs/<id>/events.jsonl` |
| **Alternatives** | (A) SQLite only. (B) JSONL only. (C) Postgres. (D) DuckDB. |
| **Why not A** | Loses the append-only, replayable event log property. SQLite updates are in-place. |
| **Why not B** | JSONL is not efficiently queryable for "give me all tasks in status X." |
| **Why not C** | External dependency. Overkill for single-machine. |
| **Why not D** | Analytical focus; weaker for transactional writes. |
| **Tradeoff** | Two stores to keep in sync. Mitigated: SQLite is the primary read/write path; JSONL is append-only and serves as audit log + crash recovery source. |

## TDR-6: Queue / Event Transport

| | |
|---|---|
| **Decision** | In-process async channels (Python asyncio.Queue) |
| **Preferred v1** | `asyncio.Queue` for in-process event delivery; JSONL file as durable backing |
| **Alternatives** | (A) Redis Streams. (B) SQLite as queue. (C) ZeroMQ. (D) Named pipes. |
| **Why not A** | External dependency. Overkill for single-process. |
| **Why not B** | Possible but slower for high-frequency events. |
| **Why not C** | External dependency. |
| **Why not D** | Cross-platform issues. |
| **Tradeoff** | In-process means no distributed event consumption. Fine for v1 (single machine, single orchestrator process). |

## TDR-7: Repository Isolation Method

| | |
|---|---|
| **Decision** | Git worktrees |
| **Preferred v1** | `git worktree add` for each parallel agent task |
| **Alternatives** | (A) Branch switching. (B) Patch files. (C) Docker containers. (D) Full repo copies. |
| **Why not A** | No parallel execution — only one branch checked out at a time. |
| **Why not B** | Cannot run tests against a patch file. |
| **Why not C** | Slow, heavyweight, requires Docker. |
| **Why not D** | Wastes disk space, slow for large repos. |
| **Tradeoff** | Worktrees consume disk (~working directory size per worktree). Acceptable for most repos. Large monorepos may need shallow worktrees (future optimization). |

## TDR-8: Verifier / Test Harness

| | |
|---|---|
| **Decision** | Subprocess execution of user-configured test commands |
| **Preferred v1** | Run `verification.command` from config in the worktree directory, check exit code |
| **Alternatives** | (A) Built-in test framework. (B) Language-specific test parsers. (C) LLM-only review. |
| **Why not A** | Users have their own test frameworks. We should run them, not replace them. |
| **Why not B** | Too many languages to support. Exit code is universal. |
| **Why not C** | LLM review misses real test failures. Deterministic testing is non-negotiable. |
| **Tradeoff** | Depends on user having a working test command. If no tests configured, tasks are marked "unverified-complete." |

## TDR-9: Observability Stack

| | |
|---|---|
| **Decision** | Structured JSON logs + CLI query commands |
| **Preferred v1** | JSON log files per run, queryable via `codex run inspect/logs` |
| **Alternatives** | (A) OpenTelemetry + Jaeger. (B) Prometheus + Grafana. (C) SQLite for logs too. |
| **Why not A** | External infrastructure. Overkill for single-machine. |
| **Why not B** | External infrastructure. |
| **Why not C** | SQLite is not ideal for append-heavy log data. JSONL is simpler and more portable. |
| **Tradeoff** | No fancy dashboards. Users query via CLI. A future UI can read the same structured logs. |

## TDR-10: Orchestrator Language

| | |
|---|---|
| **Decision** | Python 3.11+ |
| **Preferred v1** | Python for the orchestrator, provider adapters, and routing engine |
| **Alternatives** | (A) Rust. (B) TypeScript. (C) Go. |
| **Why not A (for v1)** | Slower iteration. The orchestrator is control-plane logic, not performance-critical. coding-agent-router is already Python. Rewriting to Rust is an option for v2. |
| **Why not B** | Weaker async ecosystem for subprocess management. |
| **Why not C** | Third language. |
| **Tradeoff** | Python adds a runtime dependency. Users must have Python 3.11+. The Codex CLI already requires Node.js (for its wrapper), so this adds one more runtime. |
