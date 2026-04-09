# Minimal Orchestrator Pseudocode

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Supervisor loop

```python
class Supervisor:
    def __init__(self, run: Run, config: Config, state_store, event_log,
                 routing_engine, worktree_manager, verifier, approval_gate):
        self.run = run
        self.config = config
        self.state = state_store
        self.events = event_log
        self.router = routing_engine
        self.worktrees = worktree_manager
        self.verifier = verifier
        self.approval = approval_gate
        self.adapters: dict[str, ProviderAdapter] = {}
        self.sequence = 0

    async def execute(self):
        """Main supervisor loop. Bounded by iterations, timeout, and budget (see [Architectural Principles](architectural-principles.md))."""
        self.run.status = "running"
        self.run.started_at = now()
        self.persist_run()
        self.emit("run.created", goal=self.run.goal)

        # Phase 1: Plan
        plan = await self.plan()
        if plan is None:
            self.fail_run("Planning failed")
            return

        # Phase 2: Execute tasks
        deadline = now() + self.run.timeout_seconds

        while not self.is_complete() and self.run.current_iteration < self.run.max_iterations:
            if now() > deadline:
                self.pause_run("Timeout reached")
                break

            if self.run.budget_limit and self.run.budget_spent >= self.run.budget_limit:
                self.pause_run("Budget exhausted")
                break

            self.run.current_iteration += 1

            # Get tasks that are ready (all deps satisfied)
            ready = self.scheduler.get_ready_tasks()
            if not ready:
                if self.has_running_tasks():
                    # Wait for running tasks to finish
                    await self.wait_for_any_completion()
                    continue
                else:
                    # No ready tasks, no running tasks — stuck or done
                    break

            # Dispatch ready tasks (up to parallel limit)
            batch = ready[:self.config.max_parallel_agents]
            tasks_to_dispatch = []

            for task in batch:
                try:
                    routed = await self.route_task(task)
                    worktree = await self.create_worktree(routed)
                    tasks_to_dispatch.append(worktree)
                except Exception as e:
                    self.fail_task(task.id, str(e))

            # Execute batch in parallel
            if tasks_to_dispatch:
                results = await asyncio.gather(
                    *[self.execute_task(t) for t in tasks_to_dispatch],
                    return_exceptions=True,
                )

                for task, result in zip(tasks_to_dispatch, results):
                    if isinstance(result, Exception):
                        await self.handle_task_failure(task, str(result))
                    else:
                        await self.handle_task_result(task, result)

        # Phase 3: Finalize
        if self.all_tasks_done():
            await self.complete_run()
        elif self.run.status == "running":
            self.fail_run("Supervisor loop ended without all tasks completing")

    def is_complete(self) -> bool:
        return self.run.completed_tasks + self.run.failed_tasks >= self.run.total_tasks
```

## Task creation

```python
    async def plan(self) -> list[Task] | None:
        self.emit("plan.requested", goal=self.run.goal)

        # Build repository context
        repo_context = await self.build_repo_context()

        # Invoke planner (LLM call via a provider adapter)
        planner_adapter = self.get_adapter_for_planning()
        plan_result = await planner_adapter.execute_task(
            Task(
                run_id=self.run.id,
                description=f"Decompose this goal into tasks: {self.run.goal}",
                task_type="plan",
            ),
            repo_context,
        )

        if plan_result.status != "success":
            return None

        # Parse task graph from planner output
        task_graph = json.loads(plan_result.output_text)

        tasks = []
        for t in task_graph["tasks"]:
            task = Task(
                run_id=self.run.id,
                description=t["description"],
                task_type=t.get("type", "code"),
                dependencies=t.get("dependencies", []),
                estimated_complexity=t.get("estimated_complexity"),
                verification_command=t.get("verification", self.config.verification.command),
            )
            self.state.save_task(task)
            self.emit("task.created", task_id=task.id, description=task.description)
            tasks.append(task)

        self.run.total_tasks = len(tasks)
        self.run.plan_json = task_graph
        self.persist_run()
        self.emit("plan.generated", task_count=len(tasks))

        self.scheduler = Scheduler(tasks)
        return tasks
```

## Routing selection

```python
    async def route_task(self, task: Task) -> Task:
        decision = await self.router.route_task(
            task=task,
            providers=self.router.registry.healthy_providers(),
            policy=self.config.routing,
            budget_remaining=(
                self.run.budget_limit - self.run.budget_spent
                if self.run.budget_limit else float("inf")
            ),
        )

        task.assigned_backend = decision.selected_backend
        task.assigned_model = decision.selected_model
        task.status = "routed"

        self.state.save_routing_decision(decision)
        self.state.update_task(task.id, status="routed",
                               assigned_backend=decision.selected_backend)
        self.emit("route.selected", task_id=task.id, decision=decision.model_dump())
        return task
```

## Worker dispatch

```python
    async def execute_task(self, task: Task) -> WorkerResult:
        task.status = "running"
        task.started_at = now()
        self.state.update_task(task.id, status="running", started_at=task.started_at)
        self.emit("task.started", task_id=task.id, backend=task.assigned_backend)

        adapter = self.get_adapter(task.assigned_backend)
        context = RepositoryContext(
            repo_path=self.run.repo_path,
            worktree_path=task.worktree_path,
            branch=task.worktree_branch,
            base_branch=self.run.base_branch,
        )

        try:
            result = await asyncio.wait_for(
                adapter.execute_task(task, context),
                timeout=task.timeout_seconds,
            )
        except asyncio.TimeoutError:
            result = WorkerResult(task_id=task.id, status="timeout",
                                  error_text="Task exceeded timeout")
        except Exception as e:
            result = WorkerResult(task_id=task.id, status="failure",
                                  error_text=str(e))

        return result
```

