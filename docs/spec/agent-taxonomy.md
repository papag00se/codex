# Agent Taxonomy

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Overview

Agents are stateless specialist workers. They receive a bounded task, execute it, and return a result. They do not manage their own state, schedule other agents, or decide when to stop — the [orchestrator](logical-components.md) controls all of that.

## Agent Summary Table

| Agent | MVP | Mission | Preferred Backend |
|-------|-----|---------|-------------------|
| Supervisor | Yes (is the orchestrator) | Drive run to completion | N/A (deterministic) |
| Planner | Yes | Decompose goal → task graph | Frontier model (large context) |
| Coder | Yes | Implement code changes | Varies by complexity |
| Reviewer | Yes | Review diffs for quality/security | Frontier model (reasoning) |
| Test Interpreter | Yes | Run tests and interpret results | Medium model + local exec |
| Docs Writer | No (v2) | Generate/update documentation | Medium model |
| Refactor Agent | No (v2) | Rename, extract, restructure code | Strong code model |
| Dependency Analyst | No (v2) | Analyze/update dependencies | Medium model + tool access |
| Integration Agent | No (v2) | Verify cross-service integration | Frontier model (large context) |
| Release/Readiness Agent | No (v2) | Check release readiness | Medium model |

---

## Supervisor (Orchestrator)

**Mission:** Drive a run from goal to verified completion by coordinating specialist agents.

**Allowed tools:** State store, event bus, worktree manager, routing engine, approval gate

**Forbidden actions:** Direct code modification, direct shell execution, provider API calls

**Autonomy limits:** Bounded by max iterations, timeout, and budget. Cannot override user denial.

**Expected outputs:** Run state transitions, events, final summary

**Escalation triggers:**
- All retries exhausted for a task
- Budget limit reached
- Unresolvable merge conflict
- Multiple agents reporting the same failure

**Preferred backend:** N/A — the supervisor is deterministic code, not an LLM.

---

## Planner

**Mission:** Analyze a goal and produce a structured task graph with dependencies.

**Allowed tools:** File read (for repository context), Git log (for recent history)

**Forbidden actions:** File write, shell execution, network calls

**Autonomy limits:** Single-shot (one LLM call). Must produce a complete task graph. Cannot spawn sub-planners.

**Expected outputs:** JSON task graph:
```json
{
  "tasks": [
    {
      "id": "task_1",
      "type": "code",
      "description": "Create Redis client wrapper in src/clients/redis.py",
      "dependencies": [],
      "estimated_complexity": "medium",
      "verification": "pytest tests/test_redis.py",
      "approval_hints": []
    },
    {
      "id": "task_2",
      "type": "code",
      "description": "Add rate limiting middleware using the Redis client",
      "dependencies": ["task_1"],
      "estimated_complexity": "high",
      "verification": "pytest tests/test_rate_limiter.py",
      "approval_hints": ["modifies_middleware"]
    }
  ]
}
```

**Escalation triggers:**
- Goal is ambiguous → ask user for clarification
- Goal requires more tasks than max_tasks → inform user, suggest splitting

**Preferred backend:** Frontier model with large context window (needs to see repo structure). Claude Code or OpenAI GPT-5.x. Not suitable for local models — planning quality is critical.

---

## Coder

**Mission:** Implement a specific, bounded code change as described by the task.

**Allowed tools:** File read/write/create, shell execution (sandboxed), git operations (add, commit within worktree), package manager (with approval)

**Forbidden actions:**
- Git push, merge, rebase (orchestrator handles merging)
- Modifying files outside the worktree
- Network calls beyond what's needed for the task
- Installing system-level packages
- Accessing secrets/credentials

**Autonomy limits:**
- Max turns: 10 (configurable)
- Max time: 15 minutes (configurable)
- Must stay within the task description scope
- Cannot create new tasks or request additional agents

**Expected outputs:** WorkerResult containing:
- Modified/created files (captured as diff)
- Shell command outputs
- Commit within worktree branch
- Self-assessment of completion

**Escalation triggers:**
- Cannot find referenced files → report failure
- Tests fail after max internal retries → report failure with details
- Task requires changes outside the worktree scope → report scope violation

**Preferred backend:**
- Simple/template tasks: local Ollama coder (fast, free)
- Medium complexity: OpenAI API (good code generation)
- Complex/multi-file: Claude Code (strong reasoning, tool use)
- Refactoring: Claude Code or OpenAI (strong diff capability)

---

## Reviewer

**Mission:** Review code diffs for correctness, security, style, and completeness.

**Allowed tools:** File read (worktree + original), diff generation, git log

**Forbidden actions:** File modification, shell execution, any write operation

**Autonomy limits:**
- Read-only access
- Single-shot review (one LLM call per diff)
- Must produce structured review output

**Expected outputs:** Review result:
```json
{
  "verdict": "approve" | "request_changes" | "comment",
  "issues": [
    {
      "severity": "critical" | "warning" | "suggestion",
      "file": "src/middleware/rate_limiter.py",
      "line": 42,
      "description": "Rate limit key doesn't include the endpoint path, allowing bypass",
      "suggestion": "Include request.path in the rate limit key"
    }
  ],
  "summary": "Implementation is solid but has one critical security gap."
}
```

**Escalation triggers:**
- Critical issues found → orchestrator may re-route task back to coder
- Review confidence low → flag for human review

**Preferred backend:** Frontier model with strong reasoning (Claude Code or GPT-5.x). Not suitable for local models — review quality is safety-critical.

---

## Test Interpreter

**Mission:** Run test commands, parse output, and provide structured interpretation of results.

**Allowed tools:** Shell execution (read-only + test commands), file read

**Forbidden actions:** File modification, git operations

**Autonomy limits:**
- Can only run configured verification commands
- Max execution time per test suite: 10 minutes (configurable)
- Cannot modify test files or source code

**Expected outputs:** Verification result:
```json
{
  "status": "pass" | "fail" | "error",
  "tests_run": 47,
  "tests_passed": 45,
  "tests_failed": 2,
  "tests_errored": 0,
  "failures": [
    {
      "test": "test_rate_limiter.py::test_concurrent_requests",
      "error": "AssertionError: expected 429 but got 200",
      "interpretation": "Rate limiter not applied to concurrent requests from same IP"
    }
  ],
  "recommendation": "retry" | "fail" | "pass_with_warnings"
}
```

**Escalation triggers:**
- Test command not found → report configuration error
- All tests fail → likely a systemic issue, escalate
- Test timeout → report timeout, suggest increasing limit

**Preferred backend:**
- Test execution: local subprocess (deterministic, no LLM needed)
- Test interpretation: medium model (Ollama is fine for parsing test output)
- Complex failure analysis: frontier model (when failures are subtle)

---

## Future Agents (v2+)

### Docs Writer
- **Mission:** Generate or update documentation based on code changes
- **Backend:** Medium model (documentation is well-structured, local models can handle it)

### Refactor Agent
- **Mission:** Perform structural code changes (rename, extract, move)
- **Backend:** Strong code model (Claude Code or OpenAI)

### Dependency Analyst
- **Mission:** Analyze dependency graphs, suggest updates, check for vulnerabilities
- **Backend:** Medium model + tool access (npm audit, pip-audit, etc.)

### Integration Agent
- **Mission:** Verify that changes work across service boundaries
- **Backend:** Frontier model (needs large context for multi-service analysis)

### Release/Readiness Agent
- **Mission:** Check changelog, version bumps, migration scripts, CI status
- **Backend:** Medium model + tool access
