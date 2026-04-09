# Provider Abstraction Specification

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Core concepts

| Concept | Definition |
|---------|-----------|
| **Provider** | A named backend service that can execute agent tasks (e.g., "claude-code", "openai-api", "ollama-local") |
| **Model** | A specific model within a provider (e.g., "gpt-5.4", "claude-opus-4", "qwen3-coder:30b") |
| **Provider Adapter** | Code that translates between the orchestrator's interface and the provider's native protocol |
| **Execution Target** | A provider + model + config combination ready to receive tasks |
| **Capability Metadata** | Static description of what a provider/model can do (context window, strengths, etc.) |
| **Cost Metadata** | How the provider charges: per-token, subscription, free |
| **Health Status** | Runtime state: healthy, degraded, unavailable |

## Provider types

### Tool-enabled agent backends (agentic)
These backends manage their own tool execution (shell, file editing, etc.):
- **Codex CLI** (`codex exec`) — manages its own tool loop, sandbox, approval
- **Claude Code** (`claude`) — manages its own tool loop, sandbox, approval

The orchestrator delegates a full task and collects the result. It does not intercept individual tool calls.

### API-backed execution targets (non-agentic)
These backends provide model completions without tool execution:
- **OpenAI API** — chat completions or responses API
- **Anthropic API** — messages API

The orchestrator or a thin agent wrapper manages the tool loop, calling the API for completions and executing tools locally.

### Local execution targets
- **Ollama** — local inference via HTTP API
- Behaves like an API-backed target but with zero network cost and lower quality

### Subscription-backed execution targets
A subtype of agent backends where cost is subscription-based:
- **Codex CLI with ChatGPT Plus** — subscription-funded
- **Claude Code with Claude Pro** — subscription-funded

Same interface as agent backends, but cost tracking uses subscription units instead of per-token billing.

## ProviderCapability schema

```json
{
  "provider_id": "claude-code",
  "provider_type": "agent_backend",
  "cost_category": "subscription",
  "access_method": "subprocess",
  
  "capabilities": {
    "context_window": 200000,
    "max_output_tokens": 16000,
    "supports_tool_use": true,
    "supports_streaming": true,
    "supports_structured_output": true,
    "supports_image_input": true,
    
    "strengths": {
      "code_generation": 0.95,
      "code_review": 0.95,
      "planning": 0.90,
      "refactoring": 0.90,
      "test_interpretation": 0.85,
      "documentation": 0.85,
      "bug_fixing": 0.90,
      "security_analysis": 0.90
    }
  },
  
  "cost": {
    "category": "subscription",
    "input_cost_per_1k_tokens": null,
    "output_cost_per_1k_tokens": null,
    "subscription_weight": 1.0,
    "estimated_hourly_cap": 30
  },
  
  "latency": {
    "category": "medium",
    "estimated_first_token_ms": 2000,
    "estimated_tokens_per_second": 80
  },
  
  "privacy": {
    "level": "cloud",
    "data_retention": "provider_policy",
    "region": "us"
  },
  
  "constraints": {
    "max_concurrent": 2,
    "requires_network": true,
    "requires_auth": true,
    "platforms": ["linux", "macos"]
  }
}
```

## ProviderAdapter interface

Every provider adapter must implement this interface:

