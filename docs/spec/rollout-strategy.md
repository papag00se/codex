# Rollout Strategy

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Principle: Incremental trust building

The system should earn trust incrementally, not demand it upfront. Each rollout phase increases automation while maintaining human oversight.

## Phase R0: Single-agent wrapper (Week 1-2)

**What works:** `codex run` spawns a single agent (no decomposition, no routing). Equivalent to `codex exec` but with durable state and event logging.

**Why:** Validates the CLI ↔ orchestrator IPC, state persistence, event logging, and worktree isolation without any multi-agent complexity.

**User experience:**
```bash
codex run "Fix the bug in test_auth.py"
# → Single task, single agent, no routing decision needed
# → Durable state: can inspect, resume
# → Worktree: changes are isolated
```

## Phase R1: Plan + execute (Week 3-4)

**What works:** Planner decomposes goal into tasks. Tasks execute sequentially on a single backend (user's default).

**Why:** Validates [planning](agent-taxonomy.md) quality and [task state machine](state-model.md) without routing complexity. User sees the plan and can approve before execution.

**User experience:**
```bash
codex run --plan-only "Add pagination to the API"
# → Review the plan
codex run "Add pagination to the API"
# → Plan + sequential execution + verification
```

## Phase R2: Multi-backend routing (Week 4-5)

**What works:** Routing engine selects backends per task. Multiple backends used in a single run.

**Why:** This is the core value proposition. Validates [routing](routing-architecture.md) quality, [provider adapters](provider-abstraction.md), and heterogeneous execution.

**User experience:**
```bash
codex run "Add pagination with tests"
# → Planning task: Claude Code (frontier)
# → Code task 1: OpenAI API (medium complexity)
# → Test task: Ollama (simple interpretation)
# → Routing visible via codex run inspect --routing
```

## Phase R3: Parallel execution (Week 5)

**What works:** Independent tasks execute in parallel worktrees.

**Why:** Performance improvement. Validates [worktree isolation](repository-isolation.md) and merge logic.

**User experience:**
```bash
codex run --parallel 3 "Add pagination, update docs, fix lint errors"
# → 3 independent tasks execute simultaneously
```

## Phase R4: Full verification and approval (Week 5-6)

**What works:** Verification loop, approval gates, retry with escalation.

**Why:** Safety and quality. Validates that the system catches errors and respects [policy](verification-safety.md).

**User experience:**
```bash
codex run "Upgrade database driver with migration"
# → Code changes verified with tests
# → Migration file requires approval
# → Test failure triggers retry with stronger model
```

## Adoption guardrails

1. **Start with plan-only mode.** Users should run `--plan-only` first to see what the system would do before trusting it to execute.

2. **Default to risky-only approval policy.** Don't overwhelm users with approval requests, but don't auto-approve dangerous actions either.

3. **Always produce a result branch, never auto-merge.** The human reviews the result branch and merges manually (or via PR).

4. **Cost visibility from day one.** Every run shows cost in the summary. No surprise bills.

5. **Start with 2 parallel agents max.** Increase as confidence grows. Default to sequential for the first few runs.

6. **Provide escape hatches.** `codex run cancel` works instantly. `--single-agent` bypasses all multi-agent logic. `--backend <x>` bypasses routing.
