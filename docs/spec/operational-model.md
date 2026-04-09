# Operational Model

[< Spec Index](index.md) | [Product Index](../product/index.md)

## How the CLI starts runs

The user types `codex run`. The Rust CLI handles everything the user sees. The Python orchestrator is an invisible subprocess.

```
1. User types: codex run "Add rate limiting with Redis"

2. Codex CLI (Rust — codex-rs/cli/src/main.rs):
   a. Parse arguments, load config
   b. Generate run ID: r_<nanoid>
   c. Create initial Run record in SQLite
   d. Spawn subprocess: python -m codex_orchestrator --run-id r_xxx
   e. Read JSON events from orchestrator's stdout
   f. Write user decisions to orchestrator's stdin (approvals, cancel)
   g. Render events to terminal (TUI or JSON mode)
   h. When orchestrator exits, display final summary

3. Orchestrator (Python — invisible subprocess, no user interaction):
   a. Reads run from state store
   b. Enters supervisor loop
   c. Emits JSON events on stdout as work progresses
   d. Reads user decisions from stdin (relayed by CLI)
   e. Exits with status code when done

The user sees only step 1 and the Rust CLI's rendering of events.
The orchestrator subprocess is an implementation detail.
```

## How the orchestrator persists progress

Every state change follows this pattern (see [Event Model](event-model.md) for event types):

```python
async def transition_task(task_id: str, new_status: str, **data):
    # 1. Write event to JSONL (durable, append-only)
    event = Event(
        type=f"task.{new_status}",
        run_id=self.run.id,
        task_id=task_id,
        sequence=self.next_sequence(),
        timestamp=now(),
        data=data,
    )
    self.event_log.append(event)  # fsync
    
    # 2. Update SQLite state
    self.state_store.update_task(task_id, status=new_status, **data)
    
    # 3. Emit to CLI via stdout
    print(json.dumps(event.model_dump()), flush=True)
```

**Event log is written before SQLite.** If the process crashes after JSONL write but before SQLite update, the resume logic detects the inconsistency and replays the event.

## How agents receive tasks

Agents don't "receive" tasks in a persistent queue sense. The orchestrator invokes them directly:

```python
async def dispatch_task(task: Task):
    # 1. Select provider adapter based on routing decision
    adapter = self.get_adapter(task.assigned_backend)
    
    # 2. Build repository context
    context = RepositoryContext(
        repo_path=self.run.repo_path,
        worktree_path=task.worktree_path,
        branch=task.worktree_branch,
        base_branch=self.run.base_branch,
    )
    
    # 3. Invoke adapter (blocks until complete or timeout)
    result = await asyncio.wait_for(
        adapter.execute_task(task, context),
        timeout=task.timeout_seconds,
    )
    
    return result
```

For parallel tasks, the orchestrator uses `asyncio.gather()`:

```python
ready_tasks = self.scheduler.get_ready_tasks()
dispatches = [self.dispatch_task(t) for t in ready_tasks[:self.max_parallel]]
results = await asyncio.gather(*dispatches, return_exceptions=True)
```

## How provider routing is invoked

```python
async def route_and_dispatch(task: Task):
    # 1. Ask routing engine for a decision
    decision = await self.routing_engine.route_task(
        task=task,
        providers=self.capability_registry.healthy_providers(),
        policy=self.config.routing,
        budget_remaining=self.run.budget_limit - self.run.budget_spent,
    )
    
    # 2. Persist routing decision
    self.state_store.save_routing_decision(decision)
    self.emit_event("route.selected", task_id=task.id, decision=decision)
    
    # 3. Update task with assignment
    task.assigned_backend = decision.selected_backend
    task.assigned_model = decision.selected_model
    self.state_store.update_task(task.id, assigned_backend=decision.selected_backend)
    
    # 4. Create worktree
    worktree = await self.worktree_manager.create(self.run, task)
    task.worktree_path = worktree.path
    task.worktree_branch = worktree.branch
    
    # 5. Dispatch to agent
    result = await self.dispatch_task(task)
    return result
```

## How worktrees are created

```python
class WorktreeManager:
    async def create(self, run: Run, task: Task) -> Worktree:
        worktree_dir = f"{run.repo_path}/.codex-worktrees/{run.id}/{task.id}"
        branch_name = f"codex/{run.id}/{task.id}"
        
        # Ensure result branch exists
        await self._ensure_result_branch(run)
        
        # Create worktree from result branch
        await run_git(
            "worktree", "add", worktree_dir,
            "-b", branch_name,
            f"codex/{run.id}/result",
            cwd=run.repo_path,
        )
        
        # Record in state
        self.state_store.save_worktree(Worktree(
            path=worktree_dir,
            task_id=task.id,
            run_id=run.id,
            branch=branch_name,
            status="active",
        ))
        
        return Worktree(path=worktree_dir, branch=branch_name)
```

