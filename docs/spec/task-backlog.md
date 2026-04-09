# Detailed Task Backlog

[< Spec Index](index.md) | [Product Index](../product/index.md)

## M0: Discovery and Import Analysis

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M0-1 | Codex CLI exec mode analysis | Document how `codex exec` works: input format, output format, exit codes, env vars. Test with sample prompts. | - | P0 | Low | Working example of programmatic Codex exec invocation with parsed output |
| M0-2 | Claude Code subprocess analysis | Document how `claude` CLI works: flags for non-interactive mode, output format, JSON mode, cwd support. | - | P0 | Low | Working example of programmatic Claude Code invocation with parsed output |
| M0-3 | coding-agent-router API analysis | Document all endpoints, request/response schemas, health check. Test integration. | - | P0 | Low | HTTP client wrapper that can invoke /invoke and /health |
| M0-4 | IPC prototype | Prototype: Rust CLI spawns Python subprocess, exchanges JSON on stdin/stdout. Validate latency and reliability. | - | P0 | Medium | Rust → Python subprocess with bidirectional JSON messaging working |
| M0-5 | Boundary decision document | Document which code lives where (orchestrator vs router vs CLI). Resolve boundary violations. | M0-1..3 | P0 | Low | Written document with table of decisions, reviewed |
| M0-6 | Config schema draft | Draft TOML config schema for providers, routing, approval, verification. | M0-5 | P1 | Low | TOML file with all sections, validated by schema |

## M1: Schemas, State, and Events

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M1-1 | Core Pydantic models | Define Run, Task, RoutingDecision, WorkerResult models with validation | M0-5 | P0 | Medium | Models with full field validation, JSON serialization, unit tests |
| M1-2 | Event type definitions | Define all event types with payload schemas | M0-5 | P0 | Medium | Event envelope + all event types from the [Event Model](event-model.md), unit tests |
| M1-3 | Provider models | Define ProviderCapability, CostEstimate, HealthStatus | M0-5 | P0 | Low | Models with validation and sample data |
| M1-4 | SQLite schema | Create initial schema: runs, tasks, routing_decisions, approval_requests, artifacts, worktrees tables | M1-1 | P0 | Medium | Migration script, CRUD tests pass |
| M1-5 | State store implementation | CRUD operations for all tables with WAL mode | M1-4 | P0 | Medium | All CRUD operations tested, concurrent read/write test |
| M1-6 | Event log writer/reader | Append-only JSONL writer, sequential reader, sequence numbering | M1-2 | P0 | Low | Write 1000 events, read back, verify order and content |
| M1-7 | Config schema implementation | Pydantic config model that loads from TOML | M0-6 | P1 | Low | Load sample config, validate, test defaults |

## M2: CLI Foundation

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M2-1 | `codex run` subcommand | Add Run variant to CLI subcommand enum. Parse goal and options. Spawn orchestrator subprocess. | M1-7 | P0 | Medium | `codex run "goal"` spawns Python process, displays output |
| M2-2 | `codex run status` | Read active runs from state store, display table | M1-5 | P0 | Low | Shows run ID, goal, status, task progress, elapsed |
| M2-3 | `codex run inspect` | Read run details from state store + event log, display | M1-5, M1-6 | P0 | Medium | Shows summary, supports --routing, --events, --json |
| M2-4 | `codex run cancel` | Send cancel signal to orchestrator, update state | M2-1 | P1 | Low | Running orchestrator receives cancel, shuts down gracefully |
| M2-5 | `codex run resume` | Restart orchestrator with existing run ID | M2-1 | P1 | Medium | Orchestrator starts, reads state, skips completed tasks |
| M2-6 | JSON output mode | All subcommands support --json flag | M2-1..3 | P1 | Low | All commands produce valid JSON on stdout with --json |

