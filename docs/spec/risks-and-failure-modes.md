# Risks and Failure Modes

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Risk Matrix

| # | Risk | Impact | Likelihood | Detection | Mitigation |
|---|------|--------|------------|-----------|------------|
| R1 | Runaway supervisor loops | High | Low | Iteration counter, timeout | Hard iteration limit (50), total timeout (2h), budget cap |
| R2 | Bad routing decisions | Medium | Medium | Routing logs, verification | Mandatory verification, routing explanation logs, manual override option |
| R3 | Silent degraded quality | High | Medium | Hard to detect | Verification gate, LLM review for critical tasks, quality metrics over time |
| R4 | Stale provider capability metadata | Medium | Medium | Periodic health probes | Health checks on startup + periodic refresh (5 min), config-driven capabilities (user can update) |
| R5 | Subscription/API drift | Medium | High | Provider errors | Graceful degradation, fallback routing, rate limit detection |
| R6 | Model unavailability | Medium | Medium | Health probes, error responses | Multi-provider fallback chain, offline mode with local models |
| R7 | Provider-specific UX mismatch | Low | High | User reports | Provider adapters normalize output format; accept minor differences |
| R8 | Broken repo state | High | Low | Git status checks | Worktree isolation, merge conflict detection, rollback capability |
| R9 | Duplicate event handling | Medium | Low | Sequence numbers | Idempotent event processing, (run_id, sequence) dedup |
| R10 | False verification passes | High | Low | Human review of results | Multiple verification types (tests + lint + review), human review for critical changes |
| R11 | Excessive cost | Medium | Medium | Budget tracking | Per-run budget limits, prefer subscription, cost logging, alerts at 80% |
| R12 | Local model hallucination | High | High | Verification | Mandatory verification for all local model outputs, no auto-acceptance |
| R13 | Operator overload | Medium | Medium | Approval request count | Configurable approval granularity, batch approvals, auto-approve for low-risk patterns |

## Detailed failure scenarios

### R1: Runaway supervisor loops

**Scenario:** Planner produces circular dependencies. Or: verification always fails, retries always fail, creating an infinite retry loop.

**Detection:**
- Iteration counter incremented on every supervisor loop cycle
- Timeout wall-clock timer
- Stuck detection: if no task transitions in N minutes, consider run stuck

**Mitigation:**
- Hard iteration limit: 50 (configurable, max 200)
- Total timeout: 2h (configurable)
- Per-task max retries: 3
- Per-task timeout: 15m
- Circular dependency detection in planner output validation

**Recovery:** Run transitions to `failed` state with a clear error message identifying what was looping and why.

### R2: Bad routing decisions

**Scenario:** Router sends a complex task to a local model that produces garbage, wasting a retry cycle.

**Detection:**
- Verification catches bad output
- Routing logs show the decision was low-confidence

**Mitigation:**
- [Verification](verification-safety.md) is mandatory — bad output is caught
- Retry escalation: local → API → frontier
- Quality-critical tasks (planning, review) always route to frontier models
- Routing confidence logged for post-hoc analysis

### R3: Silent degraded quality

**Scenario:** An agent produces code that passes tests but has subtle bugs, security issues, or poor design.

**Detection:** Hard to detect automatically. This is the hardest risk to mitigate.

**Mitigation:**
- LLM-based code review for non-trivial changes (reviewer agent)
- Human review of result branch before merging to main
- The system explicitly does NOT auto-merge to main — the [result branch](repository-isolation.md) is always a proposal
- Run summary includes enough context for informed human review

### R4: Stale provider capability metadata

**Scenario:** A provider's context window changes, or a model is deprecated, but the capability registry still has old data.

**Detection:** Provider errors (413 for too-large context, 404 for deprecated model)

**Mitigation:**
- Capability config is user-editable TOML — easy to update
- Health probes at startup detect model availability
- Provider errors trigger capability refresh
- New provider versions are announced — user updates config

### R5: Subscription/API drift

**Scenario:** Subscription usage cap changes. API pricing changes. Rate limits tighten.

**Detection:** 429 errors, unexpected billing, subscription cap errors

**Mitigation:**
- Rate limit handling with retry-after
- Subscription usage tracking (empirical, not hardcoded)
- Budget alerts at 80%
- Fallback to cheaper providers when limits hit
- Config-driven cost metadata (user can update pricing)

### R6: Model unavailability

**Scenario:** OpenAI API is down. Ollama crashes. Claude Code fails to start.

**Detection:** Health probes, connection errors, subprocess exit codes

**Mitigation:**
- Multi-provider fallback: if preferred provider is down, route to next-best
- Offline mode: if all cloud providers down, use local models (with quality warnings)
- If all providers down: pause run, inform user
- Provider health status cached and refreshed periodically

### R8: Broken repo state

**Scenario:** An agent leaves the worktree in a dirty state. A merge corrupts the result branch. An orphaned worktree has half-committed changes.

**Detection:** `git status` checks in worktree manager

**Mitigation:**
- Worktree isolation prevents main branch corruption
- Merge into result branch uses `--no-ff` for clean revert
- Orphaned worktrees detected and cleaned on startup
- Result branch is never force-pushed — always fast-forward or merge
- If corruption detected: fail the task, clean the worktree, recreate from scratch

### R10: False verification passes

**Scenario:** Tests pass but don't cover the changed code. Or: the verification command is misconfigured and always returns 0.

**Detection:** Difficult without coverage analysis.

**Mitigation:**
- Verification is necessary but not sufficient — human review of the result branch is the final gate
- Run summary includes files changed vs. tests run — reviewer can spot coverage gaps
- Optional: add coverage check to verification command (user-configured)
- The system does NOT claim verified code is bug-free — it claims verified code passes the configured checks

### R12: Local model hallucination

**Scenario:** Ollama model generates plausible-looking but incorrect code. Invents APIs that don't exist. Produces code for the wrong language.

**Detection:** Verification (tests fail, lint fails, type check fails)

**Mitigation:**
- **All** local model outputs go through verification — no exceptions
- If verification is not configured: local model outputs are marked `unverified-complete` with a warning
- For planning tasks: local models are never used (hardcoded rule)
- For review tasks: local models are never used (hardcoded rule)
- Router scoring penalizes local models for quality-critical tasks

### R13: Operator overload

**Scenario:** Every task triggers multiple approval requests. The user is overwhelmed with prompts and stops paying attention (approval fatigue).

**Detection:** High approval request count in run metrics

**Mitigation:**
- Sensible default policy: only truly risky actions require approval
- Batch approval: "approve all remaining shell commands in this run"
- Pattern approval: "approve all `npm install` commands" (within this run)
- The `--approve-all` flag exists for users who consciously accept the risk
- Approval count is visible in run summary for policy tuning
