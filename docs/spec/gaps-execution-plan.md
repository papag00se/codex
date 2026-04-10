# Gaps Execution Plan

[< Spec Index](index.md) | [Gaps](gaps.md)

> Last updated: 2026-04-10

## Completed (G1-G9, G14)

All high-priority gaps from the initial plan are done:

| Gap | What | Implementation |
|-----|------|----------------|
| G1 | Routing feedback loop | `feedback.rs` — records outcomes to JSONL, computes success rates, injects into classifier |
| G2 | Codebase context | `codebase_context.rs` — auto-detects languages/frameworks, caches 1hr, injects into classifier |
| G3 | Cross-session memory | `session_memory.rs` — saves/loads handoffs to `.codex-multi/memory/`, prunes to 20 |
| G4 | Prompt adaptation | `prompt_adapt.rs` — per-tier scaffolding for task/planning/evaluation |
| G5 | Streaming | `ollama.rs` `chat_stream()` — real-time deltas for reasoner path |
| G6 | Warm model preference | `ollama.rs` — warm model tracking per endpoint |
| G7 | Quality detection | `quality.rs` — empty/short/echo/refusal/repetition checks |
| G8 | Classifier cache | `classify_cache.rs` — skips LLM after 3 same-route, 30s TTL |
| G9 | Cost analytics | `cost_analytics.rs` — persistent usage_log.jsonl with aggregates |
| G14 | Budget pressure | `budget_pressure.rs` — soft pressure 50-90%, hard block 95% |

Additionally built (not in original gap list):
- **Context stripping** (`context_strip.rs`) — two strip levels for local 8K models
- **Full compaction pipeline** (`compaction/`) — runs entirely on local Ollama, no proxy
- **Failover executor** (`failover.rs`) — F1-F8 failure classification, retry/chain-walk/hard-fail decisions

## In progress

No items currently in progress. All identified gaps are resolved.

## Recently completed

### G15: Wire failover executor into request flow
DONE. Local model failures walk the failover chain. Cloud HTTP errors classified and chain walked. `CloudOverride` carries failover context for retry loop in `client.rs`.

### G16: Local coder multi-turn tool reliability
DONE. Root cause was re-classification mid-tool-loop. Fixed with sticky routing: `has_pending_local_tool_loop()` detects local coder tool calls in conversation history and skips classifier.
