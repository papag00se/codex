# Milestone Plan

[< Spec Index](index.md) | [Product Index](../product/index.md)

> **Architecture:** The orchestrator is a capability within the existing Codex agent, not a separate process. The user launches `codex` (normal TUI), and the agent spawns specialists via the existing `multi_agent_v2` system. See [Integration Model](integration-model.md).

## M0: Discovery and Code Migration (DONE)

**Goal:** Understand existing assets. Migrate routing and compaction code from coding-agent-router.

**Deliverables:**
- Architecture review of Codex CLI agent system (`AgentControl`, `multi_agent_v2`, agent roles)
- Architecture review of coding-agent-router (routing logic, compaction, tool adapter)
- Migrated Python code: routing metrics, tool adapter, Ollama client, compaction pipeline, prompts
- Pydantic schemas for routing decisions, provider capabilities
- 79 unit tests passing

**Status:** Complete. Code in `orchestrator/src/codex_orchestrator/`.

---

## M1: Routing Engine (Rust, in codex-core)

**Goal:** Port the routing algorithm from the Python reference to Rust. No separate process, no Python at runtime.

**Deliverables:**
- New Rust crate or module (e.g., `codex-routing` or within `codex-core`) containing:
  - **Task metrics extraction** — port all 27 regex patterns from [Routing Logic Reference](routing-logic-reference.md). Uses Rust `regex` crate.
  - **Token estimation** — `(len + 3) / 4` quick estimate (matching Python) plus optional `tiktoken-rs` for precise counts
  - **Route selection algorithm** — exact port of the decision flow:
    1. Filter by context window (deterministic)
    2. If one eligible, return it (deterministic)
    3. If multiple, call local Ollama router model via HTTP (LLM judgment)
    4. Parse JSON response (deterministic)
    5. Fallback order if parse fails: coder → reasoner → codex_cli (deterministic)
  - **Routing digest builder** — construct the JSON payload sent to the router model
  - **Ollama HTTP client** — async HTTP client (reqwest) with per-endpoint semaphore serialization (replaces Python's fcntl file locks)
  - **Tool-call recovery** — port embedded JSON recovery for devstral/qwen models from [Routing Logic Reference](routing-logic-reference.md)
  - **Config** — routing config section in `config.toml` with all knobs from the Python reference (model names, context windows, temperatures, timeouts)
- Unit tests matching the Python test suite (task metrics, routing decisions, tool-call recovery)
- Integration test: route a sample task description and verify decision

**Dependencies:** M0 (Python reference code + preservation docs).

**Exit criteria:**
- `route_task()` in Rust produces identical decisions to the Python reference for the same inputs
- All 27 task metrics match between Rust and Python for test fixtures
- Tool-call recovery handles all test cases from the Python suite
- Ollama client serializes requests per endpoint
- Config loads from TOML with correct defaults

---

## M2: Deterministic Supervisor Loop (Rust, in codex-core)

**Goal:** Build a supervisor loop in Rust that drives multi-agent work to completion. The LLM cannot exit the loop. See [Design Principles](design-principles.md).

**Deliverables:**
- Supervisor loop in codex-core that:
  - Accepts a goal from the user
  - Calls the LLM to decompose the goal into a task graph (LLM judgment: planning)
  - Holds the task graph with explicit states: `pending`, `running`, `completed`, `failed`
  - Dispatches ready tasks to sub-agents via existing `spawn_agent` (deterministic: dependency graph)
  - Evaluates each agent's result via LLM call: "is this task complete?" (LLM judgment: evaluation)
  - Runs verification command if configured (deterministic: subprocess + exit code)
  - Interprets verification output via LLM: "did tests pass?" (LLM judgment: interpretation)
  - Retries on failure, escalating to stronger model (deterministic: retry count, routing decision)
  - Terminates only when: all tasks terminal, budget exhausted, max iterations hit, or timeout (deterministic: never by LLM choice)
- Agent role configs:
  - `coder` — implements code changes
  - `reviewer` — reviews diffs, read-only
  - `test-runner` — runs and interprets tests
- Configuration:
  ```toml
  [orchestrator]
  max_iterations = 50
  timeout_seconds = 7200
  max_retries_per_task = 3
  verification_command = "pytest tests/"
  ```

**Dependencies:** M1 (Rust routing engine for model selection).

**Exit criteria:**
- `codex` given a complex goal spawns sub-agents and drives to completion without stopping to ask
- The loop continues as long as tasks are pending — no "shall I continue?" behavior
- Failed tasks retry automatically (up to max_retries)
- Simple goals are handled directly (supervisor detects single-task goals)
- The loop stops at timeout, max iterations, or all tasks complete — never because the LLM decided to stop

---

## M3: Routing Integration with Agent Spawning

**Goal:** Wire Rust routing engine into the supervisor loop so sub-agents automatically use the routed model.

**Deliverables:**
- When the supervisor dispatches a task, it:
  1. Calls `route_task()` (Rust, in-process) with the sub-task description
  2. Receives `RouteDecision { model, confidence, reason }`
  3. Passes `model` to `spawn_agent_with_metadata()` via the config model override
- Routing decisions emitted as events and visible in the TUI
- Fallback behavior: if no Ollama router available, use default model from config

**Dependencies:** M1 (Rust routing engine), M2 (supervisor loop dispatches tasks).

**Exit criteria:**
- Sub-agents spawned with different models based on task type
- Routing reasons visible in agent output / TUI
- Works when Ollama router is unavailable (graceful fallback to default model)

---

## M4: Verification Loop

**Goal:** After sub-agents complete, the supervisor runs verification and handles failures.

**Deliverables:**
- Supervisor loop verification step:
  1. Sub-agent completes → supervisor runs test command (deterministic: subprocess)
  2. Supervisor asks LLM to interpret output (LLM judgment: did tests pass?)
  3. Tests pass → mark task complete (deterministic)
  4. Tests fail → retry sub-agent with failure context (deterministic: retry count)
  5. Max retries exhausted → mark task failed (deterministic)
- Configurable verification command in project config:
  ```toml
  [orchestrator]
  verification_command = "pytest tests/"
  ```
- Retry behavior uses routing escalation: first retry same model, second retry stronger model

**Dependencies:** M2 (supervisor behavior), M3 (routing for escalation).

**Exit criteria:**
- Sub-agent produces code that fails tests → main agent retries with feedback
- Retry escalates to stronger model
- Max retries respected (default: 3)
- Tests pass → changes accepted

---

## M5: Observability and Polish

**Goal:** Make routing decisions, agent activity, and verification results visible and debuggable.

**Deliverables:**
- Routing decisions logged in agent messages (visible in TUI)
- Summary of which models were used for which tasks
- Cost tracking across sub-agents
- Agent role labels visible in TUI multi-agent view
- Documentation: how to configure roles, routing, verification

**Dependencies:** M3 (routing integrated), M4 (verification working).

**Exit criteria:**
- User can see routing decisions in the TUI
- User can see total cost across all sub-agents
- Documentation sufficient for setup

---

## M6: Hardening

**Goal:** End-to-end testing, edge cases, error handling.

**Deliverables:**
- End-to-end test: complex goal with 3+ sub-agents using different models
- Test: routing server down → graceful fallback
- Test: sub-agent fails → retry with escalation
- Test: all retries fail → clear error to user
- Test: simple goal → no decomposition (single agent)
- Error messages for: missing config, unavailable models, Ollama unreachable, routing failures

**Dependencies:** All previous milestones.

**Exit criteria:**
- Complex multi-agent run completes successfully with 3+ models
- All error paths produce clear messages
- Simple goals still work as before (no regression)
