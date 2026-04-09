# Implementation Order

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Recommended build order with rationale

```
Week 1: M0 (Discovery) + M1 (Schemas)
    ↓
Week 2: M2 (CLI) + M3 (Orchestrator) in parallel
    ↓
Week 3: M4 (Routing) + M5 (Providers) in parallel
    ↓
Week 4: M6 (Worktrees) + M7 (Verification/Approval)
    ↓
Week 5: M8 (Observability) + M9 (Hardening)
```

## Detailed order

### Phase A: Foundation (Week 1)

**Build order:**
1. **M0-4: IPC prototype** — Validate the Rust ↔ Python subprocess communication before building anything else. If this doesn't work cleanly, the entire architecture needs revision.
2. **M0-1, M0-2, M0-3: Provider analysis** — In parallel. Understand the exact input/output contracts of each provider.
3. **M0-5: Boundary decisions** — Depends on M0-1..3. Finalize where code lives.
4. **M1-1: Core Pydantic models** — Run, Task, WorkerResult. These are the nouns of the system.
5. **M1-2: Event types** — Define before implementation so all code emits consistent events.
6. **M1-4, M1-5: SQLite schema + state store** — The persistence layer everything writes to.
7. **M1-6: Event log** — Append-only JSONL. Simple but must be correct.
8. **M0-6, M1-7: Config schema** — Load from TOML. Needed by everything.

**Rationale:** Schemas and state come first because every other component depends on them. The IPC prototype validates the most risky architectural decision early.

### Phase B: Core Loop (Week 2)

**Build order:**
1. **M3-7: CLI-orchestrator IPC** — Wire up the Rust ↔ Python channel using the prototype from M0-4. This is the JSON-lines protocol between `codex run` (Rust) and `python -m codex_orchestrator` (Python). The Rust side spawns the subprocess; the Python side reads/writes stdin/stdout.
2. **M2-1: `codex run` subcommand (Rust)** — Add `Run` variant to `Subcommand` enum in `codex-rs/cli/src/main.rs`. It spawns the orchestrator subprocess, reads JSON events from stdout, and renders them to the terminal. **This is a Rust change.** The user types `codex run "goal"` — never `python` or `codex-orchestrator`.
3. **M3-4: Task state machine** — Define transitions before the supervisor loop (see [State Model](state-model.md)). This is the contract.
4. **M3-2: Planner interface** — Mock planner first. Returns a hardcoded 3-task plan.
5. **M3-3: Task scheduler** — Dependency resolution, ready queue. Test with mock tasks.
6. **M3-1: Supervisor main loop** — Wire it all together: plan → schedule → dispatch (mock) → complete.
7. **M3-5: Event emission** — Every state change emits an event. Verify with event log.
8. **M2-2, M2-3: status + inspect commands (Rust)** — `codex run status` and `codex run inspect` read state store and event log.

**Critical note:** During development, a temporary `python -m codex_orchestrator.cli` may exist for testing the supervisor loop in isolation. It must be clearly marked as a development tool and removed before release. The user-facing entry point is always `codex run`.

**Rationale:** The supervisor loop is the heart. Build it early, test with mocks, replace mocks with real components later.

### Phase C: Routing + Providers (Week 3)

**Build order:**
1. **M4-migrate: Absorb routing logic** — Copy `router.py`, `task_metrics.py`, `tool_adapter.py` from coding-agent-router. Adapt to async. Write unit tests verifying identical behavior.
2. **M4-migrate: Absorb compaction pipeline** — Copy `compaction/` from coding-agent-router. Write integration test with test transcript.
3. **M4-migrate: Absorb Ollama client** — Copy `ollama_client.py`, replace fcntl with asyncio.Semaphore. Test against live Ollama.
4. **M4-migrate: Copy prompts** — Copy all prompt files from coding-agent-router verbatim.
5. **M1-3: Provider models** — ProviderCapability schema.
6. **M4-1: Capability registry** — Load from config, query.
7. **M4-2: Task-level routing scorer** — Score providers for tasks, wrapping absorbed per-request logic.
8. **M5-1: ProviderAdapter ABC** — Define the interface before implementing.
9. **M5-4: Ollama adapter** — Wraps absorbed OllamaClient + tool_adapter.
10. **M5-2: Codex CLI adapter** — Absorbs `codex_client.py`, wraps as ProviderAdapter.
11. **M5-3: Claude Code adapter** — Subprocess. Parse JSON output.
12. **M4-3: Routing decision logger** — Persist decisions.
13. **M4-4: Fallback logic** — Re-route on failure (preserving deterministic order).

**Rationale:** Absorb coding-agent-router code first (steps 1-4) since everything downstream depends on it. Then build the new abstractions on top. The absorbed code provides working routing, Ollama communication, and compaction from day one.

### Phase D: Isolation + Verification (Week 4)

**Build order:**
1. **M6-1: Worktree creation** — `git worktree add` wrapper.
2. **M6-2: Worktree cleanup** — Remove on completion/failure.
3. **M6-3: Merge into result branch** — The critical path for accepting work.
4. **M7-1: Verification runner** — Run test command, check exit code.
5. **M7-2: Policy engine** — Pattern matching for approval.
6. **M7-3: Approval gate** — Interactive prompt.
7. **M7-4: Retry with feedback** — Feed verification failure to agent.
8. **M3-6: Resume logic** — Now that all components exist, test resume end-to-end.

**Rationale:** Worktrees and verification are closely coupled (verification runs in the worktree). Build them together. Resume logic tested last because it exercises everything.

### Phase E: Polish (Week 5)

**Build order:**
1. **M8-1, M8-2: Structured logging** — Retrofit logging to all components.
2. **M8-3: Run summary generator** — The payoff: human-readable run summary.
3. **M8-4, M8-5: CLI query commands** — `logs`, `inspect --routing`.
4. **M4-5, M4-6: Budget + override** — Non-critical routing features.
5. **M6-4, M6-5: Orphan cleanup + concurrency limit** — Robustness.
6. **M7-5: Retry escalation** — Route to stronger backend on retry.
7. **M9-1..6: Hardening** — End-to-end tests, crash recovery, docs.

**Rationale:** Observability comes late because it's cross-cutting — easier to add once all components exist. Hardening is the final pass.

## Critical path

```
IPC prototype → Schemas → State store → Supervisor loop → Providers → Worktrees → Verification → E2E test
```

The critical path is 8 dependencies long. Routing and observability are off the critical path and can proceed in parallel.
