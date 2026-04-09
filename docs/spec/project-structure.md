# Monorepo / Project Structure

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Recommended structure

The multi-agent orchestrator lives alongside the existing Codex CLI as a new top-level directory. The coding-agent-router remains a separate repository but is treated as a first-class dependency.

```
codex/                              # Existing Codex CLI repo (this repo)
├── codex-cli/                      # Existing: Node.js CLI wrapper
├── codex-rs/                       # Existing: Rust codebase
│   ├── cli/                        # Existing: CLI entry point (extend with `run` subcommand)
│   ├── core/                       # Existing: Core agent engine
│   ├── state/                      # Existing: SQLite state
│   └── ...                         # Other existing crates
├── sdk/                            # Existing: TypeScript/Python SDK
│
├── orchestrator/                   # NEW: Multi-agent orchestrator (Python)
│   ├── pyproject.toml              # Python project config
│   ├── src/
│   │   └── codex_orchestrator/
│   │       ├── __init__.py
│   │       ├── __main__.py         # Entry point: python -m codex_orchestrator
│   │       ├── supervisor.py       # Supervisor loop
│   │       ├── planner.py          # Planning agent interface
│   │       ├── scheduler.py        # Task scheduling and dependency resolution
│   │       │
│   │       ├── agents/
│   │       │   ├── __init__.py
│   │       │   ├── base.py         # Agent base interface
│   │       │   ├── coder.py        # Coder agent
│   │       │   ├── reviewer.py     # Reviewer agent
│   │       │   └── test_runner.py  # Test interpreter agent
│   │       │
│   │       ├── routing/
│   │       │   ├── __init__.py
│   │       │   ├── engine.py       # Task-level routing engine
│   │       │   ├── scorer.py       # Provider scoring
│   │       │   └── registry.py     # Provider capability registry
│   │       │
│   │       ├── providers/
│   │       │   ├── __init__.py
│   │       │   ├── base.py         # ProviderAdapter ABC
│   │       │   ├── codex_cli.py    # Codex CLI adapter
│   │       │   ├── claude_code.py  # Claude Code adapter
│   │       │   ├── ollama.py       # Ollama adapter
│   │       │   ├── openai_api.py   # OpenAI API adapter
│   │       │   └── anthropic_api.py # Anthropic API adapter
│   │       │
│   │       ├── policies/
│   │       │   ├── __init__.py
│   │       │   ├── engine.py       # Policy evaluation
│   │       │   └── defaults.py     # Default policy rules
│   │       │
│   │       ├── state/
│   │       │   ├── __init__.py
│   │       │   ├── store.py        # SQLite state store
│   │       │   ├── events.py       # Event log (JSONL)
│   │       │   └── migrations/     # Schema migrations
│   │       │       └── 001_initial.sql
│   │       │
│   │       ├── repo/
│   │       │   ├── __init__.py
│   │       │   └── worktree.py     # Git worktree manager
│   │       │
│   │       ├── verification/
│   │       │   ├── __init__.py
│   │       │   ├── runner.py       # Verification command runner
│   │       │   └── parser.py       # Test output parsing
│   │       │
│   │       ├── approval/
│   │       │   ├── __init__.py
│   │       │   └── gate.py         # Approval gate
│   │       │
│   │       ├── observability/
│   │       │   ├── __init__.py
│   │       │   ├── logger.py       # Structured logging
│   │       │   └── summary.py      # Run summary generation
│   │       │
│   │       └── schemas/
│   │           ├── __init__.py
│   │           ├── run.py          # Run, Task, RoutingDecision models
│   │           ├── events.py       # Event type definitions
│   │           ├── providers.py    # ProviderCapability, CostEstimate
│   │           └── config.py       # Configuration schema
│   │
│   └── tests/
│       ├── unit/
│       │   ├── test_supervisor.py
│       │   ├── test_routing.py
│       │   ├── test_scheduler.py
│       │   ├── test_policy.py
│       │   └── test_state.py
│       ├── integration/
│       │   ├── test_worktree.py
│       │   ├── test_providers.py
│       │   └── test_end_to_end.py
│       └── fixtures/
│           ├── sample_plans.json
│           ├── sample_events.jsonl
│           └── sample_config.toml
│
├── docs/
│   ├── product/                    # Product documentation (sections 1-7)
│   │   ├── 01-problem-statement.md
│   │   ├── 02-goals-and-nongoals.md
│   │   ├── 03-assumptions.md
│   │   ├── 04-key-design-questions.md
│   │   ├── 05-prd.md
│   │   ├── 06-product-concept.md
│   │   └── 07-cli-interaction-spec.md
│   └── spec/                       # Technical specification (sections 8-22)
│       ├── 08-system-architecture.md
│       ├── ...
│       └── 22-risks-and-failure-modes.md
│
└── examples/
    ├── configs/
    │   ├── minimal.toml            # Minimal config (one provider)
    │   ├── full.toml               # Full config with all providers
    │   └── offline.toml            # Offline/local-only config
    └── goals/
        ├── add-feature.md          # Example: add a feature
        ├── fix-bug.md              # Example: fix a bug
        └── refactor.md             # Example: refactoring task
```

