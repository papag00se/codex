# Routing Architecture

[< Spec Index](index.md) | [Product Index](../product/index.md)

> **Known gaps:** Routing currently has no feedback loop (same decisions regardless of outcomes), no codebase awareness (classifier doesn't know project complexity), and classifier adds 3-4s latency per request. See [Gaps](gaps.md) G1, G2, G8.

## Why routing exists

Different tasks have different optimal backends. A system that routes everything to one model is either overpaying for simple tasks or underperforming on complex ones.

| Task type | Optimal backend | Why |
|-----------|----------------|-----|
| Simple rename/template | Local Ollama | Free, fast, sufficient quality |
| Medium code generation | OpenAI API | Strong generation, reasonable cost |
| Complex multi-file change | Claude Code | Strong reasoning, tool use, large context |
| Security review | Frontier model (any) | Quality is safety-critical |
| Test execution | Local subprocess | Deterministic, no LLM needed |
| Test interpretation | Medium model | Parsing structured output is not hard |
| Planning | Frontier model | Decomposition quality drives everything downstream |

Without routing, the user must either manually choose backends or accept a one-size-fits-all approach that wastes money on simple tasks and produces poor results on complex ones.

## Routing granularity options

| Granularity | Pros | Cons |
|-------------|------|------|
| Per run | Simple, one routing decision | Wrong backend for most tasks |
| Per phase | Moderate granularity | Phases contain heterogeneous tasks |
| **Per task** | Right level for backend selection | One decision per task |
| Per retry | Can switch on failure | Extra complexity |
| Per model request | Maximum flexibility | Absorbed from coding-agent-router |

## Recommended routing granularity for v1

**Unified routing within the orchestrator** — all routing logic from coding-agent-router is absorbed as a library (see [Routing Logic Reference](routing-logic-reference.md) for every preserved heuristic).

### Per-task routing (primary)
The orchestrator selects a **backend category** for each task based on task type, complexity, and policy. This happens before the agent is spawned.

Categories:
- `codex-cli` → Codex CLI execution engine (OpenAI models)
- `claude-code` → Claude Code execution engine (Anthropic models)
- `ollama-coder` → Ollama coder model (structured tool calls)
- `ollama-reasoner` → Ollama reasoner model (plain text)
- `openai-api` → Direct OpenAI API (non-agentic completions)
- `anthropic-api` → Direct Anthropic API (non-agentic completions)
- `local-exec` → Local subprocess (deterministic, no LLM)

### Per-request routing within Ollama tasks
When a task is routed to an Ollama backend, the absorbed routing logic (from coding-agent-router) decides which specific local model handles each request. This preserves the existing per-request intelligence:
- Context window eligibility filtering (remove backends that can't fit the request)
- Task metrics extraction (27 metrics — see [Routing Logic Reference](routing-logic-reference.md))
- Router model inference (local LLM picks route when multiple are eligible)
- Deterministic fallback ordering (coder → reasoner → codex_cli)
- Tool-call recovery for devstral-style models

### Per-retry escalation
When a task fails and is retried, the orchestrator **may** escalate to a more capable backend. For example: first attempt on Ollama, retry on OpenAI, second retry on Claude Code. This is configurable per policy.

**Justification:** Absorbing the routing logic eliminates the sidecar process, HTTP overhead, and operational complexity while preserving every heuristic. The orchestrator has full visibility into both task-level and request-level routing decisions.

## How subscription-based and API-based providers differ

| Dimension | Subscription (ChatGPT Plus, Claude Pro) | API (OpenAI API, Anthropic API) |
|-----------|----------------------------------------|--------------------------------|
| Cost model | Fixed monthly fee, usage caps | Per-token billing |
| Rate limiting | Requests/day or requests/hour | Tokens/minute, requests/minute |
| Access method | Subprocess (codex exec, claude) | HTTP API |
| Tool execution | Managed by the tool's runtime | Managed by our provider adapter |
| Approval model | Tool's own approval model | N/A (raw API) |
| Context window | Tool-dependent | Explicit per model |

**Routing implication:** Prefer subscription tools until their usage cap is approached. Track usage empirically (number of invocations + estimated tokens) since subscription caps are often undocumented.

```
if subscription_available and subscription_budget_remaining > estimated_task_cost:
    route to subscription tool
elif api_budget_remaining > estimated_task_cost:
    route to API
elif local_model_capable:
    route to local
else:
    pause and inform user
```

## How local Ollama models differ

| Dimension | Cloud API / Subscription | Local Ollama |
|-----------|--------------------------|-------------|
| Cost | Per-token or subscription | Free (electricity only) |
| Latency | Network + inference | Inference only |
| Privacy | Data leaves machine | Data stays local |
| Quality | Frontier models | Smaller models, lower quality |
| Context window | Large (128K-1M) | Typically 8K-32K |
| Tool support | Full (function calling) | Variable (model-dependent) |
| Availability | Requires network | Always available |

**Routing implication:** Local models are preferred for:
- Cost-sensitive tasks
- Privacy-sensitive tasks
- Simple/template tasks where quality difference is minimal
- Offline operation

Local models are NOT preferred for:
- Planning (quality-critical)
- Security review (safety-critical)
- Complex multi-file changes
- Tasks requiring large context

**Mandatory verification for local model outputs:** Because local model quality is lower, all outputs from local models must pass verification before acceptance. This is enforced by the orchestrator, not optional.

## How Claude Code should integrate

Claude Code integrates as a **provider adapter** that wraps the `claude` subprocess:

```
ClaudeCodeAdapter:
    execute_task(task, context):
        1. Create prompt from task description + context
        2. Invoke: claude --cwd <worktree> --json --dangerously-skip-permissions <prompt>
           (or equivalent automated mode flags)
        3. Parse JSON output events
        4. Extract: files changed, commands run, final result
        5. Return WorkerResult
```

**Claude Code's own tool execution:** Claude Code has its own shell execution, file editing, and approval model. When the orchestrator delegates a task to Claude Code, it delegates the full tool-use responsibility. The orchestrator's role is:
1. Set up the worktree
2. Configure Claude Code's working directory
3. Provide the task prompt
4. Collect the result
5. Run verification independently

The orchestrator does NOT try to intercept Claude Code's individual tool calls. Claude Code is treated as an opaque execution engine — input is a task, output is a result.

**When to use Claude Code vs Anthropic API:**
- Claude Code: for tasks that require tool use (file editing, shell commands, multi-step reasoning)
- Anthropic API: for single-completion tasks (review, classification, interpretation)

## How Codex-backed paths should integrate

Same pattern as Claude Code — `codex exec` is an opaque execution engine:

```
CodexCliAdapter:
    execute_task(task, context):
        1. Create prompt from task description + context
        2. Invoke: codex exec --cwd <worktree> --json <prompt>
        3. Parse JSONL output events
        4. Extract: files changed, commands run, final result
        5. Return WorkerResult
```

Codex CLI's existing model routing (via its own config) handles per-request model selection within the Codex execution.

## How provider/model capabilities are represented

```toml
# Provider capability registry (loaded from config)

[[providers]]
id = "claude-code"
type = "subscription"
cost_category = "subscription"
access_method = "subprocess"
command = "claude"

[providers.capabilities]
context_window = 200000
code_generation = 0.95    # 0.0-1.0 quality score
code_review = 0.95
planning = 0.90
refactoring = 0.90
test_interpretation = 0.85
documentation = 0.85
tool_use = true
streaming = true

[providers.constraints]
max_concurrent = 2         # subscription may limit concurrency
estimated_requests_per_hour = 30

[[providers]]
id = "ollama-qwen3-coder"
type = "local"
cost_category = "free"
access_method = "http"
base_url = "http://127.0.0.1:11434"
model = "qwen3-coder:30b"

[providers.capabilities]
context_window = 16384
code_generation = 0.65
code_review = 0.40
planning = 0.30
refactoring = 0.50
test_interpretation = 0.60
documentation = 0.55
tool_use = true
streaming = true

[providers.constraints]
max_concurrent = 1         # Ollama serializes internally
```

## How routing decisions are computed

```python
def route_task(task, providers, policy, budget):
    # 1. Filter by capability
    eligible = [p for p in providers if p.healthy and task_fits(task, p)]
    
    # 2. Apply policy restrictions
    eligible = [p for p in eligible if policy.allows(p, task)]
    
    # 3. Apply budget constraints
    eligible = [p for p in eligible if budget.can_afford(p, task)]
    
    # 4. If manual override specified, use it
    if task.forced_backend:
        return RoutingDecision(backend=task.forced_backend, confidence=1.0)
    
    # 5. If only one eligible, use it
    if len(eligible) == 1:
        return RoutingDecision(backend=eligible[0], confidence=1.0)
    
    # 6. Score eligible providers
    scores = {}
    for p in eligible:
        scores[p] = compute_score(task, p, policy)
    
    # 7. Select highest score
    best = max(scores, key=scores.get)
    confidence = scores[best] / sum(scores.values())
    
    return RoutingDecision(
        backend=best,
        confidence=confidence,
        reason=explain_score(task, best, scores)
    )

def compute_score(task, provider, policy):
    score = 0.0
    # Quality match (task type → provider capability)
    score += provider.capabilities[task.type] * QUALITY_WEIGHT
    # Cost preference (subscription preferred if policy says so)
    if policy.prefer_subscription and provider.cost_category == "subscription":
        score += SUBSCRIPTION_BONUS
    if provider.cost_category == "free":
        score += FREE_BONUS
    # Latency preference
    if task.latency_sensitive and provider.type == "local":
        score += LOCAL_LATENCY_BONUS
    # Privacy preference
    if task.privacy_sensitive and provider.type == "local":
        score += PRIVACY_BONUS
    return score
```

## How routing decisions are logged and explained

Every routing decision produces an event:

```json
{
  "event": "route.selected",
  "run_id": "r_abc123",
  "task_id": "task_3",
  "decision": {
    "backend": "ollama-qwen3-coder",
    "confidence": 0.82,
    "reason": "Simple test generation task; local model sufficient (score: 0.82). Claude-code scored 0.78 (higher quality but subscription budget preferred for complex tasks). OpenAI scored 0.65 (API cost not justified for simple task).",
    "eligible_backends": ["ollama-qwen3-coder", "claude-code", "openai-gpt5"],
    "scores": {
      "ollama-qwen3-coder": 0.82,
      "claude-code": 0.78,
      "openai-gpt5": 0.65
    },
    "factors": {
      "task_type": "code_generation",
      "estimated_complexity": "low",
      "privacy_sensitive": false,
      "budget_remaining": 4.58
    }
  }
}
```

Queryable via:
```
$ codex run inspect r_abc123 --routing
$ codex run inspect r_abc123 --task task_3 --routing
```

## Fallback behavior

If the selected backend fails:
1. Mark the routing decision as `failed`
2. Remove the failed backend from eligible set
3. Re-route to next-best eligible backend
4. If no eligible backends remain, fail the task

Fallback order is deterministic: highest-scoring remaining backend.

## Failure behavior

| Failure | Response |
|---------|----------|
| Provider timeout | Retry once with same provider, then fallback |
| Provider 429 (rate limit) | Wait retry-after seconds, then retry |
| Provider 500 | Retry once, then fallback |
| Provider auth failure | Disable provider, fallback |
| Network error | If local available, use local; otherwise pause |
| All providers down | Pause run, inform user |

## Offline behavior

When no network is available:
1. Disable all cloud providers
2. Route all tasks to local Ollama (if available)
3. If no local model available, pause run with clear message
4. Quality warnings logged for tasks normally requiring frontier models

## Budget-aware behavior

The orchestrator tracks cumulative cost per run:
- API calls: estimated from token counts × provider pricing
- Subscription calls: tracked as "subscription units" (configurable cost weight)
- Local calls: $0

When budget reaches 80%, the orchestrator:
1. Logs a warning
2. Starts preferring cheaper backends
3. At 100%, pauses the run and informs the user

## Privacy-aware behavior

Tasks can be marked `privacy_sensitive` (by policy or planner):
- Privacy-sensitive tasks route only to local models or providers marked `privacy: high`
- If no privacy-compliant backend is available, the task fails with a clear error

## Quality-sensitive routing

Planning and review tasks always route to frontier models unless:
- Budget is exhausted
- User explicitly overrides with `--backend`
- No frontier model is available

This is a hard rule, not a heuristic. Local model planning/review quality is too low for production use.

## Confidence thresholds

Not used for gating in v1. Confidence is logged for observability but does not block routing decisions. Rationale: with only 3-5 providers, scoring is straightforward enough that the highest score is always selected. Confidence thresholds add complexity without clear benefit at this scale.

## Manual override options

- `--backend <backend>` on `codex run` — force all tasks to one backend
- `--task <id> --backend <backend>` on `codex run retry` — force a specific task to a backend
- `preferred_backend` field in task plan — planner can suggest a backend
- Policy file can require specific backends for specific task types
