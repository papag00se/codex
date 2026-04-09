# Starter File Tree

[< Spec Index](index.md) | [Product Index](../product/index.md)

See [Project Structure](project-structure.md) for the relationship between this tree and the existing Codex CLI and coding-agent-router codebases. See [Initial Data Models](initial-data-models.md) for the Pydantic model code.

```
orchestrator/
├── pyproject.toml
├── README.md
├── src/
│   └── codex_orchestrator/
│       ├── __init__.py
│       ├── __main__.py                  # IPC entry: python -m codex_orchestrator (invoked by Rust CLI only, never by user)
│       │
│       ├── schemas/
│       │   ├── __init__.py
│       │   ├── run.py                   # Run, Task models
│       │   ├── events.py               # Event envelope + all event types
│       │   ├── routing.py              # RoutingDecision, ProviderCapability
│       │   ├── approval.py             # ApprovalRequest
│       │   ├── artifacts.py            # ArtifactRecord
│       │   ├── repo.py                 # RepositoryContext
│       │   ├── worker.py              # WorkerResult
│       │   └── config.py              # Config schema
│       │
│       ├── supervisor.py               # Supervisor loop
│       ├── scheduler.py               # Task dependency resolution + scheduling
│       ├── planner.py                 # Planner agent (LLM call)
│       │
│       ├── routing/
│       │   ├── __init__.py
│       │   ├── engine.py              # route_task()
│       │   ├── scorer.py              # compute_score()
│       │   └── registry.py            # ProviderCapabilityRegistry
│       │
│       ├── providers/
│       │   ├── __init__.py
│       │   ├── base.py               # ProviderAdapter ABC
│       │   ├── codex_cli.py           # CodexCliAdapter
│       │   ├── claude_code.py         # ClaudeCodeAdapter
│       │   ├── ollama.py              # OllamaAdapter
│       │   ├── openai_api.py          # OpenAiApiAdapter
│       │   └── anthropic_api.py       # AnthropicApiAdapter
│       │
│       ├── state/
│       │   ├── __init__.py
│       │   ├── store.py               # SQLite state store
│       │   ├── events.py              # JSONL event log
│       │   └── migrations/
│       │       └── 001_initial.sql
│       │
│       ├── repo/
│       │   ├── __init__.py
│       │   └── worktree.py            # Git worktree manager
│       │
│       ├── policies/
│       │   ├── __init__.py
│       │   ├── engine.py              # Policy evaluation
│       │   └── defaults.py            # Default policy rules
│       │
│       ├── verification/
│       │   ├── __init__.py
│       │   └── runner.py              # Verification command runner
│       │
│       ├── approval/
│       │   ├── __init__.py
│       │   └── gate.py                # Approval gate (stdin/stdout)
│       │
│       └── observability/
│           ├── __init__.py
│           ├── logger.py              # Structured JSON logger
│           └── summary.py             # Run summary generator
│
├── tests/
│   ├── conftest.py                    # Shared fixtures
│   ├── unit/
│   │   ├── test_schemas.py
│   │   ├── test_supervisor.py
│   │   ├── test_scheduler.py
│   │   ├── test_routing.py
│   │   ├── test_policy.py
│   │   └── test_state_store.py
│   ├── integration/
│   │   ├── test_worktree.py
│   │   ├── test_providers.py
│   │   └── test_end_to_end.py
│   └── fixtures/
│       ├── sample_config.toml
│       ├── sample_plan.json
│       └── sample_events.jsonl
│
├── examples/
│   ├── configs/
│   │   ├── minimal.toml
│   │   ├── full.toml
│   │   └── offline.toml
│   └── goals/
│       ├── add-feature.md
│       └── fix-bug.md
│
└── docs/
    └── getting-started.md
```
