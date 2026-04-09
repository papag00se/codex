# Gaps Execution Plan

[< Spec Index](index.md) | [Gaps](gaps.md)

## Priority order

Executing G1, G2, G5, G6, G7 — the five gaps with highest impact-to-effort ratio.

### 1. G1: Routing feedback loop (highest impact)
Record routing outcomes to `.codex-multi/routing_history.jsonl`. On startup, compute success rates. Inject into classifier prompt.

**Steps:**
- Add `RoutingOutcome` struct to routing crate
- After each local/cloud response, append outcome to JSONL file
- On init, load history, compute per-model per-route success rates
- Add success rates to classifier prompt context

### 2. G2: Codebase context in classifier
Add `[context]` to config. Auto-detect on first run. Inject into classifier.

**Steps:**
- Add `ProjectContext` to config schema
- Scan cwd for: languages (by file extension), file count, test framework, presence of key files
- Cache result in `.codex-multi/context_cache.json`
- Inject into classifier prompt

### 3. G5: Streaming from local models
Wire `chat_stream` through to `ResponseEvent` deltas.

**Steps:**
- Replace `call_ollama_text` (non-streaming) with streaming variant for local responses
- Yield `OutputTextDelta` events as chunks arrive
- Send `OutputItemDone` and `Completed` at the end

### 4. G6: GPU-aware warm model preference
Track last-used model per endpoint. Prefer warm model.

**Steps:**
- Add `last_model` tracking per endpoint URL in `OllamaClientPool`
- In classifier routing, if a local endpoint's warm model is ≥ acceptable for this route, prefer it
- Avoids 10-20s cold-load penalty

### 5. G7: Per-request quality detection
Quick check before returning local responses.

**Steps:**
- After Ollama response, check: too short? empty? just repeats prompt? contains error markers?
- If quality check fails, discard and return `RouteResult::Default` (cloud fallback)
- Log the quality failure for G1 feedback