## How results are verified

```python
async def verify_task(task: Task, result: WorkerResult):
    if not task.verification_command:
        self.emit_event("verification.skipped", task_id=task.id)
        return VerificationResult(status="skipped")
    
    # Run verification command in the worktree
    proc = await asyncio.create_subprocess_exec(
        "bash", "-c", task.verification_command,
        cwd=task.worktree_path,
        stdout=asyncio.subprocess.PIPE,
        stderr=asyncio.subprocess.PIPE,
    )
    
    try:
        stdout, stderr = await asyncio.wait_for(
            proc.communicate(),
            timeout=self.config.verification.timeout_seconds,
        )
    except asyncio.TimeoutError:
        proc.kill()
        return VerificationResult(status="error", error="Verification timed out")
    
    if proc.returncode == 0:
        self.emit_event("verification.passed", task_id=task.id)
        return VerificationResult(status="pass", output=stdout.decode())
    else:
        self.emit_event("verification.failed", task_id=task.id, output=stderr.decode())
        return VerificationResult(status="fail", output=stderr.decode())
```

## How approvals pause/resume the run

```python
async def check_approval(task: Task, action: Action):
    decision = self.policy_engine.evaluate(action)
    
    if decision == "approve":
        return True
    elif decision == "deny":
        return False
    elif decision == "require_human_approval":
        # Create approval request
        request = ApprovalRequest(
            task_id=task.id,
            run_id=self.run.id,
            action_type=action.type,
            action_detail=action.detail,
            policy_rule=decision.rule,
        )
        self.state_store.save_approval(request)
        self.emit_event("approval.requested", approval=request)
        
        # Send to CLI and wait for response
        print(json.dumps({"type": "approval_request", "data": request.model_dump()}), flush=True)
        
        # Block until user responds or timeout
        response = await asyncio.wait_for(
            self.read_approval_response(request.id),
            timeout=request.timeout_seconds,
        )
        
        if response is None:
            # Timeout — apply default
            response = self.config.approval.default_on_timeout
        
        self.emit_event(f"approval.{response}", approval_id=request.id)
        return response == "approved"
```

## How crashes/restarts recover

```python
async def resume_run(run_id: str):
    # 1. Load run from SQLite
    run = self.state_store.get_run(run_id)
    
    # 2. Load events from JSONL
    events = self.event_log.read_all(run_id)
    
    # 3. Verify consistency: replay events against SQLite state
    last_event_seq = events[-1].sequence if events else 0
    # If SQLite is behind events, replay missing events
    
    # 4. Clean up orphaned worktrees
    await self.worktree_manager.cleanup_orphans(run)
    
    # 5. Reset any tasks stuck in transient states
    stuck_tasks = self.state_store.get_tasks(run_id, status=["running", "assigned", "verifying"])
    for task in stuck_tasks:
        # These were in-progress when we crashed — reset to previous stable state
        if task.retry_count < task.max_retries:
            self.state_store.update_task(task.id, status="planned", retry_count=task.retry_count + 1)
        else:
            self.state_store.update_task(task.id, status="failed", error="Crashed during execution, retries exhausted")
    
    # 6. Update run status
    self.state_store.update_run(run_id, status="running")
    self.emit_event("run.resumed", run_id=run_id, from_sequence=last_event_seq)
    
    # 7. Enter supervisor loop (will pick up planned/pending tasks)
    await self.supervisor_loop(run)
```

## How completed artifacts are published

Artifacts are files produced by agents and verification:

```python
async def publish_artifact(task: Task, artifact_type: str, content: bytes, name: str):
    # 1. Write to artifacts directory
    artifact_dir = f"~/.codex/multi-agent/runs/{task.run_id}/artifacts"
    os.makedirs(artifact_dir, exist_ok=True)
    path = f"{artifact_dir}/{name}"
    
    with open(path, "wb") as f:
        f.write(content)
    
    # 2. Record in state store
    record = ArtifactRecord(
        task_id=task.id,
        run_id=task.run_id,
        artifact_type=artifact_type,
        name=name,
        path=path,
        size_bytes=len(content),
    )
    self.state_store.save_artifact(record)
    
    # 3. Emit event
    self.emit_event("artifact.published", artifact=record)
```

The final artifact is the result branch itself:
```python
async def finalize_run(run: Run):
    # Result branch (codex/<run-id>/result) contains all merged task work
    run.result_branch = f"codex/{run.id}/result"
    run.status = "completed"
    self.state_store.update_run(run.id, status="completed", result_branch=run.result_branch)
    
    # Generate summary
    summary = await self.generate_summary(run)
    self.state_store.update_run(run.id, summary=summary)
    
    self.emit_event("run.completed", run_id=run.id, summary=summary)
```
