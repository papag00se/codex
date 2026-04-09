"""Supervisor loop — drives a run from goal to completion."""
from __future__ import annotations

import asyncio
import json
import logging
import time
from typing import Any, Callable, Dict, List, Optional

from .providers.base import ProviderAdapter
from .schemas.run import Run, Task, validate_transition
from .schemas.events import (
    Event,
    RUN_CREATED, PLAN_GENERATED, TASK_CREATED,
    ROUTE_SELECTED, TASK_STARTED, TASK_COMPLETED, TASK_FAILED,
    VERIFICATION_REQUESTED, VERIFICATION_PASSED, VERIFICATION_FAILED,
    RETRY_SCHEDULED, RUN_COMPLETED, RUN_CANCELLED,
)
from .schemas.worker import WorkerResult, RepositoryContext

logger = logging.getLogger(__name__)


class Supervisor:
    """Bounded supervisor loop for a single run."""

    def __init__(
        self,
        run: Run,
        adapter: ProviderAdapter,
        *,
        on_event: Optional[Callable[[Event], None]] = None,
        verification_command: Optional[str] = None,
    ):
        self.run = run
        self.adapter = adapter
        self.on_event = on_event or (lambda e: None)
        self.verification_command = verification_command
        self.tasks: List[Task] = []
        self._sequence = 0

    def emit(self, event_type: str, task_id: Optional[str] = None, **data: Any) -> Event:
        self._sequence += 1
        evt = Event(
            type=event_type,
            run_id=self.run.id,
            task_id=task_id,
            sequence=self._sequence,
            data=data,
        )
        self.on_event(evt)
        return evt

    async def execute(self) -> Run:
        """Main entry point — run the full supervisor loop."""
        self.run.status = "running"
        self.run.started_at = int(time.time())
        self.emit(RUN_CREATED, goal=self.run.goal, repo_path=self.run.repo_path)

        # Phase 1: Plan — for now, the goal IS the single task
        self._create_single_task()

        # Phase 2: Execute tasks
        deadline = time.time() + self.run.timeout_seconds

        while not self._is_complete():
            if self.run.current_iteration >= self.run.max_iterations:
                self.run.status = "failed"
                self.run.error = "Max iterations reached"
                break

            if time.time() > deadline:
                self.run.status = "failed"
                self.run.error = "Timeout reached"
                break

            self.run.current_iteration += 1

            # Get next ready task
            ready = self._get_ready_tasks()
            if not ready:
                break

            # Execute ready tasks (sequentially for now)
            for task in ready:
                await self._execute_task(task)

        # Phase 3: Finalize
        if self.run.failed_tasks == 0 and self.run.completed_tasks == self.run.total_tasks:
            self.run.status = "completed"
            self.run.completed_at = int(time.time())
            self.emit(RUN_COMPLETED, tasks_completed=self.run.completed_tasks)
        elif self.run.status == "running":
            self.run.status = "failed"
            self.run.error = self.run.error or "Not all tasks completed"

        return self.run

    def _create_single_task(self) -> None:
        """Create a single task from the run goal (no planner yet)."""
        task = Task(
            run_id=self.run.id,
            description=self.run.goal,
            task_type="code",
            timeout_seconds=min(self.run.timeout_seconds, 300),
        )
        if self.verification_command:
            task.verification_command = self.verification_command
        self.tasks.append(task)
        self.run.total_tasks = 1
        self.emit(PLAN_GENERATED, task_count=1)
        self.emit(TASK_CREATED, task_id=task.id, description=task.description)

    def _get_ready_tasks(self) -> List[Task]:
        """Return tasks whose dependencies are satisfied and status is planned."""
        ready = []
        completed_ids = {t.id for t in self.tasks if t.status == "completed"}
        for task in self.tasks:
            if task.status != "planned":
                continue
            deps_met = all(d in completed_ids for d in task.dependencies)
            if deps_met:
                ready.append(task)
        return ready

    def _is_complete(self) -> bool:
        return self.run.completed_tasks + self.run.failed_tasks >= self.run.total_tasks

    async def _execute_task(self, task: Task) -> None:
        """Route, dispatch, verify, and complete/fail a single task."""
        # Route (for now: use the single adapter)
        task.status = "routed"
        task.assigned_backend = "codex-cli"
        caps = await self.adapter.capabilities()
        task.assigned_model = caps.provider_id
        self.emit(ROUTE_SELECTED, task_id=task.id,
                  backend=task.assigned_backend, confidence=1.0,
                  reason="single adapter mode")

        # Dispatch
        task.status = "running"
        task.started_at = int(time.time())
        self.emit(TASK_STARTED, task_id=task.id, backend=task.assigned_backend)

        context = RepositoryContext(
            repo_path=self.run.repo_path,
            worktree_path=self.run.repo_path,  # No worktree isolation yet
            branch=self.run.base_branch,
            base_branch=self.run.base_branch,
        )

        result = await self.adapter.execute_task(task, context)

        # Update cost
        task.cost_usd = result.cost_usd
        task.tokens_input = result.tokens_input
        task.tokens_output = result.tokens_output
        self.run.budget_spent += result.cost_usd

        if result.status != "success":
            await self._handle_failure(task, result)
            return

        # Verify
        task.status = "verifying"
        self.emit(VERIFICATION_REQUESTED, task_id=task.id)

        if self.verification_command:
            verified = await self._run_verification(task)
            if not verified:
                await self._handle_verification_failure(task, result)
                return
            self.emit(VERIFICATION_PASSED, task_id=task.id)
        else:
            self.emit(VERIFICATION_PASSED, task_id=task.id, skipped=True)

        # Complete
        task.status = "completed"
        task.completed_at = int(time.time())
        task.result_summary = result.output_text
        self.run.completed_tasks += 1
        self.emit(TASK_COMPLETED, task_id=task.id,
                  files_changed=result.files_changed,
                  tokens_input=result.tokens_input,
                  tokens_output=result.tokens_output,
                  duration_seconds=result.duration_seconds)

    async def _handle_failure(self, task: Task, result: WorkerResult) -> None:
        task.retry_count += 1
        if task.retry_count <= task.max_retries:
            task.status = "planned"
            task.description += f"\n\nPREVIOUS ATTEMPT FAILED:\n{result.error_text or 'Unknown error'}\nFix the issues and try again."
            self.emit(RETRY_SCHEDULED, task_id=task.id,
                      attempt=task.retry_count, error=result.error_text)
        else:
            task.status = "failed"
            task.error = result.error_text
            self.run.failed_tasks += 1
            self.emit(TASK_FAILED, task_id=task.id, error=result.error_text)

    async def _handle_verification_failure(self, task: Task, result: WorkerResult) -> None:
        task.retry_count += 1
        if task.retry_count <= task.max_retries:
            task.status = "planned"
            task.description += "\n\nPREVIOUS ATTEMPT FAILED VERIFICATION. Fix the issues and try again."
            self.emit(RETRY_SCHEDULED, task_id=task.id,
                      attempt=task.retry_count, reason="verification_failed")
        else:
            task.status = "failed"
            task.error = "Verification failed after max retries"
            self.run.failed_tasks += 1
            self.emit(TASK_FAILED, task_id=task.id, error="verification_failed")

    async def _run_verification(self, task: Task) -> bool:
        """Run the verification command and return True if it passes."""
        if not task.verification_command:
            return True
        try:
            proc = await asyncio.create_subprocess_exec(
                "bash", "-c", task.verification_command,
                cwd=self.run.repo_path,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await asyncio.wait_for(proc.communicate(), timeout=120)
            return proc.returncode == 0
        except asyncio.TimeoutError:
            return False
        except Exception as e:
            logger.warning("Verification error: %s", e)
            return False