## M3: Orchestrator Loop

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M3-1 | Supervisor main loop | While loop with iteration counter, timeout, budget check. Process events. | M1-5, M1-6 | P0 | High | Loop runs, processes tasks, terminates at limits |
| M3-2 | Planner interface | ABC for planner. Mock planner for testing. LLM planner for production. | M1-1 | P0 | Medium | Mock planner returns hardcoded plan. LLM planner calls provider. |
| M3-3 | Task scheduler | Dependency-aware scheduling. Ready queue. Parallel dispatch up to limit. | M1-1 | P0 | High | 5-task plan with deps: correct order, parallel where possible |
| M3-4 | Task state machine | All transitions from the [State Model](state-model.md). Validate transitions. Persist on change. | M1-1, M1-5 | P0 | Medium | All valid transitions work. Invalid transitions raise error. |
| M3-5 | Event emission | Emit events for all state changes. Persist to JSONL. | M1-6, M3-4 | P0 | Low | Every state change produces an event in the log |
| M3-6 | Resume logic | Read state store, identify last completed task, re-dispatch pending. | M1-5, M3-1 | P0 | High | Simulate crash (kill process), resume completes run |
| M3-7 | CLI-orchestrator IPC | JSON messaging over stdin/stdout between Rust CLI and Python orchestrator | M2-1 | P0 | Medium | Bidirectional: CLI sends commands, orchestrator sends events |

## M4: Routing Engine

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M4-1 | Capability registry | Load provider capabilities from config. Track health. | M1-3, M1-7 | P0 | Low | Load config, query capabilities, probe health |
| M4-2 | Routing scorer | Score providers for a task. Quality × cost × policy weighting. | M4-1 | P0 | Medium | Correct scoring for test cases (high complexity → frontier, simple → local) |
| M4-3 | Routing decision logger | Persist routing decisions with full context | M1-5, M4-2 | P0 | Low | Decisions queryable in state store |
| M4-4 | Fallback logic | When selected provider fails, re-route to next best | M4-2 | P0 | Medium | Provider failure triggers fallback, logged |
| M4-5 | Budget-aware routing | Track spend, prefer cheaper when budget low | M4-2 | P1 | Low | Budget at 80% triggers preference change |
| M4-6 | Manual override | --backend flag overrides routing | M4-2 | P1 | Low | Override works, logged as confidence 1.0 |
| M4-M1 | Migrate routing logic | Copy router.py, task_metrics.py from coding-agent-router. Adapt imports. Write tests matching original behavior. See [Routing Logic Reference](routing-logic-reference.md). | - | P0 | Medium | extract_task_metrics() and route() produce identical output to coding-agent-router for test fixtures |
| M4-M2 | Migrate tool adapter | Copy tool_adapter.py. Write tests for recover_ollama_message() with embedded JSON tool-call test cases. | - | P0 | Low | All tool-call recovery paths tested |
| M4-M3 | Migrate Ollama client | Copy ollama_client.py. Replace fcntl with asyncio.Semaphore. Convert to async. | - | P0 | Medium | chat() and chat_stream() work against live Ollama |
| M4-M4 | Migrate compaction pipeline | Copy compaction/ directory. Fix imports. Write integration test. See [Compaction Reference](compaction-reference.md). | M4-M3 | P0 | High | Compaction produces valid handoff from test transcript |
| M4-M5 | Migrate prompts | Copy all prompt .md files from coding-agent-router verbatim. | - | P0 | Low | All prompts present and loadable |

## M5: Provider Adapters

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M5-1 | ProviderAdapter ABC | Define abstract base with execute_task, capabilities, health, estimate_cost, cancel | M1-3 | P0 | Low | ABC defined with type hints and docstrings |
| M5-2 | Codex CLI adapter | Wraps absorbed codex_client.py as ProviderAdapter. Subprocess `codex exec`, JSONL output parsing | M0-1, M5-1, M4-M1 | P0 | High | Execute simple task, parse result, handle timeout |
| M5-3 | Claude Code adapter | Subprocess invocation of `claude`, JSON output parsing | M0-2, M5-1 | P0 | High | Execute simple task, parse result, handle timeout |
| M5-4 | Ollama adapter | Wraps absorbed OllamaClient + tool_adapter as ProviderAdapter | M5-1, M4-M2, M4-M3 | P0 | Medium | Execute simple completion, handle streaming, tool-call recovery, timeout |
| M5-5 | Provider health checks | Health probe for each adapter (subprocess exists / HTTP responds) | M5-2..4 | P1 | Low | Health returns correct status for up/down providers |
| M5-6 | Cost estimation | Token estimation per adapter | M5-2..4 | P1 | Low | Estimates within 2x of actual for test tasks |

