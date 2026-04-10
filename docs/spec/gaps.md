# System Gaps and Future Work

[< Spec Index](index.md) | [Design Principles](design-principles.md)

> Living document. Updated as gaps are identified and closed.

## Critical gaps (directly impact usefulness)

### G1: Routing feedback loop
**Status:** DONE (feedback.rs, routing_history.jsonl, profile_context injected into classifier)
**Impact:** High — same routing mistakes repeated every session

The system makes the same routing decision for the same request type every time. No learning from outcomes. If spark fails 40% of the time on test-fix tasks in this codebase, the classifier doesn't know.

**What's needed:** After each task completes, record: model used, success/failure, tokens spent, latency. Build per-project routing profiles. Inject success rates into the classifier prompt: "For this project, cloud_fast has 90% success rate on test-fix tasks."

**Implementation:** Append routing outcomes to `.codex-multi/routing_history.jsonl`. On startup, compute success rates per model per task-type. Add to classifier context.

### G2: Codebase awareness in classifier
**Status:** DONE (codebase_context.rs, auto-detect, cached, injected into classifier)  
**Impact:** High — "fix the auth bug" classified the same for 500-line Flask app vs 500K-line multi-service platform

The classifier sees the request text but not the codebase. A project context section would let it make better decisions.

**What's needed:** `[context]` in `.codex-multi/config.toml` with project hints injected into the classifier prompt. Also auto-detect: language mix, file count, test framework.

**Implementation:** Add `[context]` config section. On first run, scan repo for languages/frameworks. Include in classifier prompt.

### G3: Cross-session memory
**Status:** DONE (session_memory.rs, .codex-multi/memory/, planner_context injection)  
**Impact:** High — system forgets everything between sessions

Compaction preserves state within a session but not across. Prior decisions, rejected approaches, architectural understanding — all lost.

**What's needed:** Persistent project knowledge that accumulates. The durable memory files (TASK_STATE.md, DECISIONS.md, FAILURES_TO_AVOID.md) are the right shape — they need to persist and grow across sessions.

**Implementation:** Save session handoffs to `.codex-multi/memory/`. On session start, load recent handoffs as context for the planner and classifier.

## Important gaps (impact quality and cost)

### G4: Prompt adaptation per model
**Status:** DONE (prompt_adapt.rs, per-tier scaffolding for task/planning/evaluation)  
**Impact:** Medium — local 9B models need more explicit prompts than frontier models

Same task description goes to every model. Weaker models need more scaffolding.

**What's needed:** Per-model prompt templates. More chain-of-thought for local, more concise for cloud.

### G5: Streaming from local models
**Status:** DEFERRED (requires rework of response translation pipeline)  
**Impact:** Medium — UI feels frozen during local responses

Ollama calls use `stream: false`. For long responses, the UI shows nothing until complete.

**What's needed:** Use `chat_stream` and translate chunks to `ResponseEvent::OutputTextDelta` in real time.

### G6: GPU-aware warm model preference
**Status:** DONE (warm_model tracking in OllamaClientPool)  
**Impact:** Medium — 10-20s cold-load penalty when switching models on same GPU

Two models on the 3080 at port 11435 (reasoner and coder). Ollama swaps models in/out. The routing should prefer the currently-loaded model.

**What's needed:** Track which model was last used on each endpoint. Prefer warm model if it's ≥80% as good as the optimal choice.

### G7: Per-request quality detection
**Status:** DONE (quality.rs, checks before returning, failures recorded to feedback)  
**Impact:** Medium — local model garbage returned to user without catch

If local model produces hallucination or incomplete response, system returns it as-is. Should detect and re-route to cloud.

**What's needed:** Quick quality check on local responses before returning. Can be simple (response too short, response is just code fences with no content, response repeats the question).

### G8: Classifier latency reduction
**Status:** DONE (classify_cache.rs, skip after 3 consecutive same-route, 30s TTL)  
**Impact:** Medium — 3-4s latency on every turn from classifier

Every request waits for the 1080 classifier before doing anything.

**What's needed:** Classification caching, async classification, or confidence-based skip. If last 3 requests all went cloud_coder, skip classifier for the next one.

### G9: Persistent cost analytics
**Status:** DONE (cost_analytics.rs, usage_log.jsonl, aggregate summaries)  
**Impact:** Low-medium — can't tune routing without data

Usage tracker resets every session. No persistent view of where tokens are going.

**What's needed:** Persist usage data to `.codex-multi/usage_history.jsonl`. CLI command or summary at session end.

## Future possibilities

### G10: Agent-to-agent communication during execution
Agents can't signal each other mid-task. Interface changes discovered by one agent aren't visible to parallel agents until failure.

### G11: Speculative execution
Start dependent tasks early with assumptions about predecessor output. Cancel if assumptions were wrong.

### G12: Multi-provider concurrent execution
Send same prompt to cheap + expensive model in parallel. Take first good response.

### G13: Model capability benchmarking
Periodically test local models on representative tasks. Update routing confidence.

### G14: Dynamic budget shifting
**Status:** DONE (budget_pressure.rs, reads rate limit headers, soft pressure 50-90%+, hard block 95%+)
Shift routing thresholds based on real-time budget consumption. More aggressive secondary routing as daily budget depletes.
