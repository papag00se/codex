"""Codex CLI provider adapter — spawns `codex exec` as a subprocess."""
from __future__ import annotations

import asyncio
import json
import logging
import shutil
import time
from typing import Any, Dict, List, Optional

from .base import ProviderAdapter
from ..schemas.run import Task
from ..schemas.worker import WorkerResult, RepositoryContext
from ..schemas.routing import ProviderCapability, CostEstimate, HealthStatus

logger = logging.getLogger(__name__)


class CodexCliAdapter(ProviderAdapter):
    def __init__(
        self,
        command: str = "codex",
        model_provider: Optional[str] = None,
        model: Optional[str] = None,
        timeout_seconds: int = 300,
    ):
        self.command = command
        self.model_provider = model_provider
        self.model = model
        self.timeout_seconds = timeout_seconds
        self._processes: Dict[str, asyncio.subprocess.Process] = {}

    async def execute_task(self, task: Task, context: RepositoryContext) -> WorkerResult:
        started = time.monotonic()
        cmd = [
            self.command, "exec",
            "--json",
            "--full-auto",
            "--ephemeral",
            "--skip-git-repo-check",
            "-C", context.worktree_path,
        ]
        # Only override model/provider if explicitly set — otherwise use
        # whatever the user's ~/.codex/config.toml already has.
        if self.model_provider:
            cmd.extend(["-c", f"model_provider={self.model_provider}"])
        if self.model:
            cmd.extend(["-c", f"model={self.model}"])
        cmd.append(task.description)

        logger.info("codex exec: %s", " ".join(cmd[:6]) + " ...")

        proc = await asyncio.create_subprocess_exec(
            *cmd,
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
            cwd=context.worktree_path,
        )
        self._processes[task.id] = proc

        try:
            stdout_bytes, stderr_bytes = await asyncio.wait_for(
                proc.communicate(),
                timeout=task.timeout_seconds or self.timeout_seconds,
            )
        except asyncio.TimeoutError:
            proc.kill()
            await proc.wait()
            return WorkerResult(
                task_id=task.id,
                status="timeout",
                error_text=f"codex exec timed out after {task.timeout_seconds}s",
                duration_seconds=time.monotonic() - started,
            )
        finally:
            self._processes.pop(task.id, None)

        elapsed = time.monotonic() - started
        stdout = stdout_bytes.decode("utf-8", errors="replace")
        stderr = stderr_bytes.decode("utf-8", errors="replace")

        # Parse JSONL events from stdout
        events = _parse_jsonl(stdout)
        errors = _extract_errors(events)
        files_changed = _extract_files_changed(events)
        output_text = _extract_agent_text(events)
        tokens_in, tokens_out = _extract_usage(events)

        # If there are error events, treat as failure regardless of exit code
        if errors:
            return WorkerResult(
                task_id=task.id,
                status="failure",
                output_text=output_text,
                error_text=errors[0],
                tokens_input=tokens_in,
                tokens_output=tokens_out,
                duration_seconds=elapsed,
                provider_metadata={"events": events, "errors": errors},
            )

        if proc.returncode == 0:
            return WorkerResult(
                task_id=task.id,
                status="success",
                files_changed=files_changed,
                output_text=output_text,
                tokens_input=tokens_in,
                tokens_output=tokens_out,
                duration_seconds=elapsed,
                provider_metadata={"events": events},
            )
        else:
            return WorkerResult(
                task_id=task.id,
                status="failure",
                output_text=output_text,
                error_text=stderr or f"codex exec exited with code {proc.returncode}",
                tokens_input=tokens_in,
                tokens_output=tokens_out,
                duration_seconds=elapsed,
                provider_metadata={"events": events},
            )

    async def capabilities(self) -> ProviderCapability:
        return ProviderCapability(
            provider_id="codex-cli",
            provider_type="agent_backend",
            cost_category="subscription",
            access_method="subprocess",
            context_window=200000,
            supports_tool_use=True,
            supports_streaming=True,
            strengths={
                "code_generation": 0.90,
                "refactoring": 0.85,
                "bug_fixing": 0.85,
                "test_interpretation": 0.80,
            },
        )

    async def health(self) -> HealthStatus:
        path = shutil.which(self.command)
        if path:
            return HealthStatus(status="healthy")
        return HealthStatus(status="unavailable", error=f"{self.command} not found in PATH")

    async def estimate_cost(self, task: Task) -> CostEstimate:
        return CostEstimate(
            estimated_input_tokens=len(task.description) // 4,
            estimated_output_tokens=2000,
            cost_category="subscription",
            subscription_units=1.0,
        )

    async def cancel(self, task_id: str) -> None:
        proc = self._processes.get(task_id)
        if proc and proc.returncode is None:
            proc.terminate()
            try:
                await asyncio.wait_for(proc.wait(), timeout=5)
            except asyncio.TimeoutError:
                proc.kill()


def _parse_jsonl(text: str) -> List[Dict[str, Any]]:
    events = []
    for line in text.splitlines():
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return events


def _extract_errors(events: List[Dict[str, Any]]) -> List[str]:
    errors = []
    for evt in events:
        if evt.get("type") == "error":
            errors.append(evt.get("message", "unknown error"))
        if evt.get("type") == "turn.failed":
            err = evt.get("error", {})
            msg = err.get("message", "") if isinstance(err, dict) else str(err)
            if msg:
                errors.append(msg)
    return errors


def _extract_agent_text(events: List[Dict[str, Any]]) -> str:
    texts = []
    for evt in events:
        if evt.get("type") == "item.completed":
            item = evt.get("item", {})
            if item.get("type") == "agent_message":
                text = item.get("text", "")
                if text:
                    texts.append(text)
    return "\n".join(texts)


def _extract_files_changed(events: List[Dict[str, Any]]) -> List[str]:
    files = set()
    for evt in events:
        if evt.get("type") == "item.completed":
            item = evt.get("item", {})
            if item.get("type") == "command_execution":
                cmd = item.get("command", "")
                # Heuristic: look for file paths in apply-patch or edit commands
                if "apply_patch" in cmd or "edit" in cmd:
                    output = item.get("aggregated_output", "")
                    for line in output.splitlines():
                        line = line.strip()
                        if line.startswith("--- ") or line.startswith("+++ "):
                            path = line[4:].strip()
                            if path and path != "/dev/null":
                                files.add(path.lstrip("ab/"))
    return sorted(files)


def _extract_usage(events: List[Dict[str, Any]]) -> tuple[int, int]:
    total_in = 0
    total_out = 0
    for evt in events:
        if evt.get("type") == "turn.completed":
            usage = evt.get("usage", {})
            total_in += usage.get("input_tokens", 0)
            total_out += usage.get("output_tokens", 0)
    return total_in, total_out
