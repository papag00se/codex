# Product Requirements Document (PRD)

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## Summary

Codex Multi-Agent is a CLI platform that turns high-level engineering goals into supervised, multi-agent workflows. It decomposes tasks, routes each to the best available backend (cloud API, subscription tool, or local model), executes in isolated Git worktrees, verifies results, and gates risky actions with human approval — all under a deterministic supervisor loop with durable state.

## Problem

Software engineers using single-agent coding CLIs face five recurring problems:

1. **Manual decomposition** — The human must break large goals into sequential agent sessions.
2. **Single-backend lock-in** — One model/provider per session, regardless of task fit.
3. **No structured verification** — Agent output is trusted without systematic testing.
4. **No crash recovery** — A failed session means starting over.
5. **No cost visibility** — No awareness of spend, no routing based on budget.

These problems compound when working on complex, multi-service systems where a single goal may touch 10+ files across multiple languages and require different model strengths for different subtasks.

## Users / Personas

### Primary: Solo developer on a complex codebase
- Works on a multi-service system (backend, frontend, infra, tests)
- Has API access to at least one cloud provider
- May have local GPU for Ollama
- Wants to issue a single goal and review the result, not manage individual agent sessions
- Cares about cost and wants subscription usage maximized before API billing

### Secondary: Tech lead doing code review
- Uses the system to review existing PRs with multi-agent analysis
- Routes review tasks to the most capable reviewer model
- Wants structured verification output, not chat

### Tertiary: Team using shared routing policies
- Organization defines which providers are approved, which operations require approval
- Policy files checked into the repository

## Target Use Cases

| ID | Use Case | Example |
|----|----------|---------|
| UC1 | Multi-file feature implementation | "Add pagination to the user list API, update the frontend, and add integration tests" |
| UC2 | Bug investigation and fix | "The CI is failing on test_auth_flow — investigate and fix" |
| UC3 | Refactoring with verification | "Extract the payment logic into a separate service module; all existing tests must pass" |
| UC4 | Code review | "Review the changes on branch feature/new-auth for security issues" |
| UC5 | Dependency update | "Update React from v18 to v19, fix all type errors and failing tests" |
| UC6 | Documentation generation | "Generate API docs for all public endpoints in the payments service" |

## User Stories

1. **As a developer**, I want to describe a goal in natural language and have the system produce a plan I can review before execution starts.
2. **As a developer**, I want each task to be routed to the best model for that task type, without me specifying which model to use.
3. **As a developer**, I want to see why a particular backend was chosen for a task, and override it if I disagree.
4. **As a developer**, I want risky operations (file deletions, infra changes, force pushes) to pause and ask for my approval.
5. **As a developer**, I want to resume a failed run from where it stopped, not re-execute completed tasks.
6. **As a developer**, I want to set a cost budget for a run and have routing respect it.
7. **As a developer**, I want to see a summary of what was done, what was verified, and what needs my review.
8. **As a developer**, I want parallel tasks to execute in separate worktrees so they don't interfere.
9. **As a developer**, I want to work offline with local models, accepting reduced capability.
10. **As a developer**, I want to inspect the full event log of a run for debugging.

## Functional Requirements

### FR1: CLI Interface
- The existing `codex` interactive TUI is the entry point — no new subcommand, no separate binary
- Multi-agent orchestration is a capability within the Codex agent, activated automatically when the goal requires decomposition
- The existing `multi_agent_v2` spawn system, agent roles, and TUI multi-agent view are the foundation
- Routing decisions exposed via MCP tools callable by the agent
- See [Integration Model](../spec/integration-model.md) for the full design

### FR2: Planning
- Decompose goal into a task graph with dependencies
- Present plan for human review before execution
- Support plan-only mode (no execution)
- Support plan modification before execution

### FR3: Task Routing
- Two-tier routing: per-task (orchestrator) + per-request (coding-agent-router)
- Route based on: task type, model capability, cost, latency, privacy, availability
- Log every routing decision with reason
- Support manual override per task

### FR4: Agent Execution
- At least 3 specialist agents for v1: coder, reviewer, test-interpreter
- Execute in isolated Git worktrees
- Bounded execution: max turns per task, max time per task
- Capture all artifacts (diffs, test output, logs)

### FR5: Verification
- Run project-defined verification commands after task completion
- Support: test execution, lint, type check, custom scripts
- Distinguish pass/fail/error
- Feed verification results back to agent for fix attempts (bounded retries)

### FR6: Approval Gates
- Configurable policy for what requires approval
- Default: approve shell commands with side effects, file deletions, git operations
- Interactive prompt in TUI mode; webhook/file-based in non-interactive mode
- Timeout with configurable default (approve/deny/pause)

