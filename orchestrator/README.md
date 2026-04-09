# Orchestrator — Reference Implementation

This directory contains **Python reference code** migrated from `coding-agent-router`. It is NOT runtime code — the actual implementation is in Rust:

- `codex-rs/routing/` — Routing engine (Rust crate, 31 tests)
- `codex-rs/supervisor/` — Deterministic supervisor loop (Rust crate, 13 tests)
- `codex-rs/core/src/tools/handlers/supervisor.rs` — Integration with Codex

## What's here

The Python code preserves every heuristic and algorithm from the original coding-agent-router so nothing is lost during the Rust port:

- `routing/metrics.py` — 27 task metrics (regex patterns, token estimation)
- `providers/tool_adapter.py` — Tool-call recovery for local models
- `providers/ollama_client.py` — Ollama HTTP client with serialization
- `compaction/` — Full compaction pipeline (chunking, extraction, merging, refinement)
- `prompts/` — LLM prompt files (shared with Rust implementation)
- `schemas/` — Pydantic data models

## How to use

Run the Python tests to verify reference behavior:

```bash
cd orchestrator
python -m venv .venv
source .venv/bin/activate
pip install -e ".[dev]"
pytest tests/ -v
```

Then compare against the Rust implementation:

```bash
cd codex-rs
cargo test -p codex-routing -p codex-supervisor
```

## Docs

- [Routing Logic Reference](../docs/spec/routing-logic-reference.md) — Every routing heuristic preserved
- [Compaction Reference](../docs/spec/compaction-reference.md) — Full compaction pipeline documented
- [Design Principles](../docs/spec/design-principles.md) — Deterministic control, intelligent judgment
- [Supervisor Integration](../docs/spec/supervisor-integration.md) — How the supervisor tool works
