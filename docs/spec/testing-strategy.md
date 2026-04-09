# Testing Strategy

[< Spec Index](index.md) | [Product Index](../product/index.md)

## Test pyramid

```
         /  E2E  \           # 2-3 tests: full run with real providers
        / Integr. \          # 10-15 tests: component integration
       /   Unit    \         # 50+ tests: individual functions
```

## Unit tests

| Target | What's tested | Mocking strategy |
|--------|--------------|------------------|
| Task state machine | All valid transitions accepted, invalid rejected | No mocks needed — pure logic |
| Routing scorer | Correct scores for different task/provider combos | Mock ProviderCapability objects |
| Task scheduler | Dependency resolution, parallel dispatch order | Mock tasks with known deps |
| Policy engine | Pattern matching against action types and details | No mocks — pure logic with config |
| Event serialization | All event types serialize/deserialize correctly | No mocks — schema validation |
| Config loading | TOML parsing, defaults, validation | Temp files with test configs |
| Run summary generator | Summary format and content | Mock state store data |

**Test framework:** pytest with pytest-asyncio

## Integration tests

| Target | What's tested | Setup |
|--------|--------------|-------|
| State store | SQLite CRUD, concurrent access, migration | Temp SQLite DB per test |
| Event log | Write, read, replay, idempotency | Temp JSONL file per test |
| Worktree manager | Create, merge, cleanup, conflict detection | Temp git repo per test |
| Supervisor + scheduler | Full loop with mock provider | Mock adapter returning canned results |
| CLI → orchestrator IPC | JSON messaging round-trip | Subprocess with mock orchestrator |
| Routing + provider registry | Route task, health check, fallback | Mock HTTP server for coding-agent-router |

**Key integration test: supervisor loop with mock provider**
```python
async def test_supervisor_loop_3_tasks():
    # Setup: temp repo, SQLite, mock adapter that returns success after 2 turns
    mock_adapter = MockAdapter(result=WorkerResult(status="success", ...))
    
    run = Run(goal="test goal", repo_path=temp_repo)
    plan = [
        Task(id="t1", description="task 1", type="code", dependencies=[]),
        Task(id="t2", description="task 2", type="code", dependencies=[]),
        Task(id="t3", description="task 3", type="code", dependencies=["t1", "t2"]),
    ]
    
    supervisor = Supervisor(run, mock_adapter, ...)
    await supervisor.execute(plan)
    
    assert run.status == "completed"
    assert run.completed_tasks == 3
    # t1 and t2 dispatched in parallel, t3 after both complete
    assert events[2].type == "task.started"  # t1
    assert events[3].type == "task.started"  # t2 (parallel)
```

## Replay tests

Verify that the event log can reconstruct state:

```python
async def test_event_replay_matches_state():
    # Run a supervisor loop, collect events
    run, events = await run_with_mock_provider(...)
    
    # Wipe SQLite state
    state_store.wipe()
    
    # Replay events
    for event in events:
        state_store.apply_event(event)
    
    # Verify state matches
    reconstructed_run = state_store.get_run(run.id)
    assert reconstructed_run.status == run.status
    assert reconstructed_run.completed_tasks == run.completed_tasks
```

## Routing tests

```python
@pytest.mark.parametrize("task_type,complexity,expected_backend", [
    ("code", "low", "ollama"),           # Simple → local
    ("code", "high", "claude-code"),     # Complex → frontier
    ("review", "any", "claude-code"),    # Review → always frontier
    ("test", "any", "local-exec"),       # Test run → local exec
    ("plan", "any", "claude-code"),      # Planning → always frontier
])
async def test_routing_score(task_type, complexity, expected_backend):
    task = Task(type=task_type, estimated_complexity=complexity)
    decision = await routing_engine.route_task(task, providers, policy, budget=100)
    assert decision.selected_backend == expected_backend
```

## Provider adapter contract tests

Each adapter must pass the same test suite (see [Provider Adapter Interface](provider-adapter-interface.md)):