## Verification gate

```python
    async def handle_task_result(self, task: Task, result: WorkerResult):
        # Update cost tracking
        self.run.budget_spent += result.cost_usd
        task.cost_usd = result.cost_usd
        task.tokens_input = result.tokens_input
        task.tokens_output = result.tokens_output

        if result.status != "success":
            await self.handle_task_failure(task, result.error_text or "Unknown error")
            return

        # Verification
        task.status = "verifying"
        self.state.update_task(task.id, status="verifying")
        self.emit("verification.requested", task_id=task.id)

        verification = await self.verifier.verify(task)

        if verification.status == "pass":
            self.emit("verification.passed", task_id=task.id)
            task.verification_status = "pass"

            # Check if approval needed
            if await self.needs_approval(task, result):
                await self.request_approval(task, result)
            else:
                await self.complete_task(task, result)

        elif verification.status == "fail":
            self.emit("verification.failed", task_id=task.id,
                      output=verification.output)
            await self.handle_verification_failure(task, verification)

        else:  # error or skipped
            task.verification_status = verification.status
            await self.complete_task(task, result)
```

## Approval pause/resume

```python
    async def request_approval(self, task: Task, result: WorkerResult):
        task.status = "awaiting_approval"
        self.state.update_task(task.id, status="awaiting_approval")

        request = ApprovalRequest(
            task_id=task.id,
            run_id=self.run.id,
            action_type="task_completion",
            action_detail=f"Accept changes from {task.assigned_backend}",
            policy_rule="verification passed, approval required by policy",
        )
        self.state.save_approval(request)
        self.emit("approval.requested", approval_id=request.id, task_id=task.id)

        # Send to CLI via stdout and wait for response on stdin
        decision = await self.approval.request(request)

        if decision == "approved":
            self.emit("approval.granted", approval_id=request.id)
            await self.complete_task(task, result)
        else:
            self.emit("approval.denied", approval_id=request.id)
            self.fail_task(task.id, "Approval denied by user")
```

## Retry logic

```python
    async def handle_task_failure(self, task: Task, error: str):
        task.retry_count += 1

        if task.retry_count <= task.max_retries:
            # Schedule retry, possibly with escalated backend
            previous_backend = task.assigned_backend
            task.status = "planned"  # Reset to planned for re-routing
            task.assigned_backend = None
            task.assigned_model = None

            self.state.update_task(task.id, status="planned",
                                   retry_count=task.retry_count)
            self.emit("retry.scheduled", task_id=task.id,
                      attempt=task.retry_count,
                      previous_backend=previous_backend,
                      reason=error)

            # Clean up old worktree
            if task.worktree_path:
                await self.worktrees.cleanup(task)

        else:
            # Max retries exhausted
            task.status = "failed"
            task.error = error
            self.run.failed_tasks += 1

            self.state.update_task(task.id, status="failed", error=error)
            self.emit("task.failed", task_id=task.id, error=error,
                      attempts=task.retry_count)

            # Skip dependent tasks
            for dep_task in self.scheduler.get_dependents(task.id):
                dep_task.status = "skipped"
                self.state.update_task(dep_task.id, status="skipped",
                                       error=f"Dependency {task.id} failed")

    async def handle_verification_failure(self, task: Task, verification):
        if task.retry_count < task.max_retries:
            # Retry with verification failure feedback
            task.description += (
                f"\n\nPREVIOUS ATTEMPT FAILED VERIFICATION:\n"
                f"{verification.output}\n"
                f"Fix the issues and try again."
            )
            await self.handle_task_failure(task, f"Verification failed: {verification.output[:200]}")
        else:
            self.fail_task(task.id, f"Verification failed after {task.max_retries} retries")
```

## Crash recovery

```python
    @classmethod
    async def resume(cls, run_id: str, config: Config) -> Supervisor:
        state = StateStore(config.state_db_path)
        event_log = EventLog(config.event_log_dir)

        # Load persisted state
        run = state.get_run(run_id)
        if run is None:
            raise ValueError(f"Run {run_id} not found")

        tasks = state.get_tasks(run_id)
        events = event_log.read_all(run_id)

        # Verify consistency
        last_seq = events[-1].sequence if events else 0

        # Reset stuck tasks (were running/verifying when we crashed)
        for task in tasks:
            if task.status in ("running", "assigned", "verifying"):
                if task.retry_count < task.max_retries:
                    state.update_task(task.id, status="planned",
                                      retry_count=task.retry_count + 1)
                else:
                    state.update_task(task.id, status="failed",
                                      error="Process crashed during execution")

        # Clean orphaned worktrees
        worktree_mgr = WorktreeManager(run.repo_path, state)
        await worktree_mgr.cleanup_orphans(run)

        # Create supervisor and resume
        supervisor = cls(run, config, state, event_log, ...)
        supervisor.scheduler = Scheduler(tasks)
        supervisor.sequence = last_seq

        run.status = "running"
        state.update_run(run_id, status="running")
        supervisor.emit("run.resumed", from_sequence=last_seq,
                        completed=run.completed_tasks, remaining=run.total_tasks - run.completed_tasks)

        await supervisor.execute()
        return supervisor
```
