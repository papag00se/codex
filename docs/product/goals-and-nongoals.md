# Goals and Non-Goals

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## Product Goals

| ID | Goal |
|----|------|
| PG1 | Single CLI entry point for multi-agent software engineering workflows |
| PG2 | Automatic task decomposition from high-level engineering goals |
| PG3 | Transparent, auditable routing of each task to the best available backend |
| PG4 | Human-readable plan review before execution begins |
| PG5 | Safe parallel execution with repository isolation |
| PG6 | Verification loop with automated testing and diff review |
| PG7 | Approval gates for risky operations (infra changes, deletions, merges) |
| PG8 | Resume/retry from any failure point without re-executing completed work |
| PG9 | Cost-aware execution with budget limits and routing preferences |
| PG10 | Support for at least 3 backend categories: cloud API, subscription tool, local model |

## Technical Goals

| ID | Goal |
|----|------|
| TG1 | Event-driven orchestration with durable state ([SQLite](../spec/state-model.md) + [append-only event log](../spec/event-model.md)) |
| TG2 | Bounded supervisor loops with explicit max-iteration and timeout limits |
| TG3 | Typed schemas for all inter-component contracts (tasks, events, routing decisions) |
| TG4 | [Provider adapter interface](../spec/provider-abstraction.md) that new backends can implement without touching core |
| TG5 | Deterministic state machine for task lifecycle (no hidden transitions) |
| TG6 | Replayable event log for debugging and auditing |
| TG7 | Repository isolation via Git worktrees for parallel agent work |
| TG8 | Clean separation: CLI surface / orchestrator / agents / router / providers |
| TG9 | Integration with existing coding-agent-router as a first-class routing service |
| TG10 | Integration with Codex CLI's existing agent execution engine for worker dispatch |

## Operational Goals

| ID | Goal |
|----|------|
| OG1 | Runs entirely on a single developer machine — no cloud infrastructure required |
| OG2 | Works offline with local Ollama models (degraded but functional) |
| OG3 | Starts in under 3 seconds for interactive use |
| OG4 | Produces human-readable run summaries after completion |
| OG5 | Logs and traces queryable without external tools (CLI subcommands) |
| OG6 | Configuration via TOML files with sensible defaults |
| OG7 | Graceful degradation when a provider is unavailable |

## Explicit Non-Goals for v1

| ID | Non-Goal | Rationale |
|----|----------|-----------|
| NG1 | Distributed multi-machine execution | Single-machine-first; distributed adds massive complexity |
| NG2 | Web dashboard UI | CLI-first; a dashboard can be layered later |
| NG3 | Cross-repository workflows | v1 targets one repo at a time |
| NG4 | Automatic deployment/release | v1 stops at code changes + verification |
| NG5 | Browser-based agents | Tool scope limited to shell, file, git, MCP |
| NG6 | Custom agent authoring by end users | v1 ships fixed agent taxonomy; extensibility comes later |
| NG7 | Real-time collaboration between humans | Single-operator system |
| NG8 | Fine-tuning or training local models | Use models as-is from providers |
| NG9 | Full Kubernetes/cloud-native packaging | Local process execution only |
| NG10 | Automatic conflict resolution across parallel agents | v1 flags conflicts for human review |
