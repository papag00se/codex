# Problem Statement

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## What the product is

A CLI-centered multi-agent coding platform that accepts high-level engineering goals, decomposes them into bounded tasks, routes each task to the most appropriate execution backend (cloud API, subscription tool, or local model), executes code changes in isolated repository contexts, verifies results, and loops under supervisor control until completion conditions are met.

The product name used throughout this document is **Codex Multi-Agent** (working title). It is built from three starting points:

- **Codex CLI** (open-source) as the initial CLI shell and single-agent execution engine
- **coding-agent-router** (existing custom project) as the per-request routing service
- **Claude Code** as a first-class agent backend alongside OpenAI and local models

## What problem it solves

Modern software engineering tasks — especially in multi-service systems — frequently exceed what a single-agent CLI session can handle well:

1. **Task scope mismatch.** A single agent given a large goal either tries to do everything in one long session (context exhaustion, drift) or requires the human to manually decompose and sequence work.

2. **Backend mismatch.** Different subtasks have different optimal backends: a quick file rename is fine for a local model; a complex architectural refactor benefits from a frontier model; a security-sensitive review may require a specific provider. Single-agent CLIs lock you into one backend per session.

3. **No structured verification.** Single-agent CLIs execute and hope. There is no systematic verification loop that runs tests, checks diffs, and gates risky operations before they land.

4. **No durable orchestration state.** If a session crashes mid-way through a multi-file change, the human must reconstruct what was done, what remains, and what failed. There is no resumable task graph.

5. **No routing transparency.** When a system does use multiple models, routing decisions are opaque — the user cannot see why a particular backend was chosen or override it.

## Why a multi-agent routed coding CLI is useful

| Single-agent CLI | Multi-agent routed CLI |
|---|---|
| One model per session | Best model per task |
| Human decomposes work | Supervisor decomposes work |
| No parallel execution | Parallel agents in isolated worktrees |
| Session = conversation transcript | Session = durable task graph + events |
| Crash = start over | Crash = resume from last checkpoint |
| No cost control | Budget-aware routing with fallbacks |
| No verification loop | Automated test + review + approval gates |
| Opaque | Auditable routing decisions + event log |

## Specific gap between single-agent CLI and this target

The gap is the **control plane**. Codex CLI today is an excellent single-agent execution engine with:
- A mature TUI and CLI interface
- Agent spawning with depth/thread limits
- Tool routing (shell, file ops, MCP)
- JSONL rollout persistence
- Session resume/fork
- Hook system for interception

What it lacks:
- **Task decomposition and planning agent** — no structured planner that produces a reviewable task graph
- **Heterogeneous backend routing** — tied to one model provider per session (OpenAI Responses API)
- **Orchestrator state machine** — no durable workflow with explicit states (planned → assigned → running → verifying → approved → done)
- **Verification as a first-class loop** — no automated test-run-and-check cycle between task completion and acceptance
- **Policy engine** — no declarative rules for what requires approval, what can auto-retry, what must hard-fail
- **Budget/cost awareness** — no tracking of spend per run or routing based on remaining budget
- **Structured observability** — rollout is append-only JSONL, not queryable structured events

The [coding-agent-router](../spec/routing-architecture.md) fills part of this gap (per-request routing with metrics and fallbacks) but operates as a standalone HTTP service, not as an integrated orchestration layer.

**The product is the missing control plane that connects Codex CLI's execution capabilities, the router's backend selection intelligence, and Claude Code's capabilities into a single supervised, auditable, resumable workflow.**