```python
class ProviderAdapterContractTests:
    """Every adapter must pass these tests."""
    
    async def test_execute_simple_task(self, adapter):
        task = Task(description="Create a file hello.txt with content 'hello'", type="code")
        result = await adapter.execute_task(task, context)
        assert result.status == "success"
        assert "hello.txt" in result.files_changed
    
    async def test_capabilities_returns_valid(self, adapter):
        cap = await adapter.capabilities()
        assert cap.context_window > 0
        assert 0 <= cap.strengths.get("code_generation", 0) <= 1.0
    
    async def test_health_returns_status(self, adapter):
        health = await adapter.health()
        assert health.status in ("healthy", "degraded", "unavailable")
    
    async def test_cancel_stops_execution(self, adapter):
        # Start a long-running task, cancel it, verify it stops
        task = Task(description="Count to a million slowly", type="code")
        dispatch = asyncio.create_task(adapter.execute_task(task, context))
        await asyncio.sleep(2)
        await adapter.cancel(task.id)
        result = await dispatch
        assert result.status in ("cancelled", "timeout")
    
    async def test_timeout_is_respected(self, adapter):
        task = Task(description="Sleep forever", type="code", timeout_seconds=5)
        result = await adapter.execute_task(task, context)
        assert result.status == "timeout"
```

## Repo isolation tests

```python
async def test_parallel_worktrees_no_interference():
    # Create two worktrees from same repo
    wt1 = await worktree_manager.create(run, task1)
    wt2 = await worktree_manager.create(run, task2)
    
    # Write different files in each
    write_file(f"{wt1.path}/file1.py", "content 1")
    write_file(f"{wt2.path}/file2.py", "content 2")
    
    # Commit in each
    git_commit(wt1.path, "task 1 changes")
    git_commit(wt2.path, "task 2 changes")
    
    # Merge both into result branch
    result1 = await worktree_manager.merge(task1)
    result2 = await worktree_manager.merge(task2)
    
    assert result1 == MergeResult.SUCCESS
    assert result2 == MergeResult.SUCCESS
    
    # Result branch has both files
    assert file_exists(result_branch, "file1.py")
    assert file_exists(result_branch, "file2.py")

async def test_conflict_detection():
    wt1 = await worktree_manager.create(run, task1)
    wt2 = await worktree_manager.create(run, task2)
    
    # Both modify same file
    write_file(f"{wt1.path}/shared.py", "version 1")
    write_file(f"{wt2.path}/shared.py", "version 2")
    
    git_commit(wt1.path, "task 1")
    git_commit(wt2.path, "task 2")
    
    result1 = await worktree_manager.merge(task1)
    result2 = await worktree_manager.merge(task2)
    
    assert result1 == MergeResult.SUCCESS
    assert result2 == MergeResult.CONFLICT
```

## Approval path tests

```python
async def test_risky_action_requires_approval():
    policy = load_policy("risky-only")
    action = Action(type="shell_exec", detail="npm install redis")
    
    decision = policy.evaluate(action)
    assert decision == "require_human_approval"

async def test_safe_action_auto_approved():
    policy = load_policy("risky-only")
    action = Action(type="shell_exec", detail="ls -la")
    
    decision = policy.evaluate(action)
    assert decision == "approve"
```

## Crash recovery tests

```python
async def test_resume_after_crash():
    # Start a run with 5 tasks
    run, orchestrator = setup_run(5_tasks)
    
    # Let 2 tasks complete
    await wait_for_n_completed(orchestrator, 2)
    
    # Simulate crash
    orchestrator.kill()
    
    # Resume
    new_orchestrator = Supervisor.resume(run.id)
    await new_orchestrator.execute()
    
    # Verify: 5 tasks completed, no duplicates
    assert run.completed_tasks == 5
    events = event_log.read_all(run.id)
    completed_events = [e for e in events if e.type == "task.completed"]
    assert len(completed_events) == 5
    assert len(set(e.task_id for e in completed_events)) == 5  # No duplicates
```

## Timeout/retry tests

```python
async def test_task_timeout_triggers_retry():
    adapter = SlowAdapter(delay=20)  # Takes 20s, timeout is 10s
    task = Task(timeout_seconds=10, max_retries=2)
    
    result = await supervisor.execute_task(task, adapter)
    
    assert task.retry_count == 1
    assert task.status in ("running", "completed")  # Retried

async def test_max_retries_exhausted():
    adapter = FailingAdapter()
    task = Task(max_retries=3)
    
    await supervisor.execute_task_with_retries(task, adapter)
    
    assert task.status == "failed"
    assert task.retry_count == 3
```

## Cost-control tests

```python
async def test_budget_limit_pauses_run():
    run = Run(budget_limit=1.0, budget_spent=0.95)
    adapter = MockAdapter(cost_per_task=0.10)
    
    # Next task would exceed budget
    await supervisor.loop_iteration()
    
    assert run.status == "paused"
    events = get_events(run.id, type="run.paused")
    assert "budget" in events[0].data["reason"].lower()
```
