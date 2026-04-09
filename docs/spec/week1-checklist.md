# Week-1 Execution Checklist

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Prerequisites

- [ ] Python 3.11+ installed
- [ ] Codex CLI installed and working (`codex --version`)
- [ ] Claude Code installed and working (`claude --version`)
- [ ] Ollama installed with at least one model (`ollama list`)
- [ ] Git 2.5+ installed (`git --version`)
- [ ] Access to this repository (`/home/jesse/src/codex`)
- [ ] Access to coding-agent-router source (`/home/jesse/src/coding-agent-router`) for migration

## Day 1: Project setup + code migration from coding-agent-router

- [ ] Create `orchestrator/` directory in this repo
- [ ] Create `pyproject.toml` with dependencies: pydantic, aiosqlite, httpx, pytest, pytest-asyncio, tiktoken
- [ ] Create `src/codex_orchestrator/__init__.py` and `__main__.py`
- [ ] **Migrate routing logic**: Copy `router.py` → `routing/engine.py`, `task_metrics.py` → `routing/metrics.py`. Adapt imports. Verify `extract_task_metrics()` and `estimate_tokens()` pass unit tests.
- [ ] **Migrate tool adapter**: Copy `tool_adapter.py` → `providers/tool_adapter.py`. Verify `recover_ollama_message()` passes unit tests with embedded JSON tool-call test cases.
- [ ] **Migrate Ollama client**: Copy `ollama_client.py` → `providers/ollama_client.py`. Replace `fcntl.flock` with `asyncio.Semaphore(1)`. Convert `chat()` and `chat_stream()` to async (httpx instead of requests).
- [ ] **Migrate prompts**: Copy all `.md` files from `coding-agent-router/app/prompts/` → `orchestrator/src/codex_orchestrator/prompts/`
- [ ] **Migrate compaction**: Copy entire `compaction/` directory → `orchestrator/src/codex_orchestrator/compaction/`. Fix imports to reference orchestrator modules.

**Exit: All migrated code compiles/imports clean. Core functions pass basic smoke tests.**

## Day 2: Provider analysis + migration verification

- [ ] **M0-1: Codex CLI exec mode**: Run `codex exec "Create a file hello.txt with content hello"` in a test directory. Document: command shape, exit codes, stdout format (JSONL events), stderr behavior. Save sample output.
- [ ] **M0-2: Claude Code subprocess**: Run `claude --help` and identify: non-interactive flags, JSON output mode, cwd flag, approval bypass flags. Run a test invocation. Document and save sample output.
- [ ] **Migration verification**: Write test that runs `extract_task_metrics()` on a sample prompt and verifies all 27 metrics match expected values.
- [ ] **Migration verification**: Write test that runs routing algorithm against mock Ollama (returning canned JSON) and verifies route selection matches coding-agent-router behavior for: preferred_backend override, context-window filtering, single-eligible fast path, fallback ordering.
- [ ] Write findings into `docs/spec/m0-provider-analysis.md`

**Exit: Documented invocation patterns for Codex CLI and Claude Code. Routing migration verified with tests.**

## Day 3: Schemas + state store

- [ ] **M1-1: Core Pydantic models**: Create `schemas/run.py` (Run, Task), `schemas/id.py` (ID generator). Write unit tests for validation and serialization.
- [ ] **M1-2: Event types**: Create `schemas/events.py` (Event envelope + type constants). Write unit tests.
- [ ] **M1-3: Provider models**: Create `schemas/routing.py` (RoutingDecision, ProviderCapability, HealthStatus, CostEstimate). Write unit tests.
- [ ] Create remaining schema files: `worker.py`, `approval.py`, `artifacts.py`, `repo.py`
- [ ] **M1-4: SQLite schema**: Create `state/migrations/001_initial.sql` with all tables from the [State Model](state-model.md).
- [ ] **M1-5: State store**: Create `state/store.py` with CRUD for runs, tasks, routing decisions. Write tests.

**Exit: All schemas defined and tested. State store CRUD working with SQLite.**

## Day 4: Event log + config

- [ ] **M1-6: Event log**: Create `state/events.py` with append (write one event) and read_all (read all events for a run). Write tests: append 100 events, read back, verify order and content.
- [ ] **M1-7: Config schema**: Create `schemas/config.py` with Pydantic model for TOML config. Create `examples/configs/minimal.toml`. Write loader that reads TOML, validates, returns Config object.
- [ ] **M0-5: Boundary decision document**: Based on Days 2-4 findings, write `docs/spec/m0-boundary-decisions.md` documenting what lives where.
- [ ] Run full test suite: `pytest tests/` — all green

**Exit: Complete data layer (schemas, state store, event log, config) with passing tests.**

## Day 5: Minimal supervisor loop

- [ ] **M3-4: Task state machine**: Implement `validate_transition()` in `schemas/run.py`. Write exhaustive tests for all valid and invalid transitions.
- [ ] **M3-2: Mock planner**: Create `planner.py` with a `MockPlanner` that returns a hardcoded 3-task plan (2 parallel tasks + 1 dependent task).
- [ ] **M3-3: Task scheduler**: Create `scheduler.py` with `get_ready_tasks()` that resolves dependencies. Test: with the mock plan, t1 and t2 are ready first, t3 is ready after both complete.
- [ ] **M3-1: Supervisor shell**: Create `supervisor.py` with the main loop from [Orchestrator Pseudocode](orchestrator-pseudocode.md). Use a `MockAdapter` that returns success after 1 second. Verify: loop processes 3 tasks, emits correct events, terminates.
- [ ] **M3-5: Event emission**: Wire event emission into the supervisor. Verify: events appear in JSONL log.
- [ ] Test the supervisor by invoking it the way the Rust CLI will: `echo '{}' | python -m codex_orchestrator --run-id test_001 --mock` — verify JSON events appear on stdout. **Do not build a user-facing Python CLI.** The only way to test interactively is through the Rust CLI once M2-1 is done.

**Exit: Supervisor loop processes a 3-task plan with mock provider, events emitted as JSON on stdout, state updated.**

## End of week review

By end of week 1, you should have:

| Artifact | Status |
|----------|--------|
| Python project with pyproject.toml | Working |
| IPC prototype (Rust ↔ Python) | Validated |
| Provider analysis documentation | Complete |
| All Pydantic schemas | Defined and tested |
| SQLite state store | CRUD working |
| JSONL event log | Append/read working |
| Config loader | TOML → Pydantic working |
| Task state machine | Tested |
| Task scheduler | Dependency resolution tested |
| Supervisor loop (mock) | Processes 3-task plan |
| Boundary decisions | Documented |
| ~30 unit tests | All passing |

**The foundation is in place. Week 2 builds real CLI integration and provider adapters.**
