# Key Design Questions

[< Product Index](index.md) | [Spec Index](../spec/index.md)

These questions must be answered by the [architecture and specification documents](../spec/index.md) that follow.

## Integration strategy

**Q1. Wrap Codex CLI or fork it?**
- Wrapping preserves upstream compatibility but limits deep integration.
- Forking allows full control but creates maintenance burden.
- Recommended investigation: Can the orchestrator use Codex CLI's SDK/exec mode as a worker dispatch mechanism without modifying Codex internals?

**Q2. How does coding-agent-router integrate?** *(RESOLVED)*
- **Decision:** Absorb routing logic, compaction, Ollama client, task metrics, and tool adapter as library modules into the orchestrator. No sidecar process.
- The proxy/compatibility endpoints are retired — not needed when the orchestrator talks to providers directly.
- See [Routing Logic Reference](../spec/routing-logic-reference.md) and [Compaction Reference](../spec/compaction-reference.md) for complete preservation of all migrated logic.
- See [Routing Architecture](../spec/routing-architecture.md) for how task-level and request-level routing compose.

**Q3. How should Claude Code integrate?**
- As a subprocess provider adapter (like the existing Codex CLI client)?
- As an API-level provider adapter (using Anthropic API directly)?
- Both? Claude Code for agentic tool-use tasks, Anthropic API for pure completion tasks?
- Key constraint: Claude Code has its own tool execution, approval, and sandbox model. The orchestrator must decide whether to delegate tool execution to Claude Code or keep it centralized.

## Routing architecture

**Q4. What is the routing granularity?**
- Per run: too coarse — different tasks need different backends.
- Per task: good default — each task gets a routing decision.
- Per model request: already handled by coding-agent-router.
- Per tool invocation: too fine-grained for v1.
- Recommended: two-tier routing. Orchestrator routes per-task (backend category). Router routes per-request (specific model within category).

**Q5. How should subscription-backed tools be abstracted versus API-backed tools?**
- Subscription tools (ChatGPT Codex, Claude Code subscription) have implicit cost but usage caps.
- API tools (OpenAI API, Anthropic API) have explicit per-token cost.
- The [provider capability registry](../spec/provider-abstraction.md) must represent both cost models.
- Routing policy must support "prefer subscription until quota exhausted, then fall back to API."

**Q6. What should local models be allowed to do?**
- Local models (Ollama) are free but lower capability.
- Should they have restricted tool access (e.g., no shell execution, read-only file access)?
- Should their outputs always pass through a verification step before acceptance?
- What about hallucination risk — should local model outputs require review by a stronger model?

## Orchestration

**Q7. How do we bound multi-agent recursion?**
- Codex CLI already has `agent_max_depth` (default: 1) and `agent_max_threads` (default: 6).
- The orchestrator must enforce its own bounds: max tasks per run, max retries per task, max total agent-hours per run.
- The supervisor loop must have a hard iteration limit.

**Q8. How should the supervisor loop work?**
- Plan → Execute → Verify → Approve → Complete, or more granular?
- Should the supervisor re-plan after partial completion (adaptive planning)?
- How does the supervisor detect that a run is stuck vs. making progress?

**Q9. How should agent-to-agent communication work?**
- Codex CLI has a mailbox pattern. Is that sufficient?
- Do agents need to share artifacts (diffs, test results) or just status?
- Should agents be able to request help from other agents, or only the supervisor?

## State and persistence

**Q10. What is the state store technology?**
- Codex CLI uses SQLite + JSONL. The router uses filesystem JSON.
- Should the orchestrator unify on SQLite for queryable state + JSONL for event log?
- How do we handle concurrent writes from parallel agents?

**Q11. How do we handle crash recovery?**
- The event log must be the source of truth for rebuild.
- Each task must be idempotent or have explicit "already completed" detection.
- What is the granularity of checkpointing — per task? Per turn within a task?

## Verification and safety

**Q12. What constitutes verification?**
- Test execution? Lint pass? Type check? Diff review by another model?
- Should verification be configurable per project (e.g., "run `make test` and check exit code")?
- How do we handle projects with no test suite?

**Q13. What requires human approval?**
- Configurable policy, but what are sensible defaults?
- Should the default be "approve everything" (maximum friction, maximum safety) or "approve only risky operations" (balanced)?
- How is "risky" defined — statically (file patterns, command patterns) or dynamically (model confidence)?

## Repository isolation

**Q14. Git worktrees or branches or patches?**
- Worktrees provide full filesystem isolation but consume disk space.
- Branches with stash/checkout are lighter but risk state pollution.
- Patches are lightweight but harder to test.
- Recommended investigation: worktrees for parallel execution, with automatic cleanup on task completion.

**Q15. How do we merge results from parallel agents?**
- If two agents edit different files, merge is trivial.
- If they edit the same file, we need conflict detection.
- v1 recommendation: flag conflicts for human review rather than attempting auto-resolution.

## Observability

**Q16. How do we keep routing decisions auditable?**
- Every routing decision must be persisted with: input features, eligible backends, selected backend, confidence, reason.
- These must be queryable via CLI (e.g., `codex run inspect <run-id> --routing`).

**Q17. What is the replay/debug strategy?**
- Can we replay a run from its event log against mock providers?
- Is this a v1 requirement or a future capability?
- Recommendation: event log format must support replay from day one, even if replay tooling is post-v1.