### FR7: State and Persistence
- Durable state for: runs, tasks, routing decisions, approvals, artifacts
- SQLite for queryable state (see [State Model](../spec/state-model.md)); append-only event log for audit (see [Event Model](../spec/event-model.md))
- Resume from any failure point
- Retain state for configurable duration (default: 30 days)

### FR8: Observability
- CLI subcommands to inspect runs, tasks, events, routing
- Structured log output (JSON)
- Run summary generation on completion
- Routing decision summary per run

### FR9: Provider Adapters
- Adapter interface that new providers can implement (see [Provider Abstraction](../spec/provider-abstraction.md))
- v1 adapters: OpenAI API, Anthropic API, Claude Code (subprocess), Ollama, Codex CLI
- Capability registry describing each provider's strengths

### FR10: Configuration
- TOML config file with layered precedence (see [CLI Interaction Spec](cli-interaction-spec.md) for config example) (global, project, CLI flags)
- Provider credentials via environment variables or config
- Routing policy via config
- Approval policy via config
- Verification commands via config

## Non-Functional Requirements

| ID | Requirement | Target |
|----|-------------|--------|
| NFR1 | CLI startup time | < 3 seconds |
| NFR2 | State store write latency | < 50ms per event |
| NFR3 | Concurrent agent limit | Configurable, default 4 |
| NFR4 | Worktree creation time | < 5 seconds |
| NFR5 | Crash recovery time | < 10 seconds to resume |
| NFR6 | Event log format | Append-only, replayable, human-readable |
| NFR7 | Offline mode | Functional with local models only |
| NFR8 | Platform support | Linux and macOS; Windows best-effort |

## Constraints

1. Must run on a single developer machine — no cloud infrastructure required.
2. Must not require modifications to upstream Codex CLI source (wrap, don't fork, where possible).
3. Must not lock into a single provider — architecture must support N providers.
4. Must preserve coding-agent-router's per-request routing logic, compaction algorithm, and tool-call recovery (absorbed as library modules — see [Routing Logic Reference](../spec/routing-logic-reference.md) and [Compaction Reference](../spec/compaction-reference.md)).
5. Must use existing Codex CLI execution engine for Codex-backed tasks.

## Success Criteria

1. A developer can issue a multi-file feature goal and get a working implementation with passing tests, without manually managing individual agent sessions.
2. Routing decisions are visible and explainable for every task in a run.
3. A run that crashes mid-execution can be resumed and completes successfully.
4. The system uses at least 3 different backend categories in a single run when appropriate.
5. Risky operations are correctly gated by approval policy.

## MVP Scope

- CLI with `run`, `status`, `inspect`, `resume`, `cancel` subcommands
- Supervisor/orchestrator with bounded loop
- Planner agent (decomposes goal into tasks)
- Coder agent (implements code changes)
- Reviewer agent (reviews diffs)
- Test-interpreter agent (runs and interprets test results)
- Routing across: OpenAI API, Claude Code, Ollama
- SQLite state store + JSONL event log
- Git worktree isolation
- Configurable verification step
- Interactive approval gate
- Resume/retry support
- Routing explanation output

## Out of Scope for MVP

- Web dashboard
- Cross-repository workflows
- Custom user-defined agents
- Deployment/release automation
- Browser-based agents
- Distributed execution
- Real-time collaboration
- Model fine-tuning

## Major Risks

| Risk | Impact | Likelihood | Mitigation |
|------|--------|------------|------------|
| Codex CLI upstream changes break our integration | High | Medium | Wrap via SDK/exec, pin version, integration tests |
| Routing quality degrades silently | High | Medium | Routing quality metrics, A/B comparison logging |
| Multi-agent runs produce conflicting changes | Medium | High | Worktree isolation, conflict detection before merge |
| Local model hallucination accepted as correct | High | Medium | Mandatory verification for local model outputs |
| Cost overrun from parallel cloud API calls | Medium | Medium | Budget limits, prefer subscription, cost tracking |
| Supervisor loop gets stuck | Medium | Low | Hard iteration limit, timeout, stuck detection |

## Open Questions

1. Should the orchestrator be a Rust binary (matching Codex CLI) or a Python process (matching coding-agent-router)?
2. How should Claude Code's own tool execution model interact with the orchestrator's tool execution?
3. Should the planner be a dedicated LLM call or a deterministic rule engine for simple cases?
4. What is the right default approval policy — approve-everything vs. approve-risky-only?
5. Should the event log use the same JSONL format as Codex CLI's rollout, or a separate schema?
