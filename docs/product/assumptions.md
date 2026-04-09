# Assumptions

[< Product Index](index.md) | [Spec Index](../spec/index.md)

## Codex CLI as a base

| Assumption | Risk |
|------------|------|
| Codex CLI's Rust codebase is stable enough to build on top of without forking | **Medium.** Upstream changes could break our extensions. Mitigation: wrap rather than modify core internals where possible. |
| The agent spawning system (AgentControl, AgentRegistry) can be extended to support orchestrator-driven dispatch | **Low.** The existing system already supports spawn with metadata, depth limits, and thread-level isolation. |
| Codex CLI's JSONL rollout system provides sufficient persistence for per-agent execution logs | **Low.** JSONL is append-only and already used for resume/fork. |
| The existing model client (Responses API) can coexist with other provider adapters | **Medium.** The client is deeply coupled to OpenAI's Responses API. The orchestrator must route through provider adapters, not directly through ModelClient. |
| Codex CLI's TUI can be extended to show multi-agent status | **Medium.** The TUI is complex (~800+ LoC in key modules). v1 may use a simpler status display and defer TUI integration. |
| The SDK (TypeScript/Python) can be used for non-interactive orchestrator execution | **Low.** The SDK already supports `codex exec` mode. |

## coding-agent-router (absorbed into orchestrator)

| Assumption | Risk |
|------------|------|
| The router's per-request routing logic can be imported as a Python library into the orchestrator | **Low.** The routing logic is pure functions + OllamaClient. No FastAPI dependency in the core logic. See [Routing Logic Reference](../spec/routing-logic-reference.md). |
| The compaction subsystem can be imported as a library into the orchestrator | **Low.** Self-contained pipeline with only OllamaClient and config as external deps. See [Compaction Reference](../spec/compaction-reference.md). |
| The compaction subsystem is valuable for long-running multi-agent sessions | **High value.** Multi-agent runs will generate large conversation histories; compaction is critical for context management. |
| The task metrics system (27 metrics) provides useful signal for routing decisions | **Low.** Already validated in production use. |
| Ollama per-endpoint serialization (file locks → asyncio.Semaphore) will behave equivalently | **Low.** Same semantics, simpler implementation since only the orchestrator talks to Ollama. |
| The compatibility proxy endpoints (Anthropic/OpenAI/Ollama API translation) are NOT needed by the orchestrator | **Low.** The orchestrator talks to providers directly through adapters. The proxy was only needed when tools didn't know about the router. |

## Claude Code participation

| Assumption | Risk |
|------------|------|
| Claude Code can be invoked as a subprocess via `claude` CLI | **Low.** This is the documented interface. |
| Claude Code's output can be captured and parsed programmatically | **Medium.** Output format may vary. Need structured output mode or JSON event parsing. |
| Claude Code can operate on a specific worktree directory | **Low.** The `--cwd` flag or equivalent exists. |
| Claude Code's approval model can be pre-configured for automated operation | **Medium.** May need `--dangerously-skip-permissions` or pre-approved tool patterns. |
| Claude Code's strengths (complex reasoning, large context, code review) complement Codex/local models | **Low.** This is well-established. |

## Local execution environment

| Assumption | Risk |
|------------|------|
| Developer machine has sufficient RAM/GPU for at least one local Ollama model | **Medium.** Not all developers have GPU. Graceful degradation to cloud-only is required. |
| Git is installed and the target repository is a Git repo | **Low.** Standard for software engineering workflows. |
| SQLite is available (ships with Python/Rust standard libraries) | **Low.** |
| Filesystem supports file locking (fcntl or equivalent) | **Low.** Standard on Linux/macOS. Windows needs alternative. |
| User has network access for cloud provider APIs (except offline mode) | **Low.** |

## Repository structure

| Assumption | Risk |
|------------|------|
| Target repositories use Git | **Low.** |
| Git worktrees are supported (Git 2.5+) | **Low.** |
| Repositories are not so large that worktree creation is prohibitively slow | **Medium.** Very large monorepos may need shallow worktrees. |
| A single repository is the unit of work for v1 | **Low.** Cross-repo is explicitly a non-goal. |

## Provider access patterns

| Assumption | Risk |
|------------|------|
| Users have API keys or subscription access for at least one cloud provider | **Low.** Required for non-offline operation. |
| Provider API rate limits are manageable for the level of concurrency in v1 | **Medium.** Multiple parallel agents hitting one provider could trigger rate limits. Router should handle 429s. |
| Provider pricing models are stable enough to encode as capability metadata | **Medium.** Pricing changes frequently. Must be configurable, not hardcoded. |

## Subscription vs API considerations

| Assumption | Risk |
|------------|------|
| Subscription-backed tools (e.g., ChatGPT Plus with Codex) have different cost profiles than API-billed calls | **Low.** This is factual. |
| Subscription tools may have usage limits (requests/day) that differ from API rate limits | **Medium.** These limits are often undocumented. Must track empirically. |
| The routing layer must treat subscription and API as different cost categories | **Low.** Already implicit in router's design. |
| Some users will want to maximize subscription usage before falling back to API billing | **High relevance.** This is a core routing policy decision. |
