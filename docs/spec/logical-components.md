# Logical Components

[< Spec Index](index.md) | [Product Index](../product/index.md)

For agent-specific details see [Agent Taxonomy](agent-taxonomy.md). For routing details see [Routing Architecture](routing-architecture.md).

## Component Summary Table

| Component | Layer | Implementation | Agentic/Deterministic |
|-----------|-------|----------------|----------------------|
| CLI Shell | CLI | Rust (extend codex-rs/cli) | Deterministic |
| Orchestrator/Supervisor | Core | Rust or Python | Deterministic (control), Agentic (planning) |
| Planner Agent | Agent | LLM-backed | Agentic |
| Specialist Workers | Agent | LLM-backed via providers | Agentic |
| Routing Engine | Routing | Deterministic rules + LLM assist | Hybrid |
| Provider Capability Registry | Routing | TOML config + runtime probing | Deterministic |
| Provider Adapters | Provider | Per-provider implementation | Deterministic |
| Policy Engine | Core | Rule matching | Deterministic |
| Approval Gate | Core | Pattern match + user interaction | Deterministic |
| Job/Task State Store | Infra | SQLite | Deterministic |
| Event Bus/Queue | Infra | In-process channel + JSONL | Deterministic |
| Repository/Worktree Manager | Infra | Git subprocess | Deterministic |
| Verifier/Reviewer | Core | Subprocess + optional LLM | Hybrid |
| Artifact Manager | Infra | Filesystem | Deterministic |
| Observability Layer | Infra | Structured logging + file traces | Deterministic |

---

## CLI Shell

**Purpose:** Parse user commands, render output, handle interactive approval, delegate to orchestrator.

**Responsibilities:**
- Parse `codex run` subcommands and options
- Validate inputs (goal not empty, config exists, etc.)
- Create initial Run record
- Start orchestrator process
- Render TUI status panel or JSON output
- Forward approval requests to user and return decisions
- Handle Ctrl+C for graceful shutdown

**Inputs:** User command-line arguments, stdin (for piped goals), user approval decisions

**Outputs:** TUI rendering, JSON events on stdout, exit code

**Failure modes:**
- Invalid arguments → exit with usage error
- Missing config → exit with config error
- Orchestrator crash → display error, suggest `codex run resume`

**Implementation notes:** Extend `codex-rs/cli/src/main.rs` with a new `Run` subcommand. Reuse existing TUI infrastructure where possible. The CLI is a thin layer — all logic lives in the orchestrator.

**Alternatives considered:** Separate binary vs subcommand. Subcommand is preferred for UX consistency.

---

## Orchestrator / Supervisor

**Purpose:** The supervisor loop that drives a run from goal to completion.

**Responsibilities:**
- Accept a goal and create a Run
- Invoke planner to decompose into task graph
- Manage task state machine (planned → routed → running → verifying → complete)
- Route tasks to backends
- Dispatch tasks to agents via provider adapters
- Collect results and run verification
- Enforce approval policy
- Handle retries with bounded attempts
- Persist state and events
- Detect completion, failure, or stuck conditions
- Support resume from persisted state

**Inputs:** Goal string, config, policy, existing state (for resume)

**Outputs:** Run state transitions, events, final run summary

**Failure modes:**
- Planner fails to decompose → fail run with error
- All agents fail on a task → fail task, escalate
- Budget exhausted → pause run, inform user
- Timeout reached → cancel remaining tasks, generate partial summary
- Crash → state is durable, resume possible

**Implementation notes:** The orchestrator is the most critical component. It must be single-threaded for state management (no concurrent mutations to the task graph) but can dispatch agent work to parallel workers. The supervisor loop is a simple `while` loop with explicit iteration counting.

