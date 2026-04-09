# M0 Execution Plan

[< Spec Index](index.md)

## Objective

Set up the orchestrator Python project and migrate all coding-agent-router logic. Validate IPC. Document provider invocation patterns.

## Steps

### 1. Create orchestrator Python project
- Create `orchestrator/` directory structure per [Starter File Tree](starter-file-tree.md)
- Create `pyproject.toml` with deps: pydantic, aiosqlite, httpx, pytest, pytest-asyncio, tiktoken
- Create `__init__.py`, `__main__.py`, schema stubs

### 2. Migrate routing logic from coding-agent-router
- Copy `router.py` core functions → `routing/engine.py`
- Copy `task_metrics.py` → `routing/metrics.py`
- Copy `tool_adapter.py` → `providers/tool_adapter.py`
- Copy `clients/ollama_client.py` → `providers/ollama_client.py` (adapt to async with httpx)
- Copy `clients/codex_client.py` → `providers/codex_cli.py`
- Copy all prompt `.md` files → `prompts/`
- Copy entire `compaction/` directory → `compaction/`
- Fix all imports to reference orchestrator modules

### 3. Create core schemas
- `schemas/id.py` — ID generation
- `schemas/run.py` — Run, Task, VALID_TRANSITIONS
- `schemas/events.py` — Event envelope + type constants
- `schemas/routing.py` — RoutingDecision, ProviderCapability
- `schemas/worker.py` — WorkerResult, RepositoryContext
- `schemas/approval.py` — ApprovalRequest
- `schemas/artifacts.py` — ArtifactRecord
- `schemas/config.py` — Config model loading from TOML

### 4. Write tests
- Unit tests for schemas (validation, serialization)
- Unit tests for task metrics (verify 27 metrics against sample prompt)
- Unit tests for routing algorithm (preferred_backend, context filtering, fallback order)
- Unit tests for tool-call recovery (embedded JSON, devstral, streaming dedup)

### 5. Provider analysis
- Document `codex exec` invocation pattern with sample output
- Document `claude` subprocess invocation pattern with sample output
- Save findings to `docs/spec/m0-provider-analysis.md`
