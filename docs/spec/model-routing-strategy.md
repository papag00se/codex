# Model Routing Strategy

[< Spec Index](index.md) | [Design Principles](design-principles.md)

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

Each task type has a deterministic failover chain. If the first model fails (timeout, error, unavailable), try the next. No LLM decides failover — the code does.

```
Classifying:
  sakura:11434/qwen3.5-9b:iq4_xs
  → gpt-5.3-codex-spark
  → gpt-5.4-mini

Light reasoning:
  sakura:11435/qwen3.5:9b
  → meru:11434/qwen3.5:9b          (redundant local)
  → gpt-5.4-mini
  → sonnet-4.6

Light coding:
  sakura:11435/qwen3.5-9b-openclaw:tools
  → gpt-5.3-codex-spark
  → gpt-5.4-mini

Compaction:
  sakura:11435/qwen3.5-9b:iq4_xs
  → gpt-5.4-mini
  → gpt-5.4

Planning:
  sakura:11435/qwen3.5:9b
  → sonnet-4.6
  → opus-4.6

Complex reasoning:
  sonnet-4.6
  → opus-4.6
  → gpt-5.4

Complex coding (needs tool access — Codex sub-agent):
  gpt-5.3-codex-spark               (secondary bucket)
  → gpt-5.4-mini                    (secondary bucket)
  → sonnet-4.6                      (secondary bucket)
  → gpt-5.4                         (primary — last resort)

Review:
  sonnet-4.6
  → opus-4.6
  → gpt-5.4
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

```bash
# Local Ollama endpoints
export OLLAMA_CLASSIFIER_URL=http://sakura-wsl.taile41496.ts.net:11434
export OLLAMA_CLASSIFIER_MODEL=qwen3.5-9b:iq4_xs
export OLLAMA_REASONER_URL=http://sakura-wsl.taile41496.ts.net:11435
export OLLAMA_REASONER_MODEL=qwen3.5:9b
export OLLAMA_CODER_URL=http://sakura-wsl.taile41496.ts.net:11435
export OLLAMA_CODER_MODEL=qwen3.5-9b-opus-openclaw-distilled:tools
export OLLAMA_COMPACTOR_URL=http://sakura-wsl.taile41496.ts.net:11435
export OLLAMA_COMPACTOR_MODEL=qwen3.5-9b:iq4_xs
export OLLAMA_REASONER_BACKUP_URL=http://meru-wsl.taile41496.ts.net:11434
export OLLAMA_REASONER_BACKUP_MODEL=qwen3.5:9b
```