```python
from abc import ABC, abstractmethod
from typing import AsyncIterator

class ProviderAdapter(ABC):
    """Adapter that normalizes a heterogeneous backend into a common interface."""
    
    @abstractmethod
    async def execute_task(
        self,
        task: Task,
        context: RepositoryContext,
        config: ProviderConfig,
    ) -> WorkerResult:
        """
        Execute a bounded task and return the result.
        
        For agent backends (Codex CLI, Claude Code): spawns the tool as a
        subprocess, passes the task prompt, collects output.
        
        For API backends (OpenAI, Anthropic, Ollama): runs a completion loop
        with tool execution, bounded by max_turns.
        
        Must be cancellable via task cancellation token.
        """
        ...
    
    @abstractmethod
    async def execute_task_streaming(
        self,
        task: Task,
        context: RepositoryContext,
        config: ProviderConfig,
    ) -> AsyncIterator[WorkerEvent]:
        """
        Stream task execution events for real-time display.
        Yields WorkerEvent instances as the task progresses.
        """
        ...
    
    @abstractmethod
    async def capabilities(self) -> ProviderCapability:
        """
        Return static + dynamic capability metadata.
        Static: from config (context window, strengths).
        Dynamic: from runtime probing (health, latency).
        """
        ...
    
    @abstractmethod
    async def health(self) -> HealthStatus:
        """
        Check provider health. Returns:
        - healthy: ready to accept tasks
        - degraded: operational but slow or limited
        - unavailable: cannot accept tasks
        """
        ...
    
    @abstractmethod
    async def estimate_cost(self, task: Task) -> CostEstimate:
        """
        Estimate the cost of executing a task on this provider.
        Returns token estimate and dollar estimate.
        For subscription providers, returns subscription units.
        For free providers, returns 0.
        """
        ...
    
    @abstractmethod
    async def cancel(self, task_id: str) -> None:
        """
        Cancel an in-progress task.
        For subprocess providers: send SIGTERM, wait, SIGKILL.
        For API providers: close the connection.
        """
        ...
```

## HealthStatus

```python
class HealthStatus:
    status: Literal["healthy", "degraded", "unavailable"]
    latency_ms: Optional[int]      # Last probe latency
    error: Optional[str]           # Error message if degraded/unavailable
    last_checked: datetime
    consecutive_failures: int
```

## CostEstimate

```python
class CostEstimate:
    estimated_input_tokens: int
    estimated_output_tokens: int
    estimated_cost_usd: Optional[float]    # None for subscription/free
    subscription_units: Optional[float]     # None for API/free
    cost_category: Literal["api", "subscription", "free"]
```

See [Provider Adapter Interface](provider-adapter-interface.md) for the ABC and skeleton code.

## v1 adapter implementations

| Adapter | Access | Tool Loop | Notes |
|---------|--------|-----------|-------|
| `CodexCliAdapter` | Subprocess (`codex exec`) | Managed by Codex CLI | Parse JSONL output events |
| `ClaudeCodeAdapter` | Subprocess (`claude`) | Managed by Claude Code | Parse JSON output; need `--json` or `--output-format json` flag |
| `OllamaAdapter` | HTTP (`/api/chat`) | Managed by adapter (thin tool loop) | Routes through coding-agent-router or direct |
| `OpenAiApiAdapter` | HTTP (Responses API) | Managed by adapter | For non-agentic completions (review, interpretation) |
| `AnthropicApiAdapter` | HTTP (Messages API) | Managed by adapter | For non-agentic completions |

## Provider configuration

```toml
# Per-provider config in multi-agent.toml

[providers.claude-code]
enabled = true
type = "agent_backend"
cost_category = "subscription"
command = "claude"
args = ["--output-format", "json"]
env = { ANTHROPIC_API_KEY = "${ANTHROPIC_API_KEY}" }

[providers.claude-code.capabilities]
context_window = 200000
code_generation = 0.95
code_review = 0.95

[providers.codex-cli]
enabled = true
type = "agent_backend"
cost_category = "subscription"
command = "codex"
args = ["exec"]

[providers.ollama]
enabled = true
type = "local"
cost_category = "free"
base_url = "http://127.0.0.1:11434"
model = "qwen3-coder:30b"

[providers.ollama.capabilities]
context_window = 16384
code_generation = 0.65

[providers.openai-api]
enabled = true
type = "api"
cost_category = "api"
api_key_env = "OPENAI_API_KEY"
base_url = "https://api.openai.com/v1"
model = "gpt-5.4"

[providers.openai-api.cost]
input_per_1k = 0.01
output_per_1k = 0.03
```
