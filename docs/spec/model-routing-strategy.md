# Model Routing Strategy

[< Spec Index](index.md) | [Design Principles](design-principles.md)

> **Resolved:** G1 (routing feedback), G2 (codebase context), G6 (warm model preference), G7 (quality detection) are all implemented. See [Gaps](gaps.md) for remaining items.

## Available infrastructure

### Local Ollama (free, always available)

| Host | GPU | Models | Role |
|------|-----|--------|------|
| `sakura-wsl:11435` | 3080 8GB | `qwen3.5:9b` | Light reasoning |
| | | `qwen3.5-9b-opus-openclaw-distilled:tools` | Light coding, tool calls |
| | | `qwen3.5-9b:iq4_xs` | Compaction |
| `meru-wsl:11434` | 3080 8GB | `qwen3.5:9b` | Light reasoning (redundant) |
| `sakura-wsl:11434` | 1080 8GB | `qwen3.5-9b:iq4_xs` | Classifying |

### Cloud — primary usage buckets (conserve)

| Model | Provider | Bucket | Strength |
|-------|----------|--------|----------|
| `gpt-5.4` | OpenAI | Primary | Top-tier coding + reasoning |
| `opus-4.6` | Anthropic | Primary | Top-tier coding + reasoning |

### Cloud — secondary usage buckets (prefer when possible)

| Model | Provider | Bucket | Strength |
|-------|----------|--------|----------|
| `gpt-5.4-mini` | OpenAI | Secondary | Good coding, cheaper |
| `gpt-5.3-codex-spark` | OpenAI | Secondary | Fast coding, cheapest |
| `sonnet-4.6` | Anthropic | Secondary | Good coding, cheaper |

## Task-to-model routing matrix

The routing decision follows this priority: **free → secondary bucket → primary bucket**. Only escalate when the cheaper tier can't handle the task.

| Task type | First choice (free) | Second choice (secondary) | Last resort (primary) |
|-----------|-------------------|--------------------------|---------------------|
| **Classifying** | `sakura:11434` qwen3.5-9b:iq4_xs | gpt-5.3-codex-spark | gpt-5.4-mini |
| **Light reasoning** | `sakura:11435` qwen3.5:9b or `meru:11434` qwen3.5:9b | gpt-5.4-mini | gpt-5.4 |
| **Light coding** | `sakura:11435` qwen3.5-9b-openclaw:tools | gpt-5.3-codex-spark | gpt-5.4-mini |
| **Compaction** | `sakura:11435` qwen3.5-9b:iq4_xs | gpt-5.4-mini | gpt-5.4 |
| **Planning** | `sakura:11435` qwen3.5:9b | sonnet-4.6 | opus-4.6 |
| **Reasoning** (complex) | — | sonnet-4.6 | opus-4.6 or gpt-5.4 |
| **Coding** (complex) | — | gpt-5.3-codex-spark or sonnet-4.6 | gpt-5.4 or opus-4.6 |
| **Review** | — | sonnet-4.6 | opus-4.6 |

## Failover chains

Each task type has a deterministic failover chain configured in `.codex-multi/config.toml`. If the first model fails (timeout, error, quality failure, unavailable), the failover executor (`failover.rs`) walks the chain. No LLM decides failover — deterministic code does.

Failure types (F1-F8) determine how failover proceeds:
- **F1 (rate limit)**: retry same model with backoff (honor retry-after header), then walk chain
- **F2 (quota exhausted)**: walk chain immediately (waiting won't help)
- **F3 (model unavailable)**: walk chain immediately
- **F4 (model not found)**: walk chain with config warning
- **F5 (auth failure)**: hard-fail, never retry
- **F6 (timeout)**: retry once, then walk chain
- **F7 (quality failure)**: walk chain immediately (different model may do better)
- **F8 (context overflow)**: walk chain to model with larger context window

```
Classification:
  classifier → light_reasoner → cloud_fast

Reasoning:
  light_reasoner → light_reasoner_backup → cloud_reasoner → cloud_coder

Coding:
  light_coder → cloud_fast → cloud_mini → cloud_reasoner → cloud_coder

Compaction:
  compactor → light_reasoner → cloud_mini

Planning:
  light_reasoner → cloud_reasoner → cloud_coder

Evaluation:
  light_reasoner → light_reasoner_backup → cloud_mini

Review:
  cloud_reasoner → cloud_coder
```

## Usage preservation strategy

The key insight from coding-agent-router: **siphon work into secondary model buckets** to avoid draining primary usage.

- `gpt-5.3-codex-spark` is the cheapest OpenAI option for simple coding tasks
- `gpt-5.4-mini` handles medium complexity without touching the gpt-5.4 bucket
- `sonnet-4.6` is the Anthropic secondary — use it before opus

The supervisor should track which bucket each call goes to and prefer secondary buckets whenever the task quality allows.

## How this maps to the supervisor

### Planning (supervisor.plan_tasks)
- Try: `sakura:11435/qwen3.5:9b` (free)
- Fallback: Codex sub-agent with default model

### Evaluation (supervisor.evaluate_completion)
- Try: `sakura:11435/qwen3.5:9b` (free)
- Fallback: Codex sub-agent

### Coding work (supervisor.dispatch_task)
- Always Codex sub-agent (needs tool access)
- Model selection: could pass model override to sub-agent config
- Prefer: spark → mini → sonnet → gpt-5.4

### Verification (supervisor.verify)
- Deterministic: run command, check exit code
- No model needed

## Configuration

All model routing is configured in `.codex-multi/config.toml` per working directory. Environment variables are supported as fallback but the TOML config is preferred.

```toml
# .codex-multi/config.toml — see full example in repo root

[models.classifier]
endpoint = "http://sakura-wsl.taile41496.ts.net:11434"
model = "qwen3.5-9b:iq4_xs"
num_ctx = 4096
provider = "ollama"
reasoning = "off"

[models.light_reasoner]
endpoint = "http://sakura-wsl.taile41496.ts.net:11435"
model = "qwen3.5-9b-opus-openclaw-distilled:tools"
num_ctx = 16384
provider = "ollama"
reasoning = "on"

[models.cloud_fast]
entries = [
    { provider = "openai", model = "gpt-5.3-codex-spark", weight = 100, reasoning = "xhigh" },
]

# Failover chains per task type
[failover]
reasoning = ["light_reasoner", "light_reasoner_backup", "cloud_reasoner", "cloud_coder"]
coding = ["light_coder", "cloud_fast", "cloud_mini", "cloud_reasoner", "cloud_coder"]

# Failover behavior
[failover.behavior]
retry_same_attempts = 2
rate_limit_default_wait_ms = 5000
rate_limit_max_wait_ms = 30000
timeout_ms = 30000
```

See [Implementation Status](implementation-status.md) for the full config format and all supported fields.