## Relationship with coding-agent-router

The routing logic, compaction system, Ollama client, tool adapter, and task metrics from `coding-agent-router` (`/home/jesse/src/coding-agent-router/`) are **absorbed into the orchestrator as library modules**. The coding-agent-router no longer runs as a sidecar service.

```
codex CLI
    └── spawns orchestrator (Python subprocess)
         ├── routing module (absorbed from coding-agent-router/app/router.py)
         ├── compaction module (absorbed from coding-agent-router/app/compaction/)
         ├── ollama client (absorbed from coding-agent-router/app/clients/ollama_client.py)
         ├── tool adapter (absorbed from coding-agent-router/app/tool_adapter.py)
         └── task metrics (absorbed from coding-agent-router/app/task_metrics.py)
```

The orchestrator directly imports and uses this code. No HTTP communication, no sidecar process. See [Routing Logic Reference](routing-logic-reference.md) and [Compaction Reference](compaction-reference.md) for complete preservation of all heuristics and algorithms.

**What is NOT absorbed:**
- Compatibility proxy endpoints (`/v1/messages`, `/v1/chat/completions`, `/v1/responses`, `/api/chat`) — not needed since the orchestrator talks to providers directly
- Spark/mini model rewriting — becomes orchestrator routing policy rules
- Transport metrics HTTP endpoints — folded into orchestrator observability

The coding-agent-router repository remains functional for standalone use (non-orchestrated sessions) but is not part of the multi-agent architecture.

## Migration map

| Source (coding-agent-router) | Target (orchestrator) | Lines | Status |
|-----|------|-------|--------|
| `app/router.py` — `RoutingService.route()`, `build_routing_digest()`, `_fallback_route()` | `orchestrator/src/codex_orchestrator/routing/engine.py` | ~230 | Migrate |
| `app/task_metrics.py` — `extract_task_metrics()`, `estimate_tokens()` | `orchestrator/src/codex_orchestrator/routing/metrics.py` | ~196 | Migrate verbatim |
| `app/clients/ollama_client.py` — `OllamaClient`, file locking | `orchestrator/src/codex_orchestrator/providers/ollama.py` | ~147 | Migrate, replace fcntl with asyncio.Semaphore |
| `app/clients/codex_client.py` — `CodexCLIClient` | `orchestrator/src/codex_orchestrator/providers/codex_cli.py` | ~42 | Migrate |
| `app/tool_adapter.py` — tool call recovery, format translation | `orchestrator/src/codex_orchestrator/providers/tool_adapter.py` | ~283 | Migrate verbatim |
| `app/compaction/` — full compaction pipeline | `orchestrator/src/codex_orchestrator/compaction/` | ~1,500 | Migrate verbatim |
| `app/config.py` — Settings dataclass | `orchestrator/src/codex_orchestrator/schemas/config.py` | ~80 | Merge into orchestrator config |
| `app/prompts/` — router/coder/reasoner/compactor prompts | `orchestrator/src/codex_orchestrator/prompts/` | — | Copy |
| `app/compat.py` — API translation | N/A (retired) | ~1,003 | Not migrated |
| `app/compaction_main.py` — proxy + Spark rewriting | N/A (retired) | ~1,306 | Not migrated |
| `app/spark_quota.py` — circuit breaker | Routing policy rules | ~332 | Rewrite as policy |
| `app/transport_metrics.py` — telemetry | Orchestrator observability | ~741 | Fold in |

## Boundary decisions

| Code | Belongs in | Action |
|------|-----------|--------|
| Per-task routing logic | orchestrator routing module | Build new |
| Per-request routing logic | orchestrator routing module (absorbed) | Migrate from coding-agent-router |
| Task metrics extraction | orchestrator routing module (absorbed) | Migrate verbatim |
| Tool call recovery | orchestrator providers module (absorbed) | Migrate verbatim |
| Ollama client + serialization | orchestrator providers module (absorbed) | Migrate, adapt to async |
| Compaction pipeline | orchestrator compaction module (absorbed) | Migrate verbatim |
| Provider capability config | orchestrator config (TOML) | Build new, absorbing env-var defaults |
| State persistence | orchestrator state module | Build new (SQLite + JSONL) |
| Worktree management | orchestrator repo module | Build new |
| Approval gating | orchestrator approval module | Build new |