**Alternatives considered:**
- Async state machine (e.g., using Rust's `async` with `tokio::select!`) — preferred for v1 due to natural fit with parallel agent dispatch
- Workflow engine (e.g., Temporal) — overkill for single-machine, adds operational complexity
- Event sourcing with separate projection — elegant but complex; v1 uses simpler event log + SQLite state

---

## Planner Agent

**Purpose:** Decompose a high-level goal into a graph of bounded tasks.

**Responsibilities:**
- Analyze the goal in the context of the repository
- Produce a task list with descriptions, types, and dependencies
- Estimate complexity for routing hints
- Identify verification requirements per task
- Flag tasks that will need approval

**Inputs:** Goal string, repository context (file tree, recent git history, language info)

**Outputs:** Task graph (list of tasks with dependency edges)

**Failure modes:**
- Goal too vague → produce a single "investigate and propose" task
- Goal too large → cap at max_tasks, suggest splitting
- LLM hallucination → tasks reference non-existent files (caught at execution time)

**Implementation notes:** The planner runs as an LLM call, not a persistent agent. It receives a structured prompt with repository context and outputs a JSON task graph. For simple goals (single-file changes), the planner may bypass LLM and produce a single task deterministically.

**Alternatives considered:**
- Persistent planner agent that iterates — more capable but harder to bound. v1 uses single-shot planning with optional re-planning on resume.
- Rule-based decomposition — insufficient for complex goals.

---

## Specialist Worker Agents

**Purpose:** Execute bounded tasks (code generation, review, test interpretation).

**Responsibilities:**
- Receive task description and repository context
- Execute within assigned worktree
- Produce artifacts (diffs, files, reviews, test output)
- Respect tool restrictions and autonomy limits
- Report completion or failure

**Inputs:** Task description, worktree path, allowed tools, turn limits

**Outputs:** WorkerResult with artifacts, status, metadata

**Failure modes:**
- Agent exceeds turn limit → forced stop, partial result reported
- Agent produces invalid output → verification catches it
- Provider fails → adapter reports failure, orchestrator retries

**Implementation notes:** Workers are invoked via provider adapters. For Codex CLI: `codex exec`. For Claude Code: `claude --cwd <worktree>`. For Ollama: direct API calls through coding-agent-router. Workers are stateless — all context comes from the task description and worktree.

---

## Routing Engine

**Purpose:** Select the best backend for each task.

**Responsibilities:**
- Evaluate task characteristics (type, complexity, context size, sensitivity)
- Match against provider capabilities
- Apply routing policy (cost preferences, provider restrictions)
- Apply budget constraints
- Produce routing decision with confidence and reason
- Fall back gracefully when preferred provider is unavailable

**Inputs:** Task metadata, provider capability registry, policy config, budget state

**Outputs:** RoutingDecision (backend, model, confidence, reason)

**Failure modes:**
- No eligible provider → fail with clear error listing why each was ineligible
- All providers degraded → route to least-degraded with warning
- Budget exhausted for API providers → route to subscription or local, or pause

**Implementation notes:** Two-tier design. Tier 1 (task-level) is in the orchestrator — rule-based with LLM assist for ambiguous cases. Tier 2 (request-level) is the existing coding-agent-router service. Tier 1 calls coding-agent-router's capabilities but makes its own per-task decision.

**Alternatives considered:**
- Single-tier routing (all in coding-agent-router) — doesn't work because the router operates per-request, not per-task. Tasks need routing before an agent is spawned.
- Pure rule-based routing — insufficient for nuanced capability matching.

---

## Provider Capability Registry

**Purpose:** Maintain metadata about what each provider/model can do.

**Responsibilities:**
- Load provider capabilities from config
- Probe provider health at startup
- Track runtime metrics (latency, error rate)
- Expose capabilities for routing queries

**Inputs:** Config file, runtime health probes

**Outputs:** ProviderCapability records

**Failure modes:**
- Config missing for a provider → provider disabled with warning
- Health probe fails → provider marked as degraded
- Capabilities stale → periodic refresh (configurable interval)

**Implementation notes:** Static capabilities from config (context window, cost tier, strengths) + dynamic health from runtime probes. Stored in memory, refreshed periodically.

---

## Provider Adapters

**Purpose:** Normalize heterogeneous backends into a common interface.

**Responsibilities:**
- Translate task + context into provider-specific invocation
- Execute against the provider (subprocess, HTTP, etc.)
- Parse provider-specific response into WorkerResult
- Handle provider-specific errors and retries
- Track cost/token usage

**Inputs:** Task, RepositoryContext, provider config

**Outputs:** WorkerResult

**Failure modes:**
- Provider timeout → return timeout error
- Provider rate limit → return rate limit error with retry-after
- Provider auth failure → return auth error, disable provider
- Subprocess crash → return crash error with stderr

**Implementation notes:** Each adapter is a struct implementing the ProviderAdapter trait/interface. v1 adapters:
- `CodexCliAdapter` — wraps `codex exec`
- `ClaudeCodeAdapter` — wraps `claude` subprocess
- `OllamaAdapter` — HTTP calls to Ollama API (using absorbed OllamaClient + tool-call recovery)
- `OpenAiApiAdapter` — HTTP calls to OpenAI API
- `AnthropicApiAdapter` — HTTP calls to Anthropic API

---

## Policy Engine

**Purpose:** Evaluate whether an action requires approval, is forbidden, or is auto-approved.

**Responsibilities:**
- Load policy from config
- Match actions against policy rules
- Return decision: approve, deny, require_human_approval

**Inputs:** Action descriptor (type, target, agent, backend)

**Outputs:** PolicyDecision

**Failure modes:**
- Policy config invalid → reject all actions (safe default)
- No matching rule → apply default policy (configurable)

**Implementation notes:** Pattern matching on action type, file paths, command strings. No LLM involvement — purely deterministic.

---

## Approval Gate

**Purpose:** Pause execution and request human decision for risky actions.

**Responsibilities:**
- Display approval request (TUI prompt or file-based)
- Wait for human response (with timeout)
- Apply default on timeout (configurable: approve/deny/pause)
- Record approval decision as event

**Inputs:** ApprovalRequest from policy engine

**Outputs:** Approval decision (granted/denied)

**Failure modes:**
- Timeout with no response → apply configured default
- User denies → task fails or is skipped (configurable)

---

## Job/Task State Store

**Purpose:** Durable storage for run and task state.

**Responsibilities:**
- Create/read/update run records
- Create/read/update task records
- Store routing decisions
- Store approval records
- Support queries by run ID, task ID, status
- Support resume (read last committed state)

**Inputs:** State mutations from orchestrator

**Outputs:** Current state records

**Failure modes:**
- SQLite lock contention → retry with backoff
- Disk full → fail with clear error
- Corruption → detected on read, require manual recovery

**Implementation notes:** SQLite with WAL mode for concurrent reads. Schema versioned with migrations. Located at `~/.codex/multi-agent/state.db`.

---

## Event Bus / Queue

**Purpose:** Transport events between components within a single process.

**Responsibilities:**
- Deliver events from producers to consumers
- Persist events to JSONL log
- Support replay from log

**Inputs:** Events from any component

**Outputs:** Events to subscribed consumers

**Failure modes:**
- Channel full → backpressure (bounded channel)
- Log write fails → surface error, do not drop event

**Implementation notes:** In-process async channels (tokio mpsc or Python asyncio.Queue). Not a network service. JSONL log is the durable backing — the in-memory channel is for real-time dispatch only.

**Alternatives considered:**
- Redis Streams — adds external dependency, overkill for single-process
- SQLite as queue — possible but slower for high-frequency events
- In-memory only — not durable enough

---

## Repository / Worktree Manager

**Purpose:** Create, manage, and clean up Git worktrees for agent isolation.

**Responsibilities:**
- Create worktree from current branch for a task
- Track worktree-to-task mapping
- Merge completed worktree changes back to main branch
- Detect conflicts between worktrees
- Clean up worktrees on task completion or failure
- Clean up orphaned worktrees on restart

**Inputs:** Repository path, branch, task ID

**Outputs:** Worktree path, merge result

**Failure modes:**
- Worktree creation fails (disk, permissions) → fail task
- Merge conflict → flag for human review
- Orphaned worktree → cleaned up on next startup

**Implementation notes:** Uses `git worktree add/remove`. Worktrees created in `<repo>/.codex-worktrees/<run-id>/<task-id>/`. Each worktree gets a unique branch name: `codex/<run-id>/<task-id>`.

---

## Verifier / Reviewer Layer

**Purpose:** Verify agent output before acceptance.

**Responsibilities:**
- Run verification commands (tests, lint, type check)
- Parse verification output (exit code, structured output if available)
- Optionally run LLM-based review (diff analysis)
- Return pass/fail with details

**Inputs:** Worktree path, verification config, agent output

**Outputs:** VerificationResult (pass/fail, details, suggestions)

**Failure modes:**
- Verification command not found → warn, skip verification
- Verification timeout → fail verification
- Verification command crashes → report as verification error (distinct from failure)

**Implementation notes:** Deterministic verification (test execution) is the primary mode. LLM-based review is optional and additive — it does not replace test execution.

---

## Artifact Manager

**Purpose:** Track and store artifacts produced by tasks.

**Responsibilities:**
- Record artifacts (diffs, files, test reports, reviews)
- Associate artifacts with tasks and runs
- Support artifact retrieval for inspection
- Clean up artifacts with run cleanup

**Inputs:** Artifact data from workers and verifier

**Outputs:** Artifact records, artifact content

**Implementation notes:** Artifacts stored in `~/.codex/multi-agent/runs/<run-id>/artifacts/`. Metadata in SQLite.

---

## Observability Layer

**Purpose:** Structured logging, metrics, and traces for debugging and auditing.

**Responsibilities:**
- Structured JSON logging with correlation IDs (run_id, task_id)
- Timing metrics for routing, execution, verification
- Trace spans for task lifecycle
- Run summary generation

**Inputs:** Events from all components

**Outputs:** Log files, metrics, trace files, summaries

**Implementation notes:** Uses tracing/structured logging. Logs to `~/.codex/multi-agent/runs/<run-id>/logs/`. No external observability infrastructure required for v1.