## M6: Repository / Worktree Isolation

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M6-1 | Worktree creation | Create worktree from current branch for a task | - | P0 | Medium | Worktree created in correct path with correct branch |
| M6-2 | Worktree cleanup | Remove worktree and branch after task completion/failure | M6-1 | P0 | Low | Worktree removed, branch deleted, no artifacts left |
| M6-3 | Merge into result branch | Merge task branch into run result branch | M6-1 | P0 | Medium | Clean merge succeeds, conflict detected and reported |
| M6-4 | Orphan cleanup | Detect and clean orphaned worktrees on startup | M6-1, M1-5 | P1 | Medium | After simulated crash, orphans detected and cleaned |
| M6-5 | Concurrency limit | Queue tasks when all worktree slots occupied | M6-1, M3-3 | P1 | Low | 5 tasks with max_parallel=2: correct queuing |

## M7: Verification and Approval

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M7-1 | Verification runner | Run configured command in worktree, capture exit code and output | - | P0 | Low | Run `pytest`, capture pass/fail, parse count |
| M7-2 | Policy engine | Load policy from config, match actions against patterns | M1-7 | P0 | Medium | Shell patterns match correctly, file patterns match |
| M7-3 | Approval gate | Interactive prompt with timeout, record decision | M7-2 | P0 | Medium | Prompt appears, timeout works, decision recorded |
| M7-4 | Retry with feedback | On verification failure, re-dispatch task with failure details | M7-1, M3-1 | P0 | Medium | Agent receives failure context, retry succeeds |
| M7-5 | Retry escalation | On retry, optionally route to stronger backend | M7-4, M4-2 | P1 | Low | Retry routes to next-best provider |

## M8: Observability

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M8-1 | Structured logger | JSON logger with run_id/task_id correlation | - | P0 | Low | All components log structured JSON |
| M8-2 | Per-task log files | Route task-specific logs to separate files | M8-1 | P1 | Low | Each task has its own log file |
| M8-3 | Run summary generator | Generate summary JSON on run completion | M1-5, M1-6 | P0 | Medium | Summary includes tasks, routing, cost, verification |
| M8-4 | `codex run logs` command | Stream/view logs with filtering | M8-1, M2-1 | P1 | Medium | Filtering by task, level, component works |
| M8-5 | Routing summary view | `inspect --routing` renders routing table | M4-3, M2-3 | P1 | Low | Table shows all tasks with backend, confidence, reason |

## M9: Hardening

| ID | Title | Description | Deps | Priority | Complexity | Acceptance Criteria |
|----|-------|-------------|------|----------|------------|---------------------|
| M9-1 | End-to-end test | Full run with real providers on a test repo | All | P0 | High | Complete run with 3+ backends, all tasks verified |
| M9-2 | Crash recovery test | Kill orchestrator, resume, verify completion | M3-6 | P0 | Medium | Run completes correctly after kill -9 |
| M9-3 | Timeout tests | Verify all timeout paths | M3-1, M5-2..4 | P0 | Medium | Task timeout, run timeout, approval timeout all fire |
| M9-4 | Offline mode test | Run with only Ollama available | M5-4 | P1 | Low | Run completes with local models, warnings logged |
| M9-5 | Error message audit | Review all error paths for clear messaging | All | P1 | Low | All errors are actionable, no stack traces to user |
| M9-6 | Getting started guide | User documentation for setup and first run | All | P1 | Medium | New user can install, configure, and run within 15 min |
