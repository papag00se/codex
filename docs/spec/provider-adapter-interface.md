# Minimal Provider Adapter Interface

[< Spec Index](index.md) | [Product Index](../product/index.md)

```python
"""
Provider adapter interface.

Every backend (Codex CLI, Claude Code, Ollama, OpenAI API, etc.) must
implement this ABC. The [orchestrator](operational-model.md) never speaks a provider's native
protocol — all interaction goes through adapters.
"""
from __future__ import annotations

from abc import ABC, abstractmethod
from typing import AsyncIterator

from ..schemas.run import Task
from ..schemas.worker import WorkerResult, RepositoryContext
from ..schemas.routing import ProviderCapability, CostEstimate, HealthStatus


class ProviderAdapter(ABC):
    """Abstract base class for provider adapters."""

    @abstractmethod
    async def execute_task(
        self,
        task: Task,
        context: RepositoryContext,
    ) -> WorkerResult:
        """
        Execute a bounded task and return the result.

        For agent backends (Codex CLI, Claude Code):
            - Build prompt from task.description + context
            - Spawn subprocess (codex exec / claude)
            - Parse output into WorkerResult
            - Respect task.timeout_seconds

        For API backends (OpenAI, Anthropic, Ollama):
            - Build messages from task.description + context
            - Call API with tool loop (bounded by task.max_turns)
            - Collect output into WorkerResult
            - Respect task.timeout_seconds

        Must handle:
            - Timeout (return WorkerResult with status="timeout")
            - Provider error (return WorkerResult with status="failure")
            - Cancellation (return WorkerResult with status="cancelled")
        """
        ...

    @abstractmethod
    async def execute_task_streaming(
        self,
        task: Task,
        context: RepositoryContext,
    ) -> AsyncIterator[WorkerEvent]:
        """
        Stream task execution events for real-time display.

        Yields WorkerEvent instances as the task progresses:
            - WorkerEvent(type="turn_start", turn=1)
            - WorkerEvent(type="output", text="Creating file...")
            - WorkerEvent(type="tool_call", tool="shell", args={...})
            - WorkerEvent(type="turn_end", turn=1)
            - WorkerEvent(type="complete", result=WorkerResult(...))
        """
        ...

    @abstractmethod
    async def capabilities(self) -> ProviderCapability:
        """
        Return capability metadata for this provider.

        Combines:
            - Static config (context_window, strengths, cost)
            - Dynamic state (health, measured latency)
        """
        ...

    @abstractmethod
    async def health(self) -> HealthStatus:
        """
        Probe provider health.

        For subprocess providers:
            - Check binary exists and is executable
            - Optionally: run a minimal test invocation

        For HTTP providers:
            - Ping health endpoint
            - Measure latency

        Returns HealthStatus with status in ("healthy", "degraded", "unavailable").
        """
        ...

    @abstractmethod
    async def estimate_cost(self, task: Task) -> CostEstimate:
        """
        Estimate cost of executing this task.

        For API providers: estimate tokens from task description length,
        multiply by per-token pricing.

        For subscription providers: return subscription_units.

        For free/local providers: return zeros.
        """
        ...

    @abstractmethod
    async def cancel(self, task_id: str) -> None:
        """
        Cancel an in-progress task.

        For subprocess providers:
            - Send SIGTERM to process group
            - Wait grace period (5s)
            - Send SIGKILL if still alive

        For HTTP providers:
            - Close the HTTP connection / cancel the request
        """
        ...


class WorkerEvent:
    """Event emitted during streaming task execution."""

    def __init__(self, type: str, **data):
        self.type = type
        self.data = data
```

## Example: CodexCliAdapter skeleton

```python
class CodexCliAdapter(ProviderAdapter):
    def __init__(self, config):
        self.command = config.get("command", "codex")
        self.default_args = config.get("args", ["exec"])

    async def execute_task(self, task, context) -> WorkerResult:
        prompt = self._build_prompt(task, context)
        args = [self.command, *self.default_args,
                "--cwd", context.worktree_path,
                "--json",
                prompt]

        proc = await asyncio.create_subprocess_exec(
            *args,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            env=self._build_env(),
        )

        try:
            stdout, stderr = await asyncio.wait_for(
                proc.communicate(),
                timeout=task.timeout_seconds,
            )
        except asyncio.TimeoutError:
            proc.kill()
            return WorkerResult(task_id=task.id, status="timeout")

        return self._parse_output(task.id, stdout, stderr, proc.returncode)

    async def capabilities(self) -> ProviderCapability:
        return ProviderCapability(
            provider_id="codex-cli",
            provider_type="agent_backend",
            cost_category="subscription",
            access_method="subprocess",
            context_window=128000,
            supports_tool_use=True,
            strengths={"code_generation": 0.90, "refactoring": 0.85},
        )

    async def health(self) -> HealthStatus:
        try:
            proc = await asyncio.create_subprocess_exec(
                self.command, "--version",
                stdout=asyncio.subprocess.PIPE,
            )
            await asyncio.wait_for(proc.communicate(), timeout=5)
            return HealthStatus(status="healthy" if proc.returncode == 0 else "unavailable")
        except Exception as e:
            return HealthStatus(status="unavailable", error=str(e))

    async def estimate_cost(self, task) -> CostEstimate:
        return CostEstimate(
            estimated_input_tokens=len(task.description) // 4,
            estimated_output_tokens=2000,
            cost_category="subscription",
            subscription_units=1.0,
        )

    async def cancel(self, task_id):
        # Track processes by task_id and send SIGTERM
        pass

    async def execute_task_streaming(self, task, context):
        # Similar to execute_task but yields events line-by-line
        pass

    def _build_prompt(self, task, context) -> str:
        return task.description

    def _build_env(self) -> dict:
        env = {"PATH": os.environ["PATH"], "HOME": os.environ["HOME"]}
        if api_key := os.environ.get("OPENAI_API_KEY"):
            env["OPENAI_API_KEY"] = api_key
        return env

    def _parse_output(self, task_id, stdout, stderr, returncode) -> WorkerResult:
        if returncode == 0:
            return WorkerResult(task_id=task_id, status="success",
                                output_text=stdout.decode())
        return WorkerResult(task_id=task_id, status="failure",
                            error_text=stderr.decode())
```
